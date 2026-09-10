<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
import { useSessions } from "../composables/useSessions";
import { useAutoIndex, AUTO_INDEX_OPTIONS, type AutoIndexInterval } from "../composables/useAutoIndex";
import { useIndexRebuildQueue } from "../composables/useIndexRebuildQueue";
import { resolveFeatureCliId } from "../composables/cliFilter";
import { isCliId, type CliId } from "../types/cli";
import { formatBytes, formatRelativeTime } from "../utils/format";
import { indexProgressPercent, indexProgressPhaseLabel } from "../utils/indexProgress";

interface IndexStats {
  sessionCount: number;
  dbSizeBytes: number;
  searchDocCount: number;
  searchIndexBytes: number;
  lastUpdatedMs: number | null;
}

const props = withDefaults(
  defineProps<{
    initialCliId?: CliId;
    cliId?: CliId;
    mode?: "all" | "global" | "cli";
  }>(),
  { mode: "all" },
);
const emit = defineEmits<{ "update:cliId": [cliId: CliId] }>();
const { cliOptions, searchIndexProgress, refresh } = useSessions();
const { autoIndexInterval, setAutoIndexInterval } = useAutoIndex();
const {
  running: rebuildAllRunning,
  total: rebuildTotal,
  currentIndex: rebuildCurrentIndex,
  currentCliId: rebuildCurrentCliId,
  currentCliName: rebuildCurrentCliName,
  overallPercent: rebuildOverallPercent,
  isQueued: isRebuildQueued,
  startRebuildAll,
} = useIndexRebuildQueue();
const FEATURE_CLI_STORAGE_KEY = "claudia-feature-cli-index";
const persistedCliId = localStorage.getItem(FEATURE_CLI_STORAGE_KEY);

// 索引没有独立 capability 标记；有对话数据即表示该 CLI 可建立索引。
const featureCliOptions = computed(() => cliOptions.value.filter((cli) => cli.hasSessions));
const cliSelectOptions = computed<SelectOption[]>(() =>
  featureCliOptions.value.map((cli) => ({
    value: cli.id,
    label: cli.name,
    cliId: cli.id,
  })),
);

const autoIndexSelectOptions = computed<SelectOption[]>(() =>
  AUTO_INDEX_OPTIONS.map((opt) => ({
    value: opt.value,
    label: opt.label,
  })),
);

const featureCliId = ref<CliId | undefined>(resolveFeatureCliId(
  featureCliOptions.value,
  props.initialCliId,
  persistedCliId && isCliId(persistedCliId) ? persistedCliId : undefined,
));

const stats = ref<IndexStats | null>(null);
const allCliStats = ref<Record<string, IndexStats>>({});
const loadError = ref("");
const rebuilding = ref(false);
const clearing = ref(false);

const isIndexingActive = computed(() => {
  return rebuilding.value || rebuildAllRunning.value || clearing.value || Boolean(searchIndexProgress.value);
});

const progressPercent = computed(() => {
  const progress = searchIndexProgress.value;
  if (!progress) return 0;
  return indexProgressPercent(progress);
});

const progressPhaseText = computed(() => {
  const progress = searchIndexProgress.value;
  if (!progress) return "";
  return indexProgressPhaseLabel(progress.phase);
});

// 全局重建主进度条：仅全局/all 卡可见，且只在队列运行时出现
const showGlobalProgress = computed(
  () => rebuildAllRunning.value && props.mode !== "cli",
);

const rebuildPhaseText = computed(() => {
  const progress = searchIndexProgress.value;
  if (progress && progress.cliId === rebuildCurrentCliId.value) {
    return indexProgressPhaseLabel(progress.phase);
  }
  return "准备中";
});

// CLI 卡底部进度条：单 CLI 重建或后台增量同步时保留，
// 但只展示与当前选中 CLI 匹配的进度，全局重建期间让位给主进度条
const showCliProgress = computed(() => {
  if (rebuildAllRunning.value || props.mode === "global") return false;
  const progress = searchIndexProgress.value;
  if (!progress) return false;
  if (props.mode === "cli") return progress.cliId === featureCliId.value;
  return true;
});

const isCurrentQueueTarget = computed(
  () => Boolean(featureCliId.value) && rebuildCurrentCliId.value === featureCliId.value,
);

