<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import OptionCard from "./OptionCard.vue";
import ToggleSwitch from "./ToggleSwitch.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { useTheme, type Theme } from "../composables/useTheme";
import { afterNextPaint } from "../utils/defer";

const localError = ref("");

const { theme } = useTheme();
const isWindows = navigator.userAgent.toLowerCase().includes("windows");

const themeOptions: Array<{ value: Theme; label: string; hint: string }> = [
  { value: "system", label: "跟随系统", hint: "自动匹配系统外观" },
  { value: "light", label: "浅色", hint: "适合明亮环境" },
  { value: "dark", label: "深色", hint: "适合长时间阅读" },
];

const activeThemeHint = computed(() => {
  return themeOptions.find((opt) => opt.value === theme.value)?.hint ?? "设置默认外观模式。";
});

const dockVisible = ref(true);
const dockToggling = ref(false);

async function loadDockVisible() {
  if (isWindows) return;
  dockToggling.value = true;
  try {
    dockVisible.value = await invoke<boolean>("get_dock_visible");
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
  } finally {
    dockToggling.value = false;
  }
}

async function onDockVisibleChange(val: boolean) {
  if (dockToggling.value) return;
  const prev = dockVisible.value;
  dockVisible.value = val;
  dockToggling.value = true;
  try {
    await invoke("set_dock_visible", { visible: val });
  } catch (e: any) {
    dockVisible.value = prev;
    localError.value = e?.message ?? String(e);
  } finally {
    dockToggling.value = false;
  }
}

const isMacOS = navigator.userAgent.toLowerCase().includes("mac");
const deskPetEnabled = ref(false);
const deskPetToggling = ref(false);

async function loadDeskPetEnabled() {
  if (!isMacOS) return;
  deskPetToggling.value = true;
  try {
    deskPetEnabled.value = await invoke<boolean>("get_desk_pet_enabled");
  } catch (e: any) {
    localError.value = e?.message ?? String(e);
  } finally {
    deskPetToggling.value = false;
  }
}

async function onDeskPetChange(val: boolean) {
  if (deskPetToggling.value) return;
  const prev = deskPetEnabled.value;
  deskPetEnabled.value = val;
  deskPetToggling.value = true;
  try {
    await invoke("set_desk_pet_enabled", { enabled: val });
  } catch (e: any) {
    deskPetEnabled.value = prev;
    localError.value = e?.message ?? String(e);
  } finally {
    deskPetToggling.value = false;
  }
}

onMounted(async () => {
  await afterNextPaint();
  void loadDockVisible();
  void loadDeskPetEnabled();
});
</script>

<template>
  <div class="settings-group">
    <div v-if="localError" class="settings-error">
      <span>{{ localError }}</span>
      <button class="clear-error" type="button" aria-label="关闭错误提示" @click="localError = ''">
        <SvgIcon name="x" :size="16" />
      </button>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">外观模式</div>
        <p class="row-hint">{{ activeThemeHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="theme" :options="themeOptions" />
      </div>
    </div>

    <div v-if="!isWindows" class="settings-row">
      <div class="row-content">
        <div class="row-title">在 Dock 中显示图标</div>
        <p class="row-hint">
          {{ dockToggling ? "正在切换 Dock 图标显示状态…" : "关闭后 SessionDock 仍可继续运行，只是不常驻 Dock。" }}
        </p>
      </div>
      <div class="row-action">
        <ToggleSwitch
          :model-value="dockVisible"
          :disabled="dockToggling"
          aria-label="在 Dock 中显示图标"
          @update:model-value="onDockVisibleChange"
        />
      </div>
    </div>

    <div v-if="isMacOS" class="settings-row">
      <div class="row-content">
        <div class="row-title">桌面宠物（试验性功能）</div>
        <p class="row-hint">
          在桌面上显示一只动画桌宠，实时反映 Agent 的运行状态（工作、待关注、告警等），支持拖拽与点击互动。
        </p>
      </div>
      <div class="row-action">
        <ToggleSwitch
          :model-value="deskPetEnabled"
          :disabled="deskPetToggling"
          aria-label="桌面宠物"
          @update:model-value="onDeskPetChange"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-group {
  display: flex;
  flex-direction: column;
}
.settings-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-4);
  background: rgba(220, 38, 38, 0.08);
  border-bottom: 1px solid rgba(220, 38, 38, 0.16);
  color: var(--color-danger);
  font-size: var(--text-sm);
}
.clear-error {
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-danger);
  cursor: pointer;
  padding: 2px;
  border-radius: 50%;
  opacity: 0.6;
  transition: all var(--transition-fast);
}
.clear-error:hover {
  opacity: 1;
  background: rgba(220, 38, 38, 0.1);
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  gap: var(--space-4);
}
.settings-row + .settings-row {
  border-top: 1px solid var(--color-border);
}
.row-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}
.row-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.row-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin: 0;
  line-height: 1.5;
}
.row-action {
  flex-shrink: 0;
}
</style>
