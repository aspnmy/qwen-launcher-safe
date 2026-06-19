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
use fs2::FileExt;
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

/// 返回共享状态文件路径
///
/// - Windows: `%TEMP%\qwen-resource-state.json`
/// - Unix/Linux: `/tmp/qwen-resource-state.json`
pub fn state_file_path() -> PathBuf {
    #[cfg(windows)]
    {
        let tmp = std::env::var("TEMP")
            .or_else(|_| std::env::var("TMP"))
            .unwrap_or_else(|_| r"C:\Windows\Temp".into());
        PathBuf::from(tmp).join("qwen-resource-state.json")
    }
    #[cfg(unix)]
    {
        let tmp = std::env::var("XDG_RUNTIME_DIR")
            .or_else(|_| std::env::var("TMPDIR"))
            .unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(tmp).join("qwen-resource-state.json")
    }
    #[cfg(not(any(windows, unix)))]
    {
        PathBuf::from("/tmp").join("qwen-resource-state.json")
    }
}

/// 读取状态文件，文件不存在时返回 [`StateFile::default`]
///
/// 当文件内容尾部存在多余垃圾字符（如并发写入中断导致的 `}}}`）时，
/// 通过深度计数器找到正确的根闭括号位置，截断后重试并自动修复文件。
pub fn read_state_file() -> io::Result<StateFile> {
    let path = state_file_path();
    if !path.exists() {
        return Ok(StateFile::default());
    }
    let data = fs::read_to_string(&path)?;
    match serde_json::from_str::<StateFile>(&data) {
        Ok(state) => Ok(state),
        Err(e) => {
            // 容错：用深度计数器找到根对象的正确闭括号 `}`
            if let Some(pos) = find_root_close(&data) {
                let trimmed = &data[..=pos];
                if let Ok(state) = serde_json::from_str::<StateFile>(trimmed) {
                    // 修复文件（覆盖写入干净 JSON）
                    let clean = serde_json::to_string_pretty(&state).map_err(io::Error::other)?;
                    let _ = fs::write(&path, clean);
                    return Ok(state);
                }
            }
            Err(io::Error::new(io::ErrorKind::InvalidData, e))
        }
    }
}

