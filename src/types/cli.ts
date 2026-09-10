export type CliId = "claude" | "codex" | "gemini" | "workbuddy" | "dsh" | "antigravity";

export interface CliDefinition {
  id: CliId;
  name: string;
  command: string;
  dataSourcePath: string;
  dataDirPath: string;
  installHint: string;
  permissionLabel: string;
  permissionHint: string;
  supportsNewSession: boolean;
  supportsResumeSession: boolean;
  supportsContextMenu: boolean;
  supportsInPlaceFork: boolean;
  supportsUsageStats: boolean;
  supportsApiProfiles: boolean;
  supportsApiLogs: boolean;
  aboutDescription: string;
  aboutCliLabel: string;
}

export interface CliStatus {
  id: CliId;
  has_sessions: boolean;
  has_binary: boolean;
}

/** 前端运行时发现结果：hasSessions 驱动展示/切换，hasBinary 驱动新建/恢复等二进制动作 */
export interface CliRuntime {
  hasSessions: boolean;
  hasBinary: boolean;
}

export interface CliPathConfig {
  id: CliId;
  defaultDataDir: string;
  effectiveDataDir: string;
  sessionsDir: string;
  hasOverride: boolean;
}

export interface ResolvedCliDefinition extends CliDefinition {
  defaultDataDirPath: string;
  hasCustomDataDir: boolean;
}

export interface CliOption extends CliDefinition {
  /** 展示门：数据目录存在会话（has_sessions） */
  hasSessions: boolean;
  /** 二进制门：检测到可执行文件（detect_cli / which） */
  hasBinary: boolean;
}

export type CliCapabilityId =
  | "contextMenu"
  | "inPlaceFork"
  | "usageStats";

export interface CliCapabilityInfo {
  id: CliCapabilityId;
  label: string;
  supported: boolean;
  enabledWhen: string;
  unsupportedBehavior: string;
  backendDependency: string;
}

type CliCapabilityMeta = {
  id: CliCapabilityId;
  label: string;
  supported: (cli: CliDefinition) => boolean;
  enabledWhen: string | ((cli: CliDefinition) => string);
  unsupportedBehavior: string | ((cli: CliDefinition) => string);
  backendDependency: string | ((cli: CliDefinition) => string);
};

const IS_WINDOWS = typeof navigator !== "undefined"
  && navigator.userAgent.toLowerCase().includes("windows");

function defaultCliDataDir(id: CliId): string {
  const dirs: Record<CliId, { win: string; unix: string }> = {
    claude: { win: "%USERPROFILE%\\.claude", unix: "~/.claude" },
    codex: { win: "%USERPROFILE%\\.codex", unix: "~/.codex" },
    gemini: { win: "%USERPROFILE%\\.gemini", unix: "~/.gemini" },
    workbuddy: { win: "%USERPROFILE%\\.workbuddy", unix: "~/.workbuddy" },
    dsh: { win: "%USERPROFILE%\\.dsh", unix: "~/.dsh" },
    antigravity: {
      win: "%USERPROFILE%\\.gemini\\antigravity-cli",
      unix: "~/.gemini/antigravity-cli",
    },
  };
  return IS_WINDOWS ? dirs[id].win : dirs[id].unix;
}

function defaultCliDataSource(id: CliId): string {
  const base = defaultCliDataDir(id);
  const sep = IS_WINDOWS ? "\\" : "/";
  const subdir: Record<CliId, string> = {
    claude: "projects",
    codex: "sessions",
    gemini: "tmp",
    workbuddy: "projects",
    dsh: "sessions",
    antigravity: "brain",
  };
  return `${base}${sep}${subdir[id]}${sep}`;
}

