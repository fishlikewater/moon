# Moon

English | [简体中文](README.zh-CN.md)

Moon is a **local-first multi-agent workspace**. Create agents and rooms, mention an agent with `@name` to run it inside the room workspace, or hand the work to a moderator agent that decomposes, assigns, collects, and reviews it.

Moon runs as a **Tauri 2 desktop app with an in-process Rust backend**. All data stays on your machine.

[![CI](https://github.com/fishlikewater/moon/actions/workflows/ci.yml/badge.svg)](https://github.com/fishlikewater/moon/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

## Features

- **Room collaboration** — `@agent` triggers a run and replies stream over WebSocket. Runs in one room are serialized; multiple mentions execute in order. Markdown, reply quotes, reactions, member filters, and live run status.
- **Orchestration** — bind a task to a room and start orchestration. The moderator decomposes, assigns, collects, and reviews until the work is ready for acceptance, with pause/resume, a round limit, and a full timeline.
- **Discussions** — the moderator runs 2–5 members through speaking rounds with references and votes, then closes with a verdict when the round or message limit is reached.
- **Multiple harnesses** — built-in `zcode`, `codex`, `kimi-code` (placeholder), and `fake` (local testing). Any local CLI that implements [Agent Client Protocol](https://agentclientprotocol.com/) can be registered as an `acp` harness, with model, session mode, and reasoning-level discovery.
- **Agents and sessions** — agents bind a persona, harness, model, and workspace. Sessions are isolated per room and agent, and can be reused, archived, deleted, or reviewed across rooms.
- **Tasks** — a cross-room board with status transitions, scheduled start, cancellation, and run history.
- **Local workspaces** — a room can bind multiple directories and every agent has its own workspace. Browse, preview, and download outputs from the file panel; attachments and images are stored with messages.
- **Observability** — every run records model, duration, token usage, tool calls, and live activity, with a dedicated run detail view and history.
- **Desktop experience** — light and dark themes, command palette, collapsible sidebar, and a responsive layout.

## Requirements

- Windows 10/11 x64 with WebView2 (included in current Windows releases) for the prebuilt installer.
- To build from source: Node.js >= 20.19, npm >= 10, and Rust stable.

## Installation

Download the latest `Moon_*_x64-setup.exe` from [Releases](https://github.com/fishlikewater/moon/releases) and run it.

Moon does not include model access. Install and sign in to at least one supported CLI (`zcode`, `codex`), or register an ACP-compatible CLI in the app.

Application data is stored in `%LOCALAPPDATA%\com.moon.workbench` on Windows.

Moon checks GitHub Releases for updates after launch and shows a non-blocking notice; open **Settings → Software update** to check manually, download, and restart into the new version. Update packages are signed with a Minisign key — if that private key is lost, existing installs must be reinstalled manually.

## Quick start

1. Create an agent and choose its harness, model, persona, and workspace.
2. Create a room, add members, and bind the workspace directories they should work in.
3. Mention an agent (`@name`) with your request, or switch the room to moderator mode and let the moderator plan and assign the work.

## Configuration

Most options live in **Settings** (`/settings`): default permission mode, discussion rounds, context budget and compaction, attachment limits, run timeout, theme, and harness binary paths.

The following environment variables are available for source builds and advanced setups:

| Variable | Default | Description |
| --- | --- | --- |
| `MOON_RUN_TIMEOUT_SECS` | `1800` | Per-run limit; `0` disables it. A value saved in Settings takes precedence. |
| `MOON_ZCODE_TRANSPORT` | `app-server` | zcode transport. Set to `cli` to fall back to the CLI. |
| `MOON_ACP_REGISTRY_URL` | Official registry | Override the ACP registry address. |
| `MOON_FAKE_HARNESS` | unset | Desktop only. Set to `1` to route fake agents through the built-in fake adapter. |

## Development

```bash
npm install

npm run dev          # browser UI with HMR, API on :3000, Vite on :5173
npm run desktop:dev  # build the web UI and launch the Tauri window
npm run desktop:build # produce the Windows installer
npm test             # shared build + web tests
npm run test:rust    # Rust tests
```

The installer is written to `desktop/src-tauri/target/release/bundle/nsis/`.

## Contributing

Issues and pull requests are welcome. Please keep behavior changes covered by tests and run `npm test` and `npm run test:rust` before opening a PR.

## License

[Apache License 2.0](LICENSE)
