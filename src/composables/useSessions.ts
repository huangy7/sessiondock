import { ref, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import { useProjectFilter } from "./useProjectFilter";
import { useTerminalApp } from "./useTerminalApp";
import { batchDeleteProgressText, type BatchDeleteProgress } from "./batchDeleteProgress";
import { useStreamingCollection } from "./useStreamingCollection";
import { sessionMatchesQuery } from "../utils/sessionFilter";
import type {
  AggregatedProjectInfo,
  ProjectInfo,
  SessionIdentity,
  SessionInfo,
  SearchResult,
} from "../types/session";
import { sessionIdentityKey } from "../types/session";
import {
  resolveVisibleCliIds,
  type CliFilterState,
} from "./cliFilter";
import {
  SUPPORTED_CLIS,
  getCliDefinition,
  isCliId,
  resolveCliDefinition,
  type CliId,
  type CliOption,
  type CliPathConfig,
  type CliRuntime,
  type CliStatus,
  type ResolvedCliDefinition,
} from "../types/cli";

export interface ProjectSessionChunkItem {
  session: SessionInfo;
  encoded_dir: string;
  original_path: string;
}

interface ScanProjectsStreamDone {
  total_sessions: number;
  cli_results: Array<{
    cli_id: string;
    total_sessions: number;
    error: string | null;
  }>;
}

export function normalizeProjectPath(path: string): string {
  const trimmed = path.trim();
  const windowsPath = /^[a-zA-Z]:[\\/]/.test(trimmed) || /^[\\/]{2}[^\\/]/.test(trimmed);
  const uncPath = /^[\\/]{2}[^\\/]/.test(trimmed);
  let normalized = trimmed.replace(/\\/g, "/");
  normalized = uncPath
    ? `//${normalized.replace(/^\/+/, "").replace(/\/{2,}/g, "/")}`
    : normalized.replace(/\/{2,}/g, "/");

  const isPosixRoot = normalized === "/";
  const isDriveRoot = /^[a-zA-Z]:\/$/.test(normalized);
  if (!isPosixRoot && !isDriveRoot) normalized = normalized.replace(/\/+$/, "");
  return windowsPath ? normalized.toLocaleLowerCase("en-US") : normalized;
}

export function aggregateProjectItems(
  items: ProjectSessionChunkItem[],
): AggregatedProjectInfo[] {
  const grouped = new Map<string, AggregatedProjectInfo>();

  for (const item of items) {
    const projectKey = normalizeProjectPath(item.original_path);
    const cliId = isCliId(item.session.cli_id) ? item.session.cli_id : null;
    const existing = grouped.get(projectKey);

    if (existing) {
      existing.sessions.push(item.session);
      if (cliId && !existing.cli_ids.includes(cliId)) existing.cli_ids.push(cliId);
      continue;
    }

    grouped.set(projectKey, {
      project_key: projectKey,
      cli_ids: cliId ? [cliId] : [],
      encoded_dir: item.encoded_dir,
      original_path: item.original_path,
      sessions: [item.session],
    });
  }

  return [...grouped.values()].map((project) => ({
    ...project,
    sessions: [...project.sessions].sort((a, b) => {
      const aTime = Date.parse(a.timestamp);
      const bTime = Date.parse(b.timestamp);
      if (Number.isNaN(aTime) && Number.isNaN(bTime)) return 0;
      if (Number.isNaN(aTime)) return 1;
      if (Number.isNaN(bTime)) return -1;
      return bTime - aTime;
    }),
  }));
}

export function shouldRefreshForCli(
  visibleCliIds: CliId[],
  eventCliId?: string,
): boolean {
  return !eventCliId || (isCliId(eventCliId) && visibleCliIds.includes(eventCliId));
}

export function groupSessionIdentities(
  items: SessionIdentity[],
): Map<CliId, string[]> {
  const groups = new Map<CliId, string[]>();
  for (const item of items) {
    const paths = groups.get(item.cliId);
    if (paths) paths.push(item.filePath);
    else groups.set(item.cliId, [item.filePath]);
  }
  return groups;
}

export function resolveLaunchCliId(
  visibleIds: CliId[],
  options: CliOption[],
  lastSuccessful?: CliId,
): CliId | undefined {
  const installedCreatable = options.filter(
    (option) => option.hasBinary && option.supportsNewSession,
  );
  const visibleCreatable = installedCreatable.filter((option) =>
    visibleIds.includes(option.id),
  );

  if (visibleCreatable.length === 1) return visibleCreatable[0].id;
  if (lastSuccessful && installedCreatable.some((option) => option.id === lastSuccessful)) {
    return lastSuccessful;
  }
  return installedCreatable[0]?.id;
}

// 复用 P0/P1 引入的流式加载 + 列表通用编排
const projectsStream = useStreamingCollection<ProjectSessionChunkItem, ScanProjectsStreamDone>({
  command: 'scan_projects',
  topic: 'scan_projects',
});

// projects 从 stream.items 按规范化工作区路径跨 CLI 聚合派生
const projects = computed<AggregatedProjectInfo[]>(() =>
  aggregateProjectItems(projectsStream.items.value)
);
const searchQuery = ref("");
const selectedSessionPath = ref<string | null>(null);
const selectedProjectDir = ref<string | null>(null);
const activeSessionIdentity = ref<SessionIdentity | null>(null);
const skipPermissions = ref(
  localStorage.getItem("claudia-skip-permissions") !== "false"
);
const { terminalAppForLaunch } = useTerminalApp();
const currentCliId = ref<CliId>((() => {
  const saved = localStorage.getItem("claudia-current-cli");
  return saved && isCliId(saved) ? saved : "claude";
})());
const cliFilter = ref<CliFilterState>((() => {
  const saved = localStorage.getItem("claudia-cli-filter");
  if (!saved) return { mode: "all" };
  try {
    const parsed = JSON.parse(saved) as Partial<CliFilterState>;
    if (parsed.mode === "all") return { mode: "all" };
    if (parsed.mode === "custom" && Array.isArray(parsed.cliIds)) {
      return {
        mode: "custom",
        cliIds: parsed.cliIds.filter(
          (cliId): cliId is CliId => typeof cliId === "string" && isCliId(cliId),
        ),
      };
    }
  } catch {
    // Invalid persisted state falls back to the safe default below.
  }
  return { mode: "all" };
})());
const persistedLaunchCliId = (() => {
  const saved = localStorage.getItem("claudia-launch-cli");
  return saved && isCliId(saved) ? saved : undefined;
})();
const lastSuccessfulLaunchCliId = ref<CliId | undefined>(persistedLaunchCliId);
const cliStatuses = ref<Record<CliId, CliRuntime>>({
  claude: { hasSessions: false, hasBinary: false },
  codex: { hasSessions: false, hasBinary: false },
  gemini: { hasSessions: false, hasBinary: false },
  workbuddy: { hasSessions: false, hasBinary: false },
  dsh: { hasSessions: false, hasBinary: false },
  antigravity: { hasSessions: false, hasBinary: false },
});
const cliSessionCounts = ref<Partial<Record<CliId, number>>>({});
const cliPathConfigs = ref<Record<CliId, CliPathConfig | null>>({
  claude: null,
  codex: null,
  gemini: null,
  workbuddy: null,
  dsh: null,
  antigravity: null,
});

const currentCli = computed<ResolvedCliDefinition>(() =>
  resolveCliDefinition(currentCliId.value, cliPathConfigs.value[currentCliId.value])
);
const cliOptions = computed<CliOption[]>(() =>
  SUPPORTED_CLIS.map((cli) => ({
    ...cli,
    hasSessions: cliStatuses.value[cli.id]?.hasSessions ?? false,
    hasBinary: cliStatuses.value[cli.id]?.hasBinary ?? false,
  }))
);
const installedCliOptions = computed<CliOption[]>(() =>
  cliOptions.value.filter((cli) => cli.hasSessions)
);
const availableCliIds = computed<CliId[]>(() =>
  installedCliOptions.value.map((cli) => cli.id)
);
const visibleCliIds = computed<CliId[]>(() =>
  resolveVisibleCliIds(cliFilter.value, availableCliIds.value)
);
const launchCliId = computed<CliId>(() =>
  resolveLaunchCliId(
    visibleCliIds.value,
    cliOptions.value,
    lastSuccessfulLaunchCliId.value,
  ) ?? lastSuccessfulLaunchCliId.value ?? currentCliId.value
);
// 二进制门布尔映射：新建会话等需要可执行文件的动作按此 gate
const cliBinaryStatuses = computed<Record<string, boolean>>(() =>
  Object.fromEntries(
    (Object.entries(cliStatuses.value) as [CliId, CliRuntime][]).map(
      ([id, runtime]) => [id, runtime?.hasBinary ?? false]
    )
  )
);

async function syncCurrentCliWithInstalled() {
  if (cliStatuses.value[currentCliId.value]?.hasSessions) return;

  const fallbackCli = installedCliOptions.value[0];
  if (!fallbackCli) return;

  currentCliId.value = fallbackCli.id;
  localStorage.setItem("claudia-current-cli", fallbackCli.id);
  await invoke("refresh_tray_menu", { cliId: fallbackCli.id }).catch((error) => {
    console.error("refresh_tray_menu failed:", error);
  });
}

async function setCliFilter(next: CliFilterState) {
  cliFilter.value = next;
  localStorage.setItem("claudia-cli-filter", JSON.stringify(next));
  clearSessionSelection();
  await refresh();
  if (globalSearchQuery.value.trim()) {
    await globalSearch(globalSearchQuery.value);
  }
}

// Live session watching
const autoFollow = ref(false);

// Sort mode: 'time' (default, most recent first) or 'name' (alphabetical)
type SortMode = "time" | "name";
const sortMode = ref<SortMode>(
  (localStorage.getItem("claudia-sort-mode") as SortMode) || "time"
);

function setSortMode(mode: SortMode) {
  sortMode.value = mode;
  localStorage.setItem("claudia-sort-mode", mode);
}

// Global search state
const globalSearchQuery = ref("");
const searchIndexProgress = ref<SearchIndexProgressPayload | null>(null);
const searchIndexBuildRequest = ref<SearchIndexBuildRequest | null>(null);
const searchIndexBuildSkipped = ref(false);
const dbMigrationProgress = ref<{ step: string; current: number; total: number } | null>(null);
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

// P3: search_sessions 流式化。复用 useStreamingCollection 的抢占语义替代手写 requestSeq。
// 模块级使用：见 useStreamingCollection JSDoc"生命周期使用规范"——单例 store 场景安全。
interface SearchSessionsStreamDone {
  total: number;
  query: string;
  // 后端 done payload 无 serde 重命名，线上为 snake_case
  pending_cli_ids: string[];
  // 已建索引但有增量未同步的 CLI(本次仍搜索了其已有内容)
  stale_cli_ids: string[];
  // 单来源搜索失败的分项错误(其余来源结果仍正常返回)
  cli_errors?: Array<{ cli_id: string; message: string }>;
}
const searchStream = useStreamingCollection<SearchResult, SearchSessionsStreamDone>({
  command: 'search_sessions',
  topic: 'search_sessions',
  slowThresholdMs: 0,  // 用户主动搜索：loading 即时显示，无 300ms 沉默期
});
const globalSearchResults = searchStream.items;
const globalSearchLoading = searchStream.isRefreshing;

// all 模式下 done 携带尚未建索引的 CLI 列表，供结果区底部按需构建引导
const searchPendingCliIds = ref<string[]>([]);
// all 模式下已建索引但有增量未同步的 CLI(结果可能缺少最新内容)
const searchStaleCliIds = ref<string[]>([]);
// 扫描/搜索的分项失败：仅记录本次结果中出现的 CLI，成功即清除其错误项
const scanCliErrors = ref<Partial<Record<CliId, string>>>({});
const searchCliErrors = ref<Partial<Record<CliId, string>>>({});
// 按需构建在飞的 CLI：进度监听放行非当前 CLI 的事件 + 按钮 loading/防重入
const buildingCliIds = ref<string[]>([]);
watch(() => searchStream.done.value, (done) => {
  searchPendingCliIds.value = done?.pending_cli_ids ?? [];
  searchStaleCliIds.value = done?.stale_cli_ids ?? [];
  // 搜索范围恒为 visibleCliIds 全集，done 的 cli_errors 即当前全部失败项，整体替换
  const errors: Partial<Record<CliId, string>> = {};
  for (const entry of done?.cli_errors ?? []) {
    if (isCliId(entry.cli_id)) errors[entry.cli_id] = entry.message;
  }
  searchCliErrors.value = errors;
});

// Pagination state
type StartupPhase = "detecting" | "synchronizing" | "scanning" | "ready";
type RefreshMode = "bootstrap" | "foreground" | "snapshot";
type SessionListIndexUpdatedPayload = {
  cliId?: string;
};
type SearchIndexProgressPayload = {
  cliId: string;
  phase?: string;
  current: number;
  total: number;
  startedAtMs?: number;
  processedBytes?: number;
  totalBytes?: number;
  currentPath?: string | null;
  currentFileBytes?: number;
  currentFileSize?: number;
};
type SearchIndexStatusPayload = {
  ready: boolean;
  requiresConfirmation: boolean;
  missingSessions: number;
  totalSessions: number;
  missingBytes: number;
  totalBytes: number;
  physicalIndexReady: boolean;
};
type SearchIndexBuildRequest = SearchIndexStatusPayload & {
  cliId: string;
  query: string;
};
type DbMigrationProgressPayload = {
  step: string;
  current: number;
  total: number;
};

const STARTUP_SLOW_MS = 2600;
let sessionIndexListenerPromise: Promise<UnlistenFn> | null = null;
let searchIndexListenerPromise: Promise<UnlistenFn> | null = null;
let dbMigrationListenerPromise: Promise<UnlistenFn> | null = null;

function ensureSessionIndexListener() {
  if (sessionIndexListenerPromise) return;

  sessionIndexListenerPromise = listen<SessionListIndexUpdatedPayload>(
    "session-list-index-updated",
    async (event) => {
      const cliId = event.payload?.cliId;
      if (!shouldRefreshForCli(visibleCliIds.value, cliId)) {
        return;
      }
      try {
        await refresh("snapshot");
        if (globalSearchQuery.value.trim()) {
          await globalSearch(globalSearchQuery.value);
        }
      } catch (e) {
        console.error("session-list-index-updated refresh failed:", e);
      }
    }
  );
}

function ensureSearchIndexListener() {
  if (searchIndexListenerPromise) return;

  searchIndexListenerPromise = listen<SearchIndexProgressPayload>(
    "search-index-progress",
    (event) => {
      const { cliId, current, total, phase } = event.payload;
      if (!visibleCliIds.value.includes(cliId as CliId) && !buildingCliIds.value.includes(cliId)) return;
      
      if (phase === "done" || (!phase && current >= total)) {
        searchIndexProgress.value = null;
      } else if (phase === "error") {
        // Terminal error frame: clear progress like "done" so the UI is not
        // stuck showing progress and the search debounce is unblocked.
        console.error("search-index-progress error:", event.payload);
        searchIndexProgress.value = null;
      } else {
        searchIndexBuildRequest.value = null;
        searchIndexBuildSkipped.value = false;
        searchIndexProgress.value = {
          ...event.payload,
          startedAtMs: searchIndexProgress.value?.startedAtMs ?? Date.now(),
        };
      }
    }
  );
}

const deleteProgress = ref<BatchDeleteProgress | null>(null);

let batchDeleteProgressListenerPromise: Promise<void> | null = null;
function ensureBatchDeleteProgressListener(): Promise<void> {
  if (batchDeleteProgressListenerPromise) return batchDeleteProgressListenerPromise;
  batchDeleteProgressListenerPromise = listen<BatchDeleteProgress>(
    "batch-delete-progress",
    (event) => {
      if (deleteProgress.value != null) deleteProgress.value = event.payload;
    }
  ).then(() => {});
  return batchDeleteProgressListenerPromise;
}

function ensureDbMigrationListener() {
  if (dbMigrationListenerPromise) return;

  dbMigrationListenerPromise = listen<DbMigrationProgressPayload>(
    "db-migration-progress",
    (event) => {
      const { step, current, total } = event.payload;
      if (step === "done" || current >= total) {
        dbMigrationProgress.value = null;
      } else {
        dbMigrationProgress.value = { step, current, total };
      }
    }
  );
}
const totalSessionCount = ref(0);
// 暴露给 UI 的入口（保留旧名字以减少调用点扰动）
const isRefreshing = projectsStream.isRefreshing;
const showLoadingIndicator = projectsStream.showLoadingIndicator;
const totalLoadedSessions = projectsStream.totalItems;
// isStreamingProjects 历史调用方语义，现等价 isRefreshing
const isStreamingProjects = isRefreshing;
const bootstrapping = ref(true);
const startupPhase = ref<StartupPhase>("detecting");
const startupSlow = ref(false);

const { isBlocked } = useProjectFilter();

const filteredProjects = computed(() => {
  const q = searchQuery.value.toLowerCase().trim();
  let result = projects.value;

  // Filter out blocked projects
  result = result.filter((p) => !isBlocked(p.original_path));

  if (q) {
    result = result
      .map((p) => {
        const projectMatches = p.original_path.toLowerCase().includes(q);
        return {
          ...p,
          sessions: projectMatches
            ? p.sessions
            : p.sessions.filter((s) => sessionMatchesQuery(s, q)),
        };
      })
      .filter((p) => p.sessions.length > 0);
  }

  if (sortMode.value === "name") {
    result = [...result].sort((a, b) =>
      a.original_path.localeCompare(b.original_path)
    );
  } else {
    result = [...result].sort((a, b) => {
      const aTime = Date.parse(a.sessions[0]?.timestamp ?? "");
      const bTime = Date.parse(b.sessions[0]?.timestamp ?? "");
      return (Number.isNaN(bTime) ? 0 : bTime) - (Number.isNaN(aTime) ? 0 : aTime);
    });
  }

  return result;
});

async function loadCliStatuses() {
  try {
    const statuses = await invoke<CliStatus[]>("list_cli_statuses");
    const next: Record<CliId, CliRuntime> = {
      claude: { hasSessions: false, hasBinary: false },
      codex: { hasSessions: false, hasBinary: false },
      gemini: { hasSessions: false, hasBinary: false },
      workbuddy: { hasSessions: false, hasBinary: false },
      dsh: { hasSessions: false, hasBinary: false },
      antigravity: { hasSessions: false, hasBinary: false },
    };
    for (const status of statuses) {
      if (isCliId(status.id)) {
        next[status.id] = {
          hasSessions: status.has_sessions,
          hasBinary: status.has_binary,
        };
      }
    }
    cliStatuses.value = next;
  } catch (e) {
    console.error("list_cli_statuses failed:", e);
  }
}

async function loadCliPathConfigs() {
  try {
    const configs = await invoke<CliPathConfig[]>("list_cli_path_configs");
    const next: Record<CliId, CliPathConfig | null> = {
      claude: null,
      codex: null,
      gemini: null,
      workbuddy: null,
      dsh: null,
      antigravity: null,
    };
    for (const config of configs) {
      if (isCliId(config.id)) {
        next[config.id] = config;
      }
    }
    cliPathConfigs.value = next;
  } catch (e) {
    console.error("list_cli_path_configs failed:", e);
  }
}

function clearActiveSession() {
  autoFollow.value = false;
  activeSessionIdentity.value = null;
  selectedSessionPath.value = null;
  selectedProjectDir.value = null;
}

function setActiveSession(identity: SessionIdentity, projectDir: string) {
  activeSessionIdentity.value = identity;
  selectedSessionPath.value = identity.filePath;
  selectedProjectDir.value = projectDir;
}

async function setCurrentCli(cliId: CliId) {
  if (!cliStatuses.value[cliId]?.hasSessions) return;
  if (currentCliId.value === cliId) return;
  currentCliId.value = cliId;
  localStorage.setItem("claudia-current-cli", cliId);
  invoke("refresh_tray_menu", { cliId }).catch((error) => {
    console.error("refresh_tray_menu failed:", error);
  });
  clearActiveSession();
  selectedSessionIdentities.value = [];
  globalSearchQuery.value = '';
  void searchStream.refresh({ cliIds: visibleCliIds.value, query: '' });
  await refresh();
}

async function ensureCliInstalled(cliId: CliId): Promise<boolean> {
  try {
    const installed = await invoke<boolean>("detect_cli", {
      cliId,
    });
    cliStatuses.value = {
      ...cliStatuses.value,
      [cliId]: {
        ...cliStatuses.value[cliId],
        hasBinary: installed,
      },
    };
    if (installed) return true;
  } catch (e) {
    console.error("detect_cli failed:", e);
  }

  await ask(getCliDefinition(cliId).installHint, { title: "提示", kind: "info" });
  return false;
}

async function ensureCurrentCliInstalled(): Promise<boolean> {
  return ensureCliInstalled(currentCliId.value);
}

function recordSuccessfulLaunch(cliId: CliId) {
  lastSuccessfulLaunchCliId.value = cliId;
  localStorage.setItem("claudia-launch-cli", cliId);
}

async function refresh(
  mode: RefreshMode = bootstrapping.value ? "bootstrap" : "foreground"
) {
  ensureSessionIndexListener();
  ensureSearchIndexListener();
  ensureDbMigrationListener();
  await refreshInner(mode);
}

async function refreshInner(mode: RefreshMode) {
  const isInitialRefresh = bootstrapping.value;
  let slowTimer: ReturnType<typeof setTimeout> | null = null;

  if (isInitialRefresh) {
    startupPhase.value = "detecting";
    startupSlow.value = false;
    slowTimer = setTimeout(() => {
      startupSlow.value = true;
    }, STARTUP_SLOW_MS);
  }

  // Safety net: clear bootstrapping after 3s even if refresh isn't done
  let maxWait: ReturnType<typeof setTimeout> | null = null;
  const finishBootstrapping = () => {
    if (maxWait) clearTimeout(maxWait);
    if (bootstrapping.value) {
      bootstrapping.value = false;
      startupSlow.value = false;
      if (slowTimer) clearTimeout(slowTimer);
    }
  };

  if (isInitialRefresh) {
    maxWait = setTimeout(finishBootstrapping, 3000);
  }

  const cliTasks = (async () => {
    await Promise.all([
      loadCliStatuses(),
      loadCliPathConfigs(),
    ]);
    if (isInitialRefresh) {
      startupPhase.value = "synchronizing";
    }
    await syncCurrentCliWithInstalled();
  })();

  const scanTask = (async () => {
    if (mode === "foreground") {
      // 后台触发索引刷新，不阻塞流式 start
      refreshVisibleSessionIndexes();
    }

    if (isInitialRefresh) {
      await cliTasks; // Ensure CLI is synced before scanning if initial
      startupPhase.value = "scanning";
    }

    const previousActiveIdentity = activeSessionIdentity.value;

    // composable 内部已处理：原子替换 / 300ms 阈值 / done with 0 真清空 / error 隐式回滚
    // snapshot（文件监听定时刷新）静默执行：不清空列表、不显示 loading pill，避免周期性闪屏
    await projectsStream.refresh(
      { cliIds: visibleCliIds.value },
      { silent: mode === "snapshot" }
    );

    // done 时同步全量计数（done payload 才有总数）
    if (projectsStream.done.value) {
      totalSessionCount.value = projectsStream.done.value.total_sessions;
      const nextScanErrors = { ...scanCliErrors.value };
      for (const result of projectsStream.done.value.cli_results ?? []) {
        if (!isCliId(result.cli_id)) continue;
        if (result.error) {
          // 分项失败：记录错误，保留该 CLI 旧计数
          nextScanErrors[result.cli_id] = result.error;
          continue;
        }
        delete nextScanErrors[result.cli_id];
        cliSessionCounts.value = {
          ...cliSessionCounts.value,
          [result.cli_id]: result.total_sessions,
        };
      }
      scanCliErrors.value = nextScanErrors;
    }

    if (previousActiveIdentity) {
      const found = projects.value.some((p) =>
        p.sessions.some(
          (session) => session.file_path === previousActiveIdentity.filePath
            && session.cli_id === previousActiveIdentity.cliId,
        )
      );
      if (!found) {
        clearActiveSession();
      }
    }

    if (isInitialRefresh) {
      startupPhase.value = "ready";
    }
  })();

  if (mode === "bootstrap") {
    void cliTasks.then(refreshVisibleSessionIndexes);
  }

  // Await everything; bootstrapping clears naturally when data is ready,
  // or after a 3s safety net (set above).
  await Promise.all([cliTasks, scanTask]);
  finishBootstrapping();
}

function refreshVisibleSessionIndexes() {
  for (const cliId of visibleCliIds.value) {
    void invoke<boolean>("refresh_session_list_index", {
      cliId,
      notify: true,
    }).catch((e) => {
      console.error(`background refresh_session_list_index failed (${cliId}):`, e);
    });
  }
}

// 单个 CLI 扫描失败后的手动重试：强制重建其列表索引并重新扫描
async function retryCliScan(cliId: CliId) {
  try {
    await invoke<boolean>("refresh_session_list_index", {
      cliId,
      notify: true,
      force: true,
    });
  } catch (e) {
    console.error(`retryCliScan refresh_session_list_index failed (${cliId}):`, e);
  }
  await refresh();
}

function findSessionProjectDir(filePath: string): string | null {
  for (const project of projects.value) {
    if (project.sessions.some((session) => session.file_path === filePath)) {
      return project.encoded_dir;
    }
  }
  return null;
}

async function ensureSessionVisible(filePath: string): Promise<string | null> {
  // 1. 快路径：已在 projects 里
  let encodedDir = findSessionProjectDir(filePath);
  if (encodedDir) return encodedDir;

  // 2. 流式加载中 → 等当前 stream 跑完再重查
  if (projectsStream.isRefreshing.value) {
    await waitForStreamToSettle();
    encodedDir = findSessionProjectDir(filePath);
    if (encodedDir) return encodedDir;
  }

  // 3. 没在加载也没找到 → 触发一次 refresh 再查
  await refresh();
  return findSessionProjectDir(filePath);
}

function waitForStreamToSettle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (!projectsStream.isRefreshing.value) {
      resolve();
      return;
    }
    const stop = watch(
      () => projectsStream.isRefreshing.value,
      (loading) => {
        if (!loading) {
          stop();
          resolve();
        }
      }
    );
  });
}

