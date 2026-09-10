<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";

export interface TabInfo {
  id: string;
  cli_id: string;
  name: string;
  sort_order: number;
  dirs: string[];
}

const props = defineProps<{
  currentScope: string;
  tabs: TabInfo[];
}>();

const emit = defineEmits<{
  "copy-to": [targetScope: string];
}>();

const rootRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);

const otherTabs = computed(() =>
  props.tabs.filter((tab) => tab.id !== props.currentScope)
);

const showGlobalOption = computed(() => props.currentScope !== "global");

const hasTargetOptions = computed(() => {
  return showGlobalOption.value || otherTabs.value.length > 0;
});

function toggleMenu() {
  isOpen.value = !isOpen.value;
}

function closeMenu() {
  isOpen.value = false;
}

function selectScope(targetScope: string) {
  emit("copy-to", targetScope);
  closeMenu();
}

function handleClickOutside(event: MouseEvent) {
  const target = event.target as Node | null;
  if (target && rootRef.value && !rootRef.value.contains(target)) {
    closeMenu();
  }
}

onMounted(() => {
  document.addEventListener("mousedown", handleClickOutside);
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", handleClickOutside);
});
</script>

<template>
  <div v-if="hasTargetOptions" ref="rootRef" class="copy-to-container">
    <button
      class="icon-btn"
      type="button"
      @click.stop="toggleMenu"
      title="复制到其他作用域..."
    >
      <SvgIcon name="external-link" :size="14" />
    </button>

    <Transition name="fade">
      <div v-if="isOpen" class="dropdown-menu">
        <div class="menu-title">复制到：</div>
        <button
          v-if="showGlobalOption"
          class="copy-to-item"
          type="button"
          @click.stop="selectScope('global')"
        >
          <SvgIcon name="globe" :size="12" class="copy-to-item-icon" />
          <span>全局</span>
        </button>
        <button
          v-for="tab in otherTabs"
          :key="tab.id"
          class="copy-to-item"
          type="button"
          @click.stop="selectScope(tab.id)"
        >
          <SvgIcon name="folder" :size="12" class="copy-to-item-icon" />
          <span>{{ tab.name }}</span>
        </button>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.copy-to-container {
  position: relative;
  display: inline-flex;
}

.icon-btn {
  border-radius: var(--radius-sm);
}

.dropdown-menu {
  position: absolute;
  right: 0;
  top: calc(100% + 4px);
  min-width: 140px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
  z-index: var(--z-dropdown);
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.menu-title {
  font-size: var(--text-2xs);
  font-weight: 600;
  color: var(--color-text-muted);
  padding: 6px var(--space-2) 4px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--color-border);
  margin-bottom: 4px;
}

.copy-to-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text);
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: var(--radius-sm);
  text-align: left;
  transition: all var(--transition-fast);
  width: 100%;
  outline: none;
  box-sizing: border-box;
}

.copy-to-item:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.copy-to-item:focus-visible {
  outline: none;
  background: var(--color-bg-hover);
  box-shadow: inset 0 0 0 2px var(--color-primary-ring);
}

.copy-to-item-icon {
  opacity: 0.6;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