function getCliStatusBadge(cliId: CliId | undefined): {
  label: string;
  type: "success" | "warning" | "danger" | "muted" | "info";
  spinning: boolean;
} {
  if (!cliId) return { label: "", type: "muted", spinning: false };
  if (rebuildCurrentCliId.value === cliId) {
    return { label: "正在建立索引…", type: "info", spinning: true };
  }
  if (isRebuildQueued(cliId)) {
    return { label: "等待队列中", type: "muted", spinning: false };
  }
  return { ...getCliCompleteness(cliId), spinning: false };
}

function getCliCompleteness(cliId: string): { label: string; type: "success" | "warning" | "danger" | "muted" } {
  const s = allCliStats.value[cliId] ?? (cliId === featureCliId.value ? stats.value : null);
  if (!s) return { label: "查询中…", type: "muted" };
  if (s.sessionCount === 0) return { label: "无会话数据", type: "muted" };
  if (s.searchDocCount === 0) return { label: "未建立索引", type: "danger" };
  if (s.searchDocCount < s.sessionCount) {
    return { label: `未完全同步 (${s.searchDocCount}/${s.sessionCount})`, type: "warning" };
  }
  return { label: `已就绪 (${s.sessionCount}/${s.sessionCount})`, type: "success" };
}

function lastUpdatedText(): string {
  // 索引为空（0 文档）时大小与时间都没有意义，统一显示占位符
  if (!stats.value || stats.value.searchDocCount === 0) return "—";
  const ms = stats.value.lastUpdatedMs;
  if (!ms) return "从未";
  return formatRelativeTime(new Date(ms).toISOString());
}

// 搜索索引的 tantivy 物理目录为全 CLI 共享：
// 字节数各 CLI 相同任取一份，文档数按 CLI 求和（全局卡展示）
const globalIndexSummary = computed(() => {
  const entries = Object.values(allCliStats.value);
  if (entries.length === 0) return null;
  const totalDocs = entries.reduce((sum, s) => sum + s.searchDocCount, 0);
  const bytes = entries[0]?.searchIndexBytes ?? 0;
  return { totalDocs, bytes };
});

const compactStatText = computed(() => {
  const s = stats.value;
  if (!s || s.searchDocCount === 0) return "—";
  return `${s.sessionCount.toLocaleString()} 对话 · ${s.searchDocCount.toLocaleString()} 文档 · 更新于 ${lastUpdatedText()}`;
});

async function loadStats() {
  if (!featureCliId.value) {
    stats.value = null;
    return;
  }
  try {
    const res = await invoke<IndexStats>("get_index_stats", {
      cliId: featureCliId.value,
    });
    stats.value = res;
    if (featureCliId.value) allCliStats.value[featureCliId.value] = res;
    loadError.value = "";
  } catch (e: any) {
    loadError.value = e?.message ?? String(e);
  }
}

async function loadAllStats() {
  const promises = featureCliOptions.value.map(async (cli) => {
    try {
      const s = await invoke<IndexStats>("get_index_stats", { cliId: cli.id });
      allCliStats.value[cli.id] = s;
    } catch {}
  });
  await Promise.allSettled(promises);
}

async function rebuildIndex() {
  if (isIndexingActive.value || !featureCliId.value) return;
  rebuilding.value = true;
  try {
    await invoke("clear_session_index", { cliId: featureCliId.value });
    await invoke("refresh_session_list_index", {
      cliId: featureCliId.value,
      notify: false,
      force: true,
    });
    await invoke("ensure_search_index_ready", {
      cliId: featureCliId.value,
    });
    await loadStats();
    await loadAllStats();
    await refresh("snapshot");
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "重建索引失败", kind: "error" });
  } finally {
    rebuilding.value = false;
  }
}

async function rebuildAllIndexes() {
  if (isIndexingActive.value) return;
  const clisToRebuild = featureCliOptions.value.map((cli) => ({
    id: cli.id,
    name: cli.name,
  }));
  if (clisToRebuild.length === 0) return;

  try {
    await startRebuildAll(clisToRebuild);
    await loadStats();
    await loadAllStats();
    await refresh("snapshot");
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "重建所有索引失败", kind: "error" });
  }
}

