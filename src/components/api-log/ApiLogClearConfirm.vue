<script setup lang="ts">
defineProps<{
  days?: number;
  loading: boolean;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [];
}>();
</script>

<template>
  <div
    class="confirm-overlay"
    :class="{ busy: loading }"
    @click.self="!loading && emit('cancel')"
  >
    <div class="confirm-dialog" :class="{ clearing: loading }">
      <h3 class="confirm-title">确认清理</h3>
      <p class="confirm-message">
        {{ days ? `确定清理 ${days} 天前的记录吗？` : '确定清理全部记录吗？' }}
      </p>
      <p class="confirm-hint">
        {{ loading ? '正在清理并整理数据库，大数据量时可能需要几秒钟。' : '如果记录很多，清理并整理数据库可能需要一点时间。' }}
      </p>
      <div v-if="loading" class="confirm-progress">
        <span class="proxy-spinner"></span>
        <span>{{ days ? '正在清理历史记录...' : '正在清理全部记录...' }}</span>
      </div>
      <div class="confirm-actions">
        <button class="btn-secondary" :disabled="loading" @click="emit('cancel')">取消</button>
        <button class="btn-danger" :disabled="loading" :class="{ loading }" @click="emit('confirm')">
          <span v-if="loading" class="proxy-loading-inline">
            <span class="proxy-spinner"></span>
            正在清理...
          </span>
          <span v-else>清理</span>
        </button>
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
.confirm-overlay.busy {
  backdrop-filter: blur(2px);
}
.confirm-dialog {
  background: var(--color-bg);
  border-radius: var(--radius-lg);
  padding: var(--space-5);
  width: 340px;
  max-width: 90%;
  box-shadow: var(--shadow-lg);
  transition: transform 180ms ease, box-shadow 180ms ease;
}
.confirm-dialog.clearing {
  transform: scale(1.01);
  box-shadow: 0 20px 40px rgba(15, 23, 42, 0.18);
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
  margin: 0;
  line-height: 1.5;
}
.confirm-hint {
  margin: var(--space-2) 0 0;
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--color-text-muted);
}
.confirm-progress {
  margin-top: var(--space-4);
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: rgba(59, 130, 246, 0.08);
  color: var(--color-primary);
  font-size: var(--text-xs);
  font-weight: 500;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-4);
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
.btn-secondary:hover:not(:disabled) {
  background: var(--color-bg-hover);
}
.btn-secondary:disabled {
  opacity: 0.55;
  cursor: not-allowed;
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
.btn-danger.loading {
  min-width: 116px;
}
.btn-danger:hover:not(:disabled) {
  opacity: 0.9;
}
.btn-danger:disabled {
  opacity: 0.8;
  cursor: wait;
}
.proxy-loading-inline {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.proxy-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: proxy-spin 0.8s linear infinite;
}
@keyframes proxy-spin {
  to { transform: rotate(360deg); }
}
</style>