function setAutoFollow(val: boolean) {
  autoFollow.value = val;
}

async function deleteSession(
  identity: SessionIdentity,
  displayName: string,
): Promise<boolean> {
  const confirmed = await ask(
    `确定要删除以下对话吗？\n\n${displayName}\n\n原始文件将移至回收站；SessionDock 内的归档、快照和索引会同步删除。`,
    { title: "删除确认", kind: "warning" }
  );
  if (!confirmed) return false;

  try {
    const outcome = await invoke<{ failed: [string, string][]; succeeded: number }>(
      "delete_sessions_to_trash",
      { cliId: identity.cliId, paths: [identity.filePath] }
    );
    if (outcome.failed.length > 0) {
      await message(`删除失败：${outcome.failed[0][1]}`, { title: "删除失败", kind: "error" });
      return false;
    }
    if (
      activeSessionIdentity.value
      && sessionIdentityKey(activeSessionIdentity.value) === sessionIdentityKey(identity)
    ) {
      clearActiveSession();
    }
    await refresh();
    return true;
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
    return false;
  }
}

interface GroupedDeleteResult {
  failed: [string, string][];
  succeeded: number;
  succeededKeys: Set<string>;
}

async function deleteIdentityGroups(identities: SessionIdentity[]): Promise<GroupedDeleteResult> {
  const result: GroupedDeleteResult = {
    failed: [],
    succeeded: 0,
    succeededKeys: new Set<string>(),
  };

  for (const [cliId, paths] of groupSessionIdentities(identities)) {
    try {
      const outcome = await invoke<{ failed: [string, string][]; succeeded: number }>(
        "delete_sessions_to_trash",
        { cliId, paths },
      );
      result.failed.push(...outcome.failed);
      result.succeeded += outcome.succeeded;
      const failedPaths = new Set(outcome.failed.map(([path]) => path));
      for (const filePath of paths) {
        if (!failedPaths.has(filePath)) {
          result.succeededKeys.add(sessionIdentityKey({ cliId, filePath }));
        }
      }
    } catch (error) {
      const reason = String(error);
      result.failed.push(...paths.map((path): [string, string] => [path, reason]));
    }
  }

  return result;
}

