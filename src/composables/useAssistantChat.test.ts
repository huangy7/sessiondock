import { describe, it, expect } from "vitest";
import { reduceAssistantEvent, activityFromToolUse, stepFromToolUse, buildHistoryMessages } from "./useAssistantChat";

describe("reduceAssistantEvent", () => {
  it("error 事件生成 error kind 气泡", () => {
    const msgs = reduceAssistantEvent([], { type: "error", text: "CLI 未找到" });
    expect(msgs).toEqual([{ role: "assistant", kind: "error", text: "CLI 未找到" }]);
  });

  it("assistant-text 流式追加到最后一条文本气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "你" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "好" });
    expect(msgs).toEqual([{ role: "assistant", kind: "text", text: "你好" }]);
  });

  it("user 事件追加用户气泡", () => {
    const msgs = reduceAssistantEvent([], { type: "user", text: "问题" });
    expect(msgs[0]).toEqual({ role: "user", kind: "text", text: "问题" });
  });

  it("reducer 不变更输入数组（immutability）", () => {
    const msgs = [{ role: "assistant", kind: "text", text: "已有" } as const];
    const after = reduceAssistantEvent(msgs, { type: "assistant-text", text: "追加" });
    expect(after).not.toBe(msgs);
    expect(msgs).toHaveLength(1);
    expect(msgs[0].text).toBe("已有");
  });

  it("error 气泡后接 assistant-text 新开气泡（不追加进 error 气泡）", () => {
    let msgs = reduceAssistantEvent([], { type: "error", text: "出错了" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "重新回答" });
    expect(msgs).toEqual([
      { role: "assistant", kind: "error", text: "出错了" },
      { role: "assistant", kind: "text", text: "重新回答" },
    ]);
  });
});

describe("activityFromToolUse", () => {
  it("按 proxy 子命令映射人话动作", () => {
    expect(activityFromToolUse("Bash", "sessiondock-proxy list --days 7")).toBe("正在查询会话列表…");
    expect(activityFromToolUse("Bash", "sessiondock-proxy grep 登录")).toBe("正在搜索会话内容…");
    expect(activityFromToolUse("Bash", "sessiondock-proxy show abc --from 1")).toBe("正在读取会话正文…");
  });
  it("未知命令给通用文案", () => {
    expect(activityFromToolUse("Bash", "ls")).toBe("正在执行命令…");
    expect(activityFromToolUse(undefined, undefined)).toBe("正在执行命令…");
  });
});

describe("stepFromToolUse（步骤动作与目标）", () => {
  it("show 提取会话 id 前缀", () => {
    expect(stepFromToolUse("sessiondock-proxy show 3f8e5341-31e2-41cd-a178 --from 1 --to 30")).toEqual(
      { action: "读取会话正文", target: "3f8e5341" },
    );
  });
  it("grep 提取关键词", () => {
    expect(stepFromToolUse('sessiondock-proxy grep "认证失败" --days 30')).toEqual({
      action: "搜索会话内容", target: "「认证失败」",
    });
    expect(stepFromToolUse("sessiondock-proxy grep 登录")).toEqual({
      action: "搜索会话内容", target: "「登录」",
    });
  });
  it("list 与未知命令无目标", () => {
    expect(stepFromToolUse("sessiondock-proxy list --days 7")).toEqual({ action: "查询会话列表", target: null });
    expect(stepFromToolUse("ls")).toEqual({ action: "执行命令", target: null });
    expect(stepFromToolUse(undefined)).toEqual({ action: "执行命令", target: null });
  });
});

describe("同动作步骤聚合", () => {
  it("连续同动作步骤聚合并收集目标（不同目标也聚合）", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy show 3f8e5341-abc --from 1" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show 29d9f42d-def" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show 3f8e5341-abc --from 31" });
    expect(msgs[0].steps).toEqual([
      { text: "读取会话正文", done: false, kind: "tool", count: 3, targets: ["3f8e5341", "29d9f42d"] },
    ]);
  });

  it("解说打断后同动作另起新步骤", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy show 3f8e5341-abc" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "再深入读一遍" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show 29d9f42d-def" });
    // 解说被后续工具吸收为 note；新工具步骤因 note 隔开不并入旧步骤
    expect(msgs[0].steps).toHaveLength(3);
    expect(msgs[0].steps?.[0]).toMatchObject({ text: "读取会话正文", targets: ["3f8e5341"] });
    expect(msgs[0].steps?.[1]).toMatchObject({ kind: "note" });
    expect(msgs[0].steps?.[2]).toMatchObject({ text: "读取会话正文", targets: ["29d9f42d"] });
  });

  it("不同动作不聚合", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy show 3f8e5341-abc" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy list" });
    expect(msgs[0].steps).toHaveLength(2);
  });

  it("摘要步数按聚合后的调用总数计算", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy show 3f8e5341-abc --from 1" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show 29d9f42d-def" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    const tools = (msgs[0].steps ?? []).filter((s) => s.kind === "tool");
    const totalCalls = tools.reduce((n, s) => n + (s.count ?? 1), 0);
    expect(totalCalls).toBe(3);
  });
});

