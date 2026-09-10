<script lang="ts">
import type { CliId } from "../types/cli";

export type DashboardMode = "assistant" | "search";

/** Dashboard 最近对话条目（由 App.vue 按 visibleCliIds 聚合排序后传入） */
export interface DashboardRecentConversation {
  cliId: CliId;
  filePath: string;
  encodedDir: string;
  projectPath: string;
  title: string;
  timestamp: string;
}

</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import type { SearchResult, SessionIdentity } from "../types/session";
import { isCliId, getCliDefinition } from "../types/cli";
import { formatRelativeTime } from "../utils/format";

const props = withDefaults(defineProps<{
  bootstrapping?: boolean;
  dataSourcePath?: string;
  showTracking?: boolean;
  showUsage?: boolean;
  showApiDebug?: boolean;
  showNewSession?: boolean;
  recentConversations?: DashboardRecentConversation[];
  activeAgentCount?: number;
  waitingAgentCount?: number;
  /** 已提交的搜索关键词（非空即展示内联结果区）；数据链路在 App.vue 复用 useSessions.globalSearch */
  searchQuery?: string;
  searchResults?: SearchResult[];
  searchLoading?: boolean;
  searchPendingCliIds?: string[];
  searchStaleCliIds?: string[];
  searchCliErrors?: Record<string, string>;
  buildingCliIds?: string[];
}>(), {
  bootstrapping: false,
  dataSourcePath: "",
  showTracking: false,
  showUsage: false,
  showApiDebug: false,
  showNewSession: true,
  recentConversations: () => [],
  activeAgentCount: 0,
  waitingAgentCount: 0,
  searchQuery: "",
  searchResults: () => [],
  searchLoading: false,
  searchPendingCliIds: () => [],
  searchStaleCliIds: () => [],
  searchCliErrors: () => ({}),
  buildingCliIds: () => [],
});

const emit = defineEmits<{
  submitAssistant: [prompt: string];
  submitSearch: [query: string];
  openSession: [identity: SessionIdentity, encodedDir: string];
  openSearchResult: [result: SearchResult];
  openFullSearch: [query: string];
  clearSearch: [];
  buildPendingSearchIndex: [cliId: string];
  buildAllPendingSearchIndexes: [cliIds: string[]];
  newSession: [];
  openHistory: [];
  openAgent: [];
  openAssistant: [];
  openTracking: [];
  openUsage: [];
    openApiDebug: [];
}>();

const mode = ref<DashboardMode>("assistant");
const draft = ref("");
const promptInputRef = ref<HTMLInputElement | null>(null);
const userName = ref("");

/** ⌘⇧F 落点：Dashboard 可见时切入搜索模式并聚焦主输入框（由 App.vue 快捷键调用） */
function focusSearch() {
  mode.value = "search";
  nextTick(() => promptInputRef.value?.focus());
}
defineExpose({ focusSearch });

const cliNameOf = (id: string) => (isCliId(id) ? getCliDefinition(id).name : id);
const searchCliErrorList = computed(() => Object.entries(props.searchCliErrors || {}));
const isAnyPendingBuilding = computed(() =>
  (props.searchPendingCliIds || []).some((id) => props.buildingCliIds?.includes(id))
);
const isAnyStaleBuilding = computed(() =>
  (props.searchStaleCliIds || []).some((id) => props.buildingCliIds?.includes(id))
);

const placeholder = computed(() =>
  mode.value === "assistant"
    ? "问 SessionDock 关于你的项目、对话或下一步…"
    : "搜索所有对话内容…",
);

function onSubmit() {
  const text = draft.value.trim();
  if (!text) return;
  if (mode.value === "assistant") {
    emit("submitAssistant", text);
  } else {
    emit("submitSearch", text);
  }
  draft.value = "";
}

function onOpenRecent(item: DashboardRecentConversation) {
  emit("openSession", { cliId: item.cliId, filePath: item.filePath }, item.encodedDir);
}

function projectLabel(projectPath: string) {
  return projectPath.split(/[/\\]/).filter(Boolean).pop() ?? projectPath;
}

const hasAgentStatus = computed(() => props.activeAgentCount > 0 || props.waitingAgentCount > 0);

// ─── 用户名与 Token KPI 摘要（优先缓存 + 后台刷新，未登录提供引导） ───





async function loadUserName() {
  try {
    const name = await invoke<string>("get_current_user_name");
    if (name && name.trim()) {
      userName.value = name.trim();
    }
  } catch {
    // 忽略异常
  }
}



