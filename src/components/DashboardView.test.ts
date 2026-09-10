import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

const { mount, flushPromises } = await import("@vue/test-utils");
const DashboardView = (await import("./DashboardView.vue")).default;

const kpiResponse = {
  kpi: {
    quotaLimit: 20,
    usedQuota: 4.82,
    totalCost: "$4.82",
    totalCostRaw: 4.82,
    cacheHitRate: 83.6,
    inputTokens: "1.0M",
    outputTokens: "0.2M",
    cachedTokens: "5.0M",
    cacheCreationInputTokens: "0.5M",
    costTrend: null,
    deptManagementMode: "none",
  },
  modelData: [],
  trendData: null,
  updatedAtStr: "10:30:00",
};

const recent = [
  {
    cliId: "claude",
    filePath: "/proj/a.jsonl",
    encodedDir: "-proj",
    projectPath: "/workspace/proj-a",
    title: "评审分支代码与去重设计对齐",
    timestamp: "2026-08-27T08:00:00Z",
  },
  {
    cliId: "codex",
    filePath: "/proj/b.jsonl",
    encodedDir: "-proj",
    projectPath: "/workspace/proj-b",
    title: "多 CLI 历史对话聚合方案",
    timestamp: "2026-08-26T08:00:00Z",
  },
];

function mountDashboard(props: Record<string, unknown> = {}) {
  return mount(DashboardView, {
    props: { recentConversations: recent, ...props },
    global: {
      stubs: {
        SvgIcon: true,
        ChatAvatar: true,
      },
    },
  });
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(null);
});

describe("DashboardView 双模式入口", () => {
  it("默认「问 SessionDock」模式，并提供「全局搜索」切换", async () => {
    const wrapper = mountDashboard();
    const modes = wrapper.findAll(".mode-btn");
    expect(modes).toHaveLength(2);
    expect(modes[0].text()).toContain("问 SessionDock");
    expect(modes[1].text()).toContain("全局搜索");
    expect(modes[0].classes()).toContain("active");
  });

  it("问 SessionDock 模式提交发出 submitAssistant", async () => {
    const wrapper = mountDashboard();
    await wrapper.find(".prompt-input").setValue("帮我总结本周工作");
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitAssistant")).toEqual([["帮我总结本周工作"]]);
    expect(wrapper.emitted("submitSearch")).toBeUndefined();
  });

  it("切换到全局搜索模式后提交发出 submitSearch", async () => {
    const wrapper = mountDashboard();
    const modes = wrapper.findAll(".mode-btn");
    await modes[1].trigger("click");
    await wrapper.find(".prompt-input").setValue("登录报错");
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitSearch")).toEqual([["登录报错"]]);
    expect(wrapper.emitted("submitAssistant")).toBeUndefined();
  });

  it("空输入不提交", async () => {
    const wrapper = mountDashboard();
    await wrapper.find(".prompt-box").trigger("submit.prevent");
    expect(wrapper.emitted("submitAssistant")).toBeUndefined();
    expect(wrapper.emitted("submitSearch")).toBeUndefined();
  });
});

describe("DashboardView 最近对话", async () => {
  it("渲染最近对话列表，点击条目发出带 cliId 的 openSession", async () => {
    const wrapper = mountDashboard();
    const rows = wrapper.findAll(".recent-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("评审分支代码与去重设计对齐");
    await rows[1].trigger("click");
    expect(wrapper.emitted("openSession")).toEqual([
      [{ cliId: "codex", filePath: "/proj/b.jsonl" }, "-proj"],
    ]);
  });

  it("无最近对话时隐藏列表区域", async () => {
    const wrapper = mountDashboard({ recentConversations: [] });
    expect(wrapper.find(".recent-section").exists()).toBe(false);
  });

  it("正确解析 Windows 风格反斜杠路径的项目标签", async () => {
    const wrapper = mountDashboard({
      recentConversations: [
        {
          cliId: "claude",
          filePath: "C:\\Users\\huangy6\\test.jsonl",
          encodedDir: "test",
          projectPath: "C:\\Users\\huangy6\\AppData\\Roaming\\com.claudia.app\\data\\assistant\\workspace",
          title: "搜索提到「重构」的会话",
          timestamp: "2026-08-27T08:00:00Z",
        },
      ],
    });
    expect(wrapper.find(".recent-meta").text()).toBe("workspace");
  });
});

describe("DashboardView 内联搜索结果区", () => {
  const searchResults = [
    {
      session_id: "s1",
      file_path: "/proj/a.jsonl",
      display_name: "登录报错排查",
      project_path: "/workspace/proj-a",
      snippet: "…登录接口返回 500，堆栈指向鉴权中间件…",
      match_count: 3,
      first_match_message_index: 12,
      cli_id: "claude",
    },
    {
      session_id: "s2",
      file_path: "/proj/b.jsonl",
      display_name: "多 CLI 聚合方案",
      project_path: "/workspace/proj-b",
      snippet: "…搜索范围沿用 visibleCliIds…",
      match_count: 1,
      first_match_message_index: null,
      cli_id: "codex",
    },
  ];

  it("无搜索关键词时不渲染内联结果区", async () => {
    const wrapper = mountDashboard();
    expect(wrapper.find(".search-inline").exists()).toBe(false);
  });

  it("带 searchQuery 与结果时渲染 CLI 标识、工作区与摘要", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    const rows = wrapper.findAll(".search-result-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("登录报错排查");
    expect(rows[0].text()).toContain("proj-a");
    expect(rows[0].text()).toContain("3 处匹配");
    expect(rows[0].text()).toContain("鉴权中间件");
  });

  it("点击结果行发出 openSearchResult 携带完整 SearchResult", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.findAll(".search-result-row")[1].trigger("click");
    expect(wrapper.emitted("openSearchResult")).toEqual([[searchResults[1]]]);
  });

  it("「在完整搜索页中查看」发出 openFullSearch 携带当前关键词", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.find(".search-full-link").trigger("click");
    expect(wrapper.emitted("openFullSearch")).toEqual([["登录报错"]]);
  });

  it("「清除」发出 clearSearch", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults });
    await wrapper.find(".search-clear-btn").trigger("click");
    expect(wrapper.emitted("clearSearch")).toHaveLength(1);
  });

  it("加载中显示搜索中状态", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults: [], searchLoading: true });
    expect(wrapper.find(".search-inline-status").text()).toContain("搜索中");
  });

  it("空结果显示空态并保留查询词", async () => {
    const wrapper = mountDashboard({ searchQuery: "登录报错", searchResults: [] });
    const status = wrapper.find(".search-inline-status");
    expect(status.exists()).toBe(true);
    expect(status.text()).toContain("未找到匹配结果");
    expect(wrapper.find(".search-inline").text()).toContain("登录报错");
  });

  it("focusSearch() 切换到搜索模式并聚焦输入框", async () => {
    const wrapper = mount(DashboardView, {
      attachTo: document.body,
      props: { recentConversations: recent },
      global: { stubs: { SvgIcon: true, ChatAvatar: true } },
    });
    (wrapper.vm as unknown as { focusSearch: () => void }).focusSearch();
    await flushPromises();
    const modes = wrapper.findAll(".mode-btn");
    expect(modes[1].classes()).toContain("active");
    expect(modes[1].attributes("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(wrapper.find(".prompt-input").element);
    wrapper.unmount();
  });
});

