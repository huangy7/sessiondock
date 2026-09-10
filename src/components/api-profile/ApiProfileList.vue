<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SvgIcon from "../icons/SvgIcon.vue";

export interface ScopeUsage {
  scope: string;
  name: string;
}

export interface ScopeBindingInfo {
  scope: string;
  name: string;
  isGlobal?: boolean;
  activeProfile?: string | null;
}

export interface Profile {
  name: string;
  content: Record<string, any>;
  isActive?: boolean;
  usedInScopes?: ScopeUsage[];
}

export interface FieldInfo {
  label: string;
  key: string;
  masked?: boolean;
}

export interface TabInfo {
  id: string;
  name: string;
  dirs: string[];
}

const props = withDefaults(
  defineProps<{
    profiles: Profile[];
    profileFields: FieldInfo[];
    renamingProfile?: string | null;
    renameInput?: string;
    currentCliName?: string;
    currentSettingsPathHint?: string;
    profileCardHint?: string;
    currentCliId?: string;
    activeScope?: string;
    tabs?: TabInfo[];
    loadError?: string;
    scopeBindings?: ScopeBindingInfo[];
  }>(),
  {
    renamingProfile: null,
    renameInput: "",
    currentCliName: "",
    currentSettingsPathHint: "",
    profileCardHint: "",
    currentCliId: "claude",
    activeScope: "global",
    tabs: () => [],
    loadError: "",
    scopeBindings: () => [],
  }
);

const emit = defineEmits<{
  "update:renameInput": [value: string];
  cardDoubleClick: [name: string];
  confirmRename: [name: string];
  cancelRename: [];
  startRename: [name: string];
  openEditor: [name: string];
  duplicate: [name: string];
  delete: [name: string];
  navigateToScope: [scopeId: string];
  create: [];
  setGlobal: [name: string];
  openRawSettings: [];
}>();

const OFFICIAL_PROFILE_NAME = "Codex Official";

function isOfficialProfile(name: string): boolean {
  return name === OFFICIAL_PROFILE_NAME;
}

function getProfileInitials(name: string): string {
  if (!name) return "CF";
  const clean = name.replace(/[^a-zA-Z0-9]/g, "");
  if (clean.length >= 2) return clean.slice(0, 2).toUpperCase();
  return name.slice(0, 2).toUpperCase();
}

interface ProfileSummary {
  rawUrl: string;
  displayUrl: string;
  rawModel: string;
  displayModel: string;
  provider: "claude" | "deepseek" | "openai" | "glm" | "gemini" | "kimi" | "default";
}

