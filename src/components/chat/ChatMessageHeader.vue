<script setup lang="ts">
import { computed } from "vue";
import CopyButton from "../CopyButton.vue";
import SvgIcon from "../icons/SvgIcon.vue";
import { formatTokenCount } from "../../utils/format";
import type { TokenUsage } from "../../types/session";

// 气泡式会话中渲染在气泡下方的 meta 行：时间戳 + 模型名 + Token/缓存指示 + 动作按钮
const props = defineProps<{
  role: string;
  timestamp: string;
  model?: string | null;
  tokenUsage?: TokenUsage | null;
  messageText: string;
  bookmarked: boolean;
  bookmarkDisabled?: boolean;
  forkAvailable?: boolean;
  proxyAvailable: boolean;
  apiActive: boolean;
  apiDisabled?: boolean;
}>();

defineEmits<{
  toggleBookmark: [];
  toggleApiDetail: [];
  forkFromHere: [];
}>();

const inputTokens = computed(() => props.tokenUsage?.input_tokens ?? 0);
const outputTokens = computed(() => props.tokenUsage?.output_tokens ?? 0);
const cacheRead = computed(() => props.tokenUsage?.cache_read_input_tokens ?? 0);
const cacheWrite = computed(() => props.tokenUsage?.cache_creation_input_tokens ?? 0);
const hasTokenUsage = computed(() => (
  inputTokens.value > 0 || outputTokens.value > 0 || cacheRead.value > 0 || cacheWrite.value > 0
));

const tokenTooltip = computed(() => {
  if (!props.tokenUsage) return "";
  const parts: string[] = [];
  if (inputTokens.value) parts.push(`输入 (上行): ${inputTokens.value.toLocaleString()}`);
  if (outputTokens.value) parts.push(`输出 (下行): ${outputTokens.value.toLocaleString()}`);
  if (cacheRead.value) {
    const totalIn = inputTokens.value + cacheRead.value;
    const rate = totalIn > 0 ? ((cacheRead.value / totalIn) * 100).toFixed(1) : "0";
    parts.push(`缓存读取: ${cacheRead.value.toLocaleString()} (${rate}% 命中)`);
  }
  if (cacheWrite.value) parts.push(`缓存写入: ${cacheWrite.value.toLocaleString()}`);
  return parts.join(" · ");
});
</script>

<template>
  <div class="message-header" :class="`role-${role}`">
    <span class="meta-timestamp">{{ timestamp }}</span>
    <span v-if="model && role === 'assistant'" class="meta-model" :title="`Model: ${model}`">
      {{ model }}
    </span>
    <span v-if="hasTokenUsage && role === 'assistant'" class="meta-tokens" :title="tokenTooltip">
      <span v-if="inputTokens || outputTokens" class="token-io">
        <span class="io-arrow">↑</span>{{ formatTokenCount(inputTokens) }}
        <span class="io-arrow">↓</span>{{ formatTokenCount(outputTokens) }}
      </span>
      <span v-if="cacheRead > 0" class="token-cache">
        <span class="cache-bolt">⚡</span>{{ formatTokenCount(cacheRead) }}
      </span>
    </span>
    <span class="meta-spacer" />
    <span class="meta-actions">
      <CopyButton :text="messageText" size="sm" />
      <button
        v-if="forkAvailable"
        class="meta-btn fork-btn"
        title="从这里继续 (Fork)"
        @click.stop="$emit('forkFromHere')"
      >
        <SvgIcon name="fork" :size="13" />
      </button>
      <button
        class="meta-btn bookmark-btn"
        :class="{ bookmarked }"
        :disabled="bookmarkDisabled"
        :title="bookmarkDisabled ? '正在解析会话身份' : (bookmarked ? '取消书签' : '添加书签')"
        @click.stop="$emit('toggleBookmark')"
      >
        <SvgIcon name="bookmark" :size="13" />
      </button>
      <button
        v-if="proxyAvailable && props.role === 'assistant'"
        class="meta-btn api-btn"
        :class="{ active: apiActive }"
        :disabled="apiDisabled"
        :title="apiDisabled ? '正在解析会话身份' : '查看 API 请求'"
        @click.stop="$emit('toggleApiDetail')"
      >
        <SvgIcon name="zap" :size="13" />
      </button>
    </span>
  </div>
</template>

<style scoped>
.message-header {
  -webkit-user-select: none;
  user-select: none;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-top: 2px;
  min-height: 24px;
  font-size: var(--text-2xs);
  width: 100%;
}

.role-user {
  justify-content: flex-end;
}

.role-assistant {
  justify-content: flex-start;
}

.meta-timestamp {
  color: var(--color-text-muted);
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.2px;
  flex-shrink: 0;
}

.meta-model {
  color: var(--color-text-muted);
  font-family: var(--font-mono);
  font-size: 10.5px;
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--color-border-light);
  max-width: min(440px, 45vw);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex-shrink: 1;
}

.meta-tokens {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm);
  padding: 1px 6px;
  white-space: nowrap;
  user-select: none;
  flex-shrink: 0;
}

.token-io {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.io-arrow {
  color: var(--color-text-tertiary, var(--color-text-muted));
  font-size: 10px;
}

.token-cache {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  color: var(--color-warning, #f59e0b);
  border-left: 1px solid var(--color-border-light);
  padding-left: 5px;
}

.cache-bolt {
  font-size: 9.5px;
}

.meta-spacer {
  flex: 1;
}

/* 动作栏常驻显示（复制/Fork/书签/API）， muted 配色保持低调 */
.meta-actions {
  display: flex;
  align-items: center;
  gap: 2px;
}

.meta-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  transition: all var(--transition-fast);
  cursor: pointer;
}

.meta-btn:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.bookmark-btn.bookmarked {
  color: var(--color-warning, #f59e0b);
  opacity: 1 !important;
}

.bookmark-btn.bookmarked:hover {
  color: var(--color-warning, #f59e0b);
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 12%, transparent);
}

.api-btn.active {
  color: var(--color-primary);
  opacity: 1 !important;
}

.api-btn.active:hover {
  color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
}

.meta-btn:disabled {
  cursor: default;
  opacity: 0.35;
}
</style>
