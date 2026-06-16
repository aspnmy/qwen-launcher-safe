//! 启动器主编排模块
//!
//! 实现完整的 Qwen 启动生命周期：
//!
//! 1. **基线记录** — 记录启动前的 Qwen 进程池
//! 2. **非阻塞启动** — 启动 Qwen 进程但不阻塞当前线程
//! 3. **子进程发现** — 轮询 5 秒检测新产生的 Qwen 子进程
//! 4. **注册与绑定** — 向共享状态文件注册实例并绑定独占 CPU 核
//! 5. **后台监控** — 自生成 `monitor` 子进程定期检查内存
//! 6. **等待退出** — 收养直接子进程后轮询所有监控 PID 直至消亡
//! 7. **清理** — 停止监控并注销所有已注册实例

use std::collections::{HashMap, HashSet};
use std::io;
use std::process::{Child, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, info, warn};

/// Ctrl+C 触发时设为 true，主循环检测后触发优雅退出
static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

use crate::config;
use crate::process;
use crate::state;

/// 执行完整的启动流程
///
/// 接收透传给 qwen 命令的参数数组，
/// 返回进程退出码。
pub fn run(args: &[String]) -> ExitCode {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .try_init();

    info!("Qwen Code 资源保护启动器 (Rust)");

    let qwen_cmd = match process::find_qwen_command() {
        Ok(cmd) => {
            info!("Qwen 命令: {:?}", cmd);
            cmd
        }
        Err(e) => {
            error!("{}", e);
            return ExitCode::from(1);
        }
    };

    // 读取工作目录配置（使子进程能加载对应目录下的 .qwen/skills/ 技能）
    let cfg = config::read_config();
    let working_dir = cfg.working_dir.as_ref().map(std::path::PathBuf::from);
    if let Some(ref wd) = working_dir {
        info!("Qwen 工作目录: {:?}", wd);
    }

    // 1. 基线记录
    let mut sys = sysinfo::System::new_all();
    sys.refresh_all();
    let baseline = process::get_qwen_pids(&sys);
    info!("基线 Qwen 进程: {} 个", baseline.len());

    // 2. 非阻塞启动 Qwen
    let qwen_child = match process::spawn_qwen(&qwen_cmd, args, working_dir.as_deref()) {
        Ok(child) => {
            info!("Qwen PID: {}", child.id());
            child
        }
        Err(e) => {
            error!("启动 Qwen 失败: {}", e);
            return ExitCode::from(1);
        }
    };

    // 3. 轮询发现子进程（至多 5 秒）
    let new_pids = poll_new_qwen_processes(&baseline);
    let (new_pids, monitored_qwen_pids) = if new_pids.is_empty() {
        warn!(
            "未发现子 node 进程，将 Qwen 主进程 PID {} 纳入监控",
            qwen_child.id()
        );
        (vec![qwen_child.id()], vec![qwen_child.id()])
    } else {
        info!("发现 {} 个新 Qwen 进程: {:?}", new_pids.len(), new_pids);
        (new_pids.clone(), new_pids)
    };

    // 4. 注册实例 + 绑定 CPU
    let registered_keys = match register_instances(&new_pids, cfg.max_memory_mb) {
        Ok(keys) => {
            info!("已注册 {} 个实例", keys.len());
            keys
        }
        Err(e) => {
            error!("注册实例失败: {}", e);
            return ExitCode::from(1);
        }
    };

    // 5. 启动后台监控
    let monitor_child = spawn_monitor(cfg.monitor_interval_sec);
    let has_monitor = monitor_child.is_ok();
    if has_monitor {
        info!("后台监控已启动");
    } else {
        warn!("后台监控启动失败");
    }

    // 设置 Ctrl+C 信号处理器
    if let Err(e) = ctrlc::set_handler(move || {
        info!("收到 Ctrl+C，开始清理资源...");
        SHOULD_EXIT.store(true, Ordering::SeqCst);
    }) {
        warn!("注册 Ctrl+C 处理器失败: {}", e);
    }

    // 6. 等待 Qwen 退出
    info!("等待 Qwen 退出中...");
    let exit_code = wait_for_qwen(qwen_child, &monitored_qwen_pids);

    // 7. 清理
    cleanup(monitor_child, &registered_keys);

    info!("Qwen Code 已退出 (code: {})", exit_code);
    ExitCode::from(exit_code as u8)
}