function getProfileSummary(profile: Profile): ProfileSummary {
  const content = profile.content || {};
  const env = content.env || {};
  const codex = content.codex || {};

  const baseUrl = env.ANTHROPIC_BASE_URL || codex.base_url || "";
  const model = env.ANTHROPIC_MODEL || codex.model || "";

  // Clean URL
  let displayUrl = "";
  if (baseUrl) {
    displayUrl = baseUrl.replace(/^https?:\/\//i, "").replace(/\/+$/, "");
  } else {
    displayUrl = props.currentCliId === "claude" ? "api.anthropic.com" : "api.openai.com";
  }

  // Model
  let displayModel = model || env.ANTHROPIC_DEFAULT_SONNET_MODEL || env.ANTHROPIC_DEFAULT_OPUS_MODEL || "";
  if (!displayModel) {
    displayModel = props.currentCliId === "claude" ? "默认 (Sonnet)" : "默认 (gpt-5.4)";
  }

  // Detect provider
  let provider: ProfileSummary["provider"] = "default";
  const lowerName = profile.name.toLowerCase();
  const lowerUrl = (baseUrl || "").toLowerCase();

  if (lowerName.includes("deepseek") || lowerUrl.includes("deepseek")) {
    provider = "deepseek";
  } else if (lowerName.includes("openai") || lowerName.includes("codex") || lowerUrl.includes("openai")) {
    provider = "openai";
  } else if (lowerName.includes("glm") || lowerName.includes("zhipu") || lowerUrl.includes("bigmodel")) {
    provider = "glm";
  } else if (lowerName.includes("gemini") || lowerUrl.includes("google")) {
    provider = "gemini";
  } else if (lowerName.includes("kimi") || lowerUrl.includes("moonshot")) {
    provider = "kimi";
  } else if (lowerName.includes("claude") || lowerUrl.includes("anthropic")) {
    provider = "claude";
  }

  return {
    rawUrl: baseUrl,
    displayUrl,
    rawModel: model,
    displayModel,
    provider,
  };
}

function hasModelOverride(profile: Profile): boolean {
  const env = profile.content?.env || {};
  return Boolean(
    env.ANTHROPIC_DEFAULT_HAIKU_MODEL ||
    env.ANTHROPIC_DEFAULT_SONNET_MODEL ||
    env.ANTHROPIC_DEFAULT_OPUS_MODEL
  );
}

function getModelOverrideTooltip(profile: Profile): string {
  const env = profile.content?.env || {};
  const lines = [];
  if (env.ANTHROPIC_DEFAULT_SONNET_MODEL) {
    lines.push(`• Sonnet: ${env.ANTHROPIC_DEFAULT_SONNET_MODEL}${env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME ? ` (${env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME})` : ""}`);
  }
  if (env.ANTHROPIC_DEFAULT_OPUS_MODEL) {
    lines.push(`• Opus: ${env.ANTHROPIC_DEFAULT_OPUS_MODEL}${env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME ? ` (${env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME})` : ""}`);
  }
  if (env.ANTHROPIC_DEFAULT_HAIKU_MODEL) {
    lines.push(`• Haiku: ${env.ANTHROPIC_DEFAULT_HAIKU_MODEL}${env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME ? ` (${env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME})` : ""}`);
  }
  return lines.length > 0 ? `模型映射:\n${lines.join("\n")}` : "自定义模型映射";
}

function getProfileTooltip(profile: Profile): string {
  const summary = getProfileSummary(profile);
  const lines = [];
  lines.push(`端点: ${summary.rawUrl || "官方默认"}`);
  lines.push(`主模型: ${summary.rawModel || "系统默认"}`);
  if (hasModelOverride(profile)) {
    lines.push(getModelOverrideTooltip(profile));
  }
  return lines.join("\n");
}

function getProfileScopes(profile: Profile): ScopeUsage[] {
  if (profile.usedInScopes && profile.usedInScopes.length > 0) {
    return profile.usedInScopes;
  }
  if (props.scopeBindings && props.scopeBindings.length > 0) {
    return props.scopeBindings
      .filter((b) => b.activeProfile === profile.name)
      .map((b) => ({
        scope: b.scope,
        name: b.name || (b.isGlobal || b.scope === "global" ? "全局" : b.scope),
      }));
  }
  if (profile.isActive) {
    const currentTab = props.tabs.find((t) => t.id === props.activeScope);
    const scopeName = currentTab ? currentTab.name : "全局";
    return [{ scope: props.activeScope || "global", name: scopeName }];
  }
  return [];
}

function isGlobalActive(profile: Profile): boolean {
  if (props.scopeBindings && props.scopeBindings.length > 0) {
    const gb = props.scopeBindings.find((b) => b.scope === "global" || b.isGlobal);
    if (gb && gb.activeProfile === profile.name) {
      return true;
    }
  }
  return false;
}

function handleCardClick(profile: Profile) {
  if (!isGlobalActive(profile)) {
    emit("setGlobal", profile.name);
  }
}

function getProjectScopes(profile: Profile): ScopeUsage[] {
  return getProfileScopes(profile).filter((s) => s.scope !== "global");
}

const isSettingsPopoverOpen = ref(false);
const copiedSettingsPath = ref(false);
const settingsPopoverRef = ref<HTMLElement | null>(null);
let copyTimeout: any = null;

function toggleSettingsPopover() {
  isSettingsPopoverOpen.value = !isSettingsPopoverOpen.value;
}

function closeSettingsPopover() {
  isSettingsPopoverOpen.value = false;
}

async function copySettingsPath() {
  if (!props.currentSettingsPathHint) return;
  try {
    await navigator.clipboard.writeText(props.currentSettingsPathHint);
    copiedSettingsPath.value = true;
    if (copyTimeout) clearTimeout(copyTimeout);
    copyTimeout = setTimeout(() => {
      copiedSettingsPath.value = false;
      closeSettingsPopover();
    }, 1200);
  } catch (err) {
    console.error("Failed to copy path:", err);
  }
}

async function openSettingsInFinder() {
  if (!props.currentSettingsPathHint) return;
  closeSettingsPopover();
  try {
    await invoke("open_path_in_file_manager", { path: props.currentSettingsPathHint });
  } catch (err) {
    console.error("Failed to open file in manager:", err);
    copySettingsPath();
  }
}

function handleOpenRawEditor() {
  closeSettingsPopover();
  emit("openRawSettings");
}

function handleClickOutside(event: MouseEvent) {
  if (
    isSettingsPopoverOpen.value &&
    settingsPopoverRef.value &&
    !settingsPopoverRef.value.contains(event.target as Node)
  ) {
    closeSettingsPopover();
  }
}

onMounted(() => {
  window.addEventListener("click", handleClickOutside);
});

onUnmounted(() => {
  window.removeEventListener("click", handleClickOutside);
  if (copyTimeout) clearTimeout(copyTimeout);
});
</script>

<template>
  <div class="profile-library-section">
    <!-- Section Header: Title & Direct Action -->
    <div class="library-section-header">
      <div class="section-title-group">
        <h3 class="section-title">全局配置库</h3>
        <span class="section-count-badge">{{ profiles.length }}</span>
        <span v-if="currentSettingsPathHint" class="section-sub-hint">
          激活后自动写入
          <div ref="settingsPopoverRef" class="settings-trigger-wrapper">
            <button
              type="button"
              class="settings-path-chip"
              :class="{ 'is-active': isSettingsPopoverOpen }"
              title="点击查看与编辑原生配置文件"
              @click.stop="toggleSettingsPopover"
            >
              <SvgIcon name="file-text" :size="11" class="chip-file-icon" />
              <span class="settings-path-text">{{ currentSettingsPathHint }}</span>
              <SvgIcon
                name="chevron-down"
                :size="10"
                class="chip-chevron-icon"
                :class="{ 'is-open': isSettingsPopoverOpen }"
              />
            </button>

            <!-- Dropdown Popover Menu -->
            <Transition name="fade-popover">
              <div v-if="isSettingsPopoverOpen" class="settings-popover-menu" @click.stop>
                <div class="popover-header">
                  <span class="popover-title">原生配置文件 (Native Settings)</span>
                  <span class="popover-path" :title="currentSettingsPathHint">{{ currentSettingsPathHint }}</span>
                </div>

                <div class="popover-menu-list">
                  <button
                    type="button"
                    class="popover-menu-item primary-action"
                    @click="handleOpenRawEditor"
                  >
                    <div class="item-icon-box">
                      <SvgIcon name="code" :size="13" />
                    </div>
                    <div class="item-text-group">
                      <span class="item-label">在应用内编辑 JSON</span>
                      <span class="item-desc">直接修改 Claude 原生配置文件</span>
                    </div>
                  </button>

                  <button
                    type="button"
                    class="popover-menu-item"
                    @click="openSettingsInFinder"
                  >
                    <div class="item-icon-box">
                      <SvgIcon name="external-link" :size="13" />
                    </div>
                    <div class="item-text-group">
                      <span class="item-label">在 Finder 中打开</span>
                      <span class="item-desc">定位到系统文件目录</span>
                    </div>
                  </button>

                  <button
                    type="button"
                    class="popover-menu-item"
                    @click="copySettingsPath"
                  >
                    <div class="item-icon-box" :class="{ 'is-copied': copiedSettingsPath }">
                      <SvgIcon :name="copiedSettingsPath ? 'check' : 'copy'" :size="13" />
                    </div>
                    <div class="item-text-group">
                      <span class="item-label">{{ copiedSettingsPath ? '已复制路径' : '复制完整路径' }}</span>
                      <span class="item-desc">复制绝对路径到剪贴板</span>
                    </div>
                  </button>
                </div>
              </div>
            </Transition>
          </div>
        </span>
      </div>

      <button
        type="button"
        class="btn-create-profile"
        @click="emit('create')"
      >
        <SvgIcon name="plus" :size="13" />
        <span>新建配置</span>
      </button>
    </div>

    <!-- Empty State -->
    <div v-if="profiles.length === 0" class="empty-state">
      <div class="empty-icon-wrap">
        <SvgIcon name="archive" :size="24" />
      </div>
      <p class="empty-title">配置库暂无可用配置</p>
      <p class="empty-desc">
        点击右上角「新建配置」或顶部「导入备份」添加 API 配置文件。
      </p>
    </div>

    <!-- Profile Cards (Click to Select, [✏️] to Edit) -->
    <div v-else class="profile-list">
      <div
        v-for="p in profiles"
        :key="p.name"
        class="profile-card"
        :class="{ 'is-global': isGlobalActive(p) }"
        :title="isGlobalActive(p) ? '当前全局生效配置（点右侧 ✏️ 编辑）' : `点击将「${p.name}」设为全局生效配置`"
        @click="handleCardClick(p)"
        @dblclick.stop="emit('openEditor', p.name)"
      >
        <!-- Left Column: Avatar, Name, Endpoint -->
        <div class="card-left-col">
          <div class="card-avatar" :class="`provider-${getProfileSummary(p).provider}`">
            <span>{{ getProfileInitials(p.name) }}</span>
          </div>

          <div class="card-name-wrap">
            <template v-if="renamingProfile === p.name">
              <input
                class="profile-rename-input"
                :value="renameInput"
                autocomplete="off"
                autocapitalize="off"
                autocorrect="off"
                spellcheck="false"
                @input="emit('update:renameInput', ($event.target as HTMLInputElement).value)"
                @click.stop
                @mousedown.stop
                @keyup.enter.stop.prevent="emit('confirmRename', p.name)"
                @keyup.escape.stop.prevent="emit('cancelRename')"
                @blur="emit('confirmRename', p.name)"
              />
            </template>
            <template v-else>
              <span class="profile-card-name" :title="p.name">{{ p.name }}</span>
            </template>
          </div>

          <!-- Endpoint Meta with Tooltip -->
          <div class="card-endpoint-meta">
            <span
              class="meta-chunk"
              :title="getProfileTooltip(p)"
            >
              <SvgIcon name="globe" :size="11" class="meta-icon" />
              <span class="meta-text mono">{{ getProfileSummary(p).displayUrl }}</span>
            </span>

            <span
              v-if="hasModelOverride(p)"
              class="override-icon-tag"
              :title="getModelOverrideTooltip(p)"
            >
              <SvgIcon name="layers" :size="11" />
            </span>
          </div>
        </div>

        <!-- Right Column: Badges, Scope Chips & Actions -->
        <div class="card-right-col" @click.stop>
          <!-- Official Badge -->
          <span v-if="isOfficialProfile(p.name)" class="official-badge">
            <span class="status-dot"></span>
            <span>官方</span>
          </span>

          <!-- Global Active Pill Badge -->
          <span
            v-if="isGlobalActive(p)"
            class="global-active-pill"
            title="当前作为全局生效配置"
          >
            <SvgIcon name="check" :size="11" />
            <span>全局生效</span>
          </span>

          <!-- Project Scope Reference Chips -->
          <div v-if="getProjectScopes(p).length > 0" class="scope-chips-group">
            <button
              v-for="s in getProjectScopes(p)"
              :key="s.scope"
              type="button"
              class="scope-chip"
              :title="`正在用于「${s.name}」项目作用域，点击跳转`"
              @click.stop="emit('navigateToScope', s.scope)"
            >
              <SvgIcon name="folder" :size="10" />
              <span>{{ s.name }}</span>
            </button>
          </div>

          <!-- Actions -->
          <div v-if="!isOfficialProfile(p.name)" class="card-actions">
            <button
              class="card-action-btn"
              title="编辑配置"
              @click.stop="emit('openEditor', p.name)"
            >
              <SvgIcon name="pencil" :size="12" />
            </button>
            <button
              class="card-action-btn"
              title="复制配置"
              @click.stop="emit('duplicate', p.name)"
            >
              <SvgIcon name="copy" :size="12" />
            </button>
            <button
              class="card-action-btn action-danger"
              title="删除配置"
              @click.stop="emit('delete', p.name)"
            >
              <SvgIcon name="trash-2" :size="12" />
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.profile-library-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.library-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 2px;
}

