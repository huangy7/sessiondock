<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import ToggleSwitch from "./ToggleSwitch.vue";
import NumberField from "./NumberField.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { useTrackingSettings } from "../composables/useTrackingSettings";

const localError = ref("");
const { trackingSettings, updateTrackingSettings } = useTrackingSettings();

const workflowEnabledDraft = ref(trackingSettings.value.enabled);
const workflowPageSizeInput = ref(String(trackingSettings.value.pageSize));
const workflowSaving = ref(false);
const workflowSuccess = ref(false);
let workflowSuccessTimer: ReturnType<typeof window.setTimeout> | undefined;

watch(
  trackingSettings,
  (settings) => {
    workflowEnabledDraft.value = settings.enabled;
    workflowPageSizeInput.value = String(settings.pageSize);
  },
  { deep: true }
);

function normalizeWorkflowPageSize() {
  const value = Math.trunc(Number(workflowPageSizeInput.value));
  if (!Number.isFinite(value)) return 80;
  return Math.min(Math.max(value, 10), 200);
}

function clearWorkflowSuccess() {
  workflowSuccess.value = false;
  if (workflowSuccessTimer) {
    window.clearTimeout(workflowSuccessTimer);
    workflowSuccessTimer = undefined;
  }
}

function setWorkflowSuccess() {
  workflowSuccess.value = true;
  if (workflowSuccessTimer) window.clearTimeout(workflowSuccessTimer);
  workflowSuccessTimer = window.setTimeout(() => {
    workflowSuccess.value = false;
    workflowSuccessTimer = undefined;
  }, 2500);
}

async function saveWorkflowSettings() {
  if (workflowSaving.value) return;
  const pageSize = normalizeWorkflowPageSize();
  workflowSaving.value = true;
  clearWorkflowSuccess();
  localError.value = "";
  try {
    if (workflowEnabledDraft.value) {
      await invoke("nps_tracking_validate_cli");
    }
    updateTrackingSettings({ enabled: workflowEnabledDraft.value, pageSize });
    workflowPageSizeInput.value = String(pageSize);
    setWorkflowSuccess();
  } catch (e: any) {
    updateTrackingSettings({ enabled: false, pageSize });
    workflowEnabledDraft.value = false;
    localError.value = e?.message ?? String(e);
  } finally {
    workflowSaving.value = false;
  }
}

async function onWorkflowEnabledChange(checked: boolean) {
  clearWorkflowSuccess();
  localError.value = "";
  const pageSize = normalizeWorkflowPageSize();
  if (!checked) {
    workflowEnabledDraft.value = false;
    updateTrackingSettings({ enabled: false, pageSize });
    workflowPageSizeInput.value = String(pageSize);
    return;
  }
  workflowSaving.value = true;
  try {
    await invoke("nps_tracking_validate_cli");
    workflowEnabledDraft.value = true;
    updateTrackingSettings({ enabled: true, pageSize });
    workflowPageSizeInput.value = String(pageSize);
    setWorkflowSuccess();
  } catch (e: any) {
    updateTrackingSettings({ enabled: false, pageSize });
    workflowEnabledDraft.value = false;
    localError.value = e?.message ?? String(e);
  } finally {
    workflowSaving.value = false;
  }
}

onBeforeUnmount(() => {
  clearWorkflowSuccess();
});
</script>

<template>
  <div class="settings-group">
    <div v-if="localError" class="settings-error">
      <span>{{ localError }}</span>
      <button class="clear-error" type="button" aria-label="关闭错误提示" @click="localError = ''">
        <SvgIcon name="x" :size="16" />
      </button>
    </div>

    <div class="settings-row" :class="{ 'is-disabled': workflowSaving }">
      <div class="row-content">
        <div class="row-title">显示工作流页面</div>
        <div v-if="workflowEnabledDraft" class="workflow-config">
          <label class="workflow-field">
            <span>拉取数量</span>
            <NumberField
              v-model="workflowPageSizeInput"
              :min="10"
              :max="200"
              :step="10"
              unit="条"
              aria-label="拉取数量"
              :disabled="workflowSaving"
              @commit="saveWorkflowSettings"
            />
          </label>
          <span v-if="workflowSuccess" class="inline-feedback success">
            <SvgIcon name="check" :size="14" />
            已保存
          </span>
        </div>
      </div>
      <div class="row-action">
        <ToggleSwitch
          :model-value="workflowEnabledDraft"
          :disabled="workflowSaving"
          aria-label="显示工作流页面"
          @update:model-value="onWorkflowEnabledChange"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-group {
  display: flex;
  flex-direction: column;
}
.settings-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-4);
  background: rgba(220, 38, 38, 0.08);
  border-bottom: 1px solid rgba(220, 38, 38, 0.16);
  color: var(--color-danger);
  font-size: var(--text-sm);
}
.clear-error {
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-danger);
  cursor: pointer;
  padding: 2px;
  border-radius: 50%;
  opacity: 0.6;
  transition: all var(--transition-fast);
}
.clear-error:hover {
  opacity: 1;
  background: rgba(220, 38, 38, 0.1);
}
.settings-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  gap: var(--space-4);
}
.settings-row.is-disabled {
  opacity: 0.7;
  cursor: not-allowed;
}
.row-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}
.row-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.row-action {
  flex-shrink: 0;
}
.workflow-config {
  display: flex;
  align-items: flex-end;
  gap: var(--space-3);
  margin-top: var(--space-3);
}
.workflow-field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.workflow-field span {
  font-size: var(--text-2xs);
  font-weight: 600;
  color: var(--color-text-muted);
}
.inline-feedback {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: var(--text-xs);
}
.inline-feedback.success {
  color: var(--color-success);
}
</style>
