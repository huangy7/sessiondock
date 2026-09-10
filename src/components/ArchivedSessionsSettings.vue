<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import { useSessions } from "../composables/useSessions";
import type { CliId } from "../types/cli";
import { formatRelativeTime } from "../utils/format";
import type { SessionIdentity } from "../types/session";

interface ArchivedSessionEntry {
  sessionPath: string;
  sessionId: string;
  projectPath: string | null;
  displayName: string | null;
  firstUserMessage: string | null;
  archivedAt: string;
  pinned: boolean;
  sourceExists: boolean;
}

interface ProjectGroup {
  projectPath: string;
  projectName: string;
  entries: ArchivedSessionEntry[];
}

interface RebuildArchiveResult {
  restored: number;
  skipped: number;
}

interface RebuildProgress {
  step: string;
  current: number;
  total: number;
}

const emit = defineEmits<{
  openSession: [identity: SessionIdentity];
}>();

const props = defineProps<{ cliId?: CliId }>();
const { refresh } = useSessions();
const activeCliId = computed(() => props.cliId);

const entries = ref<ArchivedSessionEntry[]>([]);
const loading = ref(true);
const loadError = ref("");
const busyPath = ref<string | null>(null);
const collapsedProjects = ref<Record<string, boolean>>({});
const rebuilding = ref(false);
const rebuildProgress = ref<RebuildProgress | null>(null);
let unlistenRebuild: UnlistenFn | undefined;

const rebuildProgressPercent = computed(() => {
  const p = rebuildProgress.value;
  if (!p || p.total <= 0) return 0;
  return Math.min(100, Math.round((p.current / p.total) * 100));
});

async function handleRebuildIndex() {
  if (rebuilding.value) return;
  rebuilding.value = true;
  rebuildProgress.value = null;
  try {
    unlistenRebuild?.();
    unlistenRebuild = await listen<RebuildProgress>("archive-rebuild-progress", (event) => {
      rebuildProgress.value = event.payload;
    });
    const result = await invoke<RebuildArchiveResult>("rebuild_archive_index_from_disk", {
      cliId: activeCliId.value,
    });
    await Promise.all([loadEntries(), refresh("snapshot")]);
    const skippedHint = result.skipped > 0 ? `，跳过 ${result.skipped} 个无法识别的文件` : "";
    await message(`成功扫描并恢复 ${result.restored} 个归档会话${skippedHint}！`, {
      title: "归档恢复成功",
      kind: "info",
    });
  } catch (e: any) {
    await message(`重建归档失败: ${e?.message ?? String(e)}`, {
      title: "错误",
      kind: "error",
    });
  } finally {
    unlistenRebuild?.();
    unlistenRebuild = undefined;
    rebuildProgress.value = null;
    rebuilding.value = false;
  }
}

async function loadEntries() {
  if (!activeCliId.value) {
    entries.value = [];
    loading.value = false;
    return;
  }
  loading.value = true;
  try {
    entries.value = await invoke<ArchivedSessionEntry[]>("list_archived_sessions", {
      cliId: activeCliId.value,
    });
    loadError.value = "";
  } catch (e: any) {
    loadError.value = e?.message ?? String(e);
  } finally {
    loading.value = false;
  }
}

const groupedEntries = computed<ProjectGroup[]>(() => {
  const map = new Map<string, { name: string; entries: ArchivedSessionEntry[] }>();
  for (const entry of entries.value) {
    const rawPath = entry.projectPath || "";
    const name = rawPath ? (rawPath.split("/").filter(Boolean).pop() || rawPath) : "未归类项目";
    if (!map.has(rawPath)) {
      map.set(rawPath, { name, entries: [] });
    }
    map.get(rawPath)!.entries.push(entry);
  }

  return Array.from(map.entries()).map(([projectPath, group]) => ({
    projectPath,
    projectName: group.name,
    entries: group.entries,
  }));
});

function isProjectCollapsed(projectPath: string): boolean {
  return collapsedProjects.value[projectPath] ?? true;
}

function toggleProject(projectPath: string) {
  collapsedProjects.value[projectPath] = !isProjectCollapsed(projectPath);
}

function entryTitle(entry: ArchivedSessionEntry): string {
  if (entry.displayName && entry.displayName.trim()) {
    const name = entry.displayName.trim();
    return name.length > 60 ? `${name.slice(0, 60)}…` : name;
  }
  if (entry.firstUserMessage && entry.firstUserMessage.trim()) {
    const msg = entry.firstUserMessage.trim();
    return msg.length > 60 ? `${msg.slice(0, 60)}…` : msg;
  }
  return "未命名会话";
}

