# Moon

[English](README.md) | 简体中文

Moon 是一个**本地优先的多智能体协作工作台**。创建智能体与房间，用 `@名称` 让智能体在房间工作目录中执行任务；也可以把工作交给主持人智能体，由它完成分解、指派、收集与评审。

Moon 以 **Tauri 2 桌面应用 + 同进程 Rust 后端**运行，所有数据只保存在本机。

[![CI](https://github.com/fishlikewater/moon/actions/workflows/ci.yml/badge.svg)](https://github.com/fishlikewater/moon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## 核心能力

- **房间协作**：`@智能体` 触发运行，回复通过 WebSocket 流式呈现。同一房间严格串行，多个提及按顺序执行。支持 Markdown、回复引用、表情反应、成员筛选与运行中状态。
- **自动编排**：为任务绑定房间后启动编排。主持人按「分解 → 指派 → 收集 → 评审」推进，收敛后进入待验收，并支持暂停、恢复、最大轮次与完整时间线。
- **房间讨论**：主持人组织 2–5 名成员按轮次发言、互相引用与投票，达到轮数或发言上限后给出裁决。
- **多 harness**：内置 `zcode`、`codex`、`kimi-code`（占位）与 `fake`（本地测试）。任何实现 [Agent Client Protocol](https://agentclientprotocol.com/) 的本地 CLI 都可以登记为 `acp` harness，并探测模型、会话模式与思考等级。
- **智能体与会话**：智能体绑定人设、harness、模型与工作区。会话按「房间 + 智能体」隔离，可复用、归档、删除，并提供跨房间总览。
- **任务管理**：跨房间任务看板、状态流转、定时启动、取消终态与运行记录。
- **本地工作区**：房间可绑定多个目录，每个智能体拥有独立工作区。文件面板可浏览、预览与下载产出，附件与图片随消息保存。
- **运行可观测**：每次运行记录模型、耗时、token 用量、工具调用与实时活动，并提供运行详情与历史记录。
- **桌面体验**：亮/暗主题、命令面板、可折叠侧栏与响应式布局。

## 环境要求

- Windows 10/11 x64，带 WebView2（当前 Windows 版本已内置），用于预构建安装包。
- 从源码构建需要 Node.js >= 20.19、npm >= 10 与 Rust stable。

## 安装

从 [Releases](https://github.com/fishlikewater/moon/releases) 下载最新的 `Moon_*_x64-setup.exe` 并运行。

Moon 不内置模型额度。请先安装并登录至少一个受支持的 CLI（`zcode`、`codex`），或在应用内登记兼容 ACP 的 CLI。

Windows 上的应用数据保存在 `%LOCALAPPDATA%\com.moon.workbench`。

## 快速开始

1. 创建智能体，选择 harness、模型、人设与工作区。
2. 创建房间，添加成员，并绑定它们需要工作的目录。
3. 用 `@名称` 提及智能体并说明需求；或把房间切换为主持人模式，让主持人规划并分派工作。

## 配置

大部分选项在**设置页**（`/settings`）中调整：默认权限模式、讨论轮数、上下文预算与压缩、附件限制、运行时长、主题以及 harness 可执行文件路径。

从源码运行或高级场景可使用以下环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `MOON_RUN_TIMEOUT_SECS` | `1800` | 单次运行上限；`0` 表示不限制。设置页保存的值优先。 |
| `MOON_ZCODE_TRANSPORT` | `app-server` | zcode 传输方式；设为 `cli` 时回退到 CLI。 |
| `MOON_ACP_REGISTRY_URL` | 官方目录 | 覆盖 ACP 注册表地址。 |
| `MOON_FAKE_HARNESS` | 未设置 | 仅桌面端；设为 `1` 时让 fake 智能体走内置 fake 适配器。 |

## 开发

```bash
npm install

npm run dev          # 浏览器界面（HMR），API 在 :3000，Vite 在 :5173
npm run desktop:dev  # 构建前端并启动 Tauri 窗口
npm run desktop:build # 生成 Windows 安装包
npm test             # 构建 shared + web 测试
npm run test:rust    # Rust 测试
```

安装包输出到 `desktop/src-tauri/target/release/bundle/nsis/`。

## 贡献

欢迎提交 Issue 与 Pull Request。行为变更请附带测试，并在提交前运行 `npm test` 与 `npm run test:rust`。

## 许可证

[Apache License 2.0](LICENSE)
