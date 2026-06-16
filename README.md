# qwen-launcher-safe

> **简体中文版请参阅：[README.zh-CN.md](./README.zh-CN.md)**
> **更多文档：[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)**

A Rust rewrite of the [qwen-launcher.ps1](https://github.com/aspnmy/qwen-coder) — a resource-protected wrapper for launching Qwen Code CLI.

## Features

- **Process Discovery** — Reads `qwenPath` from `config/config.json` (alongside the executable). No hardcoded paths or auto-discovery.
- **Interactive Setup Wizard** — If no config file exists or `qwenPath` is unset, `init`/`init-config` without args or `launch` enters a 3-step interactive wizard to configure qwen path, memory limit, and monitor interval.
- **`init` Alias** — `init` is a shortcut alias for `init-config`.

## First Use

```powershell
# First time: enters interactive wizard (no config → prompts for qwen path, memory, interval)
qwen-launcher-safe init
```

- **CPU Core Binding** — Each Qwen instance gets an exclusive physical CPU core for consistent performance.
- **Shared State File** — Instance registry persists at `%TEMP%\qwen-resource-state.json`, compatible with the existing PowerShell `qwen-resource-monitor` skill.
- **Background Monitor** — A self-spawned child process (`qwen-launcher-safe monitor`) periodically checks registered instances' memory usage and cleans up vanished PIDs.
- **Graceful Cleanup** — On Qwen exit, the monitor is killed and all registered instances are unregistered from the shared state file.

## Installation

```bash
cargo install --git https://github.com/aspnmy/qwen-launcher-safe.git
```

Or build from source:

```bash
git clone https://github.com/aspnmy/qwen-launcher-safe.git
cd qwen-launcher-safe
cargo build --release
```

## Usage

### Launch Qwen with resource protection

```powershell
# Basic launch (passes remaining args to qwen verbatim)
qwen-launcher-safe launch -- --model qwen-max

# Or from the cloned directory
.\target\release\qwen-launcher-safe.exe launch --
```

### First use (interactive wizard)

No config exists? `init`, `init-config`, or `launch` enters the setup wizard:

```powershell
# Interactive setup (prompts for qwen path, memory limit, monitor interval)
qwen-launcher-safe init
```

### Configure qwen path (direct)

```powershell
# Specify qwen path explicitly
qwen-launcher-safe init-config --qwen-path "C:\Users\nasAdmin\.cherrystudio\bin\qwen.exe"

# View current config
qwen-launcher-safe init-config --show
```

### Customize resource limits

```powershell
# Set memory limit to 2GB per instance
qwen-launcher-safe init-config --max-memory-mb 2048

# Set monitor polling interval to 30 seconds
qwen-launcher-safe init-config --monitor-interval 30
```

### Run monitor independently

```powershell
qwen-launcher-safe monitor -i 10
```

## Architecture

```
src/
├── main.rs       — CLI entry (clap derive, 3 subcommands)
├── config.rs     — ~/.qwen-launcher/config.json reader/writer
├── launcher.rs   — Launch orchestration (baseline → spawn → register → wait → cleanup)
├── monitor.rs    — Background resource monitor loop
├── process.rs    — Windows process utilities (search, affinity, WMI cmdline match)
└── state.rs      — Shared state file (%TEMP%\qwen-resource-state.json) types and I/O
```

## qwen 路径来源

qwen 路径**仅**来自配置文件，无自动搜索、无硬编码路径。

```
① config/config.json → qwenPath 字段（唯一来源）
```

如果配置文件不存在或 `qwenPath` 未设置，`launch` 或 `init` 会自动进入交互式配置向导。

## Config File

`<exe-dir>/config/config.json` (portable — alongside the executable)

```json
{
  "qwenPath": "C:\\Users\\nasAdmin\\.cherrystudio\\bin\\qwen.exe",
  "maxMemoryMB": 1024,
  "monitorIntervalSec": 10
}
```

## State File

`%TEMP%\qwen-resource-state.json` — shared with the existing PowerShell `qwen-resource-monitor` skill for multi-instance coordination.

## Release

This project uses a GitHub Actions release workflow (`.github/workflows/release.yml`) that builds for 6 target platforms on tag push.

### Trigger a Release

```bash
# Ensure Cargo.toml version is bumped, then:
git tag v$(grep '^version' Cargo.toml | head -1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
git push origin v$(grep '^version' Cargo.toml | head -1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
```

### Build Matrix

| OS | Target | Archive |
|----|--------|---------|
| ubuntu-latest | x86_64-unknown-linux-gnu | tar.gz |
| ubuntu-latest | x86_64-unknown-linux-musl | tar.gz |
| ubuntu-latest | aarch64-unknown-linux-gnu | tar.gz |
| macos-latest | x86_64-apple-darwin | tar.gz |
| macos-latest | aarch64-apple-darwin | tar.gz |
| windows-latest | x86_64-pc-windows-msvc | zip |

## License

MIT
