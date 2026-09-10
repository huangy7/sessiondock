<script setup lang="ts">
defineProps<{
  name: string;
}>();

defineEmits<{
  cancel: [];
  confirm: [];
}>();
</script>

<template>
  <div class="confirm-overlay" @click.self="$emit('cancel')">
    <div class="confirm-dialog">
      <h3 class="confirm-title">确认删除</h3>
      <p class="confirm-message">确定要删除配置「{{ name }}」吗？此操作无法撤销。</p>
      <div class="confirm-actions">
        <button class="btn-secondary" @click="$emit('cancel')">取消</button>
        <button class="btn-danger" @click="$emit('confirm')">删除</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.confirm-overlay {
  position: absolute;
  inset: 0;
  z-index: 10;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-xl);
}
.confirm-dialog {
  background: var(--color-bg);
  border-radius: var(--radius-lg);
  padding: var(--space-5);
  width: 340px;
  max-width: 90%;
  box-shadow: var(--shadow-lg);
}
.confirm-title {
  font-size: var(--text-base);
  font-weight: 600;
  margin: 0 0 var(--space-2);
  color: var(--color-text);
}
.confirm-message {
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  margin: 0 0 var(--space-4);
  line-height: 1.5;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}
.btn-secondary {
  padding: 7px var(--space-4);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-secondary:hover {
  background: var(--color-bg-hover);
}
.btn-danger {
  padding: 7px var(--space-4);
  font-size: var(--text-sm);
  background: var(--color-danger, #dc2626);
  color: white;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-danger:hover {
  opacity: 0.9;
}
</style>