describe("过程卡片（process block）", () => {
  it("首个 tool-use 开启过程卡片，步骤为进行中", () => {
    const msgs = reduceAssistantEvent([], {
      type: "tool-use",
      summary: "sessiondock-proxy list --days 7",
    });
    expect(msgs).toEqual([
      {
        role: "assistant",
        kind: "process",
        text: "",
        collapsed: false,
        startedAt: expect.any(Number),
        steps: [{ text: "查询会话列表", done: false, kind: "tool" }],
      },
    ]);
  });

  it("连续 tool-use 追加步骤，前序步骤标记完成", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show abc" });
    expect(msgs).toHaveLength(1);
    expect(msgs[0].steps).toEqual([
      { text: "查询会话列表", done: true, kind: "tool" },
      { text: "读取会话正文", done: false, kind: "tool" },
    ]);
  });

  it("assistant-text 到来时步骤全部标记完成，块保持敞开，文本另起气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "查到了" });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].collapsed).toBe(false);
    expect(msgs[0].steps).toEqual([{ text: "查询会话列表", done: true, kind: "tool" }]);
    expect(msgs[1]).toMatchObject({ kind: "text", text: "查到了" });
  });

  it("被工具调用打断的助手文本是过程解说，吸收进过程卡片", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "先读几个重点会话的开头" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy show abc" });
    expect(msgs).toHaveLength(1);
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].steps).toEqual([
      { text: "先读几个重点会话的开头", done: true, kind: "note" },
      { text: "读取会话正文", done: false, kind: "tool" },
    ]);
  });

  it("一轮结束：解说进卡片，只有最终答案留作气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "先过滤元查询会话" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "最终答案" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end", durationMs: 1000 });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "text", text: "最终答案" });
  });

  it("turn-end 收起卡片并记录耗时", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "答案" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end", durationMs: 32000 });
    const block = msgs[0];
    expect(block.collapsed).toBe(true);
    expect(block.durationMs).toBe(32000);
    expect(block.steps?.every((s) => s.done)).toBe(true);
  });

  it("收起后再来 tool-use 开启新卡片（多轮不串）", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy list" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    msgs = reduceAssistantEvent(msgs, { type: "tool-use", summary: "sessiondock-proxy grep 登录" });
    expect(msgs).toHaveLength(2);
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "process", collapsed: false });
    expect(msgs[1].steps).toEqual([{ text: "搜索会话内容", done: false, kind: "tool", targets: ["「登录」"] }]);
  });

  it("error 事件收起敞开的过程卡片再上屏错误", () => {
    let msgs = reduceAssistantEvent([], { type: "tool-use", summary: "sessiondock-proxy show x" });
    msgs = reduceAssistantEvent(msgs, { type: "error", text: "挂了" });
    expect(msgs[0].collapsed).toBe(true);
    expect(msgs[1]).toMatchObject({ kind: "error", text: "挂了" });
  });

  it("无过程卡片时 turn-end 不产生任何变化", () => {
    const before = reduceAssistantEvent([], { type: "assistant-text", text: "纯文本" });
    const after = reduceAssistantEvent(before, { type: "turn-end" });
    expect(after).toEqual(before);
  });
});

describe("buildHistoryMessages（历史回放）", () => {
  it("同 turn 非末尾 assistant 段收成解说卡片，末尾段留作答案", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "生成周报" },
      { role: "assistant", text: "先过滤元查询会话" },
      { role: "assistant", text: "现在可以生成周报了" },
      { role: "assistant", text: "# 周报正文" },
    ]);
    expect(msgs).toHaveLength(3);
    expect(msgs[0]).toMatchObject({ role: "user", kind: "text" });
    expect(msgs[1]).toMatchObject({ kind: "process", collapsed: true });
    expect(msgs[1].steps).toEqual([
      { text: "先过滤元查询会话", done: true, kind: "note" },
      { text: "现在可以生成周报了", done: true, kind: "note" },
    ]);
    expect(msgs[2]).toMatchObject({ kind: "text", text: "# 周报正文" });
  });

  it("单段 turn 不产生过程卡片", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "你好" },
      { role: "assistant", text: "你好！" },
    ]);
    expect(msgs).toEqual([
      { role: "user", kind: "text", text: "你好" },
      { role: "assistant", kind: "text", text: "你好！" },
    ]);
  });

  it("过滤非 user/assistant 角色", () => {
    const msgs = buildHistoryMessages([
      { role: "system", text: "x" },
      { role: "user", text: "问题" },
      { role: "assistant", text: "回答" },
    ]);
    expect(msgs).toHaveLength(2);
  });
});

