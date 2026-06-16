# qwen-launcher-safe

> **English version: [README.md](./README.md)**

[qwen-launcher.ps1](https://github.com/aspnmy/qwen-coder) 的 Rust 重构版 — Qwen Code CLI 的资源保护启动包装器。

## 特性

- **进程自动发现** — 自动搜索 `qwen` 命令，覆盖 PATH、npm/npx 全局安装位置、上级目录 `node_modules/.bin/` 链
- **配置文件兜底** — 自动搜索失败时，读取 `~/.qwen-launcher/config.json` 中手动指定的 `qwenPath`
- **CPU 核绑定** — 每个 Qwen 实例获得独占物理 CPU 核，保证性能稳定性
- **共享状态文件** — 实例注册表持久化在 `%TEMP%\qwen-resource-state.json`，与现有 PowerShell `qwen-resource-monitor` 技能兼容
- **后台监控** — 自生成子进程 (`qwen-launcher-safe monitor`) 定时检查注册实例内存使用，清理已消失的 PID
- **优雅清理** — Qwen 退出时自动停止监控、注销所有已注册实例

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

### 配置 qwen 路径

自动搜索失败时，手动指定路径：

```powershell
# 自动检测并保存配置
qwen-launcher-safe init-config --qwen-path auto

# 手动指定路径
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

## qwen 搜索顺序

```
① PATH 环境变量            (qwen.cmd / qwen.exe / qwen)
② %APPDATA%\npm\           (npm 全局 bin)
③ %LOCALAPPDATA%\qwen\bin\ (本地应用数据)
④ ~\.cherrystudio\bin\     (原版 PowerShell 脚本备选路径)
⑤ %ProgramFiles%\qwen\bin\ (Program Files)
⑥ {cwd}/node_modules/.bin/… (向上遍历父目录)
⑦ ~\.qwen-launcher\config.json 中的 qwenPath 字段
```

## 配置文件

`~\.qwen-launcher\config.json`

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
