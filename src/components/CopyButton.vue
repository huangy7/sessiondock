<script setup lang="ts">
import { ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { copyToClipboard } from "../utils/clipboard";

withDefaults(
  defineProps<{
    text: string;
    size?: "sm" | "md";
  }>(),
  {
    size: "sm",
  }
);

const copied = ref(false);

async function doCopy(text: string) {
  const ok = await copyToClipboard(text);
  if (ok) {
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1800);
  }
}
</script>

<template>
  <button
    class="copy-btn"
    :class="[`size-${size}`, { copied }]"
    :title="copied ? '已复制到剪贴板' : '复制内容'"
    @click.stop="doCopy(text)"
  >
    <SvgIcon :name="copied ? 'check' : 'copy'" :size="size === 'sm' ? 13 : 14" />
  </button>
</template>

<style scoped>
.copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  transition: all var(--transition-fast);
  cursor: pointer;
}
.size-sm {
  width: 24px;
  height: 24px;
}
.size-md {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-md);
}
.copy-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.copy-btn.copied {
  color: var(--color-success);
  background: var(--color-bg-hover);
}
</style>
