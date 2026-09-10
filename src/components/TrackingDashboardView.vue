<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
import { useTrackingSettings } from "../composables/useTrackingSettings";

type TrackingTeam = { id: number; name: string };
type TrackingSprint = { id: number; name: string; status: string };
type TrackingAction = { label: string; toStatus: number; toLabel: string };
type TrackingEvent = {
  eventType: string;
  label: string;
  description: string;
  time?: string | null;
};
type TrackingMetrics = {
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  cacheHitTokens: number;
  estimatedCost: number;
  aiAddLines?: number | null;
  humanAddLines?: number | null;
  delLines?: number | null;
};
type TrackingItem = {
  id: number;
  key: string;
  title: string;
  itemType: "task" | "req" | "bug";
  status: number;
  statusLabel: string;
  owner: string;
  sprintName: string;
  events: TrackingEvent[];
  metrics: TrackingMetrics;
  actions: TrackingAction[];
};
type TrackingDashboard = {
  teams: TrackingTeam[];
  sprints: TrackingSprint[];
  selectedSprint?: TrackingSprint | null;
  items: TrackingItem[];
  summary: {
    total: number;
    active: number;
    waiting: number;
    accepted: number;
    prMerged: number;
  };
  warnings: string[];
};

const { trackingSettings, trackingInvokeConfig, saveTrackingSettings, updateTrackingSettings } = useTrackingSettings();


const isDashboardLoading = computed(() => {
  return trackingLoading.value;
});

const trackingData = ref<TrackingDashboard | null>(null);
const trackingLoading = ref(false);
const trackingError = ref("");
const trackingBusyId = ref<number | null>(null);
const dialogItem = ref<TrackingItem | null>(null);
const dialogMode = ref<"pr" | "comment" | null>(null);
const dialogSubmitting = ref(false);
const commentDraft = ref("");
const prDraft = ref({
  prId: "",
  prTitle: "",
  fromBranch: "",
  toBranch: "master",
  session: "",
});

const teamOptions = computed<SelectOption[]>(() => [
  { value: null, label: "选择团队" },
  ...(trackingData.value?.teams ?? []).map((t) => ({
    value: t.id,
    label: t.name,
  })),
]);

const sprintOptions = computed<SelectOption[]>(() => [
  { value: null, label: "自动" },
  ...(trackingData.value?.sprints ?? []).map((s) => ({
    value: s.id,
    label: s.name,
  })),
]);

const trackingGroups = computed(() => {
  const groups = new Map<string, TrackingItem[]>();
  for (const item of trackingData.value?.items ?? []) {
    const label = item.statusLabel || "未知";
    const list = groups.get(label) ?? [];
    list.push(item);
    groups.set(label, list);
  }
  return groups;
});

async function refreshTracking() {
  if (trackingLoading.value) return;
  trackingLoading.value = true;
  trackingError.value = "";
  saveTrackingSettings();
  try {
    await waitForLoadingPaint();
    trackingData.value = await invoke<TrackingDashboard>("nps_tracking_load", {
      config: trackingInvokeConfig.value,
    });
    if (!trackingSettings.value.sprintId && trackingData.value.selectedSprint) {
      updateTrackingSettings({ sprintId: trackingData.value.selectedSprint.id });
    }
  } catch (error) {
    trackingError.value = String(error);
  } finally {
    trackingLoading.value = false;
  }
}

