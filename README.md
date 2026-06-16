# qwen-launcher-safe

A Rust rewrite of the [qwen-launcher.ps1](https://github.com/aspnmy/qwen-coder) — a resource-protected wrapper for launching Qwen Code CLI.

## Features

- **Process Discovery** — Automatically searches for `qwen` command via PATH, common npm/npx global install locations, and parent-directory `node_modules/.bin/` chains.
- **Config File Fallback** — If auto-search finds nothing, reads `~/.qwen-launcher/config.json` for a manually specified `qwenPath`.
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

### Configure qwen path

If auto-search fails, manually specify the path:

```powershell
# Auto-detect and save to config
qwen-launcher-safe init-config --qwen-path auto

# Manual path
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

## Search Order for `qwen`

```
① PATH environment variable (qwen.cmd / qwen.exe / qwen)
② %APPDATA%\npm\              (npm global bin)
③ %LOCALAPPDATA%\qwen\bin\     (local app data)
④ ~\.cherrystudio\bin\         (original fallback from PowerShell script)
⑤ %ProgramFiles%\qwen\bin\     (Program Files)
⑥ {cwd}/node_modules/.bin/...  (walk up parent directories)
⑦ ~\.qwen-launcher\config.json (manual config file, qwenPath field)
```

## Config File

`~\.qwen-launcher\config.json`

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
