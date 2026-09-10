import { beforeAll, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import SessionTree from "./SessionTree.vue";
import type { AggregatedProjectInfo, SessionIdentity, SessionInfo } from "../types/session";

vi.mock("../composables/useProjectFilter", () => ({
  useProjectFilter: () => ({ blockProject: vi.fn() }),
}));

function session(cliId: "claude" | "codex", id: string, time: string): SessionInfo {
  return {
    session_id: `${cliId}-${id}`,
    file_path: `/workspace/${cliId}/${id}.jsonl`,
    display_name: `${cliId === "claude" ? "Claude" : "Codex"} session ${id}`,
    timestamp: time,
    file_size: 100,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

const mockProjects: AggregatedProjectInfo[] = [
  {
    encoded_dir: "shared",
    original_path: "/workspace/shared",
    project_key: "/workspace/shared",
    cli_ids: ["claude", "codex"],
    sessions: [
      session("claude", "c1", "2026-08-28T10:00:00Z"),
      session("claude", "c2", "2026-08-28T09:00:00Z"),
      session("codex", "x1", "2026-08-28T08:00:00Z"),
    ],
  },
  {
    encoded_dir: "codex-only",
    original_path: "/workspace/codex-only",
    project_key: "/workspace/codex-only",
    cli_ids: ["codex"],
    sessions: [session("codex", "x2", "2026-08-28T11:00:00Z")],
  },
];

function mountTree(props: Record<string, unknown> = {}) {
  return mount(SessionTree, {
    props: {
      projects: mockProjects,
      selectedSessionIdentity: null,
      selectedSessionIdentities: [],
      searchQuery: "",
      isStreamingProjects: false,
      totalLoadedSessions: 4,
      isRefreshing: false,
      showLoadingIndicator: false,
      projectAllSelected: () => false,
      grouping: "directory",
      ...props,
    },
    global: {
      stubs: { SvgIcon: true },
    },
  });
}

describe("SessionTree grouping modes", () => {
  beforeAll(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  it("renders 2-level tree in directory mode", async () => {
    const wrapper = mountTree({ grouping: "directory" });

    // Level 1: Projects
    const projectHeaders = wrapper.findAll(".project-header");
    expect(projectHeaders).toHaveLength(2);
    expect(wrapper.findAll(".provider-node")).toHaveLength(0);

    // Expand first project
    await projectHeaders[0].trigger("click");
    const sessionItems = wrapper.findAll(".session-item");
    expect(sessionItems).toHaveLength(3); // 2 claude + 1 codex
  });

  it("renders 3-level tree in provider mode with provider headers, count capsules, and nested projects", async () => {
    const wrapper = mountTree({ grouping: "provider" });

    // Level 1: Provider nodes (Claude Code and Codex)
    const providerNodes = wrapper.findAll(".provider-node");
    expect(providerNodes).toHaveLength(2);

    // Provider headers have avatar and count capsule
    const providerHeaders = wrapper.findAll(".provider-header");
    expect(providerHeaders).toHaveLength(2);

    const claudeHeader = providerHeaders[0];
    expect(claudeHeader.find(".provider-name").text()).toBe("Claude Code");
    expect(claudeHeader.find(".provider-count-capsule").text()).toBe("2");

    const codexHeader = providerHeaders[1];
    expect(codexHeader.find(".provider-name").text()).toBe("Codex");
    expect(codexHeader.find(".provider-count-capsule").text()).toBe("2");

    // Level 2: By default provider nodes are expanded, projects inside are collapsed
    const nestedProjectHeaders = wrapper.findAll(".nested-project-header");
    // Claude has 1 project (/workspace/shared), Codex has 2 projects (/workspace/codex-only, /workspace/shared)
    expect(nestedProjectHeaders).toHaveLength(3);

    // Level 3: Expand Claude's project
    await nestedProjectHeaders[0].trigger("click");
    const claudeSessions = wrapper.findAll(".nested-session-list .session-item");
    expect(claudeSessions).toHaveLength(2);
    expect(claudeSessions[0].text()).toContain("Claude session c1");
    expect(claudeSessions[1].text()).toContain("Claude session c2");
  });

  it("maintains independent collapse states between directory mode and provider mode", async () => {
    const wrapper = mountTree({ grouping: "directory" });

    // In directory mode, expand /workspace/shared
    await wrapper.findAll(".project-header")[0].trigger("click");
    expect(wrapper.findAll(".session-item")).toHaveLength(3);

    // Switch to provider mode
    await wrapper.setProps({ grouping: "provider" });
    // Projects inside provider groups should still be in their own default collapsed state
    expect(wrapper.findAll(".session-item")).toHaveLength(0);

    // Expand /workspace/shared under Claude
    await wrapper.findAll(".nested-project-header")[0].trigger("click");
    expect(wrapper.findAll(".session-item")).toHaveLength(2);

    // Switch back to directory mode
    await wrapper.setProps({ grouping: "directory" });
    // Directory mode project should still be expanded!
    expect(wrapper.findAll(".session-item")).toHaveLength(3);
  });

  it("locates session in provider mode by opening both provider and nested project nodes", async () => {
    const targetIdentity: SessionIdentity = {
      cliId: "codex",
      filePath: "/workspace/codex/x1.jsonl",
    };

    const wrapper = mountTree({
      grouping: "provider",
      selectedSessionIdentity: null,
    });

    // Collapse Codex provider group first
    await wrapper.findAll(".provider-header")[1].trigger("click");
    expect(wrapper.findAll(".nested-project-header")).toHaveLength(1); // Only Claude projects visible

    // Set selectedSessionIdentity to Codex session
    await wrapper.setProps({ selectedSessionIdentity: targetIdentity });
    await wrapper.vm.$nextTick();

    // Now Codex provider node and nested project node should both be open
    const openSessions = wrapper.findAll(".session-item");
    expect(openSessions.length).toBeGreaterThan(0);
    const selectedItem = wrapper.find(".session-item.selected");
    expect(selectedItem.exists()).toBe(true);
    expect(selectedItem.text()).toContain("Codex session x1");
  });

  it("handles keyboard navigation across visible sessions in provider mode", async () => {
    const wrapper = mountTree({ grouping: "provider" });

    // Expand Claude's project
    await wrapper.findAll(".nested-project-header")[0].trigger("click");

    const tree = wrapper.find(".session-tree");
    await tree.trigger("keydown", { key: "ArrowDown" });

    expect(wrapper.emitted("selectSession")?.[0]).toEqual([
      { cliId: "claude", filePath: "/workspace/claude/c1.jsonl" },
      "shared",
    ]);
  });
});
