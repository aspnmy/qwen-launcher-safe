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
│                   路径：<exe>/config/config.json（便携式）
│                   字段：qwenPath, maxMemoryMB, monitorIntervalSec, workingDir
│                   函数：read_config(), write_config(), interactive_setup()
│
├── launcher.rs   启动器主编排
│                   7 步生命周期：
│                   基线 → 启动 → 轮询 → 注册 → 监控 → 等待 → 清理
│                   核心分配：HashMap 负载追踪空闲优先→负载最低
│                   信号处理：Ctrl+C (ctrlc) + Unix SIGTERM (libc)
│                   仪表盘：每 2s 实时刷新 CPU 核占用和内存使用
│
├── monitor.rs    后台资源监控
│                   循环：父进程存活检查 → 读状态 → sysinfo 检测 → 写状态
│                   两阶段设计：Phase 1 只读扫描，Phase 2 加锁批量写入
│                   孤儿进程防护：传递 parent_pid，父进程消失后自动退出
│
├── process.rs    跨平台进程工具
│                   find_qwen_command(): 仅从 config.json qwenPath 读取（单层来源）
│                   get_qwen_pids():     命令行关键字匹配 (regex)
│                   bind_cpu_core():     SetProcessAffinityMask (Windows)
│                                        sched_setaffinity (Linux)
│                   is_executable():     路径可执行性校验（Windows 检查扩展名/Unix 检查权限位）
│                   spawn_qwen():        启动前校验可执行性，非 exe/无权限时返回 InvalidInput
│
└── state.rs      共享状态文件
│                   路径：%TEMP%\qwen-resource-state.json
│                   与 PowerShell qwen-resource-monitor 技能兼容
│                   类型：StateFile → { instances, globalState }
│                   容错：JSON 尾部垃圾字符自动截断修复
│                   文件锁：fs2::FileExt 排他锁守卫，跨进程 read→modify→write 原子保护
│                   僵死清理：cleanup_stale_entries() 自动清理崩溃残留的实例记录
```

## 配置链路

qwen 路径**仅**来自配置文件，单层来源，无自动搜索。

```
find_qwen_command()
│
└─ config/config.json → qwenPath 字段（唯一来源，无降级自动搜索）
    ├─ 已设置且存在 → 返回该路径
    ├─ 已设置但不存在 → 返回 NotFound
    └─ 未设置 → 返回 NotFound（触发交互式向导）
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
