//! 共享状态文件模块
//!
//! 管理 `%TEMP%\qwen-resource-state.json` 的读写。
//! 此文件与 PowerShell 版 `qwen-resource-monitor` 技能共享，
//! 用于多实例之间的协调（CPU 核分配、内存监控）。
//!
//! # 状态文件格式
//!
//! ```json
//! {
//!   "instances": {
//!     "1234": {
//!       "pid": 1234,
//!       "startTime": "2026-06-16T...",
//!       "workingSetMB": 256,
//!       "boundCores": [0],
//!       "maxAllowedMemoryMB": 1024,
//!       "state": "running",
//!       "priority": 1,
//!       "lastHeartbeat": "2026-06-16T..."
//!     }
//!   },
//!   "globalState": {
//!     "totalInstances": 1,
//!     "physicalCores": 8
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// 单个 Qwen 实例的运行时状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    /// 进程 ID
    pub pid: u32,
    /// 启动时间（RFC 3339）
    #[serde(rename = "startTime")]
    pub start_time: String,
    /// 当前工作集内存（MB），由 monitor 子进程定期更新
    #[serde(rename = "workingSetMB")]
    pub working_set_mb: u64,
    /// 绑定的 CPU 核心索引列表
    #[serde(rename = "boundCores")]
    pub bound_cores: Vec<u32>,
    /// 允许的最大内存（MB），超限时输出告警
    #[serde(rename = "maxAllowedMemoryMB")]
    pub max_allowed_memory_mb: u64,
    /// 实例状态（"running" / "stopped"）
    pub state: String,
    /// 优先级（数字越小优先级越高）
    pub priority: u32,
    /// 最后心跳时间（RFC 3339）
    #[serde(rename = "lastHeartbeat")]
    pub last_heartbeat: String,
}

/// 全局状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalState {
    /// 当前注册的实例总数
    #[serde(rename = "totalInstances")]
    pub total_instances: u32,
    /// 系统物理 CPU 核心数
    #[serde(rename = "physicalCores")]
    pub physical_cores: u32,
}

/// 顶层状态文件结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
    /// 实例映射（key 为 PID 字符串）
    pub instances: HashMap<String, Instance>,
    /// 全局状态
    #[serde(rename = "globalState")]
    pub global_state: GlobalState,
}

impl Default for StateFile {
    fn default() -> Self {
        Self {
            instances: HashMap::new(),
            global_state: GlobalState {
                total_instances: 0,
                physical_cores: 0,
            },
        }
    }
}

/// 返回共享状态文件路径：`%TEMP%\qwen-resource-state.json`
pub fn state_file_path() -> PathBuf {
    let tmp = std::env::var("TEMP")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| r"C:\Windows\Temp".into());
    PathBuf::from(tmp).join("qwen-resource-state.json")
}

/// 读取状态文件，文件不存在时返回 [`StateFile::default`]
pub fn read_state_file() -> io::Result<StateFile> {
    let path = state_file_path();
    if !path.exists() {
        return Ok(StateFile::default());
    }
    let data = fs::read_to_string(&path)?;
    let state: StateFile =
        serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(state)
}

/// 将当前状态序列化为 JSON 并写入文件
pub fn write_state_file(state: &StateFile) -> io::Result<()> {
    let path = state_file_path();
    let data = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    fs::write(&path, data)?;
    Ok(())
}

/// 创建一个新的 `Instance` 记录
///
/// # 参数
///
/// * `pid` — 进程 ID
/// * `core` — 绑定的 CPU 核心索引
/// * `priority` — 实例优先级
/// * `max_memory_mb` — 允许的最大内存（MB）
pub fn new_instance(pid: u32, core: u32, priority: u32, max_memory_mb: u64) -> Instance {
    let now = Utc::now().to_rfc3339();
    Instance {
        pid,
        start_time: now.clone(),
        working_set_mb: 0,
        bound_cores: vec![core],
        max_allowed_memory_mb: max_memory_mb,
        state: "running".into(),
        priority,
        last_heartbeat: now,
    }
}