.section-title-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.section-count-badge {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 10px;
  background: var(--color-bg-secondary);
  color: var(--color-text-muted);
  border: 1px solid var(--color-border);
}

.section-sub-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-left: 2px;
}

.settings-trigger-wrapper {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.settings-path-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono, monospace);
  font-size: 10px;
  padding: 1.5px 6px;
  border-radius: 4px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  color: var(--color-text-secondary);
  max-width: 280px;
  cursor: pointer;
  transition: all var(--transition-fast);
  text-decoration: none;
  box-sizing: border-box;
  user-select: none;
}

.settings-path-chip:hover,
.settings-path-chip.is-active {
  border-color: var(--color-primary-light, rgba(37, 99, 235, 0.4));
  background: var(--color-bg-hover);
  color: var(--color-primary);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
}

.chip-file-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.settings-path-chip:hover .chip-file-icon,
.settings-path-chip.is-active .chip-file-icon {
  color: var(--color-primary);
}

.settings-path-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip-chevron-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
  transition: transform var(--transition-fast);
}

.chip-chevron-icon.is-open {
  transform: rotate(180deg);
}

.settings-path-chip:hover .chip-chevron-icon,
.settings-path-chip.is-active .chip-chevron-icon {
  color: var(--color-primary);
}

/* Dropdown Popover Menu */
.settings-popover-menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: var(--z-dropdown, 100);
  width: 280px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.15), 0 8px 10px -6px rgba(0, 0, 0, 0.1);
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.popover-header {
  padding: 6px 8px 8px;
  border-bottom: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.popover-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--color-text);
}

