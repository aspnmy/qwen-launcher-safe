# agent-launcher-safe

> **简体中文版请参阅：[README.zh-CN.md](./README.zh-CN.md)**
![dashboard](./docs/dashboard.png)

![dashboard](./docs/dashboard.png)
> **更多文档：[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)**

A Rust rewrite of the [agent-launcher.ps1](https://github.com/aspnmy/qwen-coder) — a general-purpose resource-protected launcher for CLI agents — enforces CPU/memory boundaries per instance.

## Features

- **Process Discovery** — Reads `qwenPath` from `config/config.json` (alongside the executable). No hardcoded paths or auto-discovery.
- **Interactive Setup Wizard** — If no config file exists or `qwenPath` is unset, `init`/`init-config` without args or `launch` enters a 3-step interactive wizard to configure qwen path, memory limit, and monitor interval.
- **`init` Alias** — `init` is a shortcut alias for `init-config`.

## First Use

```powershell
# First time: enters interactive wizard (no config → prompts for qwen path, memory, interval)
agent-launcher-safe init
```

- **CPU Core Binding** — Each Qwen instance gets an exclusive physical CPU core for consistent performance.
- **Shared State File** — Instance registry persists at `%TEMP%\qwen-resource-state.json`, compatible with the existing PowerShell `qwen-resource-monitor` skill.
- **Background Monitor** — A self-spawned child process (`agent-launcher-safe monitor`) periodically checks registered instances' memory usage and cleans up vanished PIDs.
- **Graceful Cleanup** — On Qwen exit, the monitor is killed and all registered instances are unregistered from the shared state file.
- **Working Directory Config** — `workingDir` in config sets the Qwen child process working directory, enabling project-level `.qwen/skills/` auto-loading.
- **Real-time Dashboard** — During launch, a console dashboard shows PID, CPU core, memory usage (MB), and status per instance, refreshed every 2 seconds.
- **Input Normalization** — User input is automatically normalized: fullwidth characters → halfwidth, and surrounding quotes stripped, preventing path validation errors.
- **Tolerant State File I/O** — State file writes use atomic write (temp file + rename) to prevent corruption; reads use depth-counter-based tolerance to recover from trailing garbage.
- **Signal Handling** — On Ctrl+C, resources are cleaned up gracefully (monitor stopped, instances unregistered) via an atomic flag-driven signal handler.
- **Load-balanced Core Allocation** — When all CPU cores are occupied, new instances are distributed to the least-loaded core rather than all falling back to core 0.
- **Config Fault Tolerance** — Corrupted `config.json` produces a warning log and falls back to defaults instead of panicking or silently returning zero values.
- **Linux Compatibility** — State file path supports Unix (`/tmp/qwen-resource-state.json`) with `XDG_RUNTIME_DIR` / `TMPDIR` fallback; CPU binding via `sched_setaffinity`; SIGTERM signal handler alongside Ctrl+C.
- **Cross-process File Lock** — All read-modify-write operations on the shared state file are protected by an `fs2::FileExt::lock_exclusive()` guard, preventing concurrent core allocation conflicts and registration loss.
- **Stale Instance Cleanup** — On startup and during monitoring cycles, crashed instances (PIDs that no longer exist in the process table) are automatically removed from the shared state file.
- **Orphan Monitor Prevention** — The background monitor child process tracks its parent PID and auto-exits when the parent (launcher) dies, preventing orphan processes.
- **Executable Validation** — `spawn_qwen()` validates that the configured path is a real executable before launching, returning `InvalidInput` instead of failing at runtime.
- **Timeout Force-Kill** — If Qwen processes don't exit within 24 hours, the launcher force-terminates them instead of waiting indefinitely.
- **Progress Feedback** — During the child process discovery polling window, progress messages are logged every ~0.9s so users see activity rather than a silent 5-second wait.
- **Dashboard I/O Optimization** — The real-time console dashboard reads the shared state file once per refresh cycle instead of re-reading it, reducing file I/O by ~50% during monitoring.
- **Test Suite** — 46 unit tests covering all modules: core allocation, process discovery, state file I/O, config read/write, input normalization, interactive setup wizard, and stale instance cleanup.

## CI Pipeline

Every push and PR triggers a CI check via `.github/workflows/ci.yml`:

| Platform | Steps |
|----------|-------|
| ubuntu-latest | `cargo check` + `clippy -D warnings` + `cargo fmt --check` + `cargo test` |
| windows-latest | `cargo check` + `clippy -D warnings` + `cargo test` |
| macos-latest | `cargo check` |

## Installation

```bash
cargo install --git https://github.com/aspnmy/agent-launcher-safe.git
```

Or build from source:

```bash
git clone https://github.com/aspnmy/agent-launcher-safe.git
cd agent-launcher-safe
cargo build --release
```

## Usage

### Launch Qwen with resource protection

```powershell
# Basic launch (passes remaining args to qwen verbatim)
agent-launcher-safe launch -- --model qwen-max

# Or from the cloned directory
.\target\release\agent-launcher-safe.exe launch --
```

### First use (interactive wizard)

No config exists? `init`, `init-config`, or `launch` enters the setup wizard:

```powershell
# Interactive setup (prompts for qwen path, memory limit, monitor interval)
agent-launcher-safe init
```

### Configure qwen path (direct)

```powershell
# Specify qwen path explicitly
agent-launcher-safe init-config --qwen-path "C:\Users\nasAdmin\.cherrystudio\bin\qwen.exe"

# View current config
agent-launcher-safe init-config --show
```

### Customize resource limits

```powershell
# Set memory limit to 2GB per instance
agent-launcher-safe init-config --max-memory-mb 2048

# Set monitor polling interval to 30 seconds
agent-launcher-safe init-config --monitor-interval 30
```

### Run monitor independently

```powershell
agent-launcher-safe monitor -i 10
```

## Architecture

```
src/
├── main.rs       — CLI entry (clap derive, 3 subcommands + interactive setup wizard)
├── config.rs     — config/config.json reader/writer with fault tolerance
├── launcher.rs   — Launch orchestration (baseline → spawn → register → signal → wait → cleanup)
├── monitor.rs    — Background resource monitor loop
├── process.rs    — Process utilities (discovery, CPU affinity, Qwen process regex matching)
└── state.rs      — Shared state file (%TEMP%/tmp qwen-resource-state.json) types and I/O
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
  "monitorIntervalSec": 10,
  "workingDir": "C:\\$aspnmyTools\\qwen coder"
}
```

- `workingDir` (optional): Qwen child process working directory. If set, Qwen loads project-level skills from `./.qwen/skills/` under this directory.

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