async function waitForLoadingPaint() {
  await nextTick();
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

async function runStateAction(item: TrackingItem, action: TrackingAction) {
  // 用 Tauri 异步对话框：WebView 的 window.confirm 不阻塞
  const ok = await ask(`${item.key} ${item.statusLabel} -> ${action.toLabel}`, {
    title: "状态流转确认",
    okLabel: "确认",
    cancelLabel: "取消",
  });
  if (!ok) return;
  trackingBusyId.value = item.id;
  trackingError.value = "";
  try {
    await invoke("nps_tracking_record_state_change", {
      input: {
        config: trackingInvokeConfig.value,
        itemType: item.itemType,
        id: item.id,
        title: item.title,
        fromLabel: item.statusLabel,
        toStatus: action.toStatus,
        toLabel: action.toLabel,
      },
    });
    await refreshTracking();
  } catch (error) {
    trackingError.value = String(error);
  } finally {
    trackingBusyId.value = null;
  }
}

function addPrMerged(item: TrackingItem) {
  dialogItem.value = item;
  dialogMode.value = "pr";
  prDraft.value = {
    prId: "",
    prTitle: item.title,
    fromBranch: "",
    toBranch: "master",
    session: "",
  };
}

function addCustomComment(item: TrackingItem) {
  dialogItem.value = item;
  dialogMode.value = "comment";
  commentDraft.value = "";
}

function closeTrackingDialog(force = false) {
  if (dialogSubmitting.value && !force) return;
  dialogMode.value = null;
  dialogItem.value = null;
  commentDraft.value = "";
}

async function submitPrMerged() {
  const item = dialogItem.value;
  if (!item) return;
  const prId = Number(prDraft.value.prId);
  if (!Number.isFinite(prId) || prId <= 0) {
    trackingError.value = "请填写有效的 PR ID。";
    return;
  }
  if (!prDraft.value.prTitle.trim()) {
    trackingError.value = "请填写 PR 标题。";
    return;
  }

  dialogSubmitting.value = true;
  trackingBusyId.value = item.id;
  trackingError.value = "";
  try {
    await invoke("nps_tracking_record_pr_merged", {
      input: {
        config: trackingInvokeConfig.value,
        id: item.id,
        prId,
        prTitle: prDraft.value.prTitle.trim(),
        fromBranch: prDraft.value.fromBranch.trim(),
        toBranch: prDraft.value.toBranch.trim(),
        session: prDraft.value.session.trim(),
      },
    });
    closeTrackingDialog(true);
    await refreshTracking();
  } catch (error) {
    trackingError.value = String(error);
  } finally {
    dialogSubmitting.value = false;
    trackingBusyId.value = null;
  }
}

async function submitCustomComment() {
  const item = dialogItem.value;
  if (!item) return;
  const content = commentDraft.value.trim();
  if (!content) {
    trackingError.value = "请填写评论内容。";
    return;
  }
  dialogSubmitting.value = true;
  trackingBusyId.value = item.id;
  trackingError.value = "";
  try {
    await invoke("nps_tracking_add_custom_comment", {
      input: {
        config: trackingInvokeConfig.value,
        id: item.id,
        content,
      },
    });
    closeTrackingDialog(true);
    await refreshTracking();
  } catch (error) {
    trackingError.value = String(error);
  } finally {
    dialogSubmitting.value = false;
    trackingBusyId.value = null;
  }
}

function itemTypeLabel(type: string): string {
  if (type === "bug") return "Bug";
  if (type === "req") return "Req";
  return "Task";
}

function formatTokens(value: number): string {
  if (!value) return "0";
  return value.toLocaleString();
}

function onTeamChange() {
  updateTrackingSettings({ sprintId: null });
  void refreshTracking();
}

watch(() => trackingSettings.value.sprintId, () => {
  saveTrackingSettings();
});

onMounted(() => {
  void refreshTracking();
});
</script>

<template>
  <main class="tracking-page">
    <div class="tracking-shell">
      <div class="mock-tabs">
        <div class="tab-triggers">
        </div>
        <button class="icon-btn refresh-btn" :class="{ loading: isDashboardLoading }" :disabled="isDashboardLoading" title="刷新" @click="refreshTracking">
          <SvgIcon name="refresh-cw" :size="15" :class="{ spinning: isDashboardLoading }" />
        </button>
      </div>

      <section class="tracking-panel">
        <div class="tracking-controls">
          <div class="field">
            <span>团队</span>
            <div class="tracking-select-wrap">
              <ElegantSelect
                v-model="trackingSettings.teamId"
                :options="teamOptions"
                :disabled="trackingLoading"
                size="small"
                @change="onTeamChange"
              />
            </div>
          </div>
          <div class="field">
            <span>迭代</span>
            <div class="tracking-select-wrap">
              <ElegantSelect
                v-model="trackingSettings.sprintId"
                :options="sprintOptions"
                :disabled="trackingLoading"
                size="small"
                @change="refreshTracking"
              />
            </div>
          </div>
        </div>

        <div v-if="trackingError" class="tracking-error">
          <SvgIcon name="alert-circle" :size="14" />
          {{ trackingError }}
        </div>
        <div v-for="warning in trackingData?.warnings ?? []" :key="warning" class="tracking-warning">
          {{ warning }}
        </div>

        <div v-if="trackingData" class="tracking-stats">
          <div class="tracking-stat">
            <strong>{{ trackingData.summary.total }}</strong>
            <span>工作项</span>
          </div>
          <div class="tracking-stat">
            <strong>{{ trackingData.summary.active }}</strong>
            <span>处理中</span>
          </div>
          <div class="tracking-stat">
            <strong>{{ trackingData.summary.waiting }}</strong>
            <span>待测试/验收</span>
          </div>
          <div class="tracking-stat">
            <strong>{{ trackingData.summary.prMerged }}</strong>
            <span>已记 PR</span>
          </div>
        </div>
      </section>

      <section class="tracking-panel body-panel" :class="{ 'is-refreshing': trackingLoading && trackingData }">
        <div v-if="trackingLoading && trackingData" class="refresh-overlay">
          <span class="spinner" />
          正在刷新工作流
        </div>

        <div v-if="trackingLoading && !trackingData" class="tracking-loading">
          <span class="spinner" />
          正在读取 nps-rd2
        </div>

        <div v-else-if="trackingData?.items.length" class="tracking-groups">
          <div v-for="[status, items] in trackingGroups" :key="status" class="tracking-group">
            <div class="tracking-group-title">
              <span>{{ status }}</span>
              <small>{{ items.length }}</small>
            </div>
            <div class="tracking-item-list">
              <article v-for="item in items" :key="`${item.itemType}-${item.id}`" class="tracking-item">
                <div class="tracking-item-main">
                  <div class="tracking-item-title">
                    <span class="type-badge">{{ itemTypeLabel(item.itemType) }}</span>
                    <strong>{{ item.key }}</strong>
                    <span>{{ item.title }}</span>
                  </div>
                  <div class="tracking-meta">
                    <span v-if="item.owner">{{ item.owner }}</span>
                    <span v-if="item.sprintName">{{ item.sprintName }}</span>
                    <span v-if="item.metrics.rounds">AI {{ item.metrics.rounds }} 轮</span>
                    <span v-if="item.metrics.inputTokens || item.metrics.outputTokens">
                      tokens {{ formatTokens(item.metrics.inputTokens) }} / {{ formatTokens(item.metrics.outputTokens) }}
                    </span>
                  </div>
                  <div v-if="item.events.length" class="event-line">
                    <span>{{ item.events[0].label }}</span>
                    <small v-if="item.events[0].time">{{ item.events[0].time }}</small>
                  </div>
                </div>
                <div class="tracking-actions">
                  <button
                    v-for="action in item.actions"
                    :key="action.toStatus"
                    class="action-btn"
                    :disabled="trackingBusyId === item.id"
                    @click="runStateAction(item, action)"
                  >
                    {{ action.label }}
                  </button>
                  <button class="icon-btn subtle" title="记录 PR 合并" :disabled="trackingBusyId === item.id" @click="addPrMerged(item)">
                    <SvgIcon name="git-branch" :size="13" />
                  </button>
                  <button class="icon-btn subtle" title="写评论" :disabled="trackingBusyId === item.id" @click="addCustomComment(item)">
                    <SvgIcon name="message-square" :size="13" />
                  </button>
                </div>
              </article>
            </div>
          </div>
        </div>

        <div v-else class="tracking-empty">
          当前配置下没有读取到迭代工作项
        </div>
      </section>

      
    </div>

    <div v-if="dialogMode && dialogItem" class="tracking-dialog-backdrop" @click.self="closeTrackingDialog()">
      <form v-if="dialogMode === 'pr'" class="tracking-dialog" @submit.prevent="submitPrMerged">
        <div class="dialog-head">
          <div>
            <h2>记录 PR 合并</h2>
            <p>{{ dialogItem.key }} · {{ dialogItem.title }}</p>
          </div>
          <button type="button" class="icon-btn subtle" title="关闭" aria-label="关闭" :disabled="dialogSubmitting" @click="closeTrackingDialog()">
            <SvgIcon name="x" :size="13" />
          </button>
        </div>

        <label class="field">
          <span>PR ID</span>
          <input v-model="prDraft.prId" class="text-input" type="number" min="1" required />
        </label>
        <label class="field">
          <span>PR 标题</span>
          <input v-model="prDraft.prTitle" class="text-input" required />
        </label>
        <div class="dialog-grid">
          <label class="field">
            <span>源分支</span>
            <input v-model="prDraft.fromBranch" class="text-input" />
          </label>
          <label class="field">
            <span>目标分支</span>
            <input v-model="prDraft.toBranch" class="text-input" />
          </label>
        </div>
        <label class="field">
          <span>Claude Session 行</span>
          <textarea
            v-model="prDraft.session"
            class="textarea-input"
            rows="3"
            autocomplete="off"
            autocorrect="off"
            autocapitalize="off"
            spellcheck="false"
          />
        </label>

        <div class="dialog-actions">
          <button type="button" class="action-btn" :disabled="dialogSubmitting" @click="closeTrackingDialog()">取消</button>
          <button type="submit" class="action-btn primary" :disabled="dialogSubmitting">
            {{ dialogSubmitting ? "记录中..." : "保存" }}
          </button>
        </div>
      </form>

      <form v-else class="tracking-dialog" @submit.prevent="submitCustomComment">
        <div class="dialog-head">
          <div>
            <h2>写评论</h2>
            <p>{{ dialogItem.key }} · {{ dialogItem.title }}</p>
          </div>
          <button type="button" class="icon-btn subtle" title="关闭" aria-label="关闭" :disabled="dialogSubmitting" @click="closeTrackingDialog()">
            <SvgIcon name="x" :size="13" />
          </button>
        </div>

        <label class="field">
          <span>评论内容</span>
          <textarea
            v-model="commentDraft"
            class="textarea-input"
            rows="6"
            required
            autocomplete="off"
            autocorrect="off"
            autocapitalize="off"
            spellcheck="false"
          />
        </label>

        <div class="dialog-actions">
          <button type="button" class="action-btn" :disabled="dialogSubmitting" @click="closeTrackingDialog()">取消</button>
          <button type="submit" class="action-btn primary" :disabled="dialogSubmitting">
            {{ dialogSubmitting ? "提交中..." : "提交" }}
          </button>
        </div>
      </form>
    </div>
  </main>
</template>

<style scoped>
.mock-tabs {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: var(--text-sm);
  margin-bottom: var(--space-4);
  border-bottom: 1px solid var(--color-border);
  padding-bottom: 6px;
}
.tab-triggers {
  display: flex;
  gap: var(--space-4);
}
.mock-tab {
  padding: 4px var(--space-2) var(--space-2);
  color: var(--color-text-secondary);
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition: all var(--transition-fast);
}
.mock-tab:hover {
  color: var(--color-text);
}
.mock-tab.active {
  color: var(--color-primary);
  font-weight: 600;
  border-bottom-color: var(--color-primary);
}

.tracking-page {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-6);
  background: var(--color-bg);
}
.tracking-shell {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.tracking-panel {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  background: var(--color-bg);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-4);
}
.body-panel {
  position: relative;
  min-height: 280px;
}
.body-panel.is-refreshing {
  cursor: wait;
}
.refresh-overlay {
  position: absolute;
  inset: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--color-bg), transparent 18%);
  color: var(--color-text-secondary);
  font-size: var(--text-sm);
  backdrop-filter: blur(1px);
  pointer-events: none;
}
.tracking-controls {
  display: grid;
  grid-template-columns: minmax(180px, 1fr) minmax(220px, 1.3fr);
  gap: var(--space-3);
}
.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}
.field span {
  font-size: var(--text-2xs);
  font-weight: 600;
  color: var(--color-text-muted);
}
.tracking-select-wrap {
  width: 100%;
}
.text-input,
.select-input {
  width: 100%;
  min-width: 0;
  height: 32px;
  padding: 0 var(--space-2);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  color: var(--color-text);
  font-size: var(--text-xs);
}
.textarea-input {
  width: 100%;
  min-width: 0;
  resize: vertical;
  padding: var(--space-2);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  color: var(--color-text);
  font-size: var(--text-xs);
  line-height: 1.5;
}
.text-input:focus,
.select-input:focus,
.textarea-input:focus {
  outline: none;
  border-color: var(--color-primary);
}
.select-input:disabled {
  opacity: 0.72;
  cursor: wait;
}
.icon-btn {
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.icon-btn:hover:not(:disabled) {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-primary-light);
}
.icon-btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.refresh-btn.loading {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-primary-light);
}
.icon-btn.subtle {
  width: 28px;
  height: 28px;
}
.spinning {
  animation: spin 0.8s linear infinite;
}
.tracking-error,
.tracking-warning {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  line-height: 1.5;
}
.tracking-error {
  color: var(--color-danger);
  background: rgba(220, 38, 38, 0.08);
}
.tracking-warning {
  color: var(--color-warning);
  background: rgba(245, 158, 11, 0.1);
}
.tracking-stats {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--space-2);
}
.tracking-stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg-subtle);
}
.tracking-stat strong {
  font-size: var(--text-xl);
  line-height: 1;
  color: var(--color-text);
}
.tracking-stat span {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
}
.tracking-loading,
.tracking-empty {
  min-height: 180px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
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
.tracking-groups {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.tracking-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.tracking-group-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
  font-weight: 700;
  color: var(--color-text-secondary);
}
.tracking-group-title small {
  padding: 1px 6px;
  border-radius: var(--radius-full);
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
}
.tracking-item-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.tracking-item {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-3);
  align-items: center;
  padding: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
}
.tracking-item-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.tracking-item-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--color-text);
}
.tracking-item-title strong,
.tracking-item-title span:last-child {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.type-badge {
  flex: 0 0 auto;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  font-size: var(--text-2xs);
  font-weight: 700;
}
.tracking-meta,
.event-line {
  display: flex;
  gap: var(--space-2);
  flex-wrap: wrap;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
}
.event-line span {
  color: var(--color-text-secondary);
}
.tracking-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-1);
}
.action-btn {
  height: 28px;
  padding: 0 var(--space-2);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg-hover);
  color: var(--color-text-secondary);
  font-size: var(--text-2xs);
  font-weight: 600;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.action-btn:hover:not(:disabled) {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-primary-light);
}
.action-btn.primary {
  color: white;
  border-color: var(--color-primary);
  background: var(--color-primary);
}
.action-btn.primary:hover:not(:disabled) {
  color: white;
  background: var(--color-primary-hover);
}
.action-btn:disabled {
  opacity: 0.55;
  cursor: default;
}
.tracking-dialog-backdrop {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--space-6);
  background: rgba(15, 23, 42, 0.24);
}
.tracking-dialog {
  width: min(520px, 100%);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-4);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  background: var(--color-bg);
  box-shadow: 0 18px 48px rgba(15, 23, 42, 0.18);
}
.dialog-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
}
.dialog-head h2 {
  margin: 0;
  font-size: var(--text-base);
  color: var(--color-text);
}
.dialog-head p {
  margin: 4px 0 0;
  max-width: 420px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.dialog-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}
@media (max-width: 760px) {
  .tracking-page {
    padding: var(--space-4);
  }
  .tracking-controls,
  .tracking-stats {
    grid-template-columns: 1fr;
  }
  .tracking-item {
    grid-template-columns: 1fr;
  }
  .tracking-actions {
    justify-content: flex-start;
    flex-wrap: wrap;
  }
  .tracking-item-title {
    align-items: flex-start;
    flex-wrap: wrap;
  }
  .tracking-item-title span:last-child {
    flex-basis: 100%;
    white-space: normal;
  }
  .tracking-dialog-backdrop {
    align-items: flex-end;
    padding: var(--space-3);
  }
  .dialog-grid {
    grid-template-columns: 1fr;
  }
}
</style>
