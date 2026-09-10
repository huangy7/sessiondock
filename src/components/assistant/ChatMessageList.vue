<script setup lang="ts">
import { ref, watch, nextTick, onMounted, onUnmounted } from "vue";
import type { ChatMessage } from "../../composables/useAssistantChat";
import {
  renderMarkdown,
  extractSessionRefPrefixes,
  renderSessionRefs,
  extractFollowups,
  stripFollowups,
} from "../../utils/markdown";
import { invoke } from "@tauri-apps/api/core";
import { formatTokenCount } from "../../utils/format";
import { vMermaid } from "../../directives/vMermaid";
import "../../styles/markdown.css";

const props = defineProps<{ messages: ChatMessage[]; activity: string | null; profile?: string }>();
const emit = defineEmits<{ pick: [text: string] }>();

const listEl = ref<HTMLElement | null>(null);
// 用户上翻时暂停跟随滚动
const follow = ref(true);
// 已收起过程卡片的展开状态（按消息下标）
const expanded = ref<Set<number>>(new Set());
const copiedIndex = ref<number | null>(null);

// 实时计时器（100ms 刷新一次当前时间，用于正在进行的 Thinking 实时渲染毫秒/秒数）
const now = ref(Date.now());
let liveTimer: ReturnType<typeof setInterval> | null = null;

function startLiveTimer() {
  if (liveTimer) return;
  now.value = Date.now();
  liveTimer = setInterval(() => {
    now.value = Date.now();
  }, 100);
}

function stopLiveTimer() {
  if (liveTimer) {
    clearInterval(liveTimer);
    liveTimer = null;
  }
}

watch(
  () => [props.activity, props.messages.some((m) => m.kind === "process" && !m.collapsed)],
  ([act, hasUncollapsed]) => {
    if (act || hasUncollapsed) {
      startLiveTimer();
    } else {
      stopLiveTimer();
    }
  },
  { immediate: true },
);

onUnmounted(() => {
  stopLiveTimer();
});

function getLiveElapsed(m: ChatMessage): string {
  const start = m.startedAt ?? now.value;
  const elapsedMs = Math.max(0, now.value - start);
  const total = elapsedMs / 1000;
  if (total < 60) return `${total.toFixed(1)}s`;
  return `${Math.floor(total / 60)}m ${(total % 60).toFixed(1)}s`;
}

function copyFullText(text: string, index: number) {
  navigator.clipboard.writeText(stripFollowups(text));
  copiedIndex.value = index;
  setTimeout(() => {
    if (copiedIndex.value === index) copiedIndex.value = null;
  }, 1500);
}

function toggleExpand(i: number) {
  const next = new Set(expanded.value);
  if (next.has(i)) {
    next.delete(i);
  } else {
    next.add(i);
  }
  expanded.value = next;
}

function formatDuration(ms: number): string {
  const s = ms / 1000;
  return s >= 10 ? `${Math.round(s)}s` : `${s.toFixed(1)}s`;
}

/** 收起卡片的摘要文案：工具调用总数（合并计数展开）+ 解说条数 */
function summaryText(m: ChatMessage): string {
  const steps = m.steps ?? [];
  const toolSteps = steps.filter((s) => s.kind === "tool");
  const tools = toolSteps.reduce((n, s) => n + (s.count ?? 1), 0);
  const notes = steps.length - toolSteps.length;
  const parts: string[] = [];
  if (tools > 0) parts.push(`已完成 ${tools} 步操作`);
  if (notes > 0) parts.push(tools > 0 ? `${notes} 条解说` : `${notes} 条过程解说`);
  if (m.durationMs) parts.push(`用时 ${formatDuration(m.durationMs)}`);
  return parts.join(" · ");
}

/** 最后一条 assistant 文本在生成中（activity 非空）时火花呼吸 */
function isLiveAssistant(i: number): boolean {
  const m = props.messages[i];
  return (
    props.activity !== null &&
    i === props.messages.length - 1 &&
    m.role === "assistant" &&
    m.kind === "text"
  );
}

