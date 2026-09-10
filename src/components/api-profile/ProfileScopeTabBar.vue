<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import ContextMenu from "../ContextMenu.vue";
import { useContextMenu } from "../../composables/useContextMenu";

export interface TabInfo {
  id: string;
  cli_id: string;
  name: string;
  sort_order: number;
  dirs: string[];
}

const props = defineProps<{
  activeScope: string;
  tabs: TabInfo[];
}>();

const emit = defineEmits<{
  "select-scope": [scope: string];
  "create-tab": [];
  "edit-tab": [tab: TabInfo];
  "delete-tab": [tab: TabInfo];
  "edit-raw-settings": [];
  "export-backup": [];
  "import-backup": [];
}>();

const { visible: scopeMenuVisible, x: scopeX, y: scopeY, items: scopeItems, show: showScopeMenu, close: closeScopeMenu } = useContextMenu();
const { visible: backupMenuVisible, x: backupX, y: backupY, items: backupItems, show: showBackupMenu, close: closeBackupMenu } = useContextMenu();

const activeTab = computed(() => {
  if (props.activeScope === "global") return null;
  return props.tabs.find((t) => t.id === props.activeScope) || null;
});

const currentScopeLabel = computed(() => {
  if (props.activeScope === "global") return "全局作用域";
  return activeTab.value ? activeTab.value.name : "选择作用域";
});

function openScopePicker(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  
  const options = [
    {
      label: "全局 (Global)",
      icon: props.activeScope === "global" ? "check" : "globe",
      action: () => emit("select-scope", "global"),
    },
    ...props.tabs.map((tab) => ({
      label: `${tab.name}${tab.dirs && tab.dirs.length > 0 ? ` (${tab.dirs.length} 个目录)` : ""}`,
      icon: props.activeScope === tab.id ? "check" : "folder",
      action: () => emit("select-scope", tab.id),
    })),
    {
      label: "新建项目作用域...",
      icon: "plus",
      action: () => emit("create-tab"),
    },
  ];

  showScopeMenu({ clientX: rect.left, clientY: rect.bottom + 4 } as MouseEvent, options);
}

function openBackupMenu(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  
  const options = [
    {
      label: "导入配置备份",
      icon: "upload",
      action: () => emit("import-backup"),
    },
    {
      label: "导出配置备份",
      icon: "download",
      action: () => emit("export-backup"),
    },
  ];

  showBackupMenu({ clientX: rect.right - 140, clientY: rect.bottom + 4 } as MouseEvent, options);
}
</script>

<template>
  <div class="scope-header-bar">
    <!-- Left Side: Scope Picker + Quick Edit/Delete Buttons -->
    <div class="scope-picker-group">
      <button class="scope-picker-btn" @click="openScopePicker" title="点击切换或新建配置作用域">
        <SvgIcon :name="activeScope === 'global' ? 'globe' : 'folder'" :size="15" class="picker-icon" />
        <span class="picker-title">{{ currentScopeLabel }}</span>
        <span v-if="activeTab && activeTab.dirs && activeTab.dirs.length > 0" class="picker-tag">
          {{ activeTab.dirs.length }} 个目录
        </span>
        <SvgIcon name="chevron-down" :size="13" class="picker-arrow" />
      </button>

      <template v-if="activeTab">
        <button class="scope-quick-btn" @click="emit('edit-tab', activeTab)" title="编辑当前作用域（修改名称与绑定目录）">
          <SvgIcon name="pencil" :size="14" />
        </button>
        <button class="scope-quick-btn danger" @click="emit('delete-tab', activeTab)" title="删除当前项目作用域">
          <SvgIcon name="trash-2" :size="14" />
        </button>
      </template>
    </div>

    <!-- Right Side: Backup Dropdown + Edit JSON Toolbar -->
    <div class="header-actions">
      <button class="action-btn" @click="openBackupMenu" title="导入或导出 API 配置备份">
        <SvgIcon name="archive" :size="14" />
        <span>备份 / 恢复</span>
        <SvgIcon name="chevron-down" :size="12" class="picker-arrow" />
      </button>

      <button class="action-btn" @click="emit('edit-raw-settings')" title="高级：使用应用内 JSON 编辑器修改底层原生配置">
        <SvgIcon name="code" :size="14" />
        <span>编辑 JSON</span>
      </button>
    </div>
  </div>

  <ContextMenu :visible="scopeMenuVisible" :x="scopeX" :y="scopeY" :items="scopeItems" @close="closeScopeMenu" />
  <ContextMenu :visible="backupMenuVisible" :x="backupX" :y="backupY" :items="backupItems" @close="closeBackupMenu" />
</template>

<style scoped>
.scope-header-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: var(--space-3);
  margin-bottom: var(--space-4);
  border-bottom: 1px solid var(--color-border);
  gap: 12px;
}

.scope-picker-group {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex-shrink: 1;
}

.scope-picker-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: 6px 12px;
  background: var(--color-surface, rgba(0,0,0,0.03));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-text);
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
  outline: none;
  white-space: nowrap;
  min-width: 0;
  flex-shrink: 1;
}

.scope-picker-btn:hover {
  background: var(--color-bg-hover);
  border-color: var(--color-primary);
  color: var(--color-primary);
}

.scope-quick-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-md, 6px);
  color: var(--color-text-secondary);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition-fast, 0.15s);
}

.scope-quick-btn:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}

.scope-quick-btn.danger:hover {
  color: var(--color-danger, #ef4444);
  border-color: var(--color-danger, #ef4444);
  background: var(--color-danger-light, rgba(239,68,68,0.08));
}

.picker-icon {
  color: var(--color-primary);
  flex-shrink: 0;
}

.picker-title {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex-shrink: 1;
}

.picker-tag {
  font-size: 11px;
  font-weight: 500;
  padding: 1px 6px;
  border-radius: 9999px;
  background: var(--color-primary-light, rgba(59,130,246,0.1));
  color: var(--color-primary);
  white-space: nowrap;
  flex-shrink: 0;
}

.picker-arrow {
  opacity: 0.6;
  transition: transform 0.2s ease;
  flex-shrink: 0;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  font-size: var(--text-xs, 12px);
  font-weight: 500;
  color: var(--color-text-secondary);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
  white-space: nowrap;
  flex-shrink: 0;
}

.action-btn:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}
</style>