async function deleteProject(project: ProjectInfo): Promise<boolean> {
  const count = project.sessions.length;
  const confirmed = await ask(
    `确定要删除以下项目的所有对话吗？\n\n${project.original_path}\n（共 ${count} 个对话）\n\n原始文件将移至回收站；SessionDock 内的归档、快照和索引会同步删除。`,
    { title: "删除确认", kind: "warning" }
  );
  if (!confirmed) return false;

  try {
    const identities = project.sessions
      .filter((session) => isCliId(session.cli_id))
      .map((session) => ({ cliId: session.cli_id as CliId, filePath: session.file_path }));
    const outcome = await deleteIdentityGroups(identities);
    selectedSessionIdentities.value = selectedSessionIdentities.value.filter(
      (item) => !outcome.succeededKeys.has(sessionIdentityKey(item)),
    );
    if (
      activeSessionIdentity.value
      && outcome.succeededKeys.has(sessionIdentityKey(activeSessionIdentity.value))
    ) {
      clearActiveSession();
    }
    await refresh();
    if (outcome.failed.length > 0) {
      const reasons = [...new Set(outcome.failed.map(([, reason]) => reason))];
      await message(
        `成功删除 ${outcome.succeeded} 个，${outcome.failed.length} 个失败（${reasons.join("；")}）`,
        { title: "删除完成", kind: "warning" },
      );
      return false;
    }
    return true;
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
    return false;
  }
}