/// 轮询发现新 Qwen 子进程
///
/// 在 5 秒超时内以 300ms 间隔轮询系统进程表，
/// 返回所有不在基线中的新 Qwen 相关进程 PID。
fn poll_new_qwen_processes(baseline: &HashSet<u32>) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();
        let current = process::get_qwen_pids(&sys);
        let new_pids: Vec<u32> = current
            .iter()
            .filter(|p| !baseline.contains(p))
            .copied()
            .collect();
        if !new_pids.is_empty() {
            return new_pids;
        }
        if Instant::now() >= deadline {
            return Vec::new();
        }
        thread::sleep(Duration::from_millis(300));
    }
}

/// 从核心负载表中选择最优核心
///
/// Phase 1: 优先完全空闲的核心（不在 `core_load` 中）
/// Phase 2: 无空闲核时，选负载最低的核均匀分摊
fn select_best_core(phys_cores: u32, core_load: &HashMap<u32, u32>) -> u32 {
    (0..phys_cores)
        .find(|c| !core_load.contains_key(c))
        .unwrap_or_else(|| {
            (0..phys_cores)
                .min_by_key(|c| core_load.get(c).copied().unwrap_or(0))
                .unwrap_or(0)
        })
}

/// 向共享状态文件注册实例并绑定 CPU 核
///
/// 1. 读取共享状态文件
/// 2. 收集已占用核心（避免多实例冲突）
/// 3. 为每个新 PID 分配最小空闲核心
/// 4. 写入状态文件并调用 Windows API 绑定 CPU 亲和性
fn register_instances(pids: &[u32], max_memory_mb: u64) -> io::Result<Vec<String>> {
    let mut state = state::read_state_file()?;
    let phys_cores = process::get_processor_count();
    state.global_state.physical_cores = phys_cores;

    // 统计每核心已绑定的实例数（core → count）
    let mut core_load: HashMap<u32, u32> = HashMap::with_capacity(phys_cores as usize);
    for inst in state.instances.values() {
        if inst.state == "running" {
            for c in &inst.bound_cores {
                *core_load.entry(*c).or_insert(0) += 1;
            }
        }
    }

    let mut registered = Vec::new();
    for &pid in pids {
        let pkey = pid.to_string();
        if state.instances.contains_key(&pkey) {
            continue;
        }

        let was_shared = core_load.len() >= phys_cores as usize;
        let core = select_best_core(phys_cores, &core_load);
        *core_load.entry(core).or_insert(0) += 1;

        if was_shared {
            warn!(
                "核心不足，PID {} 共享核心 {}（共享后负载 {} 实例）",
                pid, core, core_load[&core]
            );
        }

        let priority = state.instances.len() as u32 + 1;
        let inst = state::new_instance(pid, core, priority, max_memory_mb);
        state.instances.insert(pkey.clone(), inst);
        registered.push(pkey.clone());

        // 绑定 CPU
        if let Err(e) = process::bind_cpu_core(pid, core) {
            warn!("绑定 CPU 核失败 (PID {}): {}", pid, e);
        }
    }

    state.global_state.total_instances = state.instances.len() as u32;
    state::write_state_file(&state)?;
    Ok(registered)
}

/// 生成后台监控子进程
///
/// 以 `monitor --interval <秒>` 参数自生成一个子进程运行后台监控循环。
fn spawn_monitor(interval_sec: u64) -> io::Result<Child> {
    let exe = process::self_exe_path()?;
    let interval_str = interval_sec.to_string();
    let child = std::process::Command::new(&exe)
        .arg("monitor")
        .arg("--interval")
        .arg(&interval_str)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;
    Ok(child)
}

