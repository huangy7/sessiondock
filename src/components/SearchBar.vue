<script setup lang="ts">
import SvgIcon from "./icons/SvgIcon.vue";

defineProps<{ modelValue: string; placeholder?: string }>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

function clear() {
  emit("update:modelValue", "");
}
</script>

<template>
  <div class="search-bar">
    <div class="search-wrapper">
      <SvgIcon name="search" :size="14" class="search-icon" />
      <input
        type="text"
        :value="modelValue"
        @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        :placeholder="placeholder || '搜索会话...'"
        class="search-input"
        :aria-label="placeholder || '搜索会话'"
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
      />
      <button v-if="modelValue" class="clear-btn" @click="clear" aria-label="清除搜索">
        <SvgIcon name="x" :size="12" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.search-bar {
  padding: var(--space-2);
  flex-shrink: 0;
}
.search-wrapper {
  position: relative;
  display: flex;
  align-items: center;
}
.search-icon {
  position: absolute;
  left: 10px;
  color: var(--color-text-muted);
  pointer-events: none;
}
.search-input {
  width: 100%;
  box-sizing: border-box;
  padding: 6px 30px 6px 30px;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  font-size: var(--text-sm);
  font-family: var(--font-sans);
  outline: none;
  background: var(--color-bg-hover);
  color: var(--color-text);
  transition: all var(--transition-base);
}
.search-input::placeholder {
  color: var(--color-text-muted);
}
.search-input:focus {
  border-color: var(--color-primary);
  background: var(--color-bg);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.clear-btn {
  position: absolute;
  right: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-full);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.clear-btn:hover {
  background: var(--color-bg-active);
  color: var(--color-text);
}
</style>