function installHintFor(cli: CliId): string {
  if (cli === "claude") {
    return IS_WINDOWS
      ? "未检测到 Claude Code CLI，请先安装。\n\n安装方法：npm install -g @anthropic-ai/claude-code\n\nWindows 下常见可执行入口是 `%APPDATA%\\npm\\claude.cmd`；若安装后仍检测不到，请确认该目录已加入 PATH。"
      : "未检测到 Claude Code CLI，请先安装。\n\n安装方法：npm install -g @anthropic-ai/claude-code\n\n安装前需要先安装 Node.js。";
  }

  if (cli === "gemini") {
    return IS_WINDOWS
      ? "未检测到 Gemini CLI，请先安装并确保 `gemini` 可用。"
      : "未检测到 Gemini CLI，请先安装。\n\n安装方法：npm install -g @google/gemini-cli";
  }

  if (cli === "workbuddy") {
    return "未检测到 WorkBuddy 桌面应用，请先安装（/Applications/WorkBuddy.app）。";
  }

  if (cli === "antigravity") {
    return IS_WINDOWS
      ? "未检测到 Antigravity CLI，请先安装并确保 `agy` 可用。"
      : "未检测到 Antigravity CLI，请先安装并确保 `agy` 命令已加入 PATH。";
  }

  return IS_WINDOWS
    ? "未检测到 Codex CLI，请先安装并确保 `codex` 或 `%APPDATA%\\npm\\codex.cmd` 可用。"
    : "未检测到 Codex CLI，请先安装并确保 `codex` 命令已加入 PATH。";
}

export const CLI_DEFINITIONS: Record<CliId, CliDefinition> = {
  claude: {
    id: "claude",
    name: "Claude Code",
    command: "claude",
    dataSourcePath: defaultCliDataSource("claude"),
    dataDirPath: defaultCliDataDir("claude"),
    installHint: installHintFor("claude"),
    permissionLabel: "跳过权限检查",
    permissionHint: "启用后，新建或恢复会话时会自动附带 --dangerously-skip-permissions。",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: true,
    supportsInPlaceFork: true,
    supportsUsageStats: true,
    supportsApiProfiles: true,
    supportsApiLogs: true,
    aboutDescription: "Claude Code 本地会话浏览器 + 快捷启动器",
    aboutCliLabel: "Claude CLI",
  },
  codex: {
    id: "codex",
    name: "Codex",
    command: "codex",
    dataSourcePath: defaultCliDataSource("codex"),
    dataDirPath: defaultCliDataDir("codex"),
    installHint: installHintFor("codex"),
    permissionLabel: "绕过审批与沙箱",
    permissionHint:
      "启用后，Codex 会自动附带 --dangerously-bypass-approvals-and-sandbox，以最宽松权限启动。",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: true,
    supportsApiLogs: true,
    aboutDescription: "Codex 本地会话浏览器 + 快捷启动器",
    aboutCliLabel: "Codex CLI",
  },
  gemini: {
    id: "gemini",
    name: "Gemini",
    command: "gemini",
    dataSourcePath: defaultCliDataSource("gemini"),
    dataDirPath: defaultCliDataDir("gemini"),
    installHint: installHintFor("gemini"),
    permissionLabel: "YOLO 模式",
    permissionHint: "启用后，Gemini 会自动附带 -y（YOLO 模式），跳过所有工具确认。",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
    aboutDescription: "Gemini CLI 本地会话浏览器 + 快捷启动器",
    aboutCliLabel: "Gemini CLI",
  },
  workbuddy: {
    id: "workbuddy",
    name: "WorkBuddy",
    command: "workbuddy",
    dataSourcePath: defaultCliDataSource("workbuddy"),
    dataDirPath: defaultCliDataDir("workbuddy"),
    installHint: installHintFor("workbuddy"),
    permissionLabel: "",
    permissionHint: "WorkBuddy 会话恢复直接跳转 WorkBuddy 应用，不涉及权限参数。",
    supportsNewSession: false,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: true,
    supportsUsageStats: false,
    supportsApiProfiles: false,
    supportsApiLogs: false,
    aboutDescription: "WorkBuddy（腾讯 CodeBuddy）本地会话浏览器，恢复会话跳转 WorkBuddy 应用",
    aboutCliLabel: "WorkBuddy 应用",
  },
  dsh: {
    id: "dsh",
    name: "DSH",
    command: "dsh",
    dataSourcePath: "~/.dsh/sessions",
    dataDirPath: "~/.dsh",
    installHint: "安装 DSH",
    permissionLabel: "",
    permissionHint: "DSH 为 Web 端会话，不支持本地终端恢复。",
    supportsNewSession: false,
    supportsResumeSession: false,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
    aboutDescription: "DSH（DeepSeek Harness）Web 会话浏览器",
    aboutCliLabel: "DSH",
  },
  antigravity: {
    id: "antigravity",
    name: "Antigravity",
    command: "agy",
    dataSourcePath: defaultCliDataSource("antigravity"),
    dataDirPath: defaultCliDataDir("antigravity"),
    installHint: installHintFor("antigravity"),
    permissionLabel: "",
    permissionHint: "Antigravity 会话恢复直接唤起 agy 命令行。",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
    aboutDescription: "Google Antigravity CLI (agy) 本地会话浏览器 + 快捷启动器",
    aboutCliLabel: "Antigravity CLI",
  },
};