const greetingPrefix = computed(() => {
  const hour = new Date().getHours();
  if (hour >= 5 && hour < 12) return "早上好";
  if (hour >= 12 && hour < 18) return "下午好";
  return "晚上好";
});

const greetingText = computed(() => {
  const prefix = greetingPrefix.value;
  const name = userName.value.trim();
  if (name) {
    return `${prefix}，${name}，今天想从哪里开始？`;
  }
  return `${prefix}，今天想从哪里开始？`;
});


onMounted(async () => {
  void loadUserName();

  try {
  } catch {
    // 单元测试或非 Tauri 环境容错
  }
});

onUnmounted(() => {
});
</script>

<template>
  <div class="dashboard">
    <div class="dashboard-inner">
      <Transition name="banner-slide">
        <div v-if="bootstrapping" class="startup-banner">
          <span class="spinner" />
          <div class="banner-text">
            <strong>正在初始化</strong>
            <span>首次启动正在扫描对话，请稍候...</span>
          </div>
        </div>
      </Transition>

      <header class="hero">
        <div class="hero-eyebrow">
          <span>{{ greetingText }}</span>
        </div>
        <h1 class="hero-title">找到上下文，然后继续前进。</h1>
        <p class="hero-sub">浏览所有 CLI 的历史对话，或直接交给 SessionDock。</p>
      </header>

      <div class="mode-switch" role="tablist" aria-label="输入模式">
        <button
          type="button"
          role="tab"
          class="mode-btn"
          :class="{ active: mode === 'assistant' }"
          :aria-selected="mode === 'assistant'"
          @click="mode = 'assistant'"
        >
          问 SessionDock
        </button>
        <button
          type="button"
          role="tab"
          class="mode-btn"
          :class="{ active: mode === 'search' }"
          :aria-selected="mode === 'search'"
          @click="mode = 'search'"
        >
          全局搜索
        </button>
      </div>

      <form class="prompt-box" @submit.prevent="onSubmit">
        <input
          ref="promptInputRef"
          v-model="draft"
          class="prompt-input"
          type="text"
          :placeholder="placeholder"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
        />
        <button type="submit" class="prompt-send" :disabled="!draft.trim()" aria-label="发送">↑</button>
      </form>
      <div class="prompt-hint">
        <span>Enter 发送</span>
        <span class="prompt-hint-divider">·</span>
        <span>⌘ ⇧ F 切换到全局搜索</span>
      </div>

      <section v-if="searchQuery" class="search-inline" aria-label="搜索结果">
        <div class="search-inline-header">
          <h2 class="section-title">「{{ searchQuery }}」的搜索结果</h2>
          <div class="search-inline-actions">
            <button type="button" class="search-full-link" @click="emit('openFullSearch', searchQuery)">
              在完整搜索页中查看
            </button>
            <button type="button" class="search-clear-btn" @click="emit('clearSearch')">清除</button>
          </div>
        </div>
        <div v-if="searchLoading" class="search-inline-status">搜索中…</div>
        <div v-else-if="searchResults.length === 0" class="search-inline-status">
          未找到匹配结果，可在完整搜索页中建立索引后重试
        </div>
        <ul v-else class="search-inline-list">
          <li v-for="result in searchResults" :key="`${result.cli_id}:${result.file_path}`">
            <button type="button" class="search-result-row" @click="emit('openSearchResult', result)">
              <ChatAvatar role="assistant" :cliId="result.cli_id" class="search-result-cli" />
              <span class="search-result-main">
                <span class="search-result-title">
                  {{ result.display_name }}
                  <small class="search-result-badge">{{ cliNameOf(result.cli_id) }}</small>
                </span>
                <span class="search-result-meta">
                  {{ projectLabel(result.project_path) }} · {{ result.match_count }} 处匹配
                </span>
                <span class="search-result-snippet">{{ result.snippet }}</span>
              </span>
            </button>
          </li>
        </ul>

        <!-- 未索引 CLI 提示与一键构建 -->
        <div v-if="searchPendingCliIds.length && !searchLoading" class="search-inline-hint pending-cli-hint">
          <div class="hint-text-wrap">
            <SvgIcon name="info" :size="14" class="hint-icon" />
            <span>{{ searchPendingCliIds.map(cliNameOf).join("、") }} 尚未建立内容索引，当前结果可能不完整</span>
          </div>
          <div class="pending-cli-actions">
            <button
              v-if="searchPendingCliIds.length > 1"
              type="button"
              class="index-btn primary"
              :disabled="isAnyPendingBuilding"
              @click="emit('buildAllPendingSearchIndexes', searchPendingCliIds)"
            >
              一键构建全部 ({{ searchPendingCliIds.length }})
            </button>
            <button
              v-for="id in searchPendingCliIds"
              :key="id"
              type="button"
              class="index-btn secondary"
              :disabled="buildingCliIds?.includes(id)"
              @click="emit('buildPendingSearchIndex', id)"
            >
              {{ buildingCliIds?.includes(id) ? `正在构建 ${cliNameOf(id)}…` : searchPendingCliIds.length > 1 ? `仅 ${cliNameOf(id)}` : `构建 ${cliNameOf(id)} 索引` }}
            </button>
          </div>
        </div>

        <!-- 索引过期提示与一键更新 -->
        <div v-else-if="searchStaleCliIds.length && !searchLoading" class="search-inline-hint pending-cli-hint">
          <div class="hint-text-wrap">
            <SvgIcon name="info" :size="14" class="hint-icon" />
            <span>{{ searchStaleCliIds.map(cliNameOf).join("、") }} 的索引有更新未同步，结果可能缺少最新内容</span>
          </div>
          <div class="pending-cli-actions">
            <button
              v-if="searchStaleCliIds.length > 1"
              type="button"
              class="index-btn primary"
              :disabled="isAnyStaleBuilding"
              @click="emit('buildAllPendingSearchIndexes', searchStaleCliIds)"
            >
              一键更新全部 ({{ searchStaleCliIds.length }})
            </button>
            <button
              v-for="id in searchStaleCliIds"
              :key="id"
              type="button"
              class="index-btn secondary"
              :disabled="buildingCliIds?.includes(id)"
              @click="emit('buildPendingSearchIndex', id)"
            >
              {{ buildingCliIds?.includes(id) ? `正在更新 ${cliNameOf(id)}…` : searchStaleCliIds.length > 1 ? `仅 ${cliNameOf(id)}` : `更新 ${cliNameOf(id)} 索引` }}
            </button>
          </div>
        </div>

        <!-- 搜索错误提示 -->
        <div v-if="searchCliErrorList.length && !searchLoading" class="search-inline-hint search-cli-error-hint">
          <span v-for="[id, errorMessage] in searchCliErrorList" :key="id">
            {{ cliNameOf(id) }} 搜索失败：{{ errorMessage }}
          </span>
        </div>
      </section>

      <!-- 2. 最近对话全宽列表 -->
      <section v-if="recentConversations.length > 0" class="recent-section" aria-label="最近对话">
        <h2 class="section-title">最近对话</h2>
        <ul class="recent-list">
          <li v-for="item in recentConversations" :key="`${item.cliId}:${item.filePath}`">
            <button type="button" class="recent-row" @click="onOpenRecent(item)">
              <ChatAvatar role="assistant" :cliId="item.cliId" class="recent-cli" />
              <span class="recent-main">
                <span class="recent-title">{{ item.title }}</span>
                <span class="recent-meta">{{ projectLabel(item.projectPath) }}</span>
              </span>
              <span class="recent-time">{{ formatRelativeTime(item.timestamp) }}</span>
            </button>
          </li>
        </ul>
      </section>

      <div v-if="hasAgentStatus" class="agent-status-line">
        <span>
          {{ activeAgentCount }} 个 Agent 运行中<template v-if="waitingAgentCount > 0"> · {{ waitingAgentCount }} 等待输入</template>
        </span>
        <button type="button" class="agent-status-link" @click="emit('openAgent')">查看</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dashboard {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  justify-content: center;
  padding: 40px var(--space-6) 60px;
}
.dashboard-inner {
  width: 100%;
  max-width: 790px;
  min-height: 100%;
  display: flex;
  flex-direction: column;
}