async function togglePin(entry: ArchivedSessionEntry) {
  if (busyPath.value) return;
  busyPath.value = entry.sessionPath;
  try {
    await invoke("set_session_archive_pinned", {
      cliId: activeCliId.value,
      filePath: entry.sessionPath,
      pinned: !entry.pinned,
    });
    await Promise.all([loadEntries(), refresh("snapshot")]);
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "操作失败", kind: "error" });
  } finally {
    busyPath.value = null;
  }
}

async function restoreToDisk(entry: ArchivedSessionEntry) {
  if (busyPath.value) return;
  busyPath.value = entry.sessionPath;
  try {
    await invoke("restore_session_to_disk", {
      cliId: activeCliId.value,
      filePath: entry.sessionPath,
    });
    await Promise.all([loadEntries(), refresh("snapshot")]);
    await message("已成功将归档会话解压恢复至本地磁盘，CLI 可继续使用该会话。", {
      title: "恢复成功",
      kind: "info",
    });
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "恢复到磁盘失败", kind: "error" });
  } finally {
    busyPath.value = null;
  }
}

async function removeArchive(entry: ArchivedSessionEntry) {
  if (busyPath.value) return;
  const hint = entry.sourceExists
    ? "将删除此会话的归档快照（原始会话文件不受影响）。"
    : "原始会话文件已不存在，删除归档后此会话将从列表中彻底消失，无法恢复。";
  const confirmed = await ask(`${hint}\n\n${entryTitle(entry)}`, {
    title: "删除归档",
    kind: "warning",
  });
  if (!confirmed) return;
  busyPath.value = entry.sessionPath;
  try {
    await invoke("delete_session_archive", {
      cliId: activeCliId.value,
      filePath: entry.sessionPath,
    });
    await Promise.all([loadEntries(), refresh("snapshot")]);
  } catch (e: any) {
    await message(e?.message ?? String(e), { title: "删除失败", kind: "error" });
  } finally {
    busyPath.value = null;
  }
}

onMounted(loadEntries);
onBeforeUnmount(() => {
  unlistenRebuild?.();
});

watch(
  () => props.cliId,
  () => {
    void loadEntries();
  },
);
</script>

<template>
  <div class="archived-settings">
    <div class="archive-list-header">
      <span class="archive-list-title">已归档会话 ({{ entries.length }})</span>
      <button
        type="button"
        class="btn-action"
        :disabled="rebuilding || loading"
        title="扫描磁盘 archives 目录并重建数据库索引"
        @click="handleRebuildIndex"
      >
        <SvgIcon name="refresh-cw" :size="12" :class="{ spin: rebuilding }" />
        <span>{{ rebuilding ? "扫描中…" : "扫描磁盘" }}</span>
      </button>
    </div>

    <div v-if="rebuilding" class="rebuild-progress-track">
      <div class="rebuild-progress-fill" :style="{ width: `${rebuildProgressPercent}%` }"></div>
    </div>

    <p v-if="loading" class="empty-text">加载中…</p>
    <p v-else-if="loadError" class="error-text">{{ loadError }}</p>
    <p v-else-if="entries.length === 0" class="empty-text">暂无已归档会话</p>

    <div v-else class="project-groups">
      <div
        v-for="group in groupedEntries"
        :key="group.projectPath"
        class="project-group"
      >
        <!-- Project Header -->
        <div
          class="group-header"
          @click="toggleProject(group.projectPath)"
        >
          <div class="group-title">
            <SvgIcon name="folder" :size="15" class="folder-icon" />
            <span class="project-name">{{ group.projectName }}</span>
            <span class="count-badge">{{ group.entries.length }}</span>
          </div>
          <SvgIcon
            :name="isProjectCollapsed(group.projectPath) ? 'chevron-right' : 'chevron-down'"
            :size="14"
            class="chevron-icon"
          />
        </div>

        <!-- Group Entries -->
        <div
          v-show="!isProjectCollapsed(group.projectPath)"
          class="entry-list"
        >
          <div
            v-for="entry in group.entries"
            :key="entry.sessionPath"
            class="entry-row"
            :class="{ busy: busyPath === entry.sessionPath }"
            @click="emit('openSession', { cliId: activeCliId!, filePath: entry.sessionPath })"
          >
            <div class="entry-main">
              <div class="entry-title-line">
                <span class="entry-title">{{ entryTitle(entry) }}</span>
                <span v-if="entry.pinned" class="badge badge-pinned">永久保留</span>
                <span v-else-if="!entry.sourceExists" class="badge badge-archived">已归档</span>
                <span v-else class="badge badge-snapshot">已快照</span>
              </div>
              <div class="entry-meta">
                <span>归档于 {{ formatRelativeTime(entry.archivedAt) }}</span>
              </div>
            </div>
            <div class="entry-actions" @click.stop>
              <button
                class="row-icon-btn"
                :disabled="!!busyPath"
                :title="entry.pinned ? '取消永久保留' : '设为永久保留'"
                @click="togglePin(entry)"
              >
                <SvgIcon :name="entry.pinned ? 'pin-off' : 'pin'" :size="13" />
              </button>
              <button
                v-if="!entry.sourceExists"
                class="row-icon-btn"
                :disabled="!!busyPath"
                title="将归档快照解压写回磁盘，供 CLI 恢复继续对话"
                @click="restoreToDisk(entry)"
              >
                <SvgIcon name="rotate-ccw" :size="13" />
              </button>
              <button
                class="row-icon-btn danger"
                :disabled="!!busyPath"
                :title="entry.sourceExists ? '删除归档快照（源文件仍保留在磁盘上）' : '删除归档'"
                @click="removeArchive(entry)"
              >
                <SvgIcon name="trash-2" :size="13" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <p class="hint-text">
      超过 28 天的会话会自动快照归档；「永久保留」的归档不受保留天数限制，不会被自动清理。
      点击会话可跳转到主界面查看内容。
    </p>
  </div>