async function clearIndex() {
  if (isIndexingActive.value || !featureCliId.value) return;
  const confirmed = await ask(
    "将清空所选 CLI 的列表索引与搜索索引（收藏、书签、自定义命名不受影响）。确定继续吗？",
    { title: "清除索引", kind: "warning" }
  );
  if (!confirmed) return;
  // 弹窗期间按钮未禁用，可能已有其他索引任务激活，确认后复查避免并发
  if (isIndexingActive.value || !featureCliId.value) return;
  clearing.value = true;
  try {
    await invoke("clear_session_index", { cliId: featureCliId.value });
    await loadStats();
    await loadAllStats();
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "清除索引失败", kind: "error" });
  } finally {
    clearing.value = false;
  }
}

let unlisten: UnlistenFn | null = null;
onMounted(async () => {
  unlisten = await listen<{ cliId?: string }>("session-list-index-updated", (event) => {
    if (event.payload?.cliId && event.payload.cliId !== featureCliId.value) return;
    loadStats();
    loadAllStats();
  });
  await loadStats();
  await loadAllStats();
  // 挂载时若内部解析值与 v-model prop 不一致（如无会话 CLI 兜底），向上同步一次
  if (featureCliId.value && featureCliId.value !== props.cliId) {
    emit("update:cliId", featureCliId.value);
  }
});

watch(searchIndexProgress, (val, prevVal) => {
  if (!val && prevVal) {
    loadStats();
    loadAllStats();
  }
});

watch(featureCliOptions, (candidates) => {
  if (featureCliId.value && candidates.some((cli) => cli.id === featureCliId.value)) return;
  featureCliId.value = resolveFeatureCliId(candidates, props.initialCliId, featureCliId.value);
});

watch(featureCliId, (cliId) => {
  if (cliId) localStorage.setItem(FEATURE_CLI_STORAGE_KEY, cliId);
  if (cliId && cliId !== props.cliId) emit("update:cliId", cliId);
  stats.value = null;
  void loadStats();
});

watch(
  () => props.cliId,
  (cliId) => {
    if (cliId && cliId !== featureCliId.value) featureCliId.value = cliId;
  },
);

watch(
  () => props.initialCliId,
  (entryCliId) => {
    featureCliId.value = resolveFeatureCliId(
      featureCliOptions.value,
      entryCliId,
      featureCliId.value,
    );
  },
);

onBeforeUnmount(() => {
  unlisten?.();
});
</script>