const USER_PROFILE_PROMPT = [
  "请根据我的所有对话历史，为我生成一份结构化的用户画像分析报告。请包含以下维度：",
  "",
  "1. **核心性格特质**：从我的提问方式、关注点和表达习惯中，提炼 3-5 个核心性格标签（如：结果导向、细节控、好奇心强等），并引用对话中的实际表述作为佐证。",
  "",
  "2. **沟通风格与偏好**：分析我偏好的沟通方式——是逻辑导向还是发散式的？喜欢简洁回答还是深度解释？对回复格式有什么隐含偏好？",
  "",
  "3. **专业领域与技术栈**：从对话中提炼我最常涉及的技术领域、工具链和专业知识范围，评估每个领域的熟练程度。",
  "",
  "4. **工作习惯与思维模式**：我如何分解问题？倾向于自顶向下还是自底向上？是否有明显的决策风格或思维盲点？",
  "",
  '5. **MBTI 倾向预估**：基于以上分析，推测我最可能的 MBTI 类型，给出每个维度的倾向判断和置信度。',
  "",
  "6. **AI 协作最佳实践**：基于以上画像，给出 3-5 条具体建议，告诉我应该如何更好地与 AI 协作以发挥最大效率。",
  "",
  '请基于实际对话内容分析，避免泛泛而谈的「星座式」描述。如果某个维度数据不足，请标注"低置信度"。',
].join("\n");

const suggestions: { label: string; prompt: string }[] = [
  { label: "根据我的对话，帮我生成用户画像", prompt: USER_PROFILE_PROMPT },
  { label: "搜索提到「重构」的会话", prompt: "搜索提到「重构」的会话" },
  { label: "帮我总结一下本周的工作进展", prompt: "帮我总结一下本周的工作进展" },
];

/** 09 Recommendation 原语：完全由大模型在回复末尾动态输出的 Follow-up 追问建议 */
function getFollowups(text: string): string[] {
  return extractFollowups(text);
}

function onScroll() {
  const el = listEl.value;
  if (!el) return;
  follow.value = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
}

async function scrollToBottom(force = false) {
  if (force) {
    follow.value = true;
  }
  const el = listEl.value;
  if (el && follow.value) {
    el.scrollTop = el.scrollHeight;
  }
  await nextTick();
  if (!el) return;
  if (follow.value) {
    el.scrollTop = el.scrollHeight;
    const raf =
      typeof requestAnimationFrame === "function"
        ? requestAnimationFrame
        : (cb: () => void) => setTimeout(cb, 16);
    raf(() => {
      if (el && follow.value) {
        el.scrollTop = el.scrollHeight;
      }
    });
  }
}

defineExpose({ scrollToBottom, follow });

watch(
  () => [props.messages.length, props.messages[props.messages.length - 1]?.text, props.activity] as const,
  ([newLen], oldTuple) => {
    const oldLen = oldTuple?.[0];
    if (newLen === 0) {
      follow.value = true;
      return;
    }
    const lastMsg = props.messages[props.messages.length - 1];
    // 用户发出新消息时：强制置底并恢复跟随
    const isNewUserMsg = Boolean(
      lastMsg &&
        lastMsg.role === "user" &&
        (oldLen === undefined || (newLen as number) > (oldLen as number)),
    );
    void scrollToBottom(isNewUserMsg);
  },
);

// 代码块复制按钮（事件委托 + 一次性增强）
function enhanceCodeBlocks() {
  const el = listEl.value;
  if (!el) return;
  el.querySelectorAll("pre.hljs:not([data-copy-enhanced])").forEach((pre) => {
    pre.setAttribute("data-copy-enhanced", "1");
    const btn = document.createElement("button");
    btn.className = "code-copy-btn";
    btn.textContent = "复制";
    btn.addEventListener("click", () => {
      navigator.clipboard.writeText(pre.querySelector("code")?.textContent ?? "");
      btn.textContent = "已复制";
      setTimeout(() => (btn.textContent = "复制"), 1500);
    });
    pre.appendChild(btn);
  });
}

interface SessionRef {
  title: string;
  filePath: string;
  cliId: string;
}

/** 前缀 → 标题 的校验缓存（同源引用复用，避免每条消息重复调） */
const refTitles = ref<Map<string, string>>(new Map());
// 已确认无效的前缀集合：后续流式 chunk 不再重复查询
const invalidPrefixes = new Set<string>();
let resolving = false;
let pending = false;

/** 渲染单条消息：过滤动态追问标签后渲染 markdown → 引用胶囊后处理 */
function renderBody(text: string): string {
  return renderSessionRefs(renderMarkdown(stripFollowups(text)), refTitles.value);
}

