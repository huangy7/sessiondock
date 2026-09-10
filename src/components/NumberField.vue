<script setup lang="ts">
const props = defineProps<{
  modelValue: string;
  min?: number;
  max?: number;
  step?: number;
  unit?: string;
  disabled?: boolean;
  ariaLabel?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "commit", value: string): void;
}>();

function onInput(event: Event) {
  emit("update:modelValue", (event.target as HTMLInputElement).value);
}

function commit() {
  emit("commit", props.modelValue);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    commit();
  }
}
</script>

<template>
  <div class="number-field">
    <input
      class="number-input"
      type="number"
      :min="min"
      :max="max"
      :step="step"
      inputmode="numeric"
      :value="modelValue"
      :disabled="disabled"
      :aria-label="ariaLabel"
      @input="onInput"
      @blur="commit"
      @keydown="onKeydown"
    />
    <span v-if="unit" class="number-unit">{{ unit }}</span>
  </div>
</template>

<style scoped>
.number-field {
  display: inline-flex;
  align-items: center;
  min-height: 34px;
  width: 132px;
  padding: 0 10px;
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.number-field:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.number-input {
  flex: 1;
  min-width: 0;
  padding: 0;
  font-size: var(--text-sm);
  color: inherit;
  background: transparent;
  border: 0;
  outline: 0;
  appearance: textfield;
}
.number-input::-webkit-outer-spin-button,
.number-input::-webkit-inner-spin-button {
  margin: 0;
  appearance: none;
}
.number-unit {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
</style>
