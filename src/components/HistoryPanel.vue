<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import SearchBar from "./SearchBar.vue";
import SessionTree from "./SessionTree.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import type { ProjectInfo, SessionIdentity } from "../types/session";
import type { CliId, CliOption } from "../types/cli";
import type { CliFilterState } from "../composables/cliFilter";
import { useProjectFilter } from "../composables/useProjectFilter";
import { useTreeGrouping, type TreeGrouping } from "../composables/useTreeGrouping";

const sessionTreeRef = ref<InstanceType<typeof SessionTree> | null>(null);

const props = withDefaults(
  defineProps<{
    projects: ProjectInfo[];
    selectedSessionIdentity: SessionIdentity | null;
    selectedSessionIdentities: SessionIdentity[];
    contextMenuSessionIdentity?: SessionIdentity | null;
    searchQuery: string;
    sortMode: "time" | "name";
    isStreamingProjects: boolean;
    totalLoadedSessions: number;
    isRefreshing: boolean;
    showLoadingIndicator: boolean;
    bootstrapping?: boolean;
    currentCliLabel?: string;
    visibleCliIds: CliId[];
    cliOptions: CliOption[];
    cliSessionCounts: Partial<Record<CliId, number>>;
    scanCliErrors?: Partial<Record<CliId, string>>;
    projectAllSelected: (projectKey: string) => boolean;
    refreshing?: boolean;
    grouping?: TreeGrouping;
  }>(),
  {}
);

const { grouping: internalGrouping, setGrouping: setInternalGrouping } = useTreeGrouping(props.grouping);
const grouping = computed(() => props.grouping ?? internalGrouping.value);

function setGrouping(mode: TreeGrouping) {
  setInternalGrouping(mode);
  emit("update:grouping", mode);
}

const showCliFilter = ref(false);
const cliFilterTriggerRef = ref<HTMLButtonElement | null>(null);
const cliFilterMenuRef = ref<HTMLElement | null>(null);
const filterableCliOptions = computed(() => props.cliOptions.filter((option) => option.hasSessions));
const filterableCliIds = computed(() => filterableCliOptions.value.map((option) => option.id));
const visibleCliIdSet = computed(() => new Set(props.visibleCliIds));
const allCliSourcesSelected = computed(() =>
  filterableCliIds.value.length > 0
  && filterableCliIds.value.every((cliId) => visibleCliIdSet.value.has(cliId)),
);
const partiallySelectedCliSources = computed(() =>
  !allCliSourcesSelected.value && props.visibleCliIds.length > 0,
);
const cliFilterSummary = computed(() => allCliSourcesSelected.value
  ? "全部来源"
  : `${props.visibleCliIds.length}/${filterableCliIds.value.length} 个来源`,
);

function cliSessionCount(cliId: CliId): number {
  return props.cliSessionCounts[cliId] ?? 0;
}

function startRename(identity: SessionIdentity, currentName: string) {
  sessionTreeRef.value?.startRename(identity, currentName);
}

defineExpose({ startRename });

const emit = defineEmits<{
  "update:searchQuery": [value: string];
  "update:cliFilter": [next: CliFilterState];
  "update:grouping": [value: TreeGrouping];
  selectSession: [identity: SessionIdentity, encodedDir: string];
  pinSession: [identity: SessionIdentity, encodedDir: string];
  contextMenuSession: [event: MouseEvent, identity: SessionIdentity, sessionId: string, displayName: string, projectPath: string];
  contextMenuProject: [event: MouseEvent, project: ProjectInfo];
  toggleSort: [];
  refresh: [];
  renameSession: [identity: SessionIdentity, newName: string];
  toggleSelect: [identity: SessionIdentity, event: MouseEvent];
  rangeSelect: [startIdentity: SessionIdentity, endIdentity: SessionIdentity];
  clearSelection: [];
  selectProjectSessions: [projectKey: string];
  retryCli: [cliId: CliId];
}>();

function cliFilterItems(): HTMLButtonElement[] {
  const el = cliFilterMenuRef.value;
  if (!el) return [];
  return Array.from(el.querySelectorAll<HTMLButtonElement>(".cli-filter-option:not(:disabled)"));
}

async function toggleCliFilterMenu() {
  showCliFilter.value = !showCliFilter.value;
  if (showCliFilter.value) {
    await nextTick();
    cliFilterItems()[0]?.focus();
  }
}

function closeCliFilter(opts: { focusTrigger?: boolean } = {}) {
  if (!showCliFilter.value) return;
  showCliFilter.value = false;
  if (opts.focusTrigger) cliFilterTriggerRef.value?.focus();
}

