<script setup lang="ts">
import { ref, watch, onUnmounted } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";

const props = defineProps<{ container: HTMLElement | null }>();

const emit = defineEmits<{
  scrollToEnd: [];
  scrollToTop: [];
}>();

const direction = ref<"down" | "up">("down");
const visible = ref(false);
let lastScrollTop = 0;

function onScroll() {
  const el = props.container;
  if (!el) return;

  const scrollTop = el.scrollTop;
  const distanceFromBottom = el.scrollHeight - scrollTop - el.clientHeight;
  const distanceFromTop = scrollTop;

  // Determine direction based on last scroll movement
  if (scrollTop > lastScrollTop) {
    direction.value = "down";
  } else if (scrollTop < lastScrollTop) {
    direction.value = "up";
  }
  lastScrollTop = scrollTop;

  // Show when not at edges
  visible.value = distanceFromTop > 200 || distanceFromBottom > 200;

  // Force direction at edges
  if (distanceFromBottom < 50) {
    direction.value = "up";
  } else if (distanceFromTop < 50) {
    direction.value = "down";
  }
}

function handleClick() {
  const el = props.container;
  if (!el) return;
  if (direction.value === "down") {
    // Need to load all messages before scrolling to bottom
    emit("scrollToEnd");
  } else {
    emit("scrollToTop");
  }
}

watch(
  () => props.container,
  (el, oldEl) => {
    oldEl?.removeEventListener("scroll", onScroll);
    el?.addEventListener("scroll", onScroll, { passive: true });
  },
  { immediate: true },
);
onUnmounted(() => {
  props.container?.removeEventListener("scroll", onScroll);
});
</script>

<template>
  <Transition name="fade-up">
    <button
      v-if="visible"
      class="scroll-btn"
      :title="direction === 'down' ? '滚动到底部' : '滚动到顶部'"
      @click="handleClick"
    >
      <SvgIcon name="arrow-down" :size="18" :class="{ flipped: direction === 'up' }" />
    </button>
  </Transition>
</template>

<style scoped>
.scroll-btn {
  position: absolute;
  bottom: 20px;
  right: 20px;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  box-shadow: var(--shadow-md);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-base);
  z-index: var(--z-sticky);
}
.scroll-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  box-shadow: var(--shadow-lg);
}
.flipped {
  transform: rotate(180deg);
}
.fade-up-enter-active,
.fade-up-leave-active {
  transition: opacity var(--transition-base), transform var(--transition-base);
}
.fade-up-enter-from,
.fade-up-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>
