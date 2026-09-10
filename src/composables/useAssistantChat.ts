import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ProcessStep {
  text: string;
  done: boolean;
  /** tool = 工具动作；note = 被后续工具打断的助手解说文本 */
  kind: "tool" | "note";
  /** 同动作连续调用聚合计数 */
  count?: number;
  /** 聚合步骤的目标列表（会话 id 前缀 / 关键词，去重） */
  targets?: string[];
}

/** 一轮的 token 用量（input 为末次调用的上下文量，output 为生成量） */
export interface TurnUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadInputTokens: number;
  cacheCreationInputTokens: number;
}

/** 后端 wire 格式（snake_case），仅边界处使用 */
export interface RawUsage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_input_tokens?: number;
  cache_creation_input_tokens?: number;
}

/** 后端 snake_case usage → 前端 camelCase；缺字段补 0 */
function toTurnUsage(u?: RawUsage | null): TurnUsage | undefined {
  if (!u) return undefined;
  return {
    inputTokens: u.input_tokens ?? 0,
    outputTokens: u.output_tokens ?? 0,
    cacheReadInputTokens: u.cache_read_input_tokens ?? 0,
    cacheCreationInputTokens: u.cache_creation_input_tokens ?? 0,
  };
}

export interface ChatMessage {
  role: "user" | "assistant";
  kind: "text" | "error" | "process";
  text: string;
  /** 开始时间戳（用于实时计时） */
  startedAt?: number;
  /** kind === "process" 时的步骤列表 */
  steps?: ProcessStep[];
  /** 一轮结束后收起为单行摘要 */
  collapsed?: boolean;
  /** 收起摘要里的用时（来自 result 事件） */
  durationMs?: number;
  /** 一轮结束后的 token 用量（来自 result 事件或历史回放） */
  usage?: TurnUsage;
  /** 用户主动停止的标记（已生成内容保留） */
  stopped?: boolean;
}

export interface AssistantUiEvent {
  type: "user" | "assistant-text" | "error" | "tool-use" | "turn-end" | "cancelled";
  text?: string;
  name?: string;
  summary?: string;
  durationMs?: number;
  usage?: TurnUsage;
}

/** 把最近一张敞开的过程卡片收起（全部步骤标完成）；无敞开卡片则原样返回 */
function collapseOpenProcess(msgs: ChatMessage[], durationMs?: number): ChatMessage[] {
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if (m.kind !== "process") continue;
    if (m.collapsed) return msgs; // 最近的卡片已收起，说明本轮无敞开卡片
    const updated = [...msgs];
    updated[i] = {
      ...m,
      collapsed: true,
      durationMs,
      steps: (m.steps ?? []).map((s) => ({ ...s, done: true })),
    };
    return updated;
  }
  return msgs;
}

/**
 * 给「本轮」的 assistant 文本气泡打补丁：从尾部扫到最后一条 user 消息为止。
 * 本轮还没产出文本（取消发生在工具阶段/首包前）时不标记上一轮的旧消息。
 */
function attachToCurrentTurnText(
  msgs: ChatMessage[],
  patch: Partial<Pick<ChatMessage, "usage" | "stopped">>,
): ChatMessage[] {
  for (let i = msgs.length - 1; i >= 0; i--) {
    const m = msgs[i];
    if (m.role === "user") return msgs; // 越过了本轮边界
    if (m.role === "assistant" && m.kind === "text") {
      const updated = [...msgs];
      updated[i] = { ...m, ...patch };
      return updated;
    }
  }
  return msgs;
}

