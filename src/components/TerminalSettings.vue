<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import OptionCard from "./OptionCard.vue";
import ElegantSelect from "./common/ElegantSelect.vue";
import { useAgentStatusMode } from "../composables/useAgentStatusMode";
import { useTerminalApp } from "../composables/useTerminalApp";
import { useSessions } from "../composables/useSessions";
import type { AgentStatusMode } from "../types/pty";

const XTERM_RENDERER_KEY = "claudia-xterm-renderer";
type XtermRenderer = "canvas" | "webgl" | "dom";

const isWindows = navigator.userAgent.toLowerCase().includes("windows");

const rendererOptions: Array<{ value: XtermRenderer; label: string; hint: string }> = [
  { value: "canvas", label: "Canvas 2D", hint: "兼容性好，性能均衡，推荐" },
  { value: "webgl", label: "WebGL", hint: "高性能，依赖 GPU 驱动" },
  { value: "dom", label: "DOM", hint: "兼容性最高，大量输出时较慢" },
];

function isXtermRenderer(value: string): value is XtermRenderer {
  return rendererOptions.some((option) => option.value === value);
}

function loadXtermRenderer(): XtermRenderer {
  try {
    const raw = localStorage.getItem(XTERM_RENDERER_KEY);
    if (raw && isXtermRenderer(raw)) {
      return raw;
    }
  } catch {
    // ignore
  }
  return isWindows ? "canvas" : "webgl";
}

const xtermRenderer = ref<XtermRenderer>(loadXtermRenderer());

watch(xtermRenderer, (value) => {
  if (!isXtermRenderer(value)) {
    return;
  }
  try {
    localStorage.setItem(XTERM_RENDERER_KEY, value);
  } catch {
    // ignore
  }
});

const activeRendererHint = computed(() => {
  return rendererOptions.find((opt) => opt.value === xtermRenderer.value)?.hint ?? "终端渲染器。更改后新打开的终端会话生效。";
});

const { agentStatusMode } = useAgentStatusMode();

const {
  terminalApp,
  terminalAppOptions,
  detectTerminalApps,
  setTerminalApp,
} = useTerminalApp();
const { skipPermissions } = useSessions();

onMounted(() => {
  void detectTerminalApps();
});

async function onSelectTerminalApp(value: unknown) {
  if (typeof value !== "string" || !value) return;
  setTerminalApp(value);
  // 右键菜单的终端应用在注册时固化进工作流文件，设置变更后静默重注册使其生效
  try {
    if (await invoke<boolean>("is_context_menu_registered")) {
      await invoke("register_context_menu", {
        skipPermissions: skipPermissions.value,
        terminalApp: value,
      });
    }
  } catch {
    // 静默失败：设置本身已保存，用户下次手动注册时生效
  }
}

const agentStatusModeOptions: Array<{ value: AgentStatusMode; label: string; hint: string }> = [
  { value: "osc", label: "OSC（推荐）", hint: "通过终端转义序列检测状态，无需额外进程" },
  { value: "hook-relay", label: "Hook Relay（高级）", hint: "通过 claudia-proxy 接收完整 hook 数据" },
];

const activeStatusModeHint = computed(() => {
  return agentStatusModeOptions.find((opt) => opt.value === agentStatusMode.value)?.hint ?? "设置状态检测模式。";
});
</script>

<template>
  <div class="settings-group">
    <div class="settings-row" v-if="terminalAppOptions.length > 0">
      <div class="row-content">
        <div class="row-title">外部终端应用</div>
        <p class="row-hint">「在终端中恢复 / 新建会话」与系统右键菜单使用的终端，仅列出已安装的应用。</p>
      </div>
      <div class="row-action">
        <ElegantSelect
          :model-value="terminalApp ?? ''"
          :options="terminalAppOptions"
          size="small"
          min-menu-width="180px"
          @update:model-value="onSelectTerminalApp"
        />
      </div>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">终端渲染器</div>
        <p class="row-hint">{{ activeRendererHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="xtermRenderer" :options="rendererOptions" />
      </div>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">Claude 状态检测</div>
        <p class="row-hint">{{ activeStatusModeHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="agentStatusMode" :options="agentStatusModeOptions" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-group {
  display: flex;
  flex-direction: column;
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
