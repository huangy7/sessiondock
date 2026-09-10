import { nextTick, ref, shallowRef } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionIdentity, SessionInfo } from "../types/session";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  ask: vi.fn(async () => true),
  message: vi.fn(async () => undefined),
  save: vi.fn(async () => "/tmp/export.txt"),
  listeners: new Map<string, (event: { payload?: any }) => unknown>(),
  deleteFailures: new Set<string>(),
  projectRefresh: vi.fn(async () => undefined),
  searchRefresh: vi.fn(async () => undefined),
  projectItems: undefined as ReturnType<typeof shallowRef<any[]>> | undefined,
  projectDone: undefined as ReturnType<typeof ref<any>> | undefined,
  searchDone: undefined as ReturnType<typeof ref<any>> | undefined,
}));

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen.mockImplementation(async (topic, callback) => {
    mocks.listeners.set(topic, callback);
    return () => {};
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: mocks.ask,
  message: mocks.message,
  open: vi.fn(),
  save: mocks.save,
}));
vi.mock("./useProjectFilter", () => ({
  useProjectFilter: () => ({ isBlocked: () => false }),
}));
vi.mock("./useStreamingCollection", () => ({
  useStreamingCollection: ({ command }: { command: string }) => {
    const items = shallowRef<any[]>([]);
    const done = ref(command === "scan_projects" ? { total_sessions: 0, cli_results: [] } : null);
    const stream = {
      items,
      done,
      error: ref(null),
      isRefreshing: ref(false),
      showLoadingIndicator: ref(false),
      totalItems: ref(0),
      refresh: command === "scan_projects" ? mocks.projectRefresh : mocks.searchRefresh,
      cancel: vi.fn(),
    };
    if (command === "scan_projects") {
      mocks.projectItems = items;
      mocks.projectDone = done;
    }
    if (command === "search_sessions") {
      mocks.searchDone = done;
    }
    return stream;
  },
}));

const { useSessions } = await import("./useSessions");
const store = useSessions();

function session(cliId: "claude" | "codex", filePath = "/same/session.jsonl"): SessionInfo {
  return {
    session_id: `${cliId}-session`,
    file_path: filePath,
    display_name: cliId,
    timestamp: "2026-08-27T08:00:00Z",
    file_size: 1,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

const activeCodexIdentity: SessionIdentity = {
  cliId: "codex",
  filePath: "/same/session.jsonl",
};

function activateCodexSession() {
  const identityStore = store as typeof store & {
    activeSessionIdentity?: ReturnType<typeof ref<SessionIdentity | null>>;
    setActiveSession?: (identity: SessionIdentity, projectDir: string) => void;
  };
  expect(identityStore.setActiveSession).toBeTypeOf("function");
  expect(identityStore.activeSessionIdentity).toBeDefined();
  identityStore.setActiveSession!(activeCodexIdentity, "codex");
  store.autoFollow.value = true;
  return identityStore;
}

beforeEach(() => {
  vi.useRealTimers();
  mocks.invoke.mockReset();
  mocks.ask.mockClear();
  mocks.message.mockClear();
  mocks.save.mockClear();
  mocks.projectRefresh.mockClear();
  mocks.searchRefresh.mockClear();
  mocks.deleteFailures.clear();
  mocks.projectItems!.value = [
    { encoded_dir: "claude", original_path: "/workspace", session: session("claude") },
    { encoded_dir: "codex", original_path: "/workspace", session: session("codex") },
  ];
  mocks.projectDone!.value = {
    total_sessions: 2,
    cli_results: [
      { cli_id: "claude", total_sessions: 1, error: null },
      { cli_id: "codex", total_sessions: 1, error: null },
    ],
  };
  store.cliStatuses.value = {
    claude: { hasSessions: true, hasBinary: true },
    codex: { hasSessions: true, hasBinary: true },
    gemini: { hasSessions: false, hasBinary: false },
    workbuddy: { hasSessions: false, hasBinary: false },
    dsh: { hasSessions: false, hasBinary: false },
    antigravity: { hasSessions: false, hasBinary: false },
  };
  store.cliFilter.value = { mode: "custom", cliIds: ["claude"] };
  store.selectedSessionIdentities.value = [];
  store.clearActiveSession();
  store.searchQuery.value = "";

  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
    if (command === "list_cli_statuses") {
      return [
        { id: "claude", has_sessions: true, has_binary: true },
        { id: "codex", has_sessions: true, has_binary: true },
      ];
    }
    if (command === "list_cli_path_configs") return [];
    if (command === "delete_sessions_to_trash") {
      if (mocks.deleteFailures.has(args?.cliId)) throw new Error(`${args?.cliId} failed`);
      return { failed: [], succeeded: args?.paths.length ?? 0 };
    }
    if (command === "batch_export_sessions") return "exported";
    if (command === "fork_session") return "forked-session";
    if (command === "detect_cli") return true;
    return undefined;
  });
});