// 键盘契约与 ActivityBar 浮层一致；菜单未打开时不吞键（ Esc 要留给全局关闭链）
function onCliFilterKeydown(e: KeyboardEvent) {
  if (!showCliFilter.value) return;
  if (e.key === "Escape") {
    e.stopPropagation();
    e.preventDefault();
    closeCliFilter({ focusTrigger: true });
    return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const items = cliFilterItems();
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      e.key === "ArrowDown"
        ? (current + 1) % items.length
        : (current - 1 + items.length) % items.length;
    items[next]?.focus();
  }
}

function onCliFilterDocumentClick(e: MouseEvent) {
  if (!showCliFilter.value) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest?.(".cli-filter")) return;
  showCliFilter.value = false;
}

onMounted(() => document.addEventListener("click", onCliFilterDocumentClick));
onBeforeUnmount(() => document.removeEventListener("click", onCliFilterDocumentClick));

function toggleCliFilter(cliId: CliId) {
  const selected = new Set(props.visibleCliIds);
  if (selected.has(cliId)) selected.delete(cliId);
  else selected.add(cliId);

  emit("update:cliFilter", {
    mode: "custom",
    cliIds: filterableCliIds.value.filter((availableCliId) => selected.has(availableCliId)),
  });
}

function toggleAllCliSources() {
  if (allCliSourcesSelected.value) {
    emit("update:cliFilter", { mode: "custom", cliIds: [] });
  } else {
    emit("update:cliFilter", { mode: "all" });
  }
}

function restoreAllCliSources() {
  emit("update:cliFilter", { mode: "all" });
}

const { blockedProjects, unblockProject } = useProjectFilter();
const showBlockedSection = ref(false);
</script>

