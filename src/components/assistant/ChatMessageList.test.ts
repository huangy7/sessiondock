import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ChatMessageList from "./ChatMessageList.vue";
import type { ChatMessage } from "../../composables/useAssistantChat";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue([]),
}));

describe("ChatMessageList", () => {
  it("渲染空状态并能点击快捷建议", async () => {
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.find(".chat-empty").exists()).toBe(true);
    expect(wrapper.text()).toContain("有什么可以帮你？");
    expect(wrapper.text()).toContain("将以 test-profile 开始对话");

    const chips = wrapper.findAll(".suggestion-chip");
    expect(chips.length).toBeGreaterThan(0);
    await chips[0].trigger("click");
    expect(wrapper.emitted("pick")).toBeTruthy();
  });

  it("渲染 02 Thinking & 05 Tool Chips 原语化卡片与展开切换", async () => {
    const processMessage: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: true,
      durationMs: 3400,
      steps: [
        {
          kind: "tool",
          text: "检索历史会话",
          targets: ["workspace/SessionDock", "sessions"],
          count: 2,
          done: true,
        },
        {
          kind: "note",
          text: "正在提炼周报核心要点",
          done: true,
        },
      ],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [processMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const pill = wrapper.find(".thinking-pill");
    expect(pill.exists()).toBe(true);
    expect(pill.text()).toContain("Thought for 3.4s");

    // 默认折叠时未展开工具芯片
    expect(wrapper.find(".tool-chips-container").exists()).toBe(false);

    // 点击展开
    await pill.trigger("click");
    expect(wrapper.find(".tool-chips-container").exists()).toBe(true);

    const rows = wrapper.findAll(".tool-chip-row");
    expect(rows.length).toBe(2);
    expect(rows[0].text()).toContain("检索历史会话");
    expect(rows[0].text()).toContain("workspace/SessionDock · sessions");
    expect(rows[0].text()).toContain("×2");
    expect(rows[1].text()).toContain("正在提炼周报核心要点");
  });

  it("当处于未收起的思考中状态时展示实时动态计时与微光动画", async () => {
    const activeProcess: ChatMessage = {
      role: "assistant",
      kind: "process",
      text: "",
      collapsed: false,
      startedAt: Date.now() - 2500, // 2.5s 前开始
      steps: [
        { kind: "tool", text: "查询会话列表", done: false },
      ],
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [activeProcess],
        activity: "正在查询会话列表…",
        profile: "test-profile",
      },
    });

    const pill = wrapper.find(".thinking-pill");
    expect(pill.classes()).toContain("active");
    expect(wrapper.find(".shimmer-text").text()).toBe("Thinking…");
    const timer = wrapper.find(".thinking-live-timer");
    expect(timer.exists()).toBe(true);
    expect(timer.text()).toMatch(/\d+(\.\d+)?s/);
  });

  it("渲染常规 Assistant 文本回复与复制按钮", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "这是 SessionDock 助手的回答内容",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.find(".assistant-row").exists()).toBe(true);
    expect(wrapper.find(".markdown-body").text()).toContain("这是 SessionDock 助手的回答内容");
    expect(wrapper.find(".copy-icon-btn").exists()).toBe(true);
  });

  it("渲染 09 Recommendation 原语：当模型输出 «FOLLOWUPS: ...» 时动态渲染追问药丸，且正文不出现标签", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "本周完成了关于 SessionDock 助手 Thinking 和 Tool Chips 的功能重构。\n«FOLLOWUPS: 按重点与难点细化周报 | 补充单元测试用例»",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const followups = wrapper.findAll(".followup-pill");
    expect(followups.length).toBe(2);
    expect(followups[0].text()).toContain("按重点与难点细化周报");
    expect(followups[1].text()).toContain("补充单元测试用例");

    // Markdown 正文中被干净剥离，不暴露给用户
    expect(wrapper.find(".markdown-body").text()).not.toContain("«FOLLOWUPS:");
    expect(wrapper.find(".markdown-body").text()).toContain("本周完成了关于 SessionDock 助手");

    await followups[0].trigger("click");
    expect(wrapper.emitted("pick")?.[0]).toEqual(["按重点与难点细化周报"]);
  });

  it("当模型未输出 FOLLOWUPS 时不渲染追问药丸（无写死规则）", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "这是一条没有附带追问的普通回复内容",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    expect(wrapper.findAll(".followup-pill")).toHaveLength(0);
  });

  it("用户发送新消息时，即使此前上翻暂停了跟随，也会恢复 follow 为 true 并滚动到底部", async () => {
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "历史回复",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [assistantMessage],
        activity: null,
        profile: "test-profile",
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    // 模拟容器尺寸与滚动高度
    Object.defineProperty(listEl, "scrollHeight", { value: 1000, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 200; // 上翻距离底部 > 40px

    // 触发上翻滚动事件
    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    // 用户发出新消息
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "用户新发送的问题",
    };

    await wrapper.setProps({
      messages: [assistantMessage, userMessage],
    });

    // 应该恢复跟随，并将 scrollTop 置为 scrollHeight
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(true);
    expect(listEl.scrollTop).toBe(1000);
  });

  it("用户上翻后助手流式更新文本时，不打断用户阅读（不强制置底）", async () => {
    const userMessage: ChatMessage = {
      role: "user",
      kind: "text",
      text: "请总结",
    };
    const assistantMessage: ChatMessage = {
      role: "assistant",
      kind: "text",
      text: "正在",
    };

    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [userMessage, assistantMessage],
        activity: "正在思考…",
        profile: "test-profile",
      },
    });
    await new Promise((r) => setTimeout(r, 50));

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1000, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 100; // 用户上翻阅读

    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    // 助手继续流式追加文本（消息数量未增加，仅文本增长）
    await wrapper.setProps({
      messages: [userMessage, { ...assistantMessage, text: "正在输出更多内容…" }],
    });

    // follow 依然保持 false，且不强行滚动到底部
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);
    expect(listEl.scrollTop).toBe(100);
  });

  it("调用 scrollToBottom(true) 暴露方法直接重置 follow 并滚动到底部", async () => {
    const wrapper = mount(ChatMessageList, {
      props: {
        messages: [{ role: "user", kind: "text", text: "测试" }],
        activity: null,
      },
    });

    const listEl = wrapper.find(".chat-list").element as HTMLElement;
    Object.defineProperty(listEl, "scrollHeight", { value: 1200, configurable: true });
    Object.defineProperty(listEl, "clientHeight", { value: 400, configurable: true });
    listEl.scrollTop = 100;

    await wrapper.find(".chat-list").trigger("scroll");
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(false);

    await (wrapper.vm as unknown as { scrollToBottom: (force?: boolean) => Promise<void> }).scrollToBottom(true);
    expect((wrapper.vm as unknown as { follow: boolean }).follow).toBe(true);
    expect(listEl.scrollTop).toBe(1200);
  });
});