.popover-path {
  font-family: var(--font-mono, monospace);
  font-size: 9.5px;
  color: var(--color-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.popover-menu-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-top: 2px;
}

.popover-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius-md, 6px);
  border: none;
  background: transparent;
  color: var(--color-text);
  cursor: pointer;
  transition: all var(--transition-fast);
  text-align: left;
  width: 100%;
}

.popover-menu-item:hover {
  background: var(--color-bg-hover);
}

.popover-menu-item.primary-action:hover {
  background: rgba(37, 99, 235, 0.08);
}

.popover-menu-item.primary-action:hover .item-label {
  color: var(--color-primary);
}

.item-icon-box {
  width: 24px;
  height: 24px;
  border-radius: 4px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-secondary);
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.item-icon-box.is-copied {
  color: var(--color-success, #10b981);
  background: rgba(16, 185, 129, 0.1);
  border-color: rgba(16, 185, 129, 0.3);
}

.item-text-group {
  display: flex;
  flex-direction: column;
  gap: 1px;
  overflow: hidden;
}

.item-label {
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text);
}

.item-desc {
  font-size: 10px;
  color: var(--color-text-muted);
}

.fade-popover-enter-active,
.fade-popover-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}

.fade-popover-enter-from,
.fade-popover-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

.btn-create-profile {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 9px;
  border-radius: 6px;
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: var(--color-text-inverse);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.btn-create-profile:hover {
  opacity: 0.92;
  transform: translateY(-0.5px);
}

.profile-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 28px 16px;
  background: var(--color-bg-secondary);
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-md);
  text-align: center;
}