<template>
  <div class="history-panel">
    <div class="history-topbar">
      <div class="tree-grouping-toggle" role="radiogroup" aria-label="分组方式">
        <button
          type="button"
          class="grouping-btn"
          :class="{ active: grouping === 'directory' }"
          role="radio"
          :aria-checked="grouping === 'directory'"
          title="按目录分组"
          @click="setGrouping('directory')"
        >
          <SvgIcon name="folder" :size="13" />
        </button>
        <button
          type="button"
          class="grouping-btn"
          :class="{ active: grouping === 'provider' }"
          role="radio"
          :aria-checked="grouping === 'provider'"
          title="按供应商分组"
          @click="setGrouping('provider')"
        >
          <SvgIcon name="layers" :size="13" />
        </button>
      </div>

      <div class="cli-filter" @keydown="onCliFilterKeydown">
        <button
          ref="cliFilterTriggerRef"
          class="cli-filter-trigger"
          :class="{ partial: partiallySelectedCliSources }"
          type="button"
          :aria-expanded="showCliFilter"
          aria-haspopup="menu"
          title="筛选对话来源"
          @click="toggleCliFilterMenu"
        >
          <SvgIcon name="sliders" :size="13" />
          <span class="cli-filter-summary">{{ cliFilterSummary }}</span>
        </button>

        <div
          v-if="showCliFilter"
          ref="cliFilterMenuRef"
          class="cli-filter-menu"
          role="menu"
          aria-label="对话来源筛选"
          tabindex="-1"
        >
          <button
            class="cli-filter-option cli-filter-all"
            type="button"
            role="menuitemcheckbox"
            :aria-checked="allCliSourcesSelected ? 'true' : (partiallySelectedCliSources ? 'mixed' : 'false')"
            @click="toggleAllCliSources"
          >
            <span class="cli-filter-checkbox" :class="{ checked: allCliSourcesSelected, partial: partiallySelectedCliSources }">
              <SvgIcon v-if="allCliSourcesSelected" name="check" :size="11" />
              <span v-else-if="partiallySelectedCliSources" class="partial-mark"></span>
            </span>
            <span class="cli-filter-option-label">全部来源</span>
            <span class="cli-filter-option-count">{{ filterableCliIds.length }}</span>
          </button>
          <template v-for="option in filterableCliOptions" :key="option.id">
            <button
              class="cli-filter-option"
              type="button"
              role="menuitemcheckbox"
              :aria-checked="visibleCliIdSet.has(option.id)"
              :data-cli-id="option.id"
              @click="toggleCliFilter(option.id)"
            >
              <span class="cli-filter-checkbox" :class="{ checked: visibleCliIdSet.has(option.id) }">
                <SvgIcon v-if="visibleCliIdSet.has(option.id)" name="check" :size="11" />
              </span>
              <ChatAvatar role="assistant" :cliId="option.id" class="cli-filter-avatar" />
              <span class="cli-filter-option-label">{{ option.name }}</span>
              <span class="cli-filter-option-count">{{ cliSessionCount(option.id) }}</span>
            </button>
            <div
              v-if="scanCliErrors?.[option.id]"
              class="cli-filter-error"
              role="alert"
              :data-cli-error="option.id"
            >
              <SvgIcon name="alert-circle" :size="11" />
              <span class="cli-filter-error-text" :title="scanCliErrors[option.id]">
                {{ option.name }} 扫描失败
              </span>
              <button
                type="button"
                class="cli-filter-retry"
                @click="emit('retryCli', option.id)"
              >重试</button>
            </div>
          </template>
        </div>
      </div>
    </div>

    <div class="search-row">
      <SearchBar
        :modelValue="searchQuery"
        @update:modelValue="emit('update:searchQuery', $event)"
      />
      <div class="tree-actions">
        <button
          class="toolbar-btn"
          :title="sessionTreeRef?.canLocateSession ? '定位当前文件' : '当前无活跃对话'"
          :disabled="!sessionTreeRef?.canLocateSession"
          @click="sessionTreeRef?.locateCurrentSession()"
        >
          <SvgIcon name="crosshair" :size="13" />
        </button>
        <button
          class="toolbar-btn"
          :title="sortMode === 'time' ? '按名称排序' : '按时间排序'"
          @click="emit('toggleSort')"
        >
          <SvgIcon :name="sortMode === 'time' ? 'clock' : 'sort-asc'" :size="13" />
        </button>
        <button
          class="toolbar-btn"
          :title="sessionTreeRef?.allCollapsed ? '展开全部' : '折叠全部'"
          :disabled="projects.length === 0"
          @click="sessionTreeRef?.toggleCollapseAll()"
        >
          <SvgIcon :name="sessionTreeRef?.allCollapsed ? 'expand-all' : 'collapse-all'" :size="13" />
        </button>
        <button
          class="toolbar-btn"
          :title="refreshing ? '正在刷新...' : '刷新对话列表与当前内容 (⌘R)'"
          :disabled="refreshing"
          @click="emit('refresh')"
        >
          <span :class="{ spinning: refreshing }" class="refresh-icon-wrap">
            <SvgIcon :name="refreshing ? 'loader' : 'refresh-cw'" :size="13" />
          </span>
        </button>
      </div>
    </div>

    <div v-if="visibleCliIds.length === 0" class="cli-filter-empty" role="status">
      <span>未显示任何对话来源</span>
      <button type="button" @click="restoreAllCliSources">恢复全部</button>
    </div>

    <SessionTree
      ref="sessionTreeRef"
      :projects="projects"
      :selectedSessionIdentity="selectedSessionIdentity"
      :selectedSessionIdentities="selectedSessionIdentities"
      :contextMenuSessionIdentity="contextMenuSessionIdentity"
      :searchQuery="searchQuery"
      :isStreamingProjects="isStreamingProjects"
      :totalLoadedSessions="totalLoadedSessions"
      :isRefreshing="isRefreshing"
      :showLoadingIndicator="showLoadingIndicator"
      :bootstrapping="bootstrapping"
      :currentCliLabel="currentCliLabel"
      :projectAllSelected="projectAllSelected"
      :grouping="grouping"
      @selectSession="(identity: SessionIdentity, encodedDir: string) => emit('selectSession', identity, encodedDir)"
      @pinSession="(identity: SessionIdentity, encodedDir: string) => emit('pinSession', identity, encodedDir)"
      @contextMenuSession="(e: MouseEvent, identity: SessionIdentity, sessionId: string, displayName: string, projectPath: string) => emit('contextMenuSession', e, identity, sessionId, displayName, projectPath)"
      @contextMenuProject="(e: MouseEvent, project: ProjectInfo) => emit('contextMenuProject', e, project)"
      @renameSession="(identity: SessionIdentity, newName: string) => emit('renameSession', identity, newName)"
      @toggleSelect="(identity: SessionIdentity, e: MouseEvent) => emit('toggleSelect', identity, e)"
      @rangeSelect="(startIdentity: SessionIdentity, endIdentity: SessionIdentity) => emit('rangeSelect', startIdentity, endIdentity)"
      @clearSelection="emit('clearSelection')"
      @selectProjectSessions="emit('selectProjectSessions', $event)"
    />

    <div v-if="blockedProjects.size > 0" class="blocked-section">
      <button class="blocked-toggle" @click="showBlockedSection = !showBlockedSection">
        <SvgIcon :name="showBlockedSection ? 'chevron-down' : 'chevron-right'" :size="10" />
        <span>已屏蔽 {{ blockedProjects.size }} 个文件夹</span>
      </button>
      <div v-if="showBlockedSection" class="blocked-list">
        <div v-for="path in [...blockedProjects]" :key="path" class="blocked-item">
          <span class="blocked-item-name" :title="path">{{ path.split('/').pop() || path }}</span>
          <button class="blocked-item-unblock" title="取消屏蔽" @click="unblockProject(path)">
            <SvgIcon name="eye" :size="12" />
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.history-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.history-topbar {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--color-border);
}
.tree-grouping-toggle {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  background: var(--color-bg-subtle, rgba(0, 0, 0, 0.04));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
}
.grouping-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 20px;
  padding: 0;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  background: transparent;
  border: none;
  transition: all var(--transition-fast);
}
.grouping-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.grouping-btn.active {
  color: var(--color-text);
  background: var(--color-bg);
  box-shadow: var(--shadow-sm);
}
.cli-filter {
  position: relative;
  margin-left: auto;
}
.cli-filter-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 6px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--color-text-muted);
  transition: all var(--transition-fast);
}
.cli-filter-trigger:hover,
.cli-filter-trigger.partial {
  color: var(--color-text);
  background: var(--color-surface-selected);
}
.cli-filter-trigger.partial {
  color: var(--color-primary);
}
.cli-filter-trigger[aria-expanded="true"] {
  background: var(--color-bg-active);
  box-shadow: inset 0 0 0 1px var(--color-border);
}
.cli-filter-summary {
  font-size: 10px;
  white-space: nowrap;
}
.cli-filter-menu {
  position: absolute;
  z-index: 10;
  top: calc(100% + 4px);
  right: 0;
  width: 190px;
  padding: 4px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  box-shadow: var(--shadow-lg);
}
.cli-filter-option {
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 28px;
  gap: 7px;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  color: var(--color-text);
  cursor: pointer;
  text-align: left;
}
.cli-filter-option:hover,
.cli-filter-option:focus-visible {
  background: var(--color-surface-selected);
  outline: none;
}
.cli-filter-all {
  margin-bottom: 3px;
  border-bottom: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm) var(--radius-sm) 0 0;
}
.cli-filter-checkbox {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border: 1.5px solid var(--color-text-muted);
  border-radius: 3px;
  flex-shrink: 0;
}
.cli-filter-checkbox.checked {
  color: white;
  background: var(--color-primary);
  border-color: var(--color-primary);
}
.cli-filter-checkbox.partial {
  border-color: var(--color-primary);
}
.partial-mark {
  width: 7px;
  height: 2px;
  border-radius: 1px;
  background: var(--color-primary);
}
.cli-filter-avatar {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}
.cli-filter-avatar :deep(.avatar-svg) {
  width: 12px;
  height: 12px;
}
.cli-filter-avatar :deep(.brand-svg-fill) {
  width: 16px;
  height: 16px;
}
.cli-filter-option-label {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--text-xs);
}
.cli-filter-option-count {
  color: var(--color-text-muted);
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.cli-filter-error {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 3px 6px 5px 27px;
  color: var(--color-warning);
  font-size: 10px;
}
.cli-filter-error-text {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cli-filter-retry {
  flex-shrink: 0;
  padding: 1px 6px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  color: var(--color-primary);
  font-size: 10px;
  cursor: pointer;
}
.cli-filter-retry:hover {
  background: var(--color-bg-hover);
}
.cli-filter-empty {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  margin: var(--space-2) var(--space-3) 0;
  padding: var(--space-2);
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  font-size: var(--text-xs);
}
.cli-filter-empty button {
  color: var(--color-primary);
  cursor: pointer;
  font-size: inherit;
}
.blocked-section {
  border-top: 1px solid var(--color-border-light);
  padding: var(--space-1) var(--space-3);
  flex-shrink: 0;
}
.blocked-toggle {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  padding: 4px 0;
  font-size: 10px;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: color var(--transition-fast);
}
.blocked-toggle:hover { color: var(--color-text); }
.blocked-list {
  padding: var(--space-1) 0 var(--space-1) var(--space-3);
}
.blocked-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 0;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.blocked-item-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.blocked-item-unblock {
  display: flex;
  align-items: center;
  padding: 2px;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: color var(--transition-fast);
  flex-shrink: 0;
}
.blocked-item-unblock:hover { color: var(--color-primary); }
.search-row {
  display: flex;
  align-items: center;
  padding-right: var(--space-2);
}
.search-row .search-bar {
  flex: 1;
  min-width: 120px;
}
.tree-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: 4px;
  flex-shrink: 0;
}
.toolbar-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.toolbar-btn:hover {
  background: var(--color-surface-selected);
  color: var(--color-text);
}
.toolbar-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.refresh-icon-wrap {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.spinning {
  animation: spin 0.8s linear infinite;
}
</style>