/** 扫描消息里的引用前缀，批量校验并缓存有效项（async fire-and-forget） */
async function resolveRefs() {
  if (resolving) {
    pending = true; // 流式期间新引用到达：记下，待本次 in-flight 结束后补扫
    return;
  }
  resolving = true;
  try {
    const prefixes = new Set<string>();
    for (const m of props.messages) {
      if (m.role === "assistant") {
        for (const p of extractSessionRefPrefixes(m.text)) prefixes.add(p);
      }
    }
    const missing = [...prefixes].filter(
      (p) => !refTitles.value.has(p) && !invalidPrefixes.has(p),
    );
    if (missing.length === 0) return;
    const results = await invoke<(SessionRef | null)[]>("resolve_session_refs", {
      prefixes: missing,
    });
    const next = new Map(refTitles.value);
    results.forEach((r, i) => {
      if (r) next.set(missing[i], r.title);
      else invalidPrefixes.add(missing[i]); // 已确认无效：不再查询
    });
    refTitles.value = next; // 触发重渲染，有效引用变胶囊
  } catch (err) {
    console.error("解析会话引用失败:", err);
  } finally {
    resolving = false;
    if (pending) {
      pending = false;
      void resolveRefs();
    }
  }
}

function onListClick(e: MouseEvent) {
  const target = (e.target as HTMLElement).closest(".session-ref") as HTMLElement | null;
  if (!target) return;
  const prefix = target.getAttribute("data-prefix");
  if (!prefix) return;
  e.preventDefault();
  invoke("assistant_open_session", { prefix }).catch((err) => {
    console.error("打开会话失败:", err);
  });
}

onMounted(() => {
  void resolveRefs();
  enhanceCodeBlocks();
  if (props.messages.length > 0) {
    void scrollToBottom(true);
  }
});
watch(() => props.messages, () => { void resolveRefs(); nextTick(enhanceCodeBlocks); });
</script>

<template>
  <div ref="listEl" class="chat-list" @scroll.passive="onScroll" @click="onListClick">
    <div v-if="messages.length === 0 && !activity" class="chat-empty">
      <span class="empty-spark" />
      <p class="empty-title">有什么可以帮你？</p>
      <p v-if="profile" class="empty-pin-hint">将以 {{ profile }} 开始对话 · 发送后固定</p>
      <button
        v-for="s in suggestions"
        :key="s.label"
        class="suggestion-chip"
        @click="emit('pick', s.prompt)"
      >{{ s.label }}</button>
    </div>
    <TransitionGroup name="bubble">
      <div
        v-for="(m, i) in messages"
        :key="i"
        class="chat-bubble"
        :class="[`role-${m.role}`, `kind-${m.kind}`]"
      >
        <div v-if="m.kind === 'error'" class="error-text">{{ m.text }}</div>
        
        <!-- 02 Thinking & 05 Tool Chips 原语化卡片 -->
        <div v-else-if="m.kind === 'process'" class="process-card">
          <button
            type="button"
            class="thinking-pill"
            :class="{ open: !m.collapsed || expanded.has(i), active: !m.collapsed }"
            :aria-expanded="!m.collapsed || expanded.has(i)"
            @click="toggleExpand(i)"
          >
            <svg
              class="thinking-spark-icon"
              :class="{ pulsing: !m.collapsed }"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="currentColor"
            >
              <path d="M12 2l2.4 7.2L22 12l-7.6 2.8L12 22l-2.4-7.2L2 12l7.6-2.8z" />
            </svg>
            <span class="thinking-title">
              <template v-if="!m.collapsed">
                <span class="shimmer-text">Thinking…</span>
                <span class="thinking-live-timer">{{ getLiveElapsed(m) }}</span>
              </template>
              <template v-else>
                {{ m.durationMs ? `Thought for ${formatDuration(m.durationMs)}` : (m.startedAt ? `Thought for ${getLiveElapsed(m)}` : summaryText(m)) }}
              </template>
            </span>
            <svg
              class="thinking-chevron"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </button>

          <!-- 展开的工具芯片组与推理步骤 -->
          <div v-if="!m.collapsed || expanded.has(i)" class="tool-chips-container">
            <div
              v-for="(s, j) in m.steps"
              :key="j"
              class="tool-chip-row"
              :class="{ 'is-note': s.kind === 'note' }"
            >
              <span class="chip-status-icon">
                <span v-if="s.kind === 'note'" class="step-note-marker">›</span>
                <span v-else-if="s.done" class="step-check">✓</span>
                <span v-else class="thinking-dots step-dots"><i /><i /><i /></span>
              </span>
              <span class="chip-action-name">{{ s.text }}</span>
              <span v-if="s.targets?.length" class="chip-target-pill">
                {{ s.targets.join(" · ") }}
              </span>
              <span v-if="s.count && s.count > 1" class="chip-count-badge">×{{ s.count }}</span>
            </div>
          </div>
        </div>

        <div v-else-if="m.role === 'assistant'" class="assistant-row">
          <span class="spark" :class="{ breathing: isLiveAssistant(i) }" />
          <div class="assistant-body">
            <div v-mermaid class="markdown-body" v-html="renderBody(m.text)" />
            <div v-if="m.stopped || m.usage || (m.kind === 'text' && m.text)" class="msg-meta">
              <span v-if="m.stopped" class="meta-chip stopped">
                <span class="stopped-dot" />已停止
              </span>
              <span
                v-if="m.usage"
                class="meta-chip usage"
                :title="`上下文总量 ${m.usage.inputTokens + m.usage.cacheReadInputTokens + m.usage.cacheCreationInputTokens}（新输入 ${m.usage.inputTokens} · 缓存读取 ${m.usage.cacheReadInputTokens} · 缓存写入 ${m.usage.cacheCreationInputTokens}）· 输出 ${m.usage.outputTokens}`"
              ><span class="usage-dir in">↑</span>{{ formatTokenCount(m.usage.inputTokens + m.usage.cacheReadInputTokens + m.usage.cacheCreationInputTokens) }}<span class="meta-sep" /><span class="usage-dir out">↓</span>{{ formatTokenCount(m.usage.outputTokens) }}</span>
              <button
                v-if="m.kind === 'text' && m.text"
                class="copy-icon-btn"
                :class="{ copied: copiedIndex === i }"
                title="复制 Markdown 全文"
                @click="copyFullText(m.text, i)"
              >
                <span v-if="copiedIndex === i" class="copied-check">✓</span>
                <svg v-else class="copy-icon-svg" viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                  <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
                  <path d="M3.5 10.5H2.5A1.5 1.5 0 0 1 1 9V2.5A1.5 1.5 0 0 1 2.5 1h6.5A1.5 1.5 0 0 1 10.5 2.5v1" />
                </svg>
              </button>
            </div>

            <!-- 09 Recommendation 原语：回答完成后的关联追问建议 -->
            <div
              v-if="!activity && i === messages.length - 1 && m.kind === 'text' && m.text && getFollowups(m.text).length > 0"
              class="followup-pills"
            >
              <button
                v-for="item in getFollowups(m.text)"
                :key="item"
                type="button"
                class="followup-pill"
                @click="emit('pick', item)"
              >
                <span class="followup-spark">✦</span>
                <span>{{ item }}</span>
              </button>
            </div>
          </div>
        </div>
        <div v-else class="user-text">{{ m.text }}</div>
      </div>
    </TransitionGroup>
    <div v-if="activity" class="activity-row">
      <span class="thinking-dots"><i /><i /><i /></span>
      <span class="activity-text">{{ activity }}</span>
    </div>
  </div>
