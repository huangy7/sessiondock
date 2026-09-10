import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { leafIds } from "../utils/panes";

vi.mock("./useSessions", async () => {
  const { ref } = await import("vue");
  return {
    useSessions: () => ({
      projects: ref([]),
      clearActiveSession: vi.fn(),
      setActiveSession: vi.fn(),
      selectedSessionIdentities: ref([]),
      batchDeleteSessions: vi.fn(async () => true),
    }),
  };
});
vi.mock("./usePtySession", () => ({
  usePtySession: () => ({
    sessions: ref([]),
    closeSession: vi.fn(),
    createSession: vi.fn(async (projectPath, cliKind) => ({
      sessionId: `pty-${Math.random().toString(36).slice(2, 8)}`,
      projectPath: projectPath || "/test/project",
      cliKind: cliKind || "shell",
      createdAt: new Date().toISOString(),
      status: "active",
    })),
    setSessionLabel: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => true) }));

const { useTabs } = await import("./useTabs");

describe("零工作 Tab 时的 Dashboard 根页面", () => {
  it("没有任何打开的 Tab 时显示 Dashboard", () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];
    expect(tabs.openTabs.value).toHaveLength(0);
    expect(tabs.shouldShowDashboard.value).toBe(true);
  });

  it("打开一个历史对话 Tab 后隐藏 Dashboard", () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];
    tabs.openHistoryTab("/proj/a.jsonl", "proj", { pinned: true, cliId: "claude" });
    expect(tabs.shouldShowDashboard.value).toBe(false);
  });

  it("关闭最后一个 Tab 后恢复 Dashboard", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];
    tabs.openHistoryTab("/proj/a.jsonl", "proj", { pinned: true, cliId: "claude" });
    expect(tabs.shouldShowDashboard.value).toBe(false);
    await tabs.closeTab(tabs.openTabs.value[0].id);
    expect(tabs.openTabs.value).toHaveLength(0);
    expect(tabs.shouldShowDashboard.value).toBe(true);
  });
});

describe("Preview Tab 固定行为", () => {
  it("固定历史预览 Tab 时保持 tab.id 稳定，不触发重新挂载", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    // 单击打开预览 Tab
    const tab = tabs.openHistoryTab("/proj/session1.jsonl", "proj", { pinned: false, cliId: "claude" });
    const initialTabId = tab.id;
    expect(tab.isPreview).toBe(true);
    expect(initialTabId).toBe("hist-claude-/proj/session1.jsonl");
    expect(tabs.activeTabId.value).toBe(initialTabId);

    // 双击或调用 pinPreviewTab 固定
    tabs.pinPreviewTab(initialTabId);

    const pinnedTab = tabs.openTabs.value.find((t) => t.sessionPath === "/proj/session1.jsonl");
    expect(pinnedTab).toBeDefined();
    expect(pinnedTab?.isPreview).toBe(false);
    expect(pinnedTab?.id).toBe(initialTabId); // id 必须保持不变
    expect(tabs.activeTabId.value).toBe(initialTabId);
  });

  it("固定文件预览 Tab 时保持 tab.id 稳定", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    tabs.openFileTab("/workspace/src/main.ts", "/workspace", { pinned: false });
    const initialId = tabs.activeTabId.value;
    expect(initialId).toBe("file-/workspace/src/main.ts");
    const activeTab = tabs.openTabs.value.find((t) => t.id === initialId);
    expect(activeTab?.isPreview).toBe(true);

    tabs.pinPreviewTab(initialId);
    const pinnedTab = tabs.openTabs.value.find((t) => t.id === initialId);
    expect(pinnedTab?.isPreview).toBe(false);
    expect(pinnedTab?.id).toBe(initialId);
  });
});

describe("终端 Tab 与分屏融合", () => {
  it("新建 Shell 终端 Tab 并在 tabItems 中显示", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "shell", projectPath: "/my/code" });
    expect(tabs.openTabs.value).toHaveLength(1);
    expect(tabs.activeTabId.value).toBe(tab.id);
    expect(tab.type).toBe("terminal");
    expect(tab.label).toBe("终端");
    expect(tab.rootPane).toBeDefined();
    expect(tab.rootPane?.kind).toBe("leaf");

    const item = tabs.tabItems.value.find((t) => t.id === tab.id);
    expect(item?.icon).toBe("terminal");
    expect(item?.label).toBe("终端");
  });

  it("在当前终端 Tab 内分屏并更新 splitCount", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "shell" });
    await tabs.splitActiveTerminalTab("row");

    expect(tab.rootPane?.kind).toBe("split");
    expect(leafIds(tab.rootPane!).length).toBe(2);

    const item = tabs.tabItems.value.find((t) => t.id === tab.id);
    expect(item?.splitCount).toBe(2);
  });

  it("重命名 Tab", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "shell" });
    expect(tab.label).toBe("终端");

    tabs.renameTab(tab.id, "编译与测试");
    expect(tab.label).toBe("编译与测试");
    expect(tab.isCustomLabel).toBe(true);

    // 空内容重置为默认
    tabs.renameTab(tab.id, "");
    expect(tab.label).toBe("终端");
  });

  it("分屏关闭时若有多个分屏则只关分屏，仅剩 1 个时关闭 Tab", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "shell" });
    const leaf2 = await tabs.splitActiveTerminalTab("row");

    expect(leafIds(tab.rootPane!).length).toBe(2);

    // 关闭第二个分屏
    await tabs.closeTerminalPane(tab.id, leaf2!.id);
    expect(leafIds(tab.rootPane!).length).toBe(1);
    expect(tabs.openTabs.value).toHaveLength(1);

    // 关闭剩下的最后一个分屏，整个 Tab 关闭
    await tabs.closeTerminalPane(tab.id, tab.activePaneId!);
    expect(tabs.openTabs.value).toHaveLength(0);
    expect(tabs.shouldShowDashboard.value).toBe(true);
  });

  it("分屏时默认继承当前分屏所在目录且默认启动 shell", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "claude", projectPath: "/workspace/Claudia" });
    expect(tab.projectRoot).toBe("/workspace/Claudia");

    const splitLeaf = await tabs.splitActiveTerminalTab("row");
    expect(splitLeaf?.cliKind).toBe("shell");
    expect(splitLeaf?.projectPath).toBe("/workspace/Claudia");
  });

  it("支持临时最大化/全屏分屏并在再次调用时还原", async () => {
    const tabs = useTabs();
    tabs.openTabs.value = [];

    const tab = await tabs.openTerminalTab(undefined, { cliKind: "claude" });
    const leaf2 = await tabs.splitActiveTerminalTab("row");
    expect(tab.maximizedPaneId).toBeUndefined();

    // 最大化 leaf2
    tabs.toggleMaximizePane(tab.id, leaf2!.id);
    expect(tab.maximizedPaneId).toBe(leaf2!.id);

    // 再次调用还原
    tabs.toggleMaximizePane(tab.id, leaf2!.id);
    expect(tab.maximizedPaneId).toBeNull();
  });
});

