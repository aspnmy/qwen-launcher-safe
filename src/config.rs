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
///
/// 当文件内容无法解析为 JSON 时，记录警告日志并返回默认值。
pub fn read_config() -> LauncherConfig {
    let path = config_file_path();
    if !path.exists() {
        return LauncherConfig::default();
    }
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("读取配置文件失败 ({}): {}，使用默认配置", path.display(), e);
            return LauncherConfig::default();
        }
    };
    match serde_json::from_str::<LauncherConfig>(&data) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::warn!("配置文件 {:?} 格式损坏: {}，使用默认配置", path, e);
            LauncherConfig::default()
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_default_config_values() {
        let cfg = LauncherConfig::default();
        assert!(cfg.qwen_path.is_none());
        assert_eq!(cfg.max_memory_mb, 1024);
        assert_eq!(cfg.monitor_interval_sec, 10);
        assert!(cfg.working_dir.is_none());
    }

    #[test]
    fn test_read_config_when_missing() {
        // 当配置文件不存在时，read_config() 应返回默认值
        let orig = config_file_path();
        let backup = if orig.exists() {
            Some(fs::read_to_string(&orig).ok())
        } else {
            None
        };

        // 临时移除配置文件
        if orig.exists() {
            fs::rename(&orig, orig.with_extension("json.bak")).ok();
        }

        let cfg = read_config();
        assert_eq!(cfg.max_memory_mb, 1024);

        // 恢复
        if orig.with_extension("json.bak").exists() {
            fs::rename(orig.with_extension("json.bak"), &orig).ok();
        } else if let Some(Some(content)) = backup {
            fs::write(&orig, content).ok();
        }
    }

    #[test]
    fn test_read_config_corrupted_json() {
        let orig = config_file_path();
        let backup = if orig.exists() {
            fs::read_to_string(&orig).ok()
        } else {
            None
        };

        // 确保 config 目录存在
        if let Some(dir) = orig.parent() {
            fs::create_dir_all(dir).expect("创建测试配置目录");
        }

        // 写入损坏 JSON
        fs::write(&orig, r#"{ invalid json }"#).expect("写入损坏测试数据");

        // 应静默返回默认值（不 panic）
        let cfg = read_config();
        assert_eq!(cfg.max_memory_mb, 1024);
        assert!(cfg.qwen_path.is_none());

        // 恢复
        match backup {
            Some(content) => fs::write(&orig, content).ok(),
            None => fs::remove_file(&orig).ok(),
        };
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let orig = config_file_path();
        let backup = if orig.exists() {
            fs::read_to_string(&orig).ok()
        } else {
            None
        };

        let cfg = LauncherConfig {
            qwen_path: Some("C:\\test\\qwen.exe".into()),
            max_memory_mb: 2048,
            monitor_interval_sec: 30,
            working_dir: Some("C:\\test\\wd".into()),
        };
        write_config(&cfg).expect("写入配置应成功");

        let read = read_config();
        assert_eq!(read.qwen_path, Some("C:\\test\\qwen.exe".into()));
        assert_eq!(read.max_memory_mb, 2048);
        assert_eq!(read.monitor_interval_sec, 30);
        assert_eq!(read.working_dir, Some("C:\\test\\wd".into()));

        // 恢复
        match backup {
            Some(content) => fs::write(&orig, content).ok(),
            None => fs::remove_file(&orig).ok(),
        };
    }

    #[test]
    fn test_config_dir_is_absolute() {
        let dir = config_dir();
        assert!(dir.is_absolute() || dir.starts_with("."));
        assert!(dir.ends_with("config"));
    }
}
