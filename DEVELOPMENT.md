# SessionDock 开发指南

这份文档描述 **SessionDock** 的开发方式、目录结构和调试入口，帮助你快速进入日常开发与排障流程。

## 项目概览

SessionDock 是一个基于 Tauri 2 的桌面应用，前端使用 Vue 3 + TypeScript，后端使用 Rust。

- **前端**：负责界面、状态管理、Monaco Editor 集成、Markdown 渲染、设置页与 API 调试视图
- **后端**：负责本地会话扫描、CLI 配置读写、代理管理、文件系统操作与监听、数据持久化和系统集成
- **当前支持平台**：macOS、Windows、Linux
- **当前支持的 CLI**：`Claude Code`、`Codex`、`Gemini`、`Antigravity`、`WorkBuddy`、`DSH`

## 环境要求

| 工具 | 建议版本 | 说明 |
|------|----------|------|
| Node.js | 18+ | 前端构建与脚本运行 |
| npm | 8+ | 包管理 |
| Rust | 1.70+ | Tauri 后端编译 |
| rustup | 1.25+ | Rust 工具链管理 |

### macOS 依赖

```bash
xcode-select --install
```

### Windows 依赖

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（勾选“C++ 桌面开发”）
- [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)

## 快速开始

```bash
npm install
npm run tauri dev
```

这是最常用的本地开发入口。它会：
- 启动前端开发服务器（`http://localhost:1420`）
- 自动编译并启动 Tauri 桌面应用
- 在需要时预构建 `sessiondock-proxy`

## 目录结构

```text
SessionDock/
├── src/                  # Vue 3 前端源码
│   ├── components/       # 视图组件（会话树、对话窗、Monaco 编辑器等）
│   ├── composables/      # 状态管理与业务逻辑
│   ├── types/            # TypeScript 类型定义
│   └── utils/            # 通用工具函数
├── src-tauri/            # Tauri 桌面应用后端 (Rust)
│   ├── src/
│   │   ├── commands/     # 前端调用的 Tauri commands
│   │   ├── db/           # SQLite 数据库管理与迁移
│   │   ├── parser/       # 各 CLI 会话日志解析器
│   │   ├── assistant/    # 助手后端逻辑与提示词
│   │   ├── lib.rs        # 应用初始化与指令注册
│   │   └── main.rs       # 桌面端入口
│   └── tauri.conf.json   # Tauri 应用配置
├── src-tauri-proxy/      # 独立 API 反向代理服务 (Rust)
├── scripts/              # 跨平台构建与发布辅助脚本
└── docs/                 # 架构与设计规范
```

## 调试与日志

### 前端调试

在开发模式下，可以在应用窗口里通过右键或快捷键打开 WebView DevTools。

### Rust 调试

```bash
cd src-tauri
cargo check
cargo test
```

### 应用日志

SessionDock 启动时会初始化应用日志，默认写入：

- macOS：`~/Library/Application Support/com.sessiondock.app/logs/sessiondock.log`
- Windows：`%APPDATA%\com.sessiondock.app\logs\sessiondock.log`

代理服务日志默认写入：
- `logs/proxy.log`

## 应用数据目录

SessionDock 的本地数据根目录位于：

- macOS：`~/Library/Application Support/com.sessiondock.app/`
- Windows：`%APPDATA%\com.sessiondock.app\`

关键数据文件：
- `app.db`：主 SQLite 数据库（存储会话索引、书签、设置、API profile）
- `traffic/traffic.db`：API 代理流量记录数据库
- `logs/`：运行时日志

## 构建与测试

```bash
# 运行前端测试
npx vitest run

# 编译检查前端生产构建
npm run build:web

# 运行完整桌面打包
npm run tauri build
```