describe("turn-end usage 挂载", () => {
  it("turn-end 带 usage 时挂到最后一条 assistant 文本气泡", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "回答" });
    msgs = reduceAssistantEvent(msgs, {
      type: "turn-end",
      durationMs: 1200,
      usage: { inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0 },
    });
    expect(msgs[0].usage).toEqual({
      inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0,
    });
  });

  it("turn-end 无 usage 时消息不受影响", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "回答" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    expect(msgs[0].usage).toBeUndefined();
    expect(msgs[0].text).toBe("回答");
  });
});

describe("cancelled 事件", () => {
  it("给最后一条 assistant 文本打 stopped 标记并保留内容", () => {
    let msgs = reduceAssistantEvent([], { type: "assistant-text", text: "已生成一半" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[0]).toMatchObject({ kind: "text", text: "已生成一半", stopped: true });
  });

  it("收起敞开的过程卡片", () => {
    let msgs = reduceAssistantEvent([], {
      type: "tool-use", name: "Bash", summary: "sessiondock-proxy list --days 7",
    });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[0].kind).toBe("process");
    expect(msgs[0].collapsed).toBe(true);
  });

  it("没有 assistant 文本时原样返回（不报错）", () => {
    const msgs = reduceAssistantEvent([], { type: "user", text: "问" });
    const after = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(after).toHaveLength(1);
    expect(after[0].stopped).toBeUndefined();
  });

  it("新一轮未产出文本时取消，不标记上一轮的旧消息", () => {
    // 回归：停止「你可以干嘛」时标记曾错误落在上一轮「你是谁啊」的回答上
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "turn-end" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[1].stopped).toBeUndefined(); // 上一轮回答不被误标
    expect(msgs).toHaveLength(3);
  });

  it("新一轮已有文本时取消，标记本轮消息而非旧消息", () => {
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第二答一半" });
    msgs = reduceAssistantEvent(msgs, { type: "cancelled" });
    expect(msgs[1].stopped).toBeUndefined();
    expect(msgs[3]).toMatchObject({ text: "第二答一半", stopped: true });
  });

  it("turn-end 的 usage 同样只挂本轮消息", () => {
    let msgs = reduceAssistantEvent([], { type: "user", text: "第一问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第一答" });
    msgs = reduceAssistantEvent(msgs, { type: "user", text: "第二问" });
    msgs = reduceAssistantEvent(msgs, { type: "assistant-text", text: "第二答" });
    msgs = reduceAssistantEvent(msgs, {
      type: "turn-end",
      usage: { inputTokens: 10, outputTokens: 5, cacheReadInputTokens: 0, cacheCreationInputTokens: 0 },
    });
    expect(msgs[1].usage).toBeUndefined();
    expect(msgs[3].usage?.inputTokens).toBe(10);
  });
});

describe("buildHistoryMessages usage 回填", () => {
  it("assistant 段的 usage 挂到最终答案气泡（snake 转 camel）", () => {
    const msgs = buildHistoryMessages([
      { role: "user", text: "问" },
      { role: "assistant", text: "答", usage: { input_tokens: 100, output_tokens: 20 } },
    ]);
    expect(msgs[1].usage).toEqual({
      inputTokens: 100, outputTokens: 20, cacheReadInputTokens: 0, cacheCreationInputTokens: 0,
    });
  });

  it("同 turn 多段时取最后一段的 usage，前段解说进过程卡片不带 usage", () => {
    const msgs = buildHistoryMessages([
      { role: "assistant", text: "先查一下", usage: { input_tokens: 50, output_tokens: 10 } },
      { role: "assistant", text: "最终答案", usage: { input_tokens: 200, output_tokens: 30 } },
    ]);
    const answer = msgs.find((m) => m.kind === "text");
    expect(answer?.text).toBe("最终答案");
    expect(answer?.usage?.inputTokens).toBe(200);
  });

  it("usage 为 null/缺失时不挂字段", () => {
    const msgs = buildHistoryMessages([
      { role: "assistant", text: "答", usage: null },
      { role: "assistant", text: "答2" },
    ]);
    const answer = msgs.find((m) => m.kind === "text");
    expect(answer?.usage).toBeUndefined();
  });
});