</template>

<style scoped>
.archived-settings {
  display: flex;
  flex-direction: column;
}
.archive-list-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px var(--space-5, 20px) 8px;
}
.archive-list-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
}
.btn-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 5px 11px;
  font-size: 12px;
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text);
  cursor: pointer;
  white-space: nowrap;
  transition: background-color var(--transition-fast, 120ms ease), border-color var(--transition-fast, 120ms ease);
}
.btn-action:hover:not(:disabled) {
  background: var(--color-bg-hover);
  border-color: var(--color-border-hover, var(--color-border));
}
.btn-action:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
/* @keyframes spin 定义于全局 base.css */
.spin {
  animation: spin 1s linear infinite;
}
.rebuild-progress-track {
  height: 3px;
  margin: 0 var(--space-5, 20px) 10px;
  border-radius: 2px;
  background: var(--color-border);
  overflow: hidden;
}
.empty-text {
  padding: 16px;
  font-size: 13px;
  color: var(--color-text-muted);
}
.error-text {
  padding: 16px;
  font-size: 13px;
  color: var(--color-danger);
}
.project-groups {
  display: flex;
  flex-direction: column;
  max-height: 380px;
  overflow-y: auto;
}
.project-group {
  border-bottom: 1px solid var(--color-border);
}
.project-group:last-child {
  border-bottom: none;
}
.group-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 16px;
  background: var(--color-bg-subtle, rgba(0, 0, 0, 0.02));
  cursor: pointer;
  user-select: none;
  transition: background var(--transition-fast);
}
.group-header:hover {
  background: var(--color-bg-hover);
}
.group-title {
  display: flex;
  align-items: center;
  gap: 8px;
}
.folder-icon {
  color: var(--color-primary);
  opacity: 0.8;
}
.project-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
}
.count-badge {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 10px;
  background: var(--color-border);
  color: var(--color-text-muted);
}
.chevron-icon {
  color: var(--color-text-muted);
}
.entry-list {
  display: flex;
  flex-direction: column;
}
.entry-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px 8px 32px;
  border-top: 1px solid var(--color-border-subtle, var(--color-border));
  cursor: pointer;
  transition: background var(--transition-fast);
}
.entry-row:hover {
  background: var(--color-bg-hover);
}
.entry-row.busy {
  opacity: 0.5;
  pointer-events: none;
}
.entry-main {
  flex: 1;
  min-width: 0;
}
.entry-title-line {
  display: flex;
  align-items: center;
  gap: 8px;
}
.entry-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.badge {
  flex-shrink: 0;
  font-size: 11px;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
}
.badge-pinned {
  background: var(--color-primary);
  color: white;
}
.badge-archived {
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
  border: 1px solid var(--color-border);
}
.badge-snapshot {
  background: transparent;
  color: var(--color-text-muted);
  border: 1px solid var(--color-border);
}
.entry-meta {
  display: flex;
  gap: 12px;
  margin-top: 2px;
  font-size: 11px;
  color: var(--color-text-muted);
}
.entry-actions {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
  opacity: 0;
  transition: opacity var(--transition-fast);
}
.entry-row:hover .entry-actions,
.entry-row:focus-within .entry-actions,
.entry-row.busy .entry-actions {
  opacity: 1;
}
.row-icon-btn {
  display: inline-flex;
  align-items: center;
  padding: 4px;
  border: none;
  border-radius: var(--radius-sm, 4px);
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: background-color var(--transition-fast), color var(--transition-fast);
}
.row-icon-btn:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.row-icon-btn.danger:hover:not(:disabled) {
  background: rgba(255, 59, 48, 0.08);
  color: var(--color-danger);
}
.row-icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.hint-text {
  margin: 8px 16px 12px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--color-text-muted);
}
.rebuild-progress-fill {
  height: 100%;
  border-radius: 2px;
  background: var(--color-primary);
  transition: width 0.2s ease;
}
</style>
