//! 后台资源监控循环模块
//!
//! 由 `launch` 子命令自动生成的子进程运行，亦支持独立启动。
//!
//! 监控逻辑：
//! 1. 按固定间隔轮询读取共享状态文件
//! 2. 用 `sysinfo` 查询每个注册实例的内存使用（RSS）
//! 3. 更新状态文件中的 `workingSetMB` 和 `lastHeartbeat`
//! 4. 对内存超限的实例输出告警日志
//! 5. 对已消失的进程从状态文件中自动清理

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use log::{error, info, warn};

use crate::state;

/// 默认轮询间隔（秒）
const DEFAULT_INTERVAL_SECS: u64 = 10;

/// 启动监控循环
///
/// 持续运行直到进程被外部终止（由 `launch` 子命令在清理阶段 kill）。
///
/// - 未指定间隔时使用默认值 10 秒
/// - 可通过 `init-config --monitor-interval` 持久化配置
/// - 指定 `parent_pid` 后，每轮周期检查父进程是否存活，
///   若父进程已消失（崩溃/强杀），则自动退出以避免孤儿进程
pub fn run(interval_secs: Option<u64>, parent_pid: Option<u32>) -> ExitCode {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .try_init();

    let interval = interval_secs.unwrap_or(DEFAULT_INTERVAL_SECS);
    info!("后台资源监控启动，轮询间隔 {}s", interval);
    if let Some(ppid) = parent_pid {
        info!("父进程 PID: {}，退出监测已启用", ppid);
    }

    loop {
        // 检查父进程是否存活（避免孤儿进程）
        if let Some(ppid) = parent_pid {
            if !process_exists(ppid) {
                info!("父进程 (PID {}) 已退出，监控终止", ppid);
                return ExitCode::SUCCESS;
            }
        }

        if let Err(e) = check_instances() {
            error!("监控检查失败: {}", e);
        }
        thread::sleep(Duration::from_secs(interval));
    }
}

/// 检查指定 PID 的进程是否存在
///
/// 使用 sysinfo 查询进程表，适用于跨平台。
fn process_exists(pid: u32) -> bool {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    sys.process(sysinfo::Pid::from_u32(pid)).is_some()
}

/// 检查所有注册实例的内存使用
///
/// 采用两阶段设计避免借用检查冲突：
/// - **Phase 1（只读扫描）**：收集待删除和待更新的键值对
/// - **Phase 2（批量写入）**：重新读取状态文件，一次性写入所有变更
fn check_instances() -> Result<(), Box<dyn std::error::Error>> {
    let state = state::read_state_file()?;
    if state.instances.is_empty() {
        return Ok(());
    }

    // 使用 sysinfo 获取内存数据
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();

    // Phase 1: 只读扫描，收集待删除和待更新的键值对
    let mut to_remove: Vec<String> = Vec::new();
    let mut to_update: Vec<(String, u64, String)> = Vec::new(); // (key, rss_mb, heartbeat)

    for (key, inst) in &state.instances {
        if let Some(process) = sys.process(sysinfo::Pid::from_u32(inst.pid)) {
            let rss_mb = process.memory() / (1024 * 1024);
            let heartbeat = chrono::Utc::now().to_rfc3339();
            to_update.push((key.clone(), rss_mb, heartbeat));

            if rss_mb > inst.max_allowed_memory_mb {
                warn!(
                    "实例 {} (PID {}) 内存超限: {}MB > {}MB",
                    key, inst.pid, rss_mb, inst.max_allowed_memory_mb
                );
            }
        } else {
            warn!("实例 {} (PID {}) 进程已不存在", key, inst.pid);
            to_remove.push(key.clone());
        }
    }

    // Phase 2: 加锁后一次性写入变更
    if to_remove.is_empty() && to_update.is_empty() {
        return Ok(());
    }

    let _lock = state::StateFileLock::acquire()?;
    let mut state = state::read_state_file()?; // 重新读取避免过期
    for key in &to_remove {
        state.instances.remove(key);
    }
    for (key, rss_mb, heartbeat) in &to_update {
        if let Some(inst) = state.instances.get_mut(key) {
            inst.working_set_mb = *rss_mb;
            inst.last_heartbeat = heartbeat.clone();
        }
    }
    state.global_state.total_instances = state.instances.len() as u32;
    state::write_state_file(&state)?;

    if !to_remove.is_empty() {
        info!("清理 {} 个已消失的实例", to_remove.len());
    }

    Ok(())
}


