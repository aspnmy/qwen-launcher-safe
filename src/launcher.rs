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

use std::collections::HashSet;
use std::io;
use std::process::{Child, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, info, warn};

use crate::config;
use crate::process;
use crate::state;

/// 执行完整的启动流程
///
/// 接收透传给 qwen 命令的参数数组，
/// 返回进程退出码。
pub fn run(args: &[String]) -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();

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
    let registered_keys = match register_instances(&new_pids) {
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
    let monitor_child = spawn_monitor();
    let has_monitor = monitor_child.is_ok();
    if has_monitor {
        info!("后台监控已启动");
    } else {
        warn!("后台监控启动失败");
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

/// 向共享状态文件注册实例并绑定 CPU 核
///
/// 1. 读取共享状态文件
/// 2. 收集已占用核心（避免多实例冲突）
/// 3. 为每个新 PID 分配最小空闲核心
/// 4. 写入状态文件并调用 Windows API 绑定 CPU 亲和性
fn register_instances(pids: &[u32]) -> io::Result<Vec<String>> {
    let mut state = state::read_state_file()?;
    let phys_cores = process::get_physical_core_count();
    state.global_state.physical_cores = phys_cores;

    // 从配置文件读取内存限制
    let cfg = config::read_config();
    let max_memory = cfg.max_memory_mb;

    // 收集已占用核心
    let mut used_cores: HashSet<u32> = HashSet::new();
    for inst in state.instances.values() {
        if inst.state == "running" {
            for c in &inst.bound_cores {
                used_cores.insert(*c);
            }
        }
    }

    let mut registered = Vec::new();
    for &pid in pids {
        let pkey = pid.to_string();
        if state.instances.contains_key(&pkey) {
            continue;
        }

        // 分配最小空闲核心
        let core = (0..phys_cores)
            .find(|c| !used_cores.contains(c))
            .unwrap_or(0);
        used_cores.insert(core);

        let priority = state.instances.len() as u32 + 1;
        let inst = state::new_instance(pid, core, priority, max_memory);
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
/// 读取配置文件中的监控间隔，以 `monitor --interval <秒>` 参数
/// 自生成一个子进程运行后台监控循环。
fn spawn_monitor() -> io::Result<Child> {
    let exe = process::self_exe_path()?;
    let cfg = config::read_config();
    let interval_sec = format!("{}", cfg.monitor_interval_sec);
    let child = std::process::Command::new(&exe)
        .arg("monitor")
        .arg("--interval")
        .arg(&interval_sec)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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
    // 系统内存快照（只取一次，用于显示总量）
    let mut sysinfo_snapshot = sysinfo::System::new_all();
    sysinfo_snapshot.refresh_memory();
    let total_mem_gb = sysinfo_snapshot.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    loop {
        thread::sleep(Duration::from_secs(2));

        let mut sys = sysinfo::System::new_all();
        sys.refresh_all();

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
        display_dashboard(monitored_pids, total_mem_gb);
    }
}

/// 显示实时资源仪表盘：CPU 核占用 + 内存使用
fn display_dashboard(monitored_pids: &[u32], total_mem_gb: f64) {
    // 使用回车回到行首，更新仪表盘区域
    // 先读取共享状态
    let state = crate::state::read_state_file().ok();

    // 上移光标到仪表盘起始位置（除了 info! 的"等待 X 个 Qwen 进程退出..."行）
    // 简单做法：每次输出一个完整的信息块
    let mut output = String::new();

    // 清屏方式：输出多个空行覆盖之前的内容（兼容 Windows cmd 无 ANSI）
    // 使用 ANSI 清屏序列（cmd 支持 ANSI escape codes）
    output.push_str("\x1b[2J\x1b[H"); // 清屏 + 光标归位

    output.push_str("+------------------------------------------------------------+\n");
    output.push_str("|  Qwen Code 资源监控仪表盘                                   |\n");
    output.push_str("+------------------------------------------------------------+\n");
    output.push_str(&format!("|  系统物理内存: {:.1} GB", total_mem_gb));

    // 显示已用内存
    let mut sys = sysinfo::System::new_all();
    sys.refresh_memory();
    let used_mem_gb = sys.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    output.push_str(&format!("  |  已用: {:.1} GB", used_mem_gb));

    // CPU 核心数
    let phys_cores = process::get_physical_core_count();
    output.push_str(&format!("  |  物理核心: {}\n", phys_cores));
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
                    pid,
                    cores,
                    inst.working_set_mb,
                    inst.max_allowed_memory_mb,
                    inst.state
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