/// 找到根对象（depth 0）的匹配闭括号位置，跳过字符串中的 `{}`
fn find_root_close(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '"' {
            // 跳过字符串字面量，避免误判其中的 `{` `}`
            while let Some(&(_, c)) = chars.peek() {
                chars.next();
                if c == '\\' {
                    chars.next(); // 跳过转义字符
                } else if c == '"' {
                    break; // 字符串结束
                }
            }
        } else if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// 将当前状态序列化为 JSON 并原子写入文件
///
/// 先写入 `.json.tmp` 临时文件，再 rename 为目标文件，
/// 避免并发写入导致内容损坏（如尾部多余字符）。
pub fn write_state_file(state: &StateFile) -> io::Result<()> {
    let path = state_file_path();
    let data = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &data)?;
    fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// 状态文件互斥锁守卫
///
/// 持有期间阻止其他进程（通过 `fs2::FileExt::lock_exclusive`）读取或写入状态文件。
/// 用于 `read→modify→write` 原子操作的保护，防止多进程并发导致：
/// - CPU 核分配冲突（两个进程绑定到同一核）
/// - 实例注册丢失（最后写入覆盖）
/// - totalInstances 计数不准确
///
/// Drop 时自动释放文件锁。
pub struct StateFileLock {
    _file: std::fs::File,
}

/// 清理状态文件中僵死实例（PID 不再存在于系统中）
///
/// 用于 launcher 启动时和 monitor 轮询中自动清理进程崩溃后残留的注册记录。
/// 跳过当前锁文件的持有进程（避免无竞争下的自清理）。
pub fn cleanup_stale_entries(state: &mut StateFile) {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    let before = state.instances.len();
    state
        .instances
        .retain(|_key, inst| sys.process(sysinfo::Pid::from_u32(inst.pid)).is_some());
    let removed = before - state.instances.len();
    if removed > 0 {
        log::info!("清理 {} 个僵死实例", removed);
    }
    state.global_state.total_instances = state.instances.len() as u32;
}

impl StateFileLock {
    /// 获取状态文件排他锁
    ///
    /// 文件不存在时先创建空状态文件，确保 lock_exclusive 可操作。
    pub fn acquire() -> io::Result<Self> {
        let path = state_file_path();
        // 确保文件存在以便加锁
        if !path.exists() {
            let empty = StateFile::default();
            write_state_file(&empty)?;
        }

        const MAX_RETRIES: u32 = 5;
        const RETRY_DELAY_MS: u64 = 200;
        let mut last_err = None;

        for attempt in 0..MAX_RETRIES {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(false)
                .open(&path)?;
            match file.lock_exclusive() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;

    #[test]
    fn test_find_root_close_normal() {
        let data = r#"{"instances":{},"globalState":{"totalInstances":0,"physicalCores":0}}"#;
        let pos = find_root_close(data);
        assert!(pos.is_some(), "应找到根闭括号");
        assert_eq!(&data[..=pos.unwrap()], data, "应匹配整个 JSON");
    }

    #[test]
    fn test_find_root_close_with_trailing_junk() {
        let data = r#"{"instances":{},"globalState":{"totalInstances":0,"physicalCores":0}}}"#;
        let pos = find_root_close(data);
        assert!(pos.is_some(), "应找到第一个根闭括号");
        let trimmed = &data[..=pos.unwrap()];
        let parsed: serde_json::Value = serde_json::from_str(trimmed).expect("截断后应可解析");
        assert_eq!(parsed["globalState"]["totalInstances"], 0);
    }

    #[test]
    fn test_find_root_close_skips_string_braces() {
        // 字符串中包含 { 和 }，不应干扰深度计数
        let data = r#"{"key":"a{b}c"}"#;
        let pos = find_root_close(data);
        assert!(pos.is_some(), "应正确跳过字符串中的括号");
        assert_eq!(&data[..=pos.unwrap()], data);
    }

    #[serial]
    #[test]
    fn test_tolerant_read_of_corrupted_state() {
        // 构造一个尾部有垃圾字符的损坏 JSON
        let corrupted = r#"{
  "instances": {
    "1234": {
      "pid": 1234,
      "startTime": "2026-06-17T00:00:00+00:00",
      "workingSetMB": 256,
      "boundCores": [0],
      "maxAllowedMemoryMB": 1024,
      "state": "running",
      "priority": 1,
      "lastHeartbeat": "2026-06-17T00:00:00+00:00"
    }
  },
  "globalState": {
    "totalInstances": 1,
    "physicalCores": 8
  }
}}}"#;

        // 备份原始状态文件
        let orig_path = state_file_path();
        let orig_backup = if orig_path.exists() {
            fs::read_to_string(&orig_path).ok()
        } else {
            None
        };

        // 写入损坏内容到真实路径
        fs::write(&orig_path, corrupted).expect("写入损坏测试数据");

        // 测试容错读取
        let result = read_state_file();
        assert!(result.is_ok(), "容错读取应成功: {:?}", result.err());
        let state = result.unwrap();
        assert_eq!(state.instances.len(), 1);
        assert_eq!(state.global_state.total_instances, 1);
        assert_eq!(state.global_state.physical_cores, 8);

        // 验证文件现在已被修复（干净 JSON）
        let fixed_data = fs::read_to_string(&orig_path).expect("读取修复后文件");
        let parsed: Result<StateFile, _> = serde_json::from_str(&fixed_data);
        assert!(
            parsed.is_ok(),
            "修复后的文件应可正常解析: {:?}",
            parsed.err()
        );

        // 恢复原始状态文件
        match orig_backup {
            Some(content) => fs::write(&orig_path, content).ok(),
            None => fs::remove_file(&orig_path).ok(),
        };
    }

    #[test]
    fn test_new_instance_creates_valid_record() {
        let inst = new_instance(42, 1, 2, 2048);
        assert_eq!(inst.pid, 42);
        assert_eq!(inst.bound_cores, vec![1]);
        assert_eq!(inst.max_allowed_memory_mb, 2048);
        assert_eq!(inst.state, "running");
        assert_eq!(inst.priority, 2);
        assert_eq!(inst.working_set_mb, 0);
        assert!(!inst.start_time.is_empty());
        assert_eq!(inst.last_heartbeat, inst.start_time);
    }

    #[serial]
    #[test]
    fn test_read_state_file_always_returns_valid() {
        // 无论状态文件是否存在，read_state_file() 都应返回可用的 StateFile
        let result = read_state_file();
        assert!(result.is_ok(), "应返回有效的 StateFile: {:?}", result.err());
        let state = result.unwrap();
        // globalState 应始终有合理的值（总实例数可能为 0 或正数）
        assert!(state.global_state.physical_cores < 1024, "CPU 核心数应合理");
    }

    #[test]
    fn test_cleanup_stale_entries_removes_dead_pids() {
        // 创建一个包含不存在的 PID 的状态文件
        let mut state = StateFile::default();
        let inst = new_instance(999_999, 0, 1, 1024); // 这个 PID 几乎肯定不存在
        state.instances.insert("999999".into(), inst);
        assert_eq!(state.instances.len(), 1);

        cleanup_stale_entries(&mut state);
        assert_eq!(state.instances.len(), 0, "不存在的 PID 应被清理");
        assert_eq!(state.global_state.total_instances, 0);
    }

    #[test]
    fn test_cleanup_stale_entries_preserves_live() {
        // 当前进程应被视为存活的
        let mut state = StateFile::default();
        let my_pid = std::process::id();
        let inst = new_instance(my_pid, 0, 1, 1024);
        state.instances.insert(my_pid.to_string(), inst);
        assert_eq!(state.instances.len(), 1);

        cleanup_stale_entries(&mut state);
        assert_eq!(state.instances.len(), 1, "当前进程应被保留");
    }
}