/** 纯函数：把一条事件归并进消息列表（可单测）。tool 事件进过程卡片。 */
export function reduceAssistantEvent(
  msgs: ChatMessage[],
  event: AssistantUiEvent,
): ChatMessage[] {
  if (event.type === "user") {
    return [...msgs, { role: "user", kind: "text", text: event.text ?? "" }];
  }
  if (event.type === "error") {
    const collapsed = collapseOpenProcess(msgs);
    return [...collapsed, { role: "assistant", kind: "error", text: event.text ?? "" }];
  }
  if (event.type === "tool-use") {
    const { action, target } = stepFromToolUse(event.summary);
    // 被工具调用打断的助手文本是过程解说（DeepSeek 式）：吸收进过程卡片，
    // 一轮最后一段文本才作为最终答案气泡留下
    let base = msgs;
    let note: ProcessStep | null = null;
    const tail = msgs[msgs.length - 1];
    if (tail && tail.role === "assistant" && tail.kind === "text" && tail.text.trim()) {
      note = { text: tail.text, done: true, kind: "note" };
      base = msgs.slice(0, -1);
    }
    const last = base[base.length - 1];
    if (last && last.kind === "process" && !last.collapsed) {
      const steps = (last.steps ?? []).map((s) => ({ ...s, done: true }));
      if (note) steps.push(note);
      // 连续同动作聚合（被解说隔开的不并）：计数 +1、目标去重追加
      const prevStep = steps[steps.length - 1];
      if (!note && prevStep && prevStep.kind === "tool" && prevStep.text === action) {
        const targets = prevStep.targets ?? [];
        steps[steps.length - 1] = {
          ...prevStep,
          done: false,
          count: (prevStep.count ?? 1) + 1,
          targets: target && !targets.includes(target) ? [...targets, target] : targets,
        };
      } else {
        steps.push({
          text: action,
          done: false,
          kind: "tool",
          targets: target ? [target] : undefined,
        });
      }
      const updated = [...base];
      updated[updated.length - 1] = { ...last, steps };
      return updated;
    }
    const steps: ProcessStep[] = note ? [note] : [];
    steps.push({
      text: action,
      done: false,
      kind: "tool",
      targets: target ? [target] : undefined,
    });
    return [
      ...base,
      { role: "assistant", kind: "process", text: "", collapsed: false, startedAt: Date.now(), steps },
    ];
  }
  if (event.type === "turn-end") {
    const collapsed = collapseOpenProcess(msgs, event.durationMs);
    return event.usage ? attachToCurrentTurnText(collapsed, { usage: event.usage }) : collapsed;
  }
  if (event.type === "cancelled") {
    // 用户主动停止：收起过程卡片 + 给本轮的回答打「已停止」标记，内容保留
    return attachToCurrentTurnText(collapseOpenProcess(msgs), { stopped: true });
  }
  // assistant-text：先把敞开卡片的在途步骤标完成，再流式追加文本气泡
  const withDone = collapseStepsOnly(msgs);
  const last = withDone[withDone.length - 1];
  if (last && last.role === "assistant" && last.kind === "text") {
    const updated = [...withDone];
    updated[updated.length - 1] = { ...last, text: last.text + (event.text ?? "") };
    return updated;
  }
  return [...withDone, { role: "assistant", kind: "text", text: event.text ?? "" }];
}

/** 只标完成不收起（一轮中文本与工具交替时块保持敞开） */
function collapseStepsOnly(msgs: ChatMessage[]): ChatMessage[] {
  const last = msgs[msgs.length - 1];
  if (!last || last.kind !== "process" || last.collapsed) return msgs;
  const updated = [...msgs];
  updated[updated.length - 1] = {
    ...last,
    steps: (last.steps ?? []).map((s) => ({ ...s, done: true })),
  };
  return updated;
}

/** tool-use → 步骤动作与目标（动作用于聚合，目标用于后缀展示） */
export function stepFromToolUse(summary?: string): { action: string; target: string | null } {
  const cmd = summary ?? "";
  if (/\blist\b/.test(cmd)) return { action: "查询会话列表", target: null };
  const grepMatch = cmd.match(/\bgrep\s+"([^"]+)"|\bgrep\s+(\S+)/);
  if (grepMatch) return { action: "搜索会话内容", target: `「${grepMatch[1] ?? grepMatch[2]}」` };
  const showMatch = cmd.match(/\bshow\s+([0-9a-fA-F-]{8})/);
  if (showMatch) return { action: "读取会话正文", target: showMatch[1].toLowerCase() };
  if (/\bshow\b/.test(cmd)) return { action: "读取会话正文", target: null };
  if (/\bgrep\b/.test(cmd)) return { action: "搜索会话内容", target: null };
  return { action: "执行命令", target: null };
}

/**
 * 历史回放的消息构建（纯函数，可单测）。
 * 回放只有 user/assistant 纯文本（工具调用不可见），同一 turn 里非末尾的
 * assistant 段必是被工具打断的过程解说 → 收进已收起的过程卡片。
 */
