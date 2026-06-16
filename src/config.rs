//! 配置文件读写模块
//!
//! 管理 `config/config.json`（与可执行文件同级目录）的读取和写入。
//! 配置文件用于指定 qwen 路径、内存限制和监控间隔，
//! 在自动搜索失败时提供兜底方案。
//!
//! # 配置文件格式
//!
//! ```json
//! {
//!   "qwenPath": "C:\\Users\\user\\.cherrystudio\\bin\\qwen.exe",
//!   "maxMemoryMB": 1024,
//!   "monitorIntervalSec": 10
//! }
//! ```

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 启动器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// qwen 可执行文件路径（用户在配置文件中手动指定）
    #[serde(rename = "qwenPath", default)]
    pub qwen_path: Option<String>,

    /// 最大内存限制（MB），默认 1024
    #[serde(rename = "maxMemoryMB", default = "default_memory")]
    pub max_memory_mb: u64,

    /// 监控轮询间隔（秒），默认 10
    #[serde(rename = "monitorIntervalSec", default = "default_interval")]
    pub monitor_interval_sec: u64,

    /// Qwen 工作目录（使子进程能加载指定目录下的 .qwen/skills/ 技能）
    #[serde(rename = "workingDir", default)]
    pub working_dir: Option<String>,
}

const fn default_memory() -> u64 {
    1024
}
const fn default_interval() -> u64 {
    10
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            qwen_path: None,
            max_memory_mb: default_memory(),
            monitor_interval_sec: default_interval(),
            working_dir: None,
        }
    }
}

/// 返回配置文件目录：`<exe 同级>/config/`
pub fn config_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("config")
}

/// 返回配置文件路径：`<exe 同级>/config/config.json`
pub fn config_file_path() -> PathBuf {
    config_dir().join("config.json")
}

/// 读取配置文件，文件不存在时返回 [`LauncherConfig::default`]
pub fn read_config() -> LauncherConfig {
    let path = config_file_path();
    if !path.exists() {
        return LauncherConfig::default();
    }
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return LauncherConfig::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// 写入配置文件（当用户在 CLI 中指定 `init-config` 时调用）
pub fn write_config(config: &LauncherConfig) -> io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = config_file_path();
    let data = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    std::fs::write(&path, data)?;
    Ok(())
}