/// 等待 Qwen 退出
///
/// 支持两种场景：
/// - **直接进程**（qwen.exe）：`child.wait()` 阻塞直到退出，`monitored_pids` 同 PID → 循环立即退出
/// - **批处理封装**（qwen.cmd → node.exe）：`child.wait()` 先退出（cmd 包装器），
///   然后轮询监控列表中的 node PID 直到全部消亡，控制台保持打开
///
/// 等待期间实时显示 CPU 核占用和内存使用仪表盘。
fn wait_for_qwen(mut child: Child, monitored_pids: &[u32]) -> i32 {
    // 先收养直接子进程（避免僵尸进程），不关心其退出码
    let _ = child.wait();

    // 如果直接子进程就是唯一监控目标，直接返回
    if monitored_pids.is_empty() {
        return 0;
    }

    // 轮询所有监控 PID，直到全部消亡
    info!("等待 {} 个 Qwen 进程退出...", monitored_pids.len());
    let deadline = Instant::now() + Duration::from_secs(86400); // 24 小时兜底
    let total_mem_gb = {
        let mut sys = sysinfo::System::new_all();
        sys.refresh_memory();
        sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0)
    };
    loop {
        thread::sleep(Duration::from_secs(2));

        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();

        if SHOULD_EXIT.load(Ordering::SeqCst) {
            info!("检测到 Ctrl+C 信号，退出等待循环");
            println!();
            return 2; // 被信号中断
        }

        let still_alive = monitored_pids
            .iter()
            .any(|pid| sys.process(sysinfo::Pid::from_u32(*pid)).is_some());

        if !still_alive {
            println!();
            return 0;
        }

        if Instant::now() >= deadline {
            warn!("等待超时（24 小时），强制退出");
            return 1;
        }

        // 读取共享状态，更新仪表盘
        display_dashboard(monitored_pids, total_mem_gb, &sys);
    }
}

/// 显示实时资源仪表盘：CPU 核占用 + 内存使用
fn display_dashboard(monitored_pids: &[u32], total_mem_gb: f64, sys: &sysinfo::System) {
    // 使用回车回到行首，更新仪表盘区域
    // 先读取共享状态
    let state = crate::state::read_state_file().ok();

    let mut output = String::new();

    // 清屏 + 光标归位
    output.push_str("\x1b[2J\x1b[H");

    output.push_str("+------------------------------------------------------------+\n");
    output.push_str("|  Qwen Code 资源监控仪表盘                                   |\n");
    output.push_str("+------------------------------------------------------------+\n");
    output.push_str(&format!("|  系统物理内存: {:.1} GB", total_mem_gb));

    // 显示已用内存（使用传入的 sys 引用，避免重复创建）
    let used_mem_gb = sys.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    output.push_str(&format!("  |  已用: {:.1} GB", used_mem_gb));

    // CPU 核心数
    let phys_cores = process::get_processor_count();
    output.push_str(&format!("  |  逻辑处理器: {}\n", phys_cores));
    output.push_str("+------------------------------------------------------------+\n");

    // 表头
    output.push_str(&format!(
        "  {:<8}  {:<8}  {:<10}  {:<14}  {:<8}\n",
        "PID", "CPU 核", "内存(MB)", "最大内存(MB)", "状态"
    ));
    output.push_str("  ------  ------  ---------  --------------  --------\n");

    if let Some(ref state_file) = state {
        for pid in monitored_pids {
            let pkey = pid.to_string();
            if let Some(inst) = state_file.instances.get(&pkey) {
                let cores = inst
                    .bound_cores
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                output.push_str(&format!(
                    "  {:<8}  {:<8}  {:<10}  {:<14}  {:<8}\n",
                    pid, cores, inst.working_set_mb, inst.max_allowed_memory_mb, inst.state
                ));
            } else {
                // 实例已注销但进程仍在监控列表中
                output.push_str(&format!(
                    "  {:<8}  {:<8}  {:<10}  {:<14}  {:<8}\n",
                    pid, "-", "-", "-", "注销中"
                ));
            }
        }
    } else {
        for pid in monitored_pids {
            output.push_str(&format!(
                "  {:<8}  {:<8}  {:<10}  {:<14}  {:<8}\n",
                pid, "-", "-", "-", "等待注册"
            ));
        }
    }
    output.push_str("+------------------------------------------------------------+\n");
    output.push_str("|  按 Ctrl+C 终止 Qwen 并自动清理资源                          |\n");
    output.push_str("+------------------------------------------------------------+\n");

    print!("{}", output);
    use std::io::Write;
    std::io::stdout().flush().ok();
}

