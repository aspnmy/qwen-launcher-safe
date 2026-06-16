/// 后台资源监控循环 — 每 N 秒检查注册实例的内存使用
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use log::{error, info, warn};

use crate::state;

/// 默认轮询间隔（秒）
const DEFAULT_INTERVAL_SECS: u64 = 10;

/// 启动监控循环
pub fn run(interval_secs: Option<u64>) -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

    let interval = interval_secs.unwrap_or(DEFAULT_INTERVAL_SECS);
    info!("后台资源监控启动，轮询间隔 {}s", interval);

    loop {
        thread::sleep(Duration::from_secs(interval));
        if let Err(e) = check_instances() {
            error!("监控检查失败: {}", e);
        }
    }
}

/// 检查所有注册实例的内存使用
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

    // Phase 2: 一次性写入变更
    if to_remove.is_empty() && to_update.is_empty() {
        return Ok(());
    }

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