.empty-icon-wrap {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-full);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  margin-bottom: 8px;
}

.empty-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0 0 2px;
}

.empty-desc {
  font-size: 11px;
  color: var(--color-text-muted);
  margin: 0;
  max-width: 320px;
  line-height: 1.4;
}

/* Strictly Single-Line Profile Card (~42px) */
.profile-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px;
  height: 42px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  gap: 12px;
  transition: all 0.12s cubic-bezier(0.16, 1, 0.3, 1);
  cursor: pointer;
  user-select: none;
}

.profile-card:hover {
  border-color: var(--color-primary-light, rgba(37, 99, 235, 0.4));
  background: var(--color-bg-hover, rgba(0, 0, 0, 0.015));
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.04);
}

.profile-card.is-global {
  border-color: var(--color-primary);
  border-left: 3.5px solid var(--color-primary);
  background: rgba(37, 99, 235, 0.025);
}

.global-active-pill {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  background: rgba(37, 99, 235, 0.1);
  color: var(--color-primary);
  border: 1px solid rgba(37, 99, 235, 0.2);
}

.card-left-col {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.card-avatar {
  width: 24px;
  height: 24px;
  border-radius: 5px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 700;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  flex-shrink: 0;
  letter-spacing: 0.5px;
}

/* Provider branding palettes */
.card-avatar.provider-claude {
  background: linear-gradient(135deg, rgba(234, 88, 12, 0.12), rgba(217, 119, 6, 0.08));
  color: #ea580c;
  border: 1px solid rgba(234, 88, 12, 0.25);
}

.card-avatar.provider-deepseek {
  background: linear-gradient(135deg, rgba(37, 99, 235, 0.12), rgba(2, 132, 199, 0.08));
  color: #2563eb;
  border: 1px solid rgba(37, 99, 235, 0.25);
}

.card-avatar.provider-openai {
  background: linear-gradient(135deg, rgba(16, 185, 129, 0.12), rgba(5, 150, 105, 0.08));
  color: #059669;
  border: 1px solid rgba(16, 185, 129, 0.25);
}

.card-avatar.provider-glm {
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.12), rgba(139, 92, 246, 0.08));
  color: #6366f1;
  border: 1px solid rgba(99, 102, 241, 0.25);
}

