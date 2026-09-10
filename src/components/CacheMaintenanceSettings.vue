<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";

const cleaning = ref(false);
const localError = ref("");

async function cleanTempConfigs() {
  if (cleaning.value) return;
  cleaning.value = true;
  localError.value = "";
  try {
    const deleted = await invoke<number>("clean_temp_configs");
    await message(`清理完成，共删除了 ${deleted} 个临时配置文件。`, { kind: "info" });
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
    await message(e?.message ?? String(e), { title: "清理失败", kind: "error" });
  } finally {
    cleaning.value = false;
  }
}
</script>

<template>
  <div class="cache-maintenance">
    <div v-if="localError" class="maintenance-error">
      <span>{{ localError }}</span>
      <button class="clear-error" type="button" aria-label="关闭错误提示" @click="localError = ''">
        <SvgIcon name="x" :size="14" />
      </button>
    </div>

    <div class="maintenance-row">
      <div class="row-info">
        <span class="row-title">临时恢复配置</span>
        <p class="row-desc">CLI 会话恢复命令生成的临时合并配置，系统亦会自动定时清理</p>
      </div>
      <div class="row-action">
        <button
          type="button"
          class="btn-action"
          :disabled="cleaning"
          @click="cleanTempConfigs"
        >
          <SvgIcon v-if="cleaning" name="loader" :size="13" class="spin-icon" />
          <SvgIcon v-else name="trash-2" :size="13" />
          <span>{{ cleaning ? "清理中…" : "立即清理" }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cache-maintenance {
  display: flex;
  flex-direction: column;
}

.maintenance-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-2, 8px) var(--space-4, 16px);
  background: rgba(220, 38, 38, 0.08);
  border-bottom: 1px solid rgba(220, 38, 38, 0.16);
  color: var(--color-danger, #ef4444);
  font-size: var(--text-xs, 12px);
}

.clear-error {
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-danger, #ef4444);
  cursor: pointer;
  padding: 2px;
  border-radius: 50%;
  opacity: 0.6;
}

.clear-error:hover {
  opacity: 1;
}

.maintenance-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4, 16px) var(--space-5, 20px);
  gap: var(--space-4, 16px);
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

.row-action {
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

.spin-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}
</style>