export function buildHistoryMessages(
  history: { role: string; text: string; usage?: RawUsage | null }[],
): ChatMessage[] {
  const items = history.filter((m) => m.role === "user" || m.role === "assistant");
  const out: ChatMessage[] = [];
  let i = 0;
  while (i < items.length) {
    if (items[i].role === "user") {
      out.push({ role: "user", kind: "text", text: items[i].text });
      i++;
      continue;
    }
    const segs: { text: string; usage?: RawUsage | null }[] = [];
    while (i < items.length && items[i].role === "assistant") {
      segs.push({ text: items[i].text, usage: items[i].usage });
      i++;
    }
    if (segs.length > 1) {
      out.push({
        role: "assistant",
        kind: "process",
        text: "",
        collapsed: true,
        steps: segs.slice(0, -1).map((s) => ({ text: s.text, done: true, kind: "note" as const })),
      });
    }
    // 与实时路径口径一致：最终答案气泡挂同 turn 组最后一段的 usage
    const last = segs[segs.length - 1];
    const usage = toTurnUsage(last.usage);
    out.push({
      role: "assistant",
      kind: "text",
      text: last.text,
      ...(usage ? { usage } : {}),
    });
  }
  return out;
}

/** tool-use → 瞬时活动行的人话文案 */
export function activityFromToolUse(_name?: string, summary?: string): string {
  const cmd = summary ?? "";
  if (/\blist\b/.test(cmd)) return "正在查询会话列表…";
  if (/\bgrep\b/.test(cmd)) return "正在搜索会话内容…";
  if (/\bshow\b/.test(cmd)) return "正在读取会话正文…";
  return "正在执行命令…";
}

/** 后端错误原文 → 人话卡片文案 */
export function humanizeError(raw: string): string {
  if (raw.includes("未检测到 claude CLI")) return "未检测到 Claude CLI，请先安装并完成配置";
  if (raw.includes("上一轮还在进行中")) return "上一轮回复还在进行中，请稍候";
  return raw;
}

interface AssistantEventPayload {
  conversationId: string;
  type?: "done" | "error" | "cancelled";
  ok?: boolean;
  error?: string;
  event?: {
    type: "init" | "assistant-text" | "tool-use" | "result";
    text?: string;
    name?: string;
    summary?: string;
    ok?: boolean;
    error?: string;
    duration_ms?: number;
    usage?: RawUsage;
  };
}

/**
 * 助手窗口的聊天状态。本 composable 的生命周期 == 助手窗口生命周期：
 * 组件 unmount（含 HMR 触发）时 AssistantApp.vue 会调用 dispose() 清理 listener。
 *
 * 状态按对话隔离：messages/activity 视图跟随 conversationId，事件按 id 路由到
 * 各对话自己的缓冲——生成中可以切换到别的对话，后台轮次不受干扰。
 */
