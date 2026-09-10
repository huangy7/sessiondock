<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    description?: string;
    defaultValue?: string;
    placeholder?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    maxLength?: number;
  }>(),
  {
    description: "",
    defaultValue: "",
    placeholder: "请输入名称...",
    confirmLabel: "确定",
    cancelLabel: "取消",
    maxLength: 100,
  }
);

const emit = defineEmits<{
  confirm: [value: string];
  cancel: [];
  close: [];
}>();

const inputValue = ref(props.defaultValue);
const inputRef = ref<HTMLInputElement | null>(null);

let openedAt = 0;
let isMouseDownOnOverlay = false;

watch(
  () => props.open,
  async (isOpen) => {
    if (isOpen) {
      openedAt = Date.now();
      isMouseDownOnOverlay = false;
      inputValue.value = props.defaultValue;
      await nextTick();
      requestAnimationFrame(() => {
        if (inputRef.value) {
          inputRef.value.focus();
          inputRef.value.select();
        }
      });
    } else {
      openedAt = 0;
      isMouseDownOnOverlay = false;
    }
  },
  { immediate: true }
);

function handleOverlayMouseDown(e: MouseEvent) {
  isMouseDownOnOverlay = e.target === e.currentTarget;
}

function handleOverlayClick(e: MouseEvent) {
  // 刚打开 250ms 内忽略遮罩点击，防止菜单项的双击第二击或触控板误触连击导致弹窗闪退
  if (Date.now() - openedAt < 250) {
    isMouseDownOnOverlay = false;
    return;
  }
  // 必须 mousedown 和 click 都在 overlay 自身上才允许关闭，避免从卡片内选词划出卡片后释放鼠标导致误关
  if (isMouseDownOnOverlay && e.target === e.currentTarget) {
    handleCancel();
  }
  isMouseDownOnOverlay = false;
}

function handleSubmit() {
  const trimmed = inputValue.value.trim();
  if (trimmed !== props.defaultValue) {
    emit("confirm", trimmed);
  } else {
    emit("cancel");
  }
}

function handleCancel() {
  emit("cancel");
}
</script>

<template>
  <Teleport to="body">
    <Transition name="input-dialog-fade">
      <div
        v-if="open"
        class="input-dialog-overlay"
        @mousedown="handleOverlayMouseDown"
        @click="handleOverlayClick"
      >
        <div
          class="input-dialog-card"
          role="dialog"
          aria-modal="true"
          :aria-label="title"
          @mousedown.stop
          @click.stop
        >
          <div class="input-dialog-header">
            <h3 class="input-dialog-title">{{ title }}</h3>
            <button type="button" class="close-btn" title="关闭" @click="handleCancel">
              <SvgIcon name="x" :size="16" />
            </button>
          </div>

          <p v-if="description" class="input-dialog-desc">{{ description }}</p>

          <form class="input-dialog-body" @submit.prevent="handleSubmit">
            <div class="input-wrap">
              <input
                ref="inputRef"
                v-model="inputValue"
                type="text"
                class="dialog-input"
                :placeholder="placeholder"
                :maxlength="maxLength"
                autocomplete="off"
                autocorrect="off"
                autocapitalize="off"
                spellcheck="false"
                @keydown.esc.prevent.stop="handleCancel" @keydown.enter.prevent.stop="handleSubmit"
              />
              <div v-if="maxLength" class="char-count">
                {{ inputValue.length }}/{{ maxLength }}
              </div>
            </div>

            <div class="input-dialog-footer">
              <button type="button" class="btn cancel-btn" @click="handleCancel">
                {{ cancelLabel }}
              </button>
              <button type="submit" class="btn btn-primary confirm-btn" @click.prevent="handleSubmit">
                {{ confirmLabel }}
              </button>
            </div>
          </form>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.input-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal, 1000);
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--space-4);
}

.input-dialog-card {
  width: 100%;
  max-width: 440px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl, 12px);
  box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.15));
  padding: 20px 22px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.input-dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.input-dialog-title {
  font-size: var(--text-md, 15px);
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.close-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.input-dialog-desc {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  line-height: 1.5;
  margin: -6px 0 0;
}

.input-dialog-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.input-wrap {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.dialog-input {
  width: 100%;
  height: 38px;
  padding: 0 12px;
  font-size: var(--text-sm, 13px);
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  outline: none;
  transition: all var(--transition-fast);
  box-sizing: border-box;
}

.dialog-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-light);
}

.char-count {
  align-self: flex-end;
  font-size: 11px;
  color: var(--color-text-muted);
  font-variant-numeric: tabular-nums;
}

.input-dialog-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 4px;
}

.btn {
  height: 34px;
  padding: 0 16px;
  font-size: var(--text-xs, 12px);
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn:hover {
  background: var(--color-bg-hover);
  border-color: var(--color-border-hover, var(--color-border));
}

.btn-primary {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: #ffffff;
}

.btn-primary:hover {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
}

.input-dialog-fade-enter-active,
.input-dialog-fade-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.input-dialog-fade-enter-from,
.input-dialog-fade-leave-to {
  opacity: 0;
  transform: scale(0.97);
}
</style>
