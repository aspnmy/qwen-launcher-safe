# qwen-launcher-safe

> **English version: [README.md](./README.md)**
> **更多文档：[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md)**

[qwen-launcher.ps1](https://github.com/aspnmy/qwen-coder) 的 Rust 重构版 — Qwen Code CLI 的资源保护启动包装器。

## 特性

- **进程发现** — 仅从 `config/config.json` 读取 `qwenPath`。无硬编码路径、无自动搜索。
- **交互式配置向导** — 配置文件不存在或 `qwenPath` 未设置时，`init`/`init-config` 无参数或 `launch` 自动进入 3 步交互式向导（qwen 路径、内存限制、监控间隔）
- **`init` 别名** — `init` 是 `init-config` 的快捷别名
- **CPU 核绑定** — 每个 Qwen 实例获得独占物理 CPU 核，保证性能稳定性
- **共享状态文件** — 实例注册表持久化在 `%TEMP%\qwen-resource-state.json`，与现有 PowerShell `qwen-resource-monitor` 技能兼容
- **后台监控** — 自生成子进程 (`qwen-launcher-safe monitor`) 定时检查注册实例内存使用，清理已消失的 PID
- **优雅清理** — Qwen 退出时自动停止监控、注销所有已注册实例

## 首次使用

```powershell
# 首次使用：进入交互式向导（提示输入 qwen 路径、内存限制、监控间隔）
qwen-launcher-safe init
```

## 安装

```bash
cargo install --git https://github.com/aspnmy/qwen-launcher-safe.git
```

或从源码构建：

```bash
git clone https://github.com/aspnmy/qwen-launcher-safe.git
cd qwen-launcher-safe
cargo build --release
```

## 使用方法

### 启动 Qwen 并带资源保护

```powershell
# 基本启动（后续参数透传给 qwen）
qwen-launcher-safe launch -- --model qwen-max

# 或从克隆目录直接运行
.\target\release\qwen-launcher-safe.exe launch --
```

### 首次使用（交互式向导）

无配置文件时，`init`、`init-config` 或 `launch` 自动进入配置向导：

```powershell
# 交互式配置（提示输入 qwen 路径、内存限制、监控间隔）
qwen-launcher-safe init
```

### 直接配置 qwen 路径

```powershell
# 手动指定 qwen 路径
qwen-launcher-safe init-config --qwen-path "C:\Users\nasAdmin\.cherrystudio\bin\qwen.exe"

# 查看当前配置
qwen-launcher-safe init-config --show
```

### 自定义资源限制

```powershell
# 设置每个实例内存限制为 2GB
qwen-launcher-safe init-config --max-memory-mb 2048

# 设置监控轮询间隔为 30 秒
qwen-launcher-safe init-config --monitor-interval 30
```

### 独立运行监控

```powershell
qwen-launcher-safe monitor -i 10
```

## 架构

```
src/
├── main.rs       — CLI 入口（clap derive，3 个子命令）
├── config.rs     — ~/.qwen-launcher/config.json 读写
├── launcher.rs   — 启动编排（基线→启动→注册→等待→清理）
├── monitor.rs    — 后台资源监控循环
├── process.rs    — Windows 进程工具（搜索、CPU 亲和性、命令行匹配）
└── state.rs      — 共享状态文件（序列化类型和 I/O）
```

## qwen 路径来源

qwen 路径**仅**来自配置文件，无自动搜索、无硬编码路径。

```
① config/config.json → qwenPath 字段（唯一来源）
```

如果配置文件不存在或 `qwenPath` 未设置，`launch` 或 `init` 会自动进入交互式配置向导。

## 配置文件

`<exe 目录>/config/config.json`（便携式 — 与可执行文件同级）

```json
{
  "qwenPath": "C:\\Users\\nasAdmin\\.cherrystudio\\bin\\qwen.exe",
  "maxMemoryMB": 1024,
  "monitorIntervalSec": 10
}
```

## 状态文件

`%TEMP%\qwen-resource-state.json` — 与现有 PowerShell `qwen-resource-monitor` 技能共享，实现多实例协调。

## 发布

项目使用 GitHub Actions 发布工作流 (`.github/workflows/release.yml`) ，推送 tag 时自动构建 6 个目标平台。

### 触发发布

```bash
# 确保 Cargo.toml 版本号已更新，然后：
git tag v$(grep '^version' Cargo.toml | head -1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
git push origin v$(grep '^version' Cargo.toml | head -1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')
```

### 构建矩阵

| 系统 | 目标平台 | 打包格式 |
|------|---------|---------|
| ubuntu-latest | x86_64-unknown-linux-gnu | tar.gz |
| ubuntu-latest | x86_64-unknown-linux-musl | tar.gz |
| ubuntu-latest | aarch64-unknown-linux-gnu | tar.gz |
| macos-latest | x86_64-apple-darwin | tar.gz |
| macos-latest | aarch64-apple-darwin | tar.gz |
| windows-latest | x86_64-pc-windows-msvc | zip |

## 许可证

MIT