.card-avatar.provider-gemini {
  background: linear-gradient(135deg, rgba(14, 165, 233, 0.12), rgba(168, 85, 247, 0.08));
  color: #0284c7;
  border: 1px solid rgba(14, 165, 233, 0.25);
}

.card-avatar.provider-kimi {
  background: linear-gradient(135deg, rgba(79, 70, 229, 0.12), rgba(67, 56, 202, 0.08));
  color: #4f46e5;
  border: 1px solid rgba(79, 70, 229, 0.25);
}

.card-avatar.provider-default {
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
}

.card-name-wrap {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.profile-card-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
}

.profile-rename-input {
  font-size: 12px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 4px;
  border: 1px solid var(--color-primary);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  width: 120px;
}

.card-endpoint-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--color-text-muted);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-chunk {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-icon {
  color: var(--color-text-muted);
  opacity: 0.7;
  flex-shrink: 0;
}

.meta-text.mono {
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.override-icon-tag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 3px;
  background: rgba(37, 99, 235, 0.08);
  color: var(--color-primary);
  flex-shrink: 0;
}

.card-right-col {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.official-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  background: rgba(16, 185, 129, 0.1);
  color: #059669;
  border: 1px solid rgba(16, 185, 129, 0.2);
}

.status-dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: #059669;
}

.scope-chips-group {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.scope-chip {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 500;
  background: var(--color-bg-secondary);
  color: var(--color-primary);
  border: 1px solid var(--color-border);
  cursor: pointer;
  transition: all 0.12s ease;
}

.scope-chip:hover {
  background: rgba(37, 99, 235, 0.12);
  border-color: var(--color-primary);
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 1px;
  margin-left: 2px;
}

.card-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all 0.12s ease;
}

.card-action-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.card-action-btn.action-danger:hover {
  color: var(--color-danger);
  background: rgba(239, 68, 68, 0.08);
}
</style>