export function useAssistantChat() {
  // 每个对话的独立消息缓冲（含进行中的流式内容）
  const buffers = new Map<string, ChatMessage[]>();
  const runningIds = ref<Set<string>>(new Set());
  const conversationId = ref<string | null>(null);
  const messages = ref<ChatMessage[]>([]);
  const activity = ref<string | null>(null);
  let unlisten: UnlistenFn | null = null;

  /** 当前查看的对话是否正在生成 */
  const running = computed(() =>
    conversationId.value !== null && runningIds.value.has(conversationId.value),
  );

  /** 当前对话的累计输出 token（徽标数据；上下文总量由每条消息的 meta 行各自表达） */
  const conversationUsage = computed(() => {
    let outputTotal = 0;
    for (const m of messages.value) {
      if (m.role === "assistant" && m.usage) {
        outputTotal += m.usage.outputTokens;
      }
    }
    return outputTotal > 0 ? { outputTotal } : null;
  });

  /** 把事件归并进指定对话的缓冲；若该对话是当前视图则同步 messages */
  function applyToBuffer(convId: string, event: AssistantUiEvent) {
    const next = reduceAssistantEvent(buffers.get(convId) ?? [], event);
    buffers.set(convId, next);
    if (conversationId.value === convId) {
      messages.value = next;
    }
  }

  /** 切换视图到指定对话的消息缓冲 */
  function showBuffer(convId: string | null) {
    messages.value = (convId && buffers.get(convId)) || [];
  }

  async function ensureListener() {
    if (unlisten) return;
    unlisten = await listen<AssistantEventPayload>("assistant-event", (e) => {
      const p = e.payload;
      const isCurrent = conversationId.value === p.conversationId;
      if (p.type === "done") {
        // done 是兜底收起（正常路径 result 已带耗时收过）
        applyToBuffer(p.conversationId, { type: "turn-end" });
        runningIds.value.delete(p.conversationId);
        if (isCurrent) activity.value = null;
        return;
      }
      if (p.type === "error") {
        runningIds.value.delete(p.conversationId);
        applyToBuffer(p.conversationId, {
          type: "error",
          text: humanizeError(p.error ?? "未知错误"),
        });
        if (isCurrent) activity.value = null;
        return;
      }
      if (p.type === "cancelled") {
        // 用户主动停止：cancel() 已清 running，这里兜底 + 打「已停止」标记
        runningIds.value.delete(p.conversationId);
        applyToBuffer(p.conversationId, { type: "cancelled" });
        if (isCurrent) activity.value = null;
        return;
      }
      const ev = p.event;
      if (!ev) return;
      if (ev.type === "assistant-text") {
        applyToBuffer(p.conversationId, { type: "assistant-text", text: ev.text });
        if (isCurrent) activity.value = null;
      } else if (ev.type === "tool-use") {
        // 步骤进过程卡片；底部活动行让位（卡片自身有进行态）
        applyToBuffer(p.conversationId, { type: "tool-use", name: ev.name, summary: ev.summary });
        if (isCurrent) activity.value = null;
      } else if (ev.type === "result") {
        applyToBuffer(p.conversationId, {
          type: "turn-end",
          durationMs: ev.duration_ms,
          usage: toTurnUsage(ev.usage),
        });
        if (ev.ok === false) {
          // result 失败事件也要上屏（此前被静默丢弃，UI 永远无反馈）
          applyToBuffer(p.conversationId, {
            type: "error",
            text: humanizeError(ev.error ?? "助手引擎运行失败"),
          });
        }
        if (isCurrent) activity.value = null;
      }
    });
  }

  function dispose() {
    unlisten?.();
    unlisten = null;
  }

  async function send(prompt: string, profile: string, model?: string) {
    if (!prompt.trim() || running.value) return;
    // 诊断：listen 失败/挂起必须上屏，不能静默（此前 send 断在这里时 UI 零反馈）
    try {
      await Promise.race([
        ensureListener(),
        new Promise((_, reject) =>
          setTimeout(() => reject(new Error("事件监听注册超时（3s）")), 3000),
        ),
      ]);
    } catch (err) {
      messages.value = reduceAssistantEvent(messages.value, {
        type: "error",
        text: humanizeError(
          `事件监听注册失败: ${err instanceof Error ? err.message : String(err)}`,
        ),
      });
      return;
    }
    const userEvent: AssistantUiEvent = { type: "user", text: prompt };
    if (conversationId.value) {
      applyToBuffer(conversationId.value, userEvent);
    } else {
      messages.value = reduceAssistantEvent(messages.value, userEvent);
    }
    // 发送即刻给出 thinking 态：首包前的静默期（引擎启动/网络重试）也有反馈
    activity.value = "正在思考…";
    try {
      const result = await invoke<{ conversationId: string }>("assistant_send", {
        conversationId: conversationId.value,
        prompt,
        profile,
        model: model || null,
      });
      conversationId.value = result.conversationId;
      // 新对话：把已上屏的消息（含用户气泡）种入该对话的缓冲
      buffers.set(result.conversationId, messages.value);
      runningIds.value.add(result.conversationId);
    } catch (err) {
      activity.value = null;
      messages.value = reduceAssistantEvent(messages.value, {
        type: "error",
        text: humanizeError(err instanceof Error ? err.message : String(err)),
      });
    }
  }

  async function cancel() {
    if (!conversationId.value) return;
    try {
      await invoke("assistant_cancel", { conversationId: conversationId.value });
    } finally {
      runningIds.value.delete(conversationId.value);
      activity.value = null;
    }
  }

  async function loadConversation(id: string) {
    conversationId.value = id;
    activity.value = null;
    // 进行中的对话直接展示实时缓冲（生成中切换/切回都不打断）
    if (runningIds.value.has(id) && buffers.has(id)) {
      showBuffer(id);
      return;
    }
    messages.value = [];
    try {
      const history = await invoke<{ role: string; text: string; usage?: RawUsage | null }[]>(
        "assistant_conversation_messages",
        { conversationId: id },
      );
      // 防竞态：加载期间用户又切了对话则丢弃
      if (conversationId.value !== id) return;
      const msgs = buildHistoryMessages(history);
      buffers.set(id, msgs);
      messages.value = msgs;
    } catch (err) {
      console.error("加载对话历史失败:", err);
      messages.value = reduceAssistantEvent(messages.value, {
        type: "error",
        text: humanizeError(`加载历史失败: ${err instanceof Error ? err.message : String(err)}`),
      });
    }
  }

  function newConversation() {
    conversationId.value = null;
    messages.value = [];
    activity.value = null;
  }

  return { messages, running, activity, conversationId, conversationUsage, send, cancel, loadConversation, newConversation, dispose };
}
