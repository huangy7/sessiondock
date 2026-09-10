<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";
import { invoke } from "@tauri-apps/api/core";
import SvgIcon from "./icons/SvgIcon.vue";
import { openExternalUrl } from "../utils/openExternalUrl";

const emit = defineEmits<{
  close: [];
}>();

export interface CliDiagnosticInfo {
  id: string;
  name: string;
  command: string;
  installed: boolean;
  hasSessions: boolean;
}

export interface SystemDiagnostics {
  appName: string;
  appVersion: string;
  tauriVersion: string;
  os: string;
  osName: string;
  osVersion: string;
  kernelVersion: string;
  arch: string;
  cpuModel: string;
  cpuCores: number;
  memoryTotal: string;
  clis: CliDiagnosticInfo[];
}

const diag = ref<SystemDiagnostics | null>(null);
const appVersion = ref("3.0.1");

// 环境信息检测
const osPlatform = ref("");
const userAgent = typeof navigator !== "undefined" ? navigator.userAgent : "";

onMounted(async () => {
  try {
    const res = await invoke<SystemDiagnostics>("get_system_diagnostics");
    diag.value = res;
    appVersion.value = res.appVersion;
    osPlatform.value = `${res.osName} ${res.osVersion} (${res.arch})`.trim();
  } catch {
    // 纯前端或测试环境回退
    const isMac = /mac/i.test(userAgent);
    const isWin = /win/i.test(userAgent);
    const isLinux = /linux/i.test(userAgent);
    const isArm = /arm|aarch64/i.test(userAgent) || /mac/i.test(userAgent);
    
    let fallbackOs = "Desktop";
    if (isMac) fallbackOs = `macOS ${isArm ? "(Apple Silicon / ARM64)" : "(Intel / x64)"}`;
    else if (isWin) fallbackOs = "Windows (x64)";
    else if (isLinux) fallbackOs = "Linux (x64)";
    osPlatform.value = fallbackOs;
  }

  window.addEventListener("keydown", handleKeyDown);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", handleKeyDown);
});

function handleKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    emit("close");
  }
}

function getDiagnosticsText(): string {
  if (diag.value) {
    const cliLines = diag.value.clis
      .map(
        (c) =>
          `  - **${c.name}** (\`${c.command}\`): ${c.installed ? "已安装 ✓" : "未检测到 ✗"}${c.hasSessions ? " (含历史对话)" : ""}`
      )
      .join("\n");

    return [
      `### 运行环境诊断信息 (Environment Diagnostics)`,
      `- **应用版本 (App)**: ${diag.value.appName} v${diag.value.appVersion} (Tauri v${diag.value.tauriVersion})`,
      `- **操作系统 (OS)**: ${diag.value.osName} ${diag.value.osVersion} (${diag.value.arch})`,
      `- **系统内核 (Kernel)**: ${diag.value.kernelVersion}`,
      `- **处理器 (CPU)**: ${diag.value.cpuModel} (${diag.value.cpuCores} 核心)`,
      `- **物理内存 (Memory)**: ${diag.value.memoryTotal}`,
      `- **检测到的 AI CLI 状态**:`,
      cliLines,
      `- **浏览器内核 (User Agent)**: ${userAgent}`,
      `- **采样时间 (Timestamp)**: ${new Date().toISOString()}`,
    ].join("\n");
  }

  return [
    `### 运行环境诊断信息 (Environment Diagnostics)`,
    `- **应用版本 (App)**: SessionDock v${appVersion.value}`,
    `- **系统平台 (Platform)**: ${osPlatform.value}`,
    `- **浏览器内核 (User Agent)**: ${userAgent}`,
    `- **采样时间 (Timestamp)**: ${new Date().toISOString()}`,
  ].join("\n");
}

function openIssue(type: "bug" | "feature" | "search") {
  const base = "https://github.com/huangy7/sessiondock/issues";
  let targetUrl = base;

  if (type === "bug") {
    const title = encodeURIComponent("[Bug]: ");
    const body = encodeURIComponent(
      `## 问题描述\n\n请详细描述您遇到的问题与复现步骤...\n\n## 预期行为\n\n请描述预期结果...\n\n---\n${getDiagnosticsText()}`
    );
    targetUrl = `${base}/new?title=${title}&body=${body}`;
  } else if (type === "feature") {
    const title = encodeURIComponent("[Feature]: ");
    const body = encodeURIComponent(
      `## 需求背景\n\n描述您希望解决的问题或业务场景...\n\n## 建议方案\n\n您希望 SessionDock 如何实现此功能...\n\n---\n*SessionDock v${appVersion.value}*`
    );
    targetUrl = `${base}/new?title=${title}&body=${body}`;
  } else if (type === "search") {
    targetUrl = base;
  }

  void openExternalUrl(targetUrl);
  emit("close");
}
</script>

