# Qwen Launcher Safe — 开发者文档

## 模块架构

```
src/
├── main.rs       CLI 入口
│                   clap derive 枚举子命令：
│                   - Launch     → launcher::run()
│                   - Monitor    → monitor::run()
│                   - InitConfig → cmd_init_config()
│
├── config.rs     配置文件读写
│                   路径：~/.qwen-launcher/config.json
│                   字段：qwenPath, maxMemoryMB, monitorIntervalSec
│                   函数：read_config(), write_config()
│
├── launcher.rs   启动器主编排
│                   7 步生命周期：
│                   基线 → 启动 → 轮询 → 注册 → 监控 → 等待 → 清理
│
├── monitor.rs    后台资源监控
│                   循环：睡眠 → 读状态 → sysinfo 检测 → 写状态
│                   两阶段设计：Phase 1 只读扫描，Phase 2 批量写入
│
├── process.rs    Windows 进程工具
│                   find_qwen_command(): 7 层降级搜索
│                   get_qwen_pids():     命令行关键字匹配
│                   bind_cpu_core():     SetProcessAffinityMask
│
└── state.rs      共享状态文件
│                   路径：%TEMP%\qwen-resource-state.json
│                   与 PowerShell qwen-resource-monitor 技能兼容
│                   类型：StateFile → { instances, globalState }
```

## 搜索链路

```
find_qwen_command()
│
├─ ① PATH 环境变量          qwen.cmd / qwen.exe / qwen
├─ ② %APPDATA%\npm\         npm 全局 bin
├─ ③ %LOCALAPPDATA%\qwen\bin\
├─ ④ ~\.cherrystudio\bin\   原版硬编码备选
├─ ⑤ %ProgramFiles%\qwen\bin\
├─ ⑥ node_modules/.bin/…    从当前目录向上遍历
├─ ⑦ ~\.qwen-launcher\      config.json 的 qwenPath 字段
│
└─ 全部失败 → io::Error::NotFound
```

## 状态文件格式

`%TEMP%\qwen-resource-state.json`：

```json
{
  "instances": {
    "1234": {
      "pid": 1234,
      "startTime": "2026-06-16T10:00:00+00:00",
      "workingSetMB": 256,
      "boundCores": [0],
      "maxAllowedMemoryMB": 1024,
      "state": "running",
      "priority": 1,
      "lastHeartbeat": "2026-06-16T10:01:00+00:00"
    }
  },
  "globalState": {
    "totalInstances": 1,
    "physicalCores": 8
  }
}
```

## 启动生命周期

```
[launcher]                           [monitor child]
    │                                      │
    ├─ 基线记录 (sysinfo)                   │
    ├─ 非阻塞启动 qwen                      │
    ├─ 轮询 5s 发现子进程                    │
    ├─ 注册实例 + 绑定 CPU 核                │
    ├─ spawn monitor ──────────────────→     │
    │                                      ├─ 循环：
    ├─ WaitForExit() ← qwen 运行中 ←        │   睡眠 interval 秒
    │                                      │   读取状态文件
    │                                      │   sysinfo 检测内存
    │                                      │   更新状态文件
    │                                      │   清理已消失实例
    │                                      │
    ├─ kill monitor  ──────────────────→     │   (终止)
    ├─ 注销实例                              │
    │                                      │
    └─ exit                                  │
```

## 文档生成

```bash
# 生成 API 文档到 target/doc/
cargo doc --no-deps --open
```

## 编码约定

- 所有 `pub` 函数和类型必须有 `///` doc 注释
- 所有模块必须有 `//!` 模块级 doc 注释
- `unsafe` 代码必须有 `// SAFETY:` 注释说明安全前提
- `#[cfg(windows)]` / `#[cfg(not(windows))]` 配对使用，非 Windows 提供占位实现