/// 清理：停止监控子进程 + 从共享状态注销实例
fn cleanup(monitor_child: io::Result<Child>, registered_keys: &[String]) {
    // 停止监控
    if let Ok(mut child) = monitor_child {
        let _ = child.kill();
        let _ = child.wait();
    }

    // 注销实例
    if !registered_keys.is_empty() {
        let mut state = match state::read_state_file() {
            Ok(s) => s,
            Err(e) => {
                warn!("读取状态文件失败: {}", e);
                return;
            }
        };
        for key in registered_keys {
            if state.instances.remove(key).is_some() {
                info!("已注销实例 {}", key);
            }
        }
        state.global_state.total_instances = state.instances.len() as u32;
        if let Err(e) = state::write_state_file(&state) {
            warn!("写入状态文件失败: {}", e);
        }
        info!("共注销 {} 个实例", registered_keys.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn test_should_exit_default_false() {
        assert!(!SHOULD_EXIT.load(Ordering::SeqCst));
    }

    #[test]
    fn test_should_exit_signal_triggers() {
        SHOULD_EXIT.store(true, Ordering::SeqCst);
        assert!(SHOULD_EXIT.load(Ordering::SeqCst));
        SHOULD_EXIT.store(false, Ordering::SeqCst);
    }

    #[test]
    fn test_processor_count_is_reasonable() {
        let count = process::get_processor_count();
        assert!(count >= 1, "逻辑处理器数应 >= 1");
        assert!(count < 1024, "逻辑处理器数应合理");
    }

    #[test]
    fn test_select_best_core_empty_load() {
        let load = HashMap::new();
        // 无负载时选第一个空闲核 (0)
        assert_eq!(select_best_core(4, &load), 0);
    }

    #[test]
    fn test_select_best_core_first_free() {
        let mut load = HashMap::new();
        load.insert(0, 1);
        load.insert(2, 1);
        // core 0 和 2 已被占，应选 core 1（第一个空闲）
        assert_eq!(select_best_core(4, &load), 1);
    }

    #[test]
    fn test_select_best_core_all_occupied_equal() {
        let mut load = HashMap::new();
        for i in 0..4 {
            load.insert(i, 1);
        }
        // 所有核负载均为 1，应选索引最小的 (0)
        assert_eq!(select_best_core(4, &load), 0);
    }

    #[test]
    fn test_select_best_core_least_loaded() {
        let mut load = HashMap::new();
        load.insert(0, 3);
        load.insert(1, 5);
        load.insert(2, 1);
        load.insert(3, 3);
        // core 2 负载最低 (1)
        assert_eq!(select_best_core(4, &load), 2);
    }

    #[test]
    fn test_select_best_core_tie_breaker() {
        let mut load = HashMap::new();
        load.insert(0, 2);
        load.insert(1, 1);
        load.insert(2, 1);
        load.insert(3, 2);
        // core 1 和 2 负载相同 (均为 1)，选最小索引 (1)
        assert_eq!(select_best_core(4, &load), 1);
    }
}