.banner-slide-enter-active,
.banner-slide-leave-active {
  transition: opacity 180ms ease, transform 200ms ease, max-height 200ms ease;
  overflow: hidden;
}
.banner-slide-enter-from,
.banner-slide-leave-to { opacity: 0; transform: translateY(-6px); max-height: 0; }
.banner-slide-enter-to,
.banner-slide-leave-from { opacity: 1; transform: translateY(0); max-height: 80px; }
.startup-banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  background: var(--color-bg-secondary);
  margin-bottom: var(--space-4);
}
.spinner {
  width: 15px;
  height: 15px;
  border-radius: var(--radius-full);
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
.banner-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.banner-text strong { color: var(--color-text); font-size: var(--text-sm); }

.hero {
  margin-top: 30px;
}
.hero-eyebrow {
  display: flex;
  align-items: center;
  font-size: 14px;
  color: var(--color-text-secondary, #65758f);
  margin-bottom: 0;
}
.hero-title {
  margin: 16px 0 8px;
  font-family: var(--font-sans);
  font-size: 34px;
  line-height: 1.2;
  font-weight: 700;
  letter-spacing: -0.03em;
  color: var(--color-text, #172033);
}
.hero-sub {
  margin: 0;
  font-size: 15px;
  color: var(--color-text-muted, #76839a);
  line-height: 1.5;
}

.mode-switch {
  display: flex;
  gap: 5px;
  background: var(--color-bg-secondary, #f3f5f8);
  width: max-content;
  padding: 4px;
  border-radius: 10px;
  margin-top: 28px;
  border: 1px solid var(--color-border-light, #e7ebf1);
}
.mode-btn {
  border: 0;
  background: transparent;
  padding: 7px 15px;
  border-radius: 7px;
  color: var(--color-text-muted, #69768b);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast), box-shadow var(--transition-fast);
}
.mode-btn:hover {
  color: var(--color-text, #1d2a40);
}
.mode-btn.active {
  background: var(--color-bg, #fff);
  color: var(--color-text, #1d2a40);
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(36, 52, 80, 0.1);
}

.prompt-box {
  margin-top: 10px;
  border: 1px solid var(--color-border, #dbe2ec);
  border-radius: 14px;
  background: var(--color-bg, #fff);
  padding: 10px 10px 10px 16px;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  box-shadow: 0 10px 30px rgba(44, 65, 96, 0.04);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.prompt-box:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.prompt-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-size: 14px;
  font-family: var(--font-sans);
  color: var(--color-text, #172033);
}
.prompt-input::placeholder {
  color: var(--color-text-muted, #9aa5b5);
}
.prompt-send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 8px;
  background: var(--color-text, #172033);
  color: var(--color-bg, #fff);
  font-size: 14px;
  font-weight: 700;
  cursor: pointer;
  flex-shrink: 0;
  transition: opacity var(--transition-fast);
}
.prompt-send:disabled {
  opacity: 0.35;
  cursor: default;
}
.prompt-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 4px 0;
  font-size: 11px;
  color: var(--color-text-muted, #a1acbb);
}
.prompt-hint-divider {
  opacity: 0.5;
}

.search-inline {
  margin-top: var(--space-4);
}
.search-inline-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}
.search-inline-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}
.search-full-link,
.search-clear-btn {
  border: 0;
  background: transparent;
  padding: 0;
  font-size: var(--text-xs);
  cursor: pointer;
}
.search-full-link {
  color: var(--color-primary);
}
.search-clear-btn {
  color: var(--color-text-muted);
}
.search-clear-btn:hover {
  color: var(--color-text);
}
.search-inline-status {
  padding: var(--space-4) var(--space-1);
  font-size: var(--text-sm);
  color: var(--color-text-muted);
}
.search-inline-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--color-border-light, #ebeff4);
}
.search-result-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  width: 100%;
  padding: var(--space-3) var(--space-1);
  border: 0;
  border-bottom: 1px solid var(--color-border-light, #ebeff4);
  border-radius: var(--radius-lg);
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}
.search-result-row:hover {
  background: var(--color-bg-hover, #f5f7fa);
}
.search-result-cli {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  margin-top: 1px;
}
.search-result-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.search-result-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.search-result-badge {
  flex-shrink: 0;
  padding: 0 var(--space-1);
  border-radius: var(--radius-sm);
  font-size: var(--text-2xs);
  line-height: 16px;
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
}
.search-result-meta {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.search-result-snippet {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: var(--leading-normal);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.search-inline-hint {
  margin-top: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-lg);
  font-size: var(--text-xs);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.search-inline-hint.pending-cli-hint {
  background: var(--color-bg-secondary);
  border: 1px dashed var(--color-border);
  color: var(--color-text-secondary);
}
.search-inline-hint.search-cli-error-hint {
  background: var(--color-bg-danger-subtle, rgba(239, 68, 68, 0.1));
  color: var(--color-danger);
  border: 1px solid var(--color-danger-border, rgba(239, 68, 68, 0.2));
}
.hint-text-wrap {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.hint-icon {
  flex-shrink: 0;
  color: var(--color-primary);
}
.pending-cli-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.index-btn {
  height: 26px;
  padding: 0 var(--space-3);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  font-weight: 500;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all var(--transition-fast);
}
.index-btn.primary {
  background: var(--color-primary);
  color: var(--color-text-inverse, #fff);
}
.index-btn.primary:hover:not(:disabled) {
  background: var(--color-primary-hover);
}
.index-btn.secondary {
  color: var(--color-text-secondary);
  background: var(--color-bg, #fff);
  border-color: var(--color-border);
}
.index-btn.secondary:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.index-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

/* ─── 1. 今日 Token 概览横向条 ─── */
.kpi-section {
  margin-top: 28px;
}
.kpi-horizontal-card {
  display: flex;
  align-items: center;
  width: 100%;
  padding: 14px 22px;
  border: 1px solid var(--color-border-light, #e5eaf1);
  border-radius: 14px;
  background: var(--color-bg, #fff);
  box-shadow: 0 1px 3px rgba(36, 52, 80, 0.03);
  cursor: pointer;
  text-align: left;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.kpi-horizontal-card:hover {
  border-color: var(--color-border);
  box-shadow: 0 4px 12px rgba(36, 52, 80, 0.06);
}
.kpi-col {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.kpi-label {
  font-size: 11px;
  color: var(--color-text-muted, #8794a7);
  white-space: nowrap;
}
.kpi-val {
  font-size: 18px;
  font-weight: 700;
  letter-spacing: -0.02em;
  color: var(--color-text, #172033);
  white-space: nowrap;
}
.kpi-divider {
  width: 1px;
  height: 26px;
  background: var(--color-border-light, #edf0f4);
  margin: 0 20px;
  flex-shrink: 0;
}
.kpi-updated {
  font-size: 11px;
  color: var(--color-text-muted, #a1acbb);
  margin-left: auto;
  padding-left: 12px;
  white-space: nowrap;
  flex-shrink: 0;
}

.kpi-login-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 20px;
  border: 1px dashed var(--color-border, #e5eaf1);
  border-radius: 14px;
  background: var(--color-bg-secondary, #f8fafc);
}
.kpi-login-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.kpi-login-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text, #172033);
}
.kpi-login-desc {
  margin: 0;
  font-size: 12px;
  color: var(--color-text-muted, #76839a);
  line-height: 1.4;
}
.kpi-login-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 7px 18px;
  border: 1px solid var(--color-border, #dce3ee);
  border-radius: 8px;
  background: var(--color-bg, #fff);
  color: var(--color-text, #172033);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--transition-fast), border-color var(--transition-fast);
}
.kpi-login-btn:hover {
  background: var(--color-bg-hover, #f0f3f7);
  border-color: var(--color-border-hover, #cbd5e1);
}

/* ─── 2. 最近对话全宽列表 ─── */
.recent-section {
  margin-top: 32px;
}
.section-title {
  font-size: 14px;
  font-weight: 650;
  margin: 0 0 12px;
  color: var(--color-text, #172033);
}
.recent-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--color-border-light, #ebeff4);
}
.recent-row {
  display: grid;
  grid-template-columns: 24px 1fr auto;
  gap: 12px;
  align-items: center;
  border: 0;
  border-bottom: 1px solid var(--color-border-light, #ebeff4);
  padding: 13px 4px;
  border-radius: 8px;
  width: 100%;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}
.recent-row:hover {
  background: var(--color-bg-hover, #f5f7fa);
}
.recent-cli {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
}
.recent-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.recent-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text, #27344a);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.recent-meta {
  font-size: 11px;
  color: var(--color-text-muted, #9aa6b6);
}
.recent-time {
  font-size: 11px;
  color: var(--color-text-muted, #9aa6b6);
  flex-shrink: 0;
  white-space: nowrap;
}

.agent-status-line {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  margin-top: var(--space-6);
}
.agent-status-link {
  border: 0;
  background: transparent;
  color: var(--color-primary);
  font-size: var(--text-xs);
  cursor: pointer;
  padding: 0;
}

@media (max-width: 600px) {
  .dashboard {
    padding: var(--space-4);
  }
  .kpi-horizontal-card {
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    padding: 14px;
  }
  .kpi-divider {
    display: none;
  }
  .kpi-updated {
    margin-left: 0;
    padding-left: 0;
  }
}
</style>
