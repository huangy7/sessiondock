<script setup lang="ts">
defineProps<{
  maxOutputTokens: string;
  disableExperimentalBetas: string;
  disableNonessentialTraffic: string;
}>();

defineEmits<{
  updateMaxOutputTokens: [value: string];
  updateDisableExperimentalBetas: [value: string];
  updateDisableNonessentialTraffic: [value: string];
}>();

import ElegantSelect from "../common/ElegantSelect.vue";

const booleanOptions = [
  { value: "", label: "使用 Claude 默认" },
  { value: "1", label: "1" },
  { value: "0", label: "0" }
];
</script>

<template>
  <section class="settings-section">
    <h3 class="section-title">Claude Code 运行参数</h3>
    <p class="section-hint">写入 `settings.json` 的 `env`；留空则使用 Claude Code 默认值</p>
    <div class="field-row">
      <div class="field-group">
        <label class="field-label">最大输出 Tokens</label>
        <input
          class="field-input"
          type="text"
          :value="maxOutputTokens"
          placeholder="64000"
          @input="$emit('updateMaxOutputTokens', ($event.target as HTMLInputElement).value)"
        />
        <span class="field-hint">限制单次响应输出上限</span>
      </div>
      <div class="field-group">
        <label class="field-label">禁用实验 Beta</label>
        <div class="elegant-select-wrapper">
          <ElegantSelect
            :model-value="disableExperimentalBetas"
            :options="booleanOptions"
            @update:model-value="$emit('updateDisableExperimentalBetas', $event as string)"
          />
        </div>
        <span class="field-hint">关闭实验性 Beta 功能</span>
      </div>
      <div class="field-group">
        <label class="field-label">关闭非必要流量</label>
        <div class="elegant-select-wrapper">
          <ElegantSelect
            :model-value="disableNonessentialTraffic"
            :options="booleanOptions"
            @update:model-value="$emit('updateDisableNonessentialTraffic', $event as string)"
          />
        </div>
        <span class="field-hint">减少后台与非关键网络请求</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-section {
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.02);
}

.section-title {
  font-size: var(--text-sm);
  font-weight: 600;
  margin: 0 0 2px;
  color: var(--color-text);
}

.section-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  margin: 0 0 var(--space-3);
  line-height: 1.4;
}

.field-row {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: var(--space-3);
  align-items: flex-start;
}

@media (max-width: 680px) {
  .field-row {
    grid-template-columns: 1fr;
  }
}

.field-group {
  margin-bottom: 0;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.field-label {
  display: flex;
  align-items: center;
  height: 18px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-bottom: 6px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.field-input {
  width: 100%;
  height: 36px;
  box-sizing: border-box;
  padding: 0 var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-family: var(--font-mono);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  line-height: 34px;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.field-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
}

.elegant-select-wrapper {
  height: 36px;
  display: flex;
  align-items: center;
}

.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 6px;
  line-height: 1.4;
}
</style>