<template>
  <div class="feedback-backdrop" @click.self="emit('close')">
    <div class="feedback-dialog">
      <!-- 弹窗顶部 -->
      <div class="feedback-header">
        <div class="header-left">
          <div class="github-icon-badge">
            <SvgIcon name="github" :size="20" />
          </div>
          <div>
            <h3 class="feedback-title">问题反馈与社区交流</h3>
            <p class="feedback-subtitle">SessionDock 是开源项目，所有反馈将直通 GitHub 社区追踪</p>
          </div>
        </div>
        <button class="feedback-close-btn" title="关闭 (Esc)" aria-label="关闭" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <!-- 三个分类选项卡片 -->
      <div class="feedback-body">
        <div class="options-grid">
          <!-- Bug Report -->
          <div class="feedback-card" @click="openIssue('bug')">
            <div class="card-icon bug-icon">
              <SvgIcon name="bug" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">提交缺陷报告 (Bug Report)</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">遇到报错、崩溃或界面渲染异常？前往 GitHub 提交缺陷报告</p>
            </div>
          </div>

          <!-- Feature Request -->
          <div class="feedback-card" @click="openIssue('feature')">
            <div class="card-icon feature-icon">
              <SvgIcon name="lightbulb" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">提议新功能 (Feature Request)</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">建议支持新的 AI CLI、会话分析能力或改进现有的界面交互体验</p>
            </div>
          </div>

          <!-- Search Existing -->
          <div class="feedback-card" @click="openIssue('search')">
            <div class="card-icon search-icon">
              <SvgIcon name="search" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">查阅已有 Issues 与讨论</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">搜索是否已有开发者提交过相同问题或已发布的临时解决方案</p>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部栏 -->
      <div class="feedback-footer">
        <div class="footer-tip">
          <SvgIcon name="info" :size="13" />
          <span>点击选项将在系统默认浏览器中打开 GitHub 页面</span>
        </div>
        <div class="footer-actions">
          <button class="btn-cancel" type="button" @click="emit('close')">
            关闭
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.feedback-backdrop {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal, 1000);
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(4px);
  display: flex;
  align-items: center;
  justify-content: center;
  animation: fadeIn 0.15s var(--ease-standard, ease);
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.feedback-dialog {
  width: 500px;
  max-width: 92vw;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 12px);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: popIn 0.18s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes popIn {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(4px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

.feedback-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4, 16px) var(--space-5, 20px);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
}

.header-left {
  display: flex;
  align-items: center;
  gap: var(--space-3, 12px);
}

.github-icon-badge {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md, 8px);
  background: var(--color-bg-hover, rgba(255, 255, 255, 0.08));
  border: 1px solid var(--color-border);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text);
  flex-shrink: 0;
}

.feedback-title {
  margin: 0;
  font-size: var(--text-base, 15px);
  font-weight: 600;
  color: var(--color-text);
  line-height: 1.3;
}

.feedback-subtitle {
  margin: 2px 0 0;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
}

.feedback-close-btn {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm, 6px);
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-fast, 0.15s);
}

.feedback-close-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.feedback-close-btn:active {
  transform: scale(0.92);
}

.feedback-body {
  padding: var(--space-4, 16px) var(--space-5, 20px);
  display: flex;
  flex-direction: column;
}

.options-grid {
  display: flex;
  flex-direction: column;
  gap: var(--space-2, 8px);
}

.feedback-card {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3, 12px);
  padding: 12px 14px;
  border-radius: var(--radius-md, 8px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
  user-select: none;
}

.feedback-card:hover {
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}

.feedback-card:active {
  transform: scale(0.985);
  opacity: 0.9;
}

.card-icon {
  width: 34px;
  height: 34px;
  border-radius: var(--radius-sm, 6px);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  margin-top: 2px;
}

.bug-icon {
  background: rgba(239, 68, 68, 0.12);
  color: #ef4444;
}

.feature-icon {
  background: rgba(245, 158, 11, 0.12);
  color: #f59e0b;
}

.search-icon {
  background: rgba(59, 130, 246, 0.12);
  color: #3b82f6;
}

.card-content {
  flex: 1;
  min-width: 0;
}

.card-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2, 8px);
}

.card-title {
  font-size: var(--text-sm, 13px);
  font-weight: 600;
  color: var(--color-text);
}

.external-indicator {
  color: var(--color-text-muted);
  opacity: 0.6;
  transition: opacity 0.15s, transform 0.15s;
}

.feedback-card:hover .external-indicator {
  opacity: 1;
  color: var(--color-primary);
  transform: translate(1px, -1px);
}

.card-desc {
  margin: 3px 0 0;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-secondary);
  line-height: 1.4;
}

.feedback-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3, 12px) var(--space-5, 20px);
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
}

.footer-tip {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--text-2xs, 11px);
  color: var(--color-text-muted);
}

.footer-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2, 8px);
}

.btn-cancel {
  padding: 6px 14px;
  font-size: var(--text-xs, 12px);
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
}

.btn-cancel:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.btn-cancel:active {
  transform: scale(0.96);
}
</style>