const CLI_CAPABILITY_META: CliCapabilityMeta[] = [
  {
    id: "contextMenu",
    label: "系统右键菜单",
    supported: (cli) => cli.supportsContextMenu,
    enabledWhen: "仅在桌面端且当前 CLI 标记支持时显示。",
    unsupportedBehavior: "设置页直接隐藏注册入口，并显示“不支持系统右键菜单集成”提示。",
    backendDependency: (cli) =>
      cli.id === "claude"
        ? "`context_menu.rs` + Claude CLI 启动命令。"
        : "当前未接入，继续保留 Claude-only 实现。",
  },
  {
    id: "inPlaceFork",
    label: "任意消息 Fork",
    supported: (cli) => cli.supportsInPlaceFork,
    enabledWhen: "仅在聊天消息悬浮动作栏按当前 CLI 能力显示（消息需带 uuid 锚点）。",
    unsupportedBehavior: "隐藏“从这里继续”入口；即使误调，后端也会拒绝执行。",
    backendDependency:
      "`useSessions.ts` 的前端 guard + `commands/session.rs::fork_session` 的后端分支保护。",
  },
  {
    id: "usageStats",
    label: "用量统计",
    supported: (cli) => cli.supportsUsageStats,
    enabledWhen: "工具栏 Usage 入口和会话详情统计仅在支持时展示。",
    unsupportedBehavior: "隐藏 Usage 入口，并在相关空状态文案里直接说明当前 CLI 不支持。",
    backendDependency:
      "`commands/session.rs::get_usage_stats` / `get_session_stats` + `multi_cli_session.rs::extract_usage_records`。",
  },
];

export const SUPPORTED_CLIS = Object.values(CLI_DEFINITIONS);

export function isCliId(value: string): value is CliId {
  return value === "claude" || value === "codex" || value === "gemini" || value === "workbuddy" || value === "dsh" || value === "antigravity";
}

export function getCliDefinition(id: CliId): CliDefinition {
  return CLI_DEFINITIONS[id];
}

export function getCliCapabilityMatrix(cli: CliDefinition): CliCapabilityInfo[] {
  return CLI_CAPABILITY_META.map((item) => ({
    id: item.id,
    label: item.label,
    supported: item.supported(cli),
    enabledWhen:
      typeof item.enabledWhen === "function" ? item.enabledWhen(cli) : item.enabledWhen,
    unsupportedBehavior:
      typeof item.unsupportedBehavior === "function"
        ? item.unsupportedBehavior(cli)
        : item.unsupportedBehavior,
    backendDependency:
      typeof item.backendDependency === "function"
        ? item.backendDependency(cli)
        : item.backendDependency,
  }));
}

export function resolveCliDefinition(
  id: CliId,
  pathConfig?: CliPathConfig | null,
): ResolvedCliDefinition {
  const base = CLI_DEFINITIONS[id];

  return {
    ...base,
    dataDirPath: pathConfig?.effectiveDataDir ?? base.dataDirPath,
    dataSourcePath: pathConfig?.sessionsDir ?? base.dataSourcePath,
    defaultDataDirPath: pathConfig?.defaultDataDir ?? base.dataDirPath,
    hasCustomDataDir: pathConfig?.hasOverride ?? false,
  };
}