<template>
  <div class="index-settings">
    <!-- 全局策略与维护（mode === 'global' 或 'all'） -->
    <template v-if="mode === 'global' || mode === 'all'">
      <div class="setting-row">
        <div class="row-info">
          <span class="row-title">自动增量同步</span>
          <p class="row-desc">后台定时扫描增量会话并同步全文索引，避免搜索时提示更新</p>
        </div>
        <div class="row-control">
          <ElegantSelect
            :model-value="autoIndexInterval"
            :options="autoIndexSelectOptions"
            data-testid="auto-index-select"
            @update:model-value="setAutoIndexInterval($event as AutoIndexInterval)"
          />
        </div>
      </div>

      <div class="setting-row">
        <div class="row-info">
          <span class="row-title">搜索索引（全部 CLI 共享）</span>
          <p class="row-desc">
            {{ globalIndexSummary ? `${formatBytes(globalIndexSummary.bytes)} · ${globalIndexSummary.totalDocs.toLocaleString()} 条文档` : "查询中…" }}
          </p>
        </div>
        <div class="row-control-action">
          <button
            type="button"
            class="btn-action"
            :disabled="isIndexingActive"
            title="依次重算所有已激活 CLI 的全文索引，适用于排查全局搜索异常"
            @click="rebuildAllIndexes"
          >
            <SvgIcon v-if="rebuildAllRunning" name="loader" :size="13" class="spin-icon" />
            <SvgIcon v-else name="refresh-cw" :size="13" />
            <span>{{ rebuildAllRunning ? `重建中 (${rebuildCurrentIndex + 1}/${rebuildTotal})` : "重建全部索引" }}</span>
          </button>
        </div>
      </div>
    </template>

    <!-- 单 CLI 状态详情与专项维护（mode === 'cli' 或 'all'） -->
    <template v-if="mode === 'cli' || mode === 'all'">
      <div class="setting-row cli-picker-row">
        <div class="row-info">
          <span class="row-title">查看 CLI 状态</span>
          <p class="row-desc">切换查看不同 CLI 的索引指标并进行单项维护</p>
        </div>
        <div class="row-control">
          <ElegantSelect
            v-model="featureCliId"
            :options="cliSelectOptions"
            data-testid="index-cli-select"
          />
        </div>
      </div>

      <div class="cli-status-section">
        <div class="status-row">
          <span class="status-label">索引状态</span>
          <span
            v-if="featureCliId"
            class="completeness-badge"
            :class="getCliStatusBadge(featureCliId).type"
          >
            <SvgIcon
              v-if="getCliStatusBadge(featureCliId).spinning"
              name="loader"
              :size="11"
              class="spin-icon"
            />
            {{ getCliStatusBadge(featureCliId).label }}
          </span>
          <span v-else class="text-muted">-</span>
          <div class="status-actions">
            <button
              type="button"
              class="btn-action"
              :disabled="isIndexingActive"
              title="清空旧索引并全量重算该 CLI 的所有会话（不影响收藏、书签与重命名）"
              @click="rebuildIndex"
            >
              <SvgIcon v-if="rebuilding" name="loader" :size="13" class="spin-icon" />
              <SvgIcon v-else name="search" :size="13" />
              <span>{{ rebuilding ? "重建中…" : "重建" }}</span>
            </button>
            <button
              type="button"
              class="btn-danger-outline"
              :disabled="isIndexingActive"
              title="清空该 CLI 的本地索引缓存"
              @click="clearIndex"
            >
              <SvgIcon v-if="clearing" name="loader" :size="13" class="spin-icon" />
              <SvgIcon v-else name="trash-2" :size="13" />
              <span>{{ clearing ? "清除中…" : "清除" }}</span>
            </button>
          </div>
        </div>
        <p class="status-line">{{ compactStatText }}</p>
        <p v-if="isCurrentQueueTarget" class="section-hint">
          当前 CLI 正在随全局任务重建中
        </p>
      </div>
    </template>

    <p v-if="loadError" class="error-banner">{{ loadError }}</p>

    <!-- 全局重建主进度条 -->
    <Transition name="progress-fade">
      <div v-if="showGlobalProgress" class="progress-box">
        <div class="progress-info">
          <span class="progress-label">
            <SvgIcon name="loader" :size="13" class="spin-icon" />
            正在重建全部索引 ({{ rebuildCurrentIndex + 1 }}/{{ rebuildTotal }})
          </span>
          <span class="progress-percent">{{ rebuildOverallPercent }}%</span>
        </div>
        <div class="progress-track">
          <div class="progress-bar" :style="{ width: `${rebuildOverallPercent}%` }"></div>
        </div>
        <p class="progress-sub">当前: {{ rebuildCurrentCliName }} · {{ rebuildPhaseText }}</p>
      </div>
      <!-- 单 CLI / 后台增量同步进度条 -->
      <div v-else-if="showCliProgress" class="progress-box">
        <div class="progress-info">
          <span class="progress-label">
            <SvgIcon name="loader" :size="13" class="spin-icon" />
            {{ progressPhaseText }}
          </span>
          <span class="progress-percent">{{ progressPercent }}%</span>
        </div>
        <div class="progress-track">
          <div class="progress-bar" :style="{ width: `${progressPercent}%` }"></div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.index-settings {
  display: flex;
  flex-direction: column;
}

/* 通用设置行 */
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4, 16px) var(--space-5, 20px);
  gap: var(--space-4, 16px);
}

.setting-row + .setting-row {
  border-top: 1px solid var(--color-border);
}

.row-info {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}

.row-title {
  font-size: var(--text-sm, 13px);
  font-weight: 500;
  color: var(--color-text);
}

.row-desc {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  line-height: 1.5;
  margin: 0;
}

.row-control {
  min-width: 140px;
  flex-shrink: 0;
}

.row-control-action {
  flex-shrink: 0;
}

.btn-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 6px 13px;
  font-size: 12.5px;
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text);
  cursor: pointer;
  white-space: nowrap;
  transition: background-color var(--transition-fast, 120ms ease), border-color var(--transition-fast, 120ms ease);
}