async function exportSession(
  identity: SessionIdentity,
  format: "txt" | "markdown" | "json" | "jsonl" = "txt",
  projectPath?: string,
  _sessionId?: string,
  selectedIndexes?: number[]
) {
  const extMap = { txt: "txt", markdown: "md", json: "json", jsonl: "jsonl" } as const;
  const nameMap = { txt: "Text", markdown: "Markdown", json: "JSON", jsonl: "JSON Lines" } as const;
  const ext = extMap[format];

  const baseName = projectPath
    ? projectPath.split(/[\\/]/).filter(Boolean).pop() ?? "session"
    : "session";

  const safeName = baseName.replace(/[<>:"/\\|?* ]/g, "_");
  const now = new Date();
  const dateStr = `${now.getFullYear()}${(now.getMonth() + 1).toString().padStart(2, "0")}${now.getDate().toString().padStart(2, "0")}_${now.getHours().toString().padStart(2, "0")}${now.getMinutes().toString().padStart(2, "0")}${now.getSeconds().toString().padStart(2, "0")}`;
  const suffix = selectedIndexes && selectedIndexes.length > 0 ? "_selected" : "";

  const defaultPath = `${safeName}_${dateStr}${suffix}.${ext}`;

  const savePath = await save({
    filters: [{ name: nameMap[format], extensions: [ext] }],
    defaultPath,
  });
  if (!savePath) return;

  try {
    const msg = await invoke<string>("export_session", {
      cliId: identity.cliId,
      filePath: identity.filePath,
      savePath,
      format,
      selectedIndexes: selectedIndexes || null,
    });
    await message(msg, { title: "导出", kind: "info" });
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

async function newSession(
  projectPath?: string,
  profileName?: string | null,
  skipOverride?: boolean,
  cliId: CliId = launchCliId.value,
) {
  const launchCli = getCliDefinition(cliId);
  if (!launchCli.supportsNewSession) {
    await message(`${launchCli.name} 暂不支持从 Claudia 新建对话`, {
      title: "提示",
      kind: "info",
    });
    return;
  }
  if (!(await ensureCliInstalled(cliId))) {
    return;
  }

  let targetPath = projectPath;
  if (!targetPath) {
    const selected = await open({ directory: true });
    if (!selected) return;
    targetPath = selected as string;
  }

  try {
    await invoke("open_in_terminal", {
      cliId,
      projectPath: targetPath,
      sessionId: null,
      skipPermissions: skipOverride ?? skipPermissions.value,
      profileName: profileName || undefined,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
    recordSuccessfulLaunch(cliId);
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

async function ensureSessionReadyOnDisk(
  filePath: string | undefined,
  sessionId: string | undefined,
  cliId: CliId,
): Promise<boolean> {
  let targetPath = filePath;
  if (!targetPath && sessionId) {
    const item = projectsStream.items.value.find(
      (i) => i.session.session_id === sessionId && i.session.cli_id === cliId,
    );
    if (item) {
      targetPath = item.session.file_path;
    }
  }
  if (!targetPath) return true;

  const item = projectsStream.items.value.find(
    (i) => i.session.file_path === targetPath && i.session.cli_id === cliId,
  );
  if (item?.session.is_archived) {
    try {
      await invoke("restore_session_to_disk", {
        cliId,
        filePath: targetPath,
      });
      item.session.is_archived = false;
      await refresh("snapshot");
    } catch (e: any) {
      console.warn("restore_session_to_disk error:", e);
    }
  }
  return true;
}

async function resumeSession(
  projectPath: string,
  sessionId: string,
  cliId: CliId,
  profileName?: string | null,
  skipOverride?: boolean,
) {
  const targetCli = getCliDefinition(cliId);
  if (!targetCli.supportsResumeSession) {
    return;
  }
  if (!(await ensureCliInstalled(cliId))) {
    return;
  }

  await ensureSessionReadyOnDisk(undefined, sessionId, cliId);

  try {
    await invoke("open_in_terminal", {
      cliId,
      projectPath,
      sessionId,
      skipPermissions: skipOverride ?? skipPermissions.value,
      profileName: profileName || undefined,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

/// Fork 会话：以 anchorUuid 对应消息为终点复制截断转录，返回新 sessionId。
/// 只负责产生新会话文件，启动/复制命令由调用方编排。
async function forkSession(identity: SessionIdentity, anchorUuid: string): Promise<string> {
  return invoke<string>("fork_session", {
    cliId: identity.cliId,
    filePath: identity.filePath,
    anchorUuid,
  });
}

async function setCliDataDirOverride(cliId: CliId, dataDir: string | null) {
  await invoke("set_cli_data_dir", {
    cliId,
    dataDir,
  });
  await loadCliPathConfigs();
  if (visibleCliIds.value.includes(cliId)) {
    await refresh();
  }
}

async function registerContextMenu(cliId: CliId = currentCliId.value) {
  if (!getCliDefinition(cliId).supportsContextMenu) {
    await message("当前 CLI 暂不支持系统右键菜单集成。", {
      title: "提示",
      kind: "info",
    });
    return;
  }

  try {
    const msg = await invoke<string>("register_context_menu", {
      skipPermissions: skipPermissions.value,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
    await message(msg, { title: "提示", kind: "info" });
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

async function unregisterContextMenu() {
  try {
    const msg = await invoke<string>("unregister_context_menu");
    await message(msg, { title: "提示", kind: "info" });
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

async function globalSearch(query: string) {
  globalSearchQuery.value = query;
  const trimmedQuery = query.trim();

  if (searchDebounceTimer) {
    clearTimeout(searchDebounceTimer);
    searchDebounceTimer = null;
  }

  if (!trimmedQuery) {
    searchIndexBuildRequest.value = null;
    searchIndexBuildSkipped.value = false;
    // 空 query：refresh 触发后端快路径（不查 DB，立即 emit done(0)），
    // composable 的 done-with-0-chunk 分支清空 items
    void searchStream.refresh({ cliIds: visibleCliIds.value, query: '' });
    return;
  }

  const frontCliId = visibleCliIds.value.includes(currentCliId.value)
    ? currentCliId.value
    : visibleCliIds.value[0];
  searchDebounceTimer = setTimeout(async () => {
    searchDebounceTimer = null;
    try {
      if (searchIndexProgress.value) {
        return;
      }
      // 抢占检查：debounce 期间 query 可能已改
      if (globalSearchQuery.value.trim() !== trimmedQuery) {
        return;
      }
      await searchStream.refresh({
        cliIds: visibleCliIds.value,
        frontCliId,
        query: trimmedQuery,
      });
    } catch (e) {
      console.error('search_sessions_stream failed:', e);
    }
  }, 300);
}

async function startSearchIndexBuild() {
  const request = searchIndexBuildRequest.value;
  if (!request) return;
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = false;
  buildingCliIds.value = [...buildingCliIds.value, request.cliId];
  try {
    await invoke('ensure_search_index_ready', { cliId: request.cliId });
    if (globalSearchQuery.value.trim() !== request.query) {
      return;
    }
    await searchStream.refresh({
      cliIds: visibleCliIds.value,
      frontCliId: visibleCliIds.value.includes(currentCliId.value)
        ? currentCliId.value
        : visibleCliIds.value[0],
      query: request.query,
    });
  } catch (e) {
    console.error('search index build failed:', e);
  } finally {
    buildingCliIds.value = buildingCliIds.value.filter((id) => id !== request.cliId);
  }
}

// all 模式结果区底部「未索引 CLI」的构建入口；大索引复用确认弹窗流程
async function buildPendingSearchIndex(cliId: string) {
  if (buildingCliIds.value.includes(cliId)) return;
  try {
    const status = await invoke<SearchIndexStatusPayload>('get_search_index_status', { cliId });
    if (!status.ready && status.requiresConfirmation) {
      searchIndexBuildSkipped.value = false;
      searchIndexBuildRequest.value = { ...status, cliId, query: globalSearchQuery.value.trim() };
      return;
    }
    buildingCliIds.value = [...buildingCliIds.value, cliId];
    try {
      await invoke('ensure_search_index_ready', { cliId });
      await searchStream.refresh({
        cliIds: visibleCliIds.value,
        frontCliId: visibleCliIds.value.includes(currentCliId.value)
          ? currentCliId.value
          : visibleCliIds.value[0],
        query: globalSearchQuery.value.trim(),
      });
      searchPendingCliIds.value = searchPendingCliIds.value.filter((id) => id !== cliId);
      searchStaleCliIds.value = searchStaleCliIds.value.filter((id) => id !== cliId);
    } finally {
      buildingCliIds.value = buildingCliIds.value.filter((id) => id !== cliId);
    }
  } catch (e) {
    console.error('build pending search index failed:', e);
  }
}

async function buildAllPendingSearchIndexes(cliIds?: string[]) {
  const targetIds = (cliIds ?? searchPendingCliIds.value).filter(
    (id) => !buildingCliIds.value.includes(id)
  );
  if (targetIds.length === 0) return;
  for (const id of targetIds) {
    await buildPendingSearchIndex(id);
  }
}

function dismissSearchIndexBuild() {
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = true;
}

// 组件挂载时清除过期状态用：不设置 'skipped'，
// 与 dismissSearchIndexBuild（用户主动关闭）区分。
function resetSearchIndexBuildState() {
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = false;
}

function setSkipPermissions(val: boolean) {
  skipPermissions.value = val;
  localStorage.setItem("claudia-skip-permissions", String(val));
}

const selectedSessionIdentities = ref<SessionIdentity[]>([]);

function findSessionIdentity(filePath: string, cliId?: CliId): SessionIdentity | null {
  const matches: SessionIdentity[] = [];
  for (const project of projects.value) {
    for (const session of project.sessions) {
      if (session.file_path !== filePath || !isCliId(session.cli_id)) continue;
      const identity = { cliId: session.cli_id, filePath };
      if (cliId && session.cli_id === cliId) return identity;
      matches.push(identity);
    }
  }
  // Temporary path-only Task 4 compatibility is safe only while unambiguous.
  return matches.length === 1 ? matches[0] : null;
}

function toggleSessionSelect(value: string | SessionIdentity, cliId?: CliId) {
  const identity = typeof value === "string" ? findSessionIdentity(value, cliId) : value;
  if (!identity) return;
  const key = sessionIdentityKey(identity);
  const idx = selectedSessionIdentities.value.findIndex(
    (item) => sessionIdentityKey(item) === key,
  );
  if (idx !== -1) {
    selectedSessionIdentities.value = [
      ...selectedSessionIdentities.value.slice(0, idx),
      ...selectedSessionIdentities.value.slice(idx + 1),
    ];
  } else {
    selectedSessionIdentities.value = [...selectedSessionIdentities.value, identity];
  }
}

function clearSessionSelection() {
  selectedSessionIdentities.value = [];
}

function selectAllSessions() {
  const allSessions: SessionIdentity[] = [];
  for (const p of filteredProjects.value) {
    for (const s of p.sessions) {
      if (isCliId(s.cli_id)) {
        allSessions.push({ cliId: s.cli_id, filePath: s.file_path });
      }
    }
  }
  selectedSessionIdentities.value = allSessions;
}

function selectProjectSessions(projectKey: string) {
  const project = filteredProjects.value.find(
    (p) => p.project_key === projectKey || p.encoded_dir === projectKey,
  );
  if (!project) return;
  const projectIdentities = project.sessions
    .filter((session) => isCliId(session.cli_id))
    .map((session) => ({ cliId: session.cli_id as CliId, filePath: session.file_path }));
  const current = new Map(
    selectedSessionIdentities.value.map((item) => [sessionIdentityKey(item), item]),
  );
  const allSelected = projectIdentities.every((item) => current.has(sessionIdentityKey(item)));
  if (allSelected) {
    for (const item of projectIdentities) {
      current.delete(sessionIdentityKey(item));
    }
  } else {
    for (const item of projectIdentities) {
      current.set(sessionIdentityKey(item), item);
    }
  }
  selectedSessionIdentities.value = [...current.values()];
}

function isProjectAllSelected(projectKey: string): boolean {
  const project = filteredProjects.value.find(
    (p) => p.project_key === projectKey || p.encoded_dir === projectKey,
  );
  if (!project || project.sessions.length === 0) return false;
  const selected = new Set(
    selectedSessionIdentities.value.map((item) => sessionIdentityKey(item)),
  );
  return project.sessions.every(
    (session) => isCliId(session.cli_id)
      && selected.has(sessionIdentityKey({ cliId: session.cli_id, filePath: session.file_path })),
  );
}

const isAllSelected = computed(() => {
  if (selectedSessionIdentities.value.length === 0) return false;
  const allIdentities = new Set<string>();
  for (const p of filteredProjects.value) {
    for (const s of p.sessions) {
      if (isCliId(s.cli_id)) {
        allIdentities.add(sessionIdentityKey({ cliId: s.cli_id, filePath: s.file_path }));
      }
    }
  }
  if (allIdentities.size === 0) return false;
  const selected = new Set(
    selectedSessionIdentities.value.map((item) => sessionIdentityKey(item)),
  );
  return (
    allIdentities.size === selected.size &&
    [...allIdentities].every((key) => selected.has(key))
  );
});

async function batchDeleteSessions(): Promise<boolean> {
  const count = selectedSessionIdentities.value.length;
  if (count === 0) return false;

  const confirmed = await ask(
    `确定要删除选中的 ${count} 个对话吗？\n\n原始文件将移至回收站；SessionDock 内的归档、快照和索引会同步删除。`,
    { title: "批量删除确认", kind: "warning" }
  );
  if (!confirmed) return false;

  deleteProgress.value = { done: 0, total: count, currentPath: "" };

  try {
    await ensureBatchDeleteProgressListener();
    const attempted = [...selectedSessionIdentities.value];
    const outcome = await deleteIdentityGroups(attempted);
    deleteProgress.value = null;
    if (
      activeSessionIdentity.value
      && outcome.succeededKeys.has(sessionIdentityKey(activeSessionIdentity.value))
    ) {
      clearActiveSession();
    }
    selectedSessionIdentities.value = attempted.filter(
      (item) => !outcome.succeededKeys.has(sessionIdentityKey(item)),
    );
    await refresh();
    if (outcome.failed.length > 0) {
      await message(
        `成功删除 ${outcome.succeeded} 个，${outcome.failed.length} 个失败（${outcome.failed[0][1]}）`,
        { title: "删除完成", kind: "warning" }
      );
      return false;
    }
    return true;
  } catch (e) {
    deleteProgress.value = null;
    await message(String(e), { title: "错误", kind: "error" });
    return false;
  }
}

async function batchExportSessions(
  mode: "merged" | "separate",
  format: "txt" | "markdown" | "json" | "jsonl" = "txt"
) {
  const count = selectedSessionIdentities.value.length;
  if (count === 0) return;

  const extMap = { txt: "txt", markdown: "md", json: "json", jsonl: "jsonl" } as const;
  const nameMap = { txt: "Text", markdown: "Markdown", json: "JSON", jsonl: "JSON Lines" } as const;
  const ext = extMap[format];

  const defaultPath = `batch_export.${ext}`;

  const savePath = await save({
    filters: [{ name: nameMap[format], extensions: [ext] }],
    defaultPath,
  });
  if (!savePath) return;

  try {
    const msg = await invoke<string>("batch_export_sessions", {
      sessions: [...selectedSessionIdentities.value],
      savePath,
      format,
      mode,
    });
    await message(msg, { title: "批量导出", kind: "info" });
  } catch (e) {
    await message(String(e), { title: "错误", kind: "error" });
  }
}

export function useSessions() {
  ensureSessionIndexListener();
  ensureSearchIndexListener();
  return {
    projects,
    searchQuery,
    activeSessionIdentity,
    selectedSessionPath,
    selectedProjectDir,
    filteredProjects,
    totalSessionCount,
    skipPermissions,
    setSkipPermissions,
    cliFilter,
    setCliFilter,
    visibleCliIds,
    launchCliId,
    recordSuccessfulLaunch,
    currentCliId,
    currentCli,
    cliStatuses,
    cliSessionCounts,
    cliPathConfigs,
    cliOptions,
    cliBinaryStatuses,
    installedCliOptions,
    setCurrentCli,
    loadCliStatuses,
    loadCliPathConfigs,
    setCliDataDirOverride,
    sortMode,
    setSortMode,
    globalSearchQuery,
    globalSearchResults,
    globalSearchLoading,
    searchIndexProgress,
    searchIndexBuildRequest,
    searchIndexBuildSkipped,
    dbMigrationProgress,
    refresh,
    clearActiveSession,
    setActiveSession,
    ensureSessionVisible,
    deleteSession,
    deleteProject,
    exportSession,
    newSession,
    resumeSession,
    forkSession,
    ensureSessionReadyOnDisk,
    ensureCurrentCliInstalled,
    registerContextMenu,
    unregisterContextMenu,
    globalSearch,
    searchPendingCliIds,
    searchStaleCliIds,
    scanCliErrors,
    searchCliErrors,
    retryCliScan,
    buildingCliIds,
    startSearchIndexBuild,
    buildPendingSearchIndex,
    buildAllPendingSearchIndexes,
    dismissSearchIndexBuild,
    resetSearchIndexBuildState,
    selectedSessionIdentities,
    toggleSessionSelect,
    clearSessionSelection,
    selectAllSessions,
    isAllSelected,
    selectProjectSessions,
    isProjectAllSelected,
    batchDeleteSessions,
    batchDeleteProgressText,
    deleteProgress,
    batchExportSessions,
    isStreamingProjects,
    totalLoadedSessions,
    isRefreshing,
    showLoadingIndicator,
    bootstrapping,
    startupPhase,
    startupSlow,
    autoFollow,
    setAutoFollow,
  };
}