describe("useSessions store wiring", () => {
  it("passes visible CLI IDs to project refresh and ignores hidden CLI update events", async () => {
    await store.refresh("snapshot");
    expect(mocks.projectRefresh).toHaveBeenLastCalledWith(
      { cliIds: ["claude"] },
      { silent: true },
    );

    mocks.projectRefresh.mockClear();
    await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "codex" } });
    expect(mocks.projectRefresh).not.toHaveBeenCalled();

    await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "claude" } });
    expect(mocks.projectRefresh).toHaveBeenCalledTimes(1);
  });

  it("preserves counts for hidden CLI sources while refreshing the visible tree", async () => {
    store.cliSessionCounts.value = { claude: 3, codex: 8 };
    mocks.projectDone!.value = {
      total_sessions: 2,
      cli_results: [
        { cli_id: "claude", total_sessions: 2, error: null },
      ],
    };

    await store.refresh("snapshot");

    expect(store.cliSessionCounts.value).toEqual({ claude: 2, codex: 8 });
  });

  it("does not issue a supplemental title search outside the visible project data", async () => {
    vi.useFakeTimers();
    store.searchQuery.value = "hidden";
    await nextTick();
    await vi.advanceTimersByTimeAsync(350);

    expect(mocks.invoke.mock.calls.some(([command]) => command === "search_cross_cli_titles")).toBe(false);
  });

  it("uses the explicit session identity for deletion when paths collide", async () => {
    await store.deleteSession(
      { cliId: "codex", filePath: "/same/session.jsonl" },
      "Codex session",
    );

    expect(mocks.invoke).toHaveBeenCalledWith("delete_sessions_to_trash", {
      cliId: "codex",
      paths: ["/same/session.jsonl"],
    });
  });

  it("keeps the active Codex session when deleting Claude at the same path", async () => {
    const identityStore = activateCodexSession();

    await store.deleteSession(
      { cliId: "claude", filePath: "/same/session.jsonl" },
      "Claude session",
    );

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("keeps the active Codex session when deleting a Claude-only project with the same path", async () => {
    const identityStore = activateCodexSession();
    const claudeProject = {
      encoded_dir: "claude",
      original_path: "/workspace",
      sessions: [session("claude")],
    };

    await store.deleteProject(claudeProject);

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("keeps the active Codex session when batch-deleting Claude at the same path", async () => {
    const identityStore = activateCodexSession();
    store.selectedSessionIdentities.value = [
      { cliId: "claude", filePath: "/same/session.jsonl" },
    ];

    await store.batchDeleteSessions();

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("does not guess a CLI for an ambiguous legacy path-only selection", () => {
    store.currentCliId.value = "claude";

    store.toggleSessionSelect("/same/session.jsonl");

    expect(store.selectedSessionIdentities.value).toEqual([]);
  });

  it("uses explicit CLI context for restore, resume, and fork operations", async () => {
    mocks.projectItems!.value = [
      {
        encoded_dir: "codex",
        original_path: "/workspace",
        session: { ...session("codex"), is_archived: true },
      },
    ];

    await store.ensureSessionReadyOnDisk("/same/session.jsonl", undefined, "codex");
    await store.resumeSession("/workspace", "codex-session", "codex");
    await store.forkSession(
      { cliId: "codex", filePath: "/same/session.jsonl" },
      "anchor",
    );

    expect(mocks.invoke).toHaveBeenCalledWith("restore_session_to_disk", {
      cliId: "codex",
      filePath: "/same/session.jsonl",
    });
    expect(mocks.invoke).toHaveBeenCalledWith("open_in_terminal", expect.objectContaining({
      cliId: "codex",
      sessionId: "codex-session",
    }));
    expect(mocks.invoke).toHaveBeenCalledWith("fork_session", {
      cliId: "codex",
      filePath: "/same/session.jsonl",
      anchorUuid: "anchor",
    });
  });

  it("preserves composite identities in the batch export request", async () => {
    const identities: SessionIdentity[] = [
      { cliId: "claude", filePath: "/same/session.jsonl" },
      { cliId: "codex", filePath: "/same/session.jsonl" },
    ];
    store.selectedSessionIdentities.value = identities;

    await store.batchExportSessions("merged", "json");

    expect(mocks.invoke).toHaveBeenCalledWith("batch_export_sessions", {
      sessions: identities,
      savePath: "/tmp/export.txt",
      format: "json",
      mode: "merged",
    });
  });

  it("continues after one CLI deletion rejects, refreshes, and retains only failed identities", async () => {
    const identities: SessionIdentity[] = [
      { cliId: "claude", filePath: "/claude/a.jsonl" },
      { cliId: "codex", filePath: "/codex/b.jsonl" },
      { cliId: "gemini", filePath: "/gemini/c.jsonl" },
    ];
    store.selectedSessionIdentities.value = identities;
    mocks.deleteFailures.add("codex");

    const allSucceeded = await store.batchDeleteSessions();

    const deleteCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "delete_sessions_to_trash",
    );
    expect(deleteCalls.map(([, args]) => args.cliId)).toEqual(["claude", "codex", "gemini"]);
    expect(mocks.projectRefresh).toHaveBeenCalled();
    expect(store.selectedSessionIdentities.value).toEqual([identities[1]]);
    expect(allSucceeded).toBe(false);
  });

  it("continues deleting the other project sources and refreshes after a partial failure", async () => {
    store.cliFilter.value = { mode: "all" };
    mocks.deleteFailures.add("claude");

    const allSucceeded = await store.deleteProject(store.projects.value[0]);

    const deleteCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "delete_sessions_to_trash",
    );
    expect(deleteCalls.map(([, args]) => args.cliId)).toEqual(["claude", "codex"]);
    expect(mocks.projectRefresh).toHaveBeenCalled();
    expect(allSucceeded).toBe(false);
  });

  it("exposes per-CLI scan errors and clears them once that CLI scans successfully", async () => {
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 0, error: "读取数据目录失败" },
      ],
    };

    await store.refresh("snapshot");
    expect(store.scanCliErrors.value).toEqual({ claude: "读取数据目录失败" });

    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 1, error: null },
      ],
    };

    await store.refresh("snapshot");
    expect(store.scanCliErrors.value).toEqual({});
  });

  it("keeps scan errors of CLIs absent from the latest scan results", async () => {
    store.scanCliErrors.value = { codex: "旧错误" };
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 1, error: null },
      ],
    };

    await store.refresh("snapshot");

    expect(store.scanCliErrors.value).toEqual({ codex: "旧错误" });
  });

  it("retryCliScan forces an index rebuild for the CLI and refreshes", async () => {
    await store.retryCliScan("claude");

    expect(mocks.invoke).toHaveBeenCalledWith("refresh_session_list_index", {
      cliId: "claude",
      notify: true,
      force: true,
    });
    expect(mocks.projectRefresh).toHaveBeenCalled();
  });

  it("surfaces per-CLI search errors from the search done payload and clears them on the next clean run", async () => {
    mocks.searchDone!.value = {
      total: 0,
      query: "q",
      pending_cli_ids: [],
      stale_cli_ids: [],
      cli_errors: [{ cli_id: "codex", message: "索引损坏" }],
    };
    await nextTick();
    expect(store.searchCliErrors.value).toEqual({ codex: "索引损坏" });

    mocks.searchDone!.value = {
      total: 1,
      query: "q",
      pending_cli_ids: [],
      stale_cli_ids: [],
      cli_errors: [],
    };
    await nextTick();
    expect(store.searchCliErrors.value).toEqual({});
  });
});
