<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import ToggleSwitch from "./ToggleSwitch.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import { useSessions } from "../composables/useSessions";
import { shortenHomePath } from "../utils/format";
import { toggleVisibleCli } from "../composables/cliFilter";
import { resolveCliDefinition, type CliId, type ResolvedCliDefinition } from "../types/cli";

const localError = ref("");
const {
  cliOptions,
  cliPathConfigs,
  cliSessionCounts,
  cliFilter,
  visibleCliIds,
  installedCliOptions,
  setCliFilter,
  setCliDataDirOverride,
  refresh,
} = useSessions();

const availableCliIds = computed<CliId[]>(() => {
  const installed = installedCliOptions.value.map((cli) => cli.id);
  return installed.length > 0 ? installed : cliOptions.value.map((c) => c.id);
});

interface DataSourceRowItem {
  cliId: CliId;
  name: string;
  enabled: boolean;
  hasSessions: boolean;
  sessionCount: number;
  resolved: ResolvedCliDefinition;
}

const dataSourceRows = computed<DataSourceRowItem[]>(() => {
  return cliOptions.value.map((cli) => {
    const resolved = resolveCliDefinition(cli.id, cliPathConfigs.value[cli.id]);
    const sessionCount = cliSessionCounts.value[cli.id] ?? 0;
    const hasSessions = cli.hasSessions || sessionCount > 0;
    const enabled = visibleCliIds.value.includes(cli.id);

    return {
      cliId: cli.id,
      name: cli.name,
      enabled,
      hasSessions,
      sessionCount,
      resolved,
    };
  });
});

function toggleCli(cliId: CliId) {
  // If it's not yet in availableCliIds (e.g. 0 sessions), add it to available set for toggling
  const available = availableCliIds.value.includes(cliId)
    ? availableCliIds.value
    : [...availableCliIds.value, cliId];

  const nextState = toggleVisibleCli(cliFilter.value, available, cliId);
  setCliFilter(nextState);
}

async function pickDataDir(cliId: CliId) {
  const item = dataSourceRows.value.find((i) => i.cliId === cliId);
  if (!item) return;
  localError.value = "";
  try {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: item.resolved.dataDirPath,
      title: `选择 ${item.name} 数据目录`,
    });
    if (typeof selected === "string" && selected.trim()) {
      await setCliDataDirOverride(cliId, selected.trim());
      await refresh();
    }
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
  }
}

async function resetDataDir(cliId: CliId) {
  const item = dataSourceRows.value.find((i) => i.cliId === cliId);
  if (!item) return;
  localError.value = "";
  try {
    await setCliDataDirOverride(cliId, null);
    await refresh();
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
  }
}

async function openPathInSystem(path: string) {
  if (!path) return;
  localError.value = "";
  try {
    await invoke("open_path_in_file_manager", { path });
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
  }
}
</script>

<template>
  <div class="data-source-settings">
    <div v-if="localError" class="settings-error">
      <span>{{ localError }}</span>
      <button class="clear-error" type="button" aria-label="关闭错误提示" @click="localError = ''">
        <SvgIcon name="x" :size="16" />
      </button>
    </div>

    <div class="datasource-list">
      <div
        v-for="item in dataSourceRows"
        :key="item.cliId"
        class="datasource-row"
        :class="{ disabled: !item.enabled }"
      >
        <!-- Left: Provider Brand, Name, Path & Actions -->
        <div class="row-main">
          <div class="row-title-line">
            <ChatAvatar role="assistant" :cli-id="item.cliId" class="provider-avatar" />
            <span class="provider-name">{{ item.name }}</span>
            <span v-if="item.resolved.hasCustomDataDir" class="badge-custom">自定义</span>
          </div>

          <div class="row-path-line">
            <span class="path-label" :title="item.resolved.dataSourcePath">
              {{ shortenHomePath(item.resolved.dataSourcePath) }}
            </span>
            <button
              class="icon-action-btn"
              type="button"
              :title="'在访达/资源管理器中打开：' + item.resolved.dataSourcePath"
              @click="openPathInSystem(item.resolved.dataSourcePath)"
            >
              <SvgIcon name="external-link" :size="13" />
            </button>
            <button
              class="icon-action-btn"
              type="button"
              :title="'更改 ' + item.name + ' 数据目录'"
              @click="pickDataDir(item.cliId)"
            >
              <SvgIcon name="folder-open" :size="13" />
            </button>
            <button
              v-if="item.resolved.hasCustomDataDir"
              class="icon-action-btn reset-btn"
              type="button"
              title="恢复为默认数据目录"
              @click="resetDataDir(item.cliId)"
            >
              <SvgIcon name="rotate-ccw" :size="13" />
            </button>
          </div>
        </div>

        <!-- Right: Stats, Status label, Toggle switch -->
        <div class="row-meta">
          <span class="stat-count">{{ item.sessionCount }} 个会话</span>
          <span class="status-label" :class="item.enabled ? 'enabled' : 'disabled'">
            {{ item.enabled ? "已启用" : "已禁用" }}
          </span>
          <ToggleSwitch
            :model-value="item.enabled"
            :aria-label="'启用或禁用 ' + item.name"
            @update:model-value="toggleCli(item.cliId)"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.data-source-settings {
  display: flex;
  flex-direction: column;
  padding: var(--space-2) 0;
}

.settings-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-4);
  margin-bottom: var(--space-4);
  background: var(--color-danger-bg, rgba(239, 68, 68, 0.1));
  border: 1px solid var(--color-danger-border, rgba(239, 68, 68, 0.2));
  border-radius: var(--radius-md);
  color: var(--color-danger, #ef4444);
  font-size: var(--text-sm);
}

.clear-error {
  background: transparent;
  border: none;
  color: currentColor;
  cursor: pointer;
  padding: 2px;
  display: flex;
  align-items: center;
  opacity: 0.7;
}

.clear-error:hover {
  opacity: 1;
}

.datasource-list {
  display: flex;
  flex-direction: column;
}

.datasource-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) 0;
  border-bottom: 1px solid var(--color-border, rgba(255, 255, 255, 0.08));
  gap: var(--space-4);
  transition: opacity var(--transition-fast);
}

.datasource-row:last-child {
  border-bottom: none;
}

.row-main {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.row-title-line {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.provider-avatar {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
}

.provider-name {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}

.badge-custom {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: var(--radius-full, 999px);
  background: var(--color-primary-bg, rgba(59, 130, 246, 0.12));
  color: var(--color-primary, #3b82f6);
  font-weight: 500;
}

.row-path-line {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

.path-label {
  font-family: var(--font-mono, monospace);
  color: var(--color-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.icon-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-text-muted);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
  transition: all var(--transition-fast);
}

.icon-action-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover, rgba(255, 255, 255, 0.08));
}

.icon-action-btn.reset-btn:hover {
  color: var(--color-warning, #f59e0b);
}

.row-meta {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  flex-shrink: 0;
}

.stat-count {
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.status-label {
  font-size: var(--text-sm);
  white-space: nowrap;
}

.status-label.enabled {
  color: var(--color-text-secondary);
}

.status-label.disabled {
  color: var(--color-danger, #ef4444);
}
</style>