.btn-action:hover:not(:disabled) {
  background: var(--color-bg-hover);
  border-color: var(--color-border-hover, var(--color-border));
}

.btn-action:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

/* CLI 详情区 */
.cli-picker-row {
  padding-bottom: var(--space-3, 12px);
}

/* CLI 状态压缩行式 */
.cli-status-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 4px var(--space-5, 20px) 16px;
}

.status-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.status-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text);
}

.status-actions {
  margin-left: auto;
  display: flex;
  gap: 8px;
}

.status-line {
  margin: 0;
  font-size: 12px;
  color: var(--color-text-muted);
  font-variant-numeric: tabular-nums;
}

.text-muted {
  color: var(--color-text-muted);
}

.completeness-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: var(--radius-sm, 4px);
  font-size: 11px;
  font-weight: 500;
  line-height: 1.2;
}

.completeness-badge.success {
  background: rgba(52, 199, 89, 0.12);
  color: #248a3d;
}

[data-theme="dark"] .completeness-badge.success {
  background: rgba(52, 199, 89, 0.2);
  color: #34c759;
}

.completeness-badge.warning {
  background: rgba(255, 149, 0, 0.12);
  color: #c97500;
}

[data-theme="dark"] .completeness-badge.warning {
  background: rgba(255, 149, 0, 0.2);
  color: #ff9f0a;
}

.completeness-badge.danger {
  background: rgba(255, 59, 48, 0.12);
  color: #d70015;
}

[data-theme="dark"] .completeness-badge.danger {
  background: rgba(255, 59, 48, 0.2);
  color: #ff453a;
}

.completeness-badge.muted {
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
}

.completeness-badge.info {
  gap: 4px;
  background: rgba(0, 122, 255, 0.12);
  color: #0066d6;
}

[data-theme="dark"] .completeness-badge.info {
  background: rgba(10, 132, 255, 0.2);
  color: #409cff;
}

/* 操作按钮 */
.btn-action,
.btn-danger-outline {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 6px 14px;
  font-size: 13px;
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  white-space: nowrap;
  transition: background-color var(--transition-fast, 120ms ease), border-color var(--transition-fast, 120ms ease);
}

.btn-action {
  border: 1px solid var(--color-border);
  color: var(--color-text);
  background: var(--color-bg);
}

.btn-action:hover:not(:disabled) {
  background: var(--color-bg-hover);
  border-color: var(--color-border-hover, var(--color-border));
}

.btn-danger-outline {
  border: 1px solid var(--color-border);
  background: transparent;
  color: var(--color-danger);
}

.btn-danger-outline:hover:not(:disabled) {
  background: rgba(255, 59, 48, 0.08);
  border-color: var(--color-danger);
}

.btn-action:disabled,
.btn-danger-outline:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.section-hint {
  font-size: 12px;
  line-height: 1.6;
  color: var(--color-text-muted);
  margin: 0;
}

.error-banner {
  margin: 8px var(--space-5, 20px) 0;
  padding: 8px 12px;
  border-radius: var(--radius-md, 6px);
  background: rgba(255, 59, 48, 0.08);
  border: 1px solid rgba(255, 59, 48, 0.2);
  font-size: 12px;
  color: var(--color-danger);
}

/* 进度条 */
.progress-box {
  margin: 12px var(--space-5, 20px) var(--space-4, 16px);
  padding: 10px 14px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.progress-info {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  color: var(--color-text-muted);
}

.progress-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-weight: 500;
  color: var(--color-text);
}

.progress-percent {
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono, monospace);
  font-weight: 600;
  color: var(--color-primary);
}

.progress-sub {
  margin: 0;
  font-size: 11px;
  color: var(--color-text-muted);
}

.progress-track {
  width: 100%;
  height: 4px;
  background: var(--color-border);
  border-radius: 2px;
  overflow: hidden;
}

.progress-bar {
  height: 100%;
  background: var(--color-primary);
  border-radius: 2px;
  transition: width 0.3s ease;
}

.spin-icon {
  animation: spin 1.2s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.progress-fade-enter-active,
.progress-fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.progress-fade-enter-from,
.progress-fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