</template>

<style scoped>
.chat-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-3);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.chat-empty {
  margin: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
}
.empty-title {
  color: var(--color-text-secondary);
  font-size: var(--text-md);
  font-weight: 600;
  margin: 0 0 var(--space-2);
}
.empty-spark {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-full);
  background: linear-gradient(135deg, #89b4fa, #cba6f7);
  box-shadow: 0 0 24px rgba(137, 180, 250, 0.5);
  animation: spark-breathe 2.4s ease-in-out infinite;
  margin-bottom: var(--space-1);
}
.empty-pin-hint {
  margin: 0 0 var(--space-2);
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  font-family: var(--font-mono);
}
.suggestion-chip {
  padding: 6px 14px;
  border-radius: var(--radius-full);
  border: 1px solid var(--color-border);
  background: var(--color-bg-hover);
  color: var(--color-text-secondary);
  font-size: 12px;
  cursor: pointer;
}
.suggestion-chip:hover {
  border-color: var(--color-primary);
  color: var(--color-text);
}
.chat-bubble {
  max-width: 88%;
  font-size: 13px;
  line-height: 1.6;
}
.role-user {
  align-self: flex-end;
  background: var(--color-primary, #4f6ef7);
  color: var(--color-text-inverse);
  padding: 8px 12px;
  border-radius: var(--radius-lg, 12px);
}
.user-text {
  white-space: pre-wrap;
  word-break: break-word;
}
.role-assistant {
  align-self: flex-start;
  max-width: 100%;
  color: var(--color-text);
}
/* 助手火花：渐变 orb，生成中呼吸 */
.assistant-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
}
.copy-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  opacity: 0.45;
  transition: opacity 0.15s ease, background 0.15s ease, color 0.15s ease;
}
.assistant-row:hover .copy-icon-btn {
  opacity: 0.85;
}
.copy-icon-btn:hover {
  opacity: 1;
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.copy-icon-btn.copied {
  opacity: 1;
  color: var(--color-success, #34a853);
}
.copied-check {
  font-size: 11px;
  font-weight: bold;
}
.spark {
  flex-shrink: 0;
  width: 9px;
  height: 9px;
  margin-top: 7px;
  border-radius: var(--radius-full);
  background: linear-gradient(135deg, #89b4fa, #cba6f7);
  box-shadow: 0 0 6px rgba(137, 180, 250, 0.45);
}
.spark.breathing {
  animation: spark-breathe 1.6s ease-in-out infinite;
}
@keyframes spark-breathe {
  0%, 100% { transform: scale(1); box-shadow: 0 0 6px rgba(137, 180, 250, 0.45); }
  50% { transform: scale(1.35); box-shadow: 0 0 12px rgba(203, 166, 247, 0.7); }
}
.assistant-body {
  flex: 1;
  min-width: 0;
}
.msg-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 5px;
}
/* meta chip：贴气泡的弱化小胶囊，等宽数字 + 方向符号着色 */
.meta-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 7px;
  border-radius: var(--radius-full);
  border: 1px solid var(--color-border-light);
  background: var(--color-bg-secondary);
  font-size: 10px;
  font-family: var(--font-mono);
  line-height: 1.5;
  color: var(--color-text-muted);
  user-select: none;
}
.meta-chip.stopped {
  color: var(--color-warning);
  border-color: transparent;
  background: transparent;
  padding-left: 2px;
}
.stopped-dot {
  width: 5px;
  height: 5px;
  border-radius: 1px;
  background: var(--color-warning);
}
.usage-dir {
  font-size: 9px;
}
.usage-dir.in {
  color: var(--color-info);
}
.usage-dir.out {
  color: var(--color-role-assistant);
}
.meta-sep {
  width: 1px;
  height: 8px;
  background: var(--color-border);
  margin: 0 3px;
}
.kind-error {
  align-self: stretch;
  max-width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--color-danger);
  border-radius: var(--radius-md);
  background: var(--color-danger-bg, rgba(220, 60, 60, 0.08));
  color: var(--color-danger);
}
.activity-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--color-text-muted);
  font-size: 12px;
  padding: 2px 4px;
}
/* 02 Thinking & 05 Tool Chips 原语化卡片 */
.kind-process {
  align-self: flex-start;
  max-width: 100%;
}
.process-card {
  font-size: 12px;
  color: var(--color-text-muted);
}
.thinking-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: var(--radius-md, 8px);
  border: 1px solid var(--color-border-light, #e7ebf1);
  background: var(--color-bg-secondary, #f8fafc);
  color: var(--color-text-secondary, #48484a);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--transition-fast), border-color var(--transition-fast), color var(--transition-fast), box-shadow var(--transition-fast);
}
.thinking-pill:hover {
  background: var(--color-bg-hover, #f0f3f8);
  color: var(--color-text, #172033);
  border-color: var(--color-border);
}
.thinking-pill.active {
  background: var(--color-bg-tertiary, rgba(0, 122, 255, 0.05));
  border-color: rgba(0, 122, 255, 0.25);
  box-shadow: 0 1px 4px rgba(0, 122, 255, 0.08);
}
.thinking-spark-icon {
  color: var(--color-primary, #007aff);
  opacity: 0.85;
  flex-shrink: 0;
  transition: transform 0.2s ease, filter 0.2s ease;
}
.thinking-spark-icon.pulsing {
  animation: spark-pulse 1.4s ease-in-out infinite;
}
@keyframes spark-pulse {
  0%, 100% { transform: scale(1); opacity: 0.8; }
  50% { transform: scale(1.25); opacity: 1; filter: drop-shadow(0 0 3px var(--color-primary, #007aff)); }
}
.thinking-title {
  display: inline-flex;
  align-items: center;
  font-size: 12px;
  font-weight: 500;
}
.shimmer-text {
  background-image: linear-gradient(
    90deg,
    var(--color-text-muted, #8e8e93) 25%,
    var(--color-text, #172033) 50%,
    var(--color-text-muted, #8e8e93) 75%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer-text 1.6s linear infinite;
  font-weight: 500;
}
@keyframes shimmer-text {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}
.thinking-live-timer {
  font-family: var(--font-mono, monospace);
  font-size: 11.5px;
  color: var(--color-text-muted);
  font-variant-numeric: tabular-nums;
  margin-left: 5px;
}
.thinking-chevron {
  transition: transform 0.2s cubic-bezier(0.23, 1, 0.32, 1);
  opacity: 0.6;
  margin-left: 2px;
}
.thinking-pill.open .thinking-chevron {
  transform: rotate(180deg);
  opacity: 0.9;
}

.tool-chips-container {
  display: flex;
  flex-direction: column;
  gap: 5px;
  margin-top: 8px;
  padding: 4px 0 4px 12px;
  border-left: 2px solid var(--color-border-light, #e5eaf1);
}
.tool-chip-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--color-text-secondary);
  animation: step-in 0.15s ease-out;
}
@keyframes step-in {
  from { opacity: 0; transform: translateY(3px); }
  to { opacity: 1; transform: translateY(0); }
}
.chip-status-icon {
  width: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.step-check {
  color: var(--color-success, #34a853);
  font-size: 11px;
  font-weight: 600;
}
.step-dots {
  justify-content: center;
}
.step-dots i {
  width: 3px;
  height: 3px;
}
.chip-action-name {
  font-weight: 500;
  color: var(--color-text);
  font-size: 12px;
}
.chip-target-pill {
  display: inline-flex;
  align-items: center;
  padding: 1px 7px;
  border-radius: var(--radius-sm, 6px);
  background: var(--color-bg-secondary, #f8fafc);
  border: 1px solid var(--color-border-light, #e7ebf1);
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text-muted);
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chip-count-badge {
  font-size: 10px;
  font-family: var(--font-mono);
  color: var(--color-primary);
  background: var(--color-primary-light);
  padding: 1px 5px;
  border-radius: 4px;
  font-weight: 600;
}
/* 解说步骤（被工具打断的助手文本）：正文字体、更弱的对比度 */
.tool-chip-row.is-note .chip-action-name {
  font-weight: normal;
  color: var(--color-text-muted);
  opacity: 0.85;
}
.step-note-marker {
  color: var(--color-text-muted);
  font-size: 12px;
}
.thinking-dots {
  display: inline-flex;
  gap: 3px;
}
.thinking-dots i {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--color-text-muted);
  animation: thinking-bounce 1.2s infinite;
}
.thinking-dots i:nth-child(2) {
  animation-delay: 0.2s;
}
.thinking-dots i:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes thinking-bounce {
  0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
  30% { transform: translateY(-3px); opacity: 1; }
}
.bubble-enter-active {
  transition: opacity 0.15s ease-out, transform 0.15s ease-out;
}
.bubble-enter-from {
  opacity: 0;
  transform: translateY(4px);
}
.chat-list :deep(.code-copy-btn) {
  position: absolute;
  top: 4px;
  right: 4px;
  font-size: 11px;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  background: var(--color-bg-active);
  color: var(--color-text-secondary);
  opacity: 0;
  transition: opacity 0.15s;
}
.chat-list :deep(pre.hljs) {
  position: relative;
}
.chat-list :deep(pre.hljs:hover .code-copy-btn) {
  opacity: 1;
}
.markdown-body :deep(.session-ref) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 15px;
  padding: 0 4px;
  margin-left: 2px;
  border-radius: var(--radius-full);
  background: var(--color-bg-active);
  color: var(--color-text-muted);
  font-size: 10px;
  font-family: var(--font-mono);
  line-height: 1;
  cursor: pointer;
  user-select: none;
  vertical-align: baseline;
  transition: background var(--transition-fast), color var(--transition-fast);
}
.markdown-body :deep(.session-ref):hover {
  background: var(--color-primary-light);
  color: var(--color-primary);
}

/* 09 Recommendation 原语：追问建议药丸 */
.followup-pills {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 10px;
  animation: step-in 0.2s ease-out;
}
.followup-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid var(--color-border-light, #e7ebf1);
  background: var(--color-bg-secondary, #f8fafc);
  color: var(--color-text-secondary, #48484a);
  font-size: 11.5px;
  cursor: pointer;
  transition: all 0.15s ease;
}
.followup-pill:hover {
  border-color: var(--color-primary, #007aff);
  color: var(--color-primary, #007aff);
  background: var(--color-bg-hover, #f0f3f8);
  transform: translateY(-1px);
}
.followup-spark {
  font-size: 9px;
  color: var(--color-primary, #007aff);
  opacity: 0.85;
}
</style>
