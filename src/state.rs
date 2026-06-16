use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// 单个 Qwen 实例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub pid: u32,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "workingSetMB")]
    pub working_set_mb: u64,
    #[serde(rename = "boundCores")]
    pub bound_cores: Vec<u32>,
    #[serde(rename = "maxAllowedMemoryMB")]
    pub max_allowed_memory_mb: u64,
    pub state: String,
    pub priority: u32,
    #[serde(rename = "lastHeartbeat")]
    pub last_heartbeat: String,
}

/// 全局状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalState {
    #[serde(rename = "totalInstances")]
    pub total_instances: u32,
    #[serde(rename = "physicalCores")]
    pub physical_cores: u32,
}

/// 顶层状态文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFile {
    pub instances: HashMap<String, Instance>,
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

/// 共享状态文件路径：%TEMP%\qwen-resource-state.json
pub fn state_file_path() -> PathBuf {
    let tmp = std::env::var("TEMP")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| r"C:\Windows\Temp".into());
    PathBuf::from(tmp).join("qwen-resource-state.json")
}

/// 读取状态文件，不存在则返回默认值
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

/// 写入状态文件
pub fn write_state_file(state: &StateFile) -> io::Result<()> {
    let path = state_file_path();
    let data = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    fs::write(&path, data)?;
    Ok(())
}

/// 创建一个 Instance 记录
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