/// 一次性状态展示（前台界面）
///
/// 读取共享状态文件和系统信息，输出当前资源监控快照。
/// 不进入后台循环，输出后立即退出。
pub fn run_status() -> ExitCode {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_secs()
        .try_init();

    let state = match state::read_state_file() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读取状态文件失败: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    sys.refresh_memory();

    let total_mem_gb = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let used_mem_gb = sys.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let phys_cores = state.global_state.physical_cores;
    let total_instances = state.instances.len();

    println!("+------------------------------------------------------------+");
    println!("|  Qwen Code 资源监控状态 (v{})                              |", env!("CARGO_PKG_VERSION"));
    println!("+------------------------------------------------------------+");
    println!("|  系统物理内存: {:.1} GB  |  已用: {:.1} GB  |  逻辑处理器: {}",
        total_mem_gb, used_mem_gb, phys_cores);
    println!("+------------------------------------------------------------+");

    if state.instances.is_empty() {
        println!("|  (无注册实例)                                               |");
    } else {
        println!("  {:<8}  {:<8}  {:<10}  {:<10}  {:<8}  {:<16}",
            "PID", "CPU 核", "内存(MB)", "最大(MB)", "状态", "最后心跳");
        println!("  ------  ------  ---------  ---------  --------  ----------------");

        for inst in state.instances.values() {
            let alive = sys.process(sysinfo::Pid::from_u32(inst.pid)).is_some();
            let state_str = if alive { "running" } else { "dead" };
            let cores = inst.bound_cores.iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            // 提取心跳时间中的 HH:MM:SS 部分
            let hb_short = if inst.last_heartbeat.len() >= 19 {
                &inst.last_heartbeat[11..19]
            } else {
                "-"
            };
            println!("  {:<8}  {:<8}  {:<10}  {:<10}  {:<8}  {:<16}",
                inst.pid, cores, inst.working_set_mb, inst.max_allowed_memory_mb,
                state_str, hb_short);
        }
    }

    // 检查僵死锁文件
    let lock_path = state::state_file_path().with_extension("json.lock");
    let stale_lock = if lock_path.exists() {
        if let Ok(lock_pid_str) = std::fs::read_to_string(&lock_path) {
            if let Ok(lock_pid) = lock_pid_str.trim().parse::<u32>() {
                let mut s = sysinfo::System::new();
                s.refresh_process(sysinfo::Pid::from_u32(lock_pid));
                if s.process(sysinfo::Pid::from_u32(lock_pid)).is_none() {
                    format!("1 (僵死 PID {})", lock_pid)
                } else {
                    "0".to_string()
                }
            } else {
                "1 (损坏)".to_string()
            }
        } else {
            "1 (不可读)".to_string()
        }
    } else {
        "0".to_string()
    };

    println!("+------------------------------------------------------------+");
    println!("|  注册实例: {}  |  锁文件状态: {:<42} |",
        total_instances, if stale_lock == "0" { "正常" } else { &stale_lock });
    println!("+------------------------------------------------------------+");

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_interval_constant() {
        assert_eq!(DEFAULT_INTERVAL_SECS, 10);
    }

    #[test]
    fn test_check_instances_empty_state() {
        // 状态文件不存在或为空时，check_instances 不应 panic
        // 注：文件锁可能因其他测试残留的打开句柄而失败，此处只验证无 panic
        let _result = check_instances();
    }
}
