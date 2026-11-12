# Moon

Moon 是一个**本地优先的多智能体协作工作台**：创建智能体与房间，用 `@成员` 让智能体在房间工作目录中执行任务；也可以把工作交给主持人智能体，自动完成分解、指派、收集与评审。

桌面端基于 **Tauri 2 + Rust**，前后端同进程运行，数据只保存在本机；同时提供浏览器开发模式。

[![CI](https://github.com/fishlikewater/moon/actions/workflows/ci.yml/badge.svg)](https://github.com/fishlikewater/moon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## 核心能力

- **房间协作**：`@智能体` 触发运行，回复经 WebSocket 流式呈现；同一房间严格串行，多个提及按顺序执行。支持 Markdown、回复引用、表情反应、成员筛选与运行中状态。
- **自动编排**：为任务绑定房间后启动编排，主持人按「分解 → 指派 → 收集 → 评审」推进，收敛后进入待验收；支持暂停、恢复、最大轮次熔断与完整时间线。
- **房间讨论**：主持人组织 2–5 名成员按轮次发言、互相引用与投票，达到轮数或发言上限后裁决结束。
- **多 harness**：内置 `zcode`、`codex`、`kimi-code`（占位）与 `fake`（本地测试）；任何支持 [Agent Client Protocol](https://agentclientprotocol.com/) 的本地 CLI 都可以登记为 `acp` harness，并探测其模型、会话模式与思考等级。
- **智能体与会话**：智能体绑定人设、harness、模型与工作区；会话按「房间 + 智能体」隔离，可复用、归档、删除，并提供跨房间会话总览。
- **任务管理**：跨房间任务看板、状态流转、定时启动、取消终态与运行记录。
- **本地工作区**：房间可绑定多个目录，智能体拥有独立工作区；文件面板可浏览、预览与下载产出，附件与图片随消息保存。
- **运行可观测**：每次运行记录模型、耗时、token 用量、工具调用与实时活动，支持独立运行详情页与历史回放。
- **桌面体验**：亮/暗主题、命令面板、可折叠侧栏与响应式布局。

## 架构

```text
┌────────────┐      HTTP / WebSocket      ┌──────────────────────────┐
│  web       │ ─────────────────────────► │  server-rs               │
│ React + TS │                            │  axum + sqlx + SQLite    │
└────────────┘                            └───────────┬──────────────┘
                                                      │ spawn / stdio
                                          ┌───────────▼──────────────┐
                                          │ harness 子进程            │
                                          │ zcode / codex / ACP CLI  │
                                          └──────────────────────────┘
```

- `web`：React 19 + Vite + TypeScript，负责全部界面与本地交互状态。
- `shared`：前后端共享的 API 契约类型，唯一类型来源。
- `server-rs`：唯一后端运行时（axum、sqlx/SQLite、WebSocket），负责房间、消息、任务、讨论、编排、会话与 harness 适配。
- `desktop`：Tauri 2 桌面壳，在进程内启动 `server-rs`，无需 Node sidecar。

## 环境要求

- Node.js >= 20.19，npm >= 10
- Rust stable（`cargo` / `rustc`）
- Windows 桌面构建另需 WebView2 运行时（Windows 10/11 通常已内置）与 Tauri NSIS 打包工具（首次构建自动下载）

## 快速开始

```bash
npm install
npm run dev
```

- Web：<http://localhost:5173>
- API：<http://localhost:3000>，健康检查 `GET /api/health`
- 数据库：`data/moon.db`，harness 会话：`data/harness-sessions/`

只启动后端：

```bash
cargo run --manifest-path server-rs/Cargo.toml
```

### 桌面端

```bash
npm run desktop:dev     # 构建前端并启动 Tauri 开发窗口
npm run desktop:build   # 产出 Windows 安装包
```

安装包输出到 `desktop/src-tauri/target/release/bundle/nsis/`。桌面端数据位于系统用户数据目录（Windows 为 `%LOCALAPPDATA%\com.moon.workbench`）。

> harness 需要本机已安装并登录对应 CLI（如 `zcode`、`codex`）或配置好 API Key；Moon 不内置模型额度。桌面端可用 `MOON_FAKE_HARNESS=1` 启动，用内置 fake 适配器跑通完整交互（Web 开发模式不支持该变量）。

## 配置

常用环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `MOON_HOST` | `127.0.0.1` | API 监听地址 |
| `MOON_PORT` | `3000` | API 监听端口 |
| `MOON_DATA_DIR` | `data` | 数据目录 |
| `MOON_DB_PATH` | `data/moon.db` | SQLite 路径 |
| `MOON_STORAGE_ROOT` | `data/harness-sessions` | harness 会话存储 |
| `MOON_RUN_TIMEOUT_SECS` | `1800` | 单次运行上限；`0` 不限制（设置页保存后以设置为准） |
| `MOON_ZCODE_TRANSPORT` | `app-server` | zcode 传输；设为 `cli` 回退 CLI（不支持附加目录） |
| `MOON_FAKE_HARNESS` | 未设置 | 仅桌面端：设为 `1` 时用内置 fake 适配器接管 fake 智能体 |
| `MOON_ACP_REGISTRY_URL` | 官方目录 | 覆盖 ACP 注册表 CDN 地址 |

设置页（`/settings`）还可调整默认权限模式、讨论轮数、上下文预算与压缩、附件大小/数量以及运行超时。

## 测试与构建

```bash
npm test        # 构建 shared + web 测试
npm run test:rust
npm run build   # 构建 shared + web
```

按变更面运行更小的验证：

```bash
npm test -w @moon/web -- messageView
cargo test --manifest-path server-rs/Cargo.toml --test api_agents_rooms
```

## 发布

1. 同步 `package.json` 与 `desktop/src-tauri/tauri.conf.json` 的 `version`。
2. 提交并推送：`git commit -am "chore: release v0.2.0" && git push`。
3. 打 tag 并推送：`git tag v0.2.0 && git push origin v0.2.0`。

`release.yml` 会校验 tag 与两处版本一致，构建 Windows 安装包并生成 `SHA256.txt`（均在 `desktop/src-tauri/target/release/bundle/nsis/`），然后创建同名 GitHub Release；`ci.yml` 在每次 push 到 `main` 与 PR 时运行测试、前端构建与桌面打包。

## 目录结构

```text
.
├── web/                 # React 前端
├── shared/              # 共享 API 类型
├── server-rs/           # Rust 后端
│   ├── src/adapters/    # zcode / codex / acp / fake 适配器
│   ├── src/services/    # 房间、任务、讨论、编排、会话等业务
│   └── tests/           # 后端集成测试
├── desktop/             # Tauri 2 桌面壳
└── package.json         # npm workspaces 与脚本
```

## 贡献

欢迎提交 Issue 与 Pull Request。提交前请：

1. 保持与现有代码风格一致，文本文件使用 UTF-8（无 BOM）。
2. 行为变更附带能表达意图的测试；修复 bug 优先补回归测试。
3. 运行 `npm test` 与 `npm run test:rust`，确保全绿。

## 许可证

[Apache License 2.0](LICENSE)
