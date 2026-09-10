<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { addProxyStatusChangedListener, type ProxyStatusInfo } from "../composables/useProxy";
import { useActivityBar } from "../composables/useActivityBar";
import SvgIcon from "./icons/SvgIcon.vue";
import type { CliId } from "../types/cli";

const props = defineProps<{
  projectCount: number;
  sessionCount: number;
  messageCount: number | null;
  cliId: CliId;
  cliDetected?: boolean;
  rightSidebarOpen?: boolean;
  refreshing?: boolean;
}>();

const emit = defineEmits<{
  toggleRightSidebar: [];
  refreshSession: [];
}>();

const { isExpanded, toggleExpanded } = useActivityBar();

const proxyStatus = ref<ProxyStatusInfo | null>(null);
let pollTimer: ReturnType<typeof setInterval> | null = null;
let removeProxyStatusListener: (() => void) | null = null;

async function loadProxyStatus() {
  try {
    const s = await invoke<ProxyStatusInfo>("proxy_status", { cliId: props.cliId });
    proxyStatus.value = s.enabled && s.running ? s : null;
  } catch (_) {
    proxyStatus.value = null;
  }
}

watch(() => props.cliId, () => {
  proxyStatus.value = null;
  loadProxyStatus();
});

onMounted(() => {
  loadProxyStatus();
  removeProxyStatusListener = addProxyStatusChangedListener(({ cliId, status }) => {
    if (cliId !== props.cliId) return;
    proxyStatus.value = status?.enabled && status?.running ? status : null;
  });
  // Poll every 10s to detect proxy state changes (manual stop, legacy cleanup, etc.)
  pollTimer = setInterval(loadProxyStatus, 10000);
});

onUnmounted(() => {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  removeProxyStatusListener?.();
  removeProxyStatusListener = null;
});

// Allow SettingsView to trigger a refresh after enable/disable
defineExpose({ loadProxyStatus });
</script>

<template>
  <div class="status-bar">
    <div class="status-left">
      <button
        type="button"
        class="status-tool-btn"
        :class="{ active: isExpanded }"
        :title="isExpanded ? '收起导航栏' : '展开导航栏'"
        :aria-label="isExpanded ? '收起导航栏' : '展开导航栏'"
        @click="toggleExpanded"
      >
        <SvgIcon :name="isExpanded ? 'panel-left-close' : 'panel-left-open'" :size="12" />
      </button>
      <span class="status-sep">|</span>
      <span class="status-item">项目: {{ projectCount }}</span>
      <span class="status-sep">|</span>
      <span class="status-item">会话: {{ sessionCount }}</span>
    </div>
    <div class="status-right">
      <button
        v-if="messageCount !== null"
        type="button"
        class="status-session-refresh-btn"
        :disabled="refreshing"
        :title="refreshing ? '正在重新加载...' : '点击重新加载对话'"
        :aria-label="refreshing ? '正在重新加载对话' : '点击重新加载对话'"
        @click="emit('refreshSession')"
      >
        <SvgIcon
          name="refresh-cw"
          :size="11"
          class="refresh-icon"
          :class="{ spin: refreshing }"
        />
        <span class="message-count-text">{{ messageCount }} 条消息</span>
      </button>
      <template v-if="proxyStatus">
        <span class="status-sep">|</span>
        <span class="status-proxy">
          <span class="proxy-indicator"></span>
          代理 :{{ proxyStatus.port }}
        </span>
      </template>
      <span class="status-sep">|</span>
      <button
        type="button"
        class="status-tool-btn"
        :class="{ active: rightSidebarOpen }"
        :title="rightSidebarOpen ? '收起辅助栏' : '展开辅助栏'"
        :aria-label="rightSidebarOpen ? '收起辅助栏' : '展开辅助栏'"
        @click="emit('toggleRightSidebar')"
      >
        <SvgIcon name="panel-right" :size="12" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.status-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 var(--space-3);
  height: var(--statusbar-height);
  background: var(--color-bg-secondary);
  border-top: 1px solid var(--color-border);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  flex-shrink: 0;
}
.status-left,
.status-right {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.status-sep {
  color: var(--color-border);
}
.status-proxy {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text-muted);
}
.proxy-indicator {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-success, #22c55e);
  flex-shrink: 0;
}
.status-tool-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: var(--radius-sm, 4px);
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.status-tool-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.status-tool-btn.active {
  color: var(--color-primary);
}
.status-session-refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 20px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm, 4px);
  background: transparent;
  color: var(--color-text-muted);
  font-size: var(--text-xs);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.status-session-refresh-btn:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
  border-color: var(--color-border-subtle, var(--color-border));
}

.status-session-refresh-btn:active:not(:disabled) {
  background: var(--color-bg-active, var(--color-bg-hover));
}

.status-session-refresh-btn:disabled {
  cursor: default;
  opacity: 0.8;
}

.refresh-icon.spin {
  animation: spin 0.8s linear infinite;
}
</style>
