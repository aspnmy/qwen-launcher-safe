//! Qwen Code 资源保护启动器 — CLI 入口和子命令分发
//!
//! 提供三个子命令：
//! - `launch` — 启动 Qwen 并自动注册资源监控
//! - `monitor` — 后台资源监控循环
//! - `init-config` — 初始化或更新配置文件
//!
//! 双击 `.exe` 无参数时默认进入 `launch` 模式（自动搜索 qwen 并启动）。
//!
//! # 使用示例
//!
//! ```bash
//! # 双击启动（无参） — 等价于 launch 无参数
//! qwen-launcher-safe.exe
//!
//! # 启动 Qwen 并传递参数
//! qwen-launcher-safe launch -- --model qwen-max
//!
//! # 配置 qwen 路径
//! qwen-launcher-safe init-config --qwen-path auto
//!
//! # 查看配置
//! qwen-launcher-safe init-config --show
//! ```

use std::process::ExitCode;

use clap::Parser;

mod config;
mod launcher;
mod monitor;
mod process;
mod state;

/// Qwen Code 资源保护启动器 — Rust 重构版
#[derive(Parser, Debug)]
#[command(name = "qwen-launcher-safe", version, about)]
enum Cli {
    /// 启动 Qwen 并自动注册资源监控
    Launch {
        /// Qwen 额外参数，透传给 qwen 命令（支持连字符参数）
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        qwen_args: Vec<String>,
    },
    /// 后台资源监控（由 launch 子命令自动启动）
    Monitor {
        /// 轮询间隔（秒）
        #[arg(short, long, default_value_t = 10)]
        interval: u64,
    },
    /// 初始化或更新 .config 配置文件
    #[command(alias = "init")]
    InitConfig {
        /// qwen 可执行文件路径，传 "auto" 自动搜索
        #[arg(long)]
        qwen_path: Option<String>,
        /// 最大内存限制（MB）
        #[arg(long)]
        max_memory_mb: Option<u64>,
        /// 监控轮询间隔（秒）
        #[arg(long)]
        monitor_interval: Option<u64>,
        /// 显示当前配置
        #[arg(long)]
        show: bool,
    },
}

/// 交互式配置向导
///
/// 当配置文件不存在且用户未指定任何参数时自动进入。
/// 引导用户配置 qwen 路径、内存限制和监控间隔。
fn interactive_setup() -> ExitCode {
    use std::io::{stdin, stdout, Write};

    println!("+----------------------------------------------+");
    println!("|  未检测到配置文件，进入交互式配置向导         |");
    println!("|  （直接回车使用默认值）                       |");
    println!("+----------------------------------------------+");
    println!();

    // ── 步骤 1：Qwen 路径 ──
    println!("[步骤 1/3] Qwen 可执行文件路径");
    let qwen_path = match process::find_qwen_command() {
        Ok(auto_path) => {
            let display = auto_path.to_string_lossy().to_string();
            print!("  自动搜索到: {}\n  是否使用此路径？[Y/n]: ", display);
            stdout().flush().ok();
            let mut line = String::new();
            stdin().read_line(&mut line).ok();
            let line = line.trim().to_lowercase();
            if line.is_empty() || line == "y" || line == "yes" {
                Some(display)
            } else {
                print!("  请输入 qwen 路径（或留空跳过）: ");
                stdout().flush().ok();
                let mut manual = String::new();
                stdin().read_line(&mut manual).ok();
                let manual = manual.trim().to_string();
                if manual.is_empty() {
                    None
                } else {
                    Some(manual)
                }
            }
        }
        Err(_) => {
            print!("  自动搜索失败，请输入 qwen 路径（或留空跳过）: ");
            stdout().flush().ok();
            let mut manual = String::new();
            stdin().read_line(&mut manual).ok();
            let manual = manual.trim().to_string();
            if manual.is_empty() {
                None
            } else {
                Some(manual)
            }
        }
    };

    // ── 步骤 2：内存限制 ──
    println!();
    println!("[步骤 2/3] 最大内存限制 (MB) [默认 1024]");
    print!("  请输入: ");
    stdout().flush().ok();
    let mut line = String::new();
    stdin().read_line(&mut line).ok();
    let line = line.trim();
    let max_memory_mb = if line.is_empty() {
        1024
    } else {
        line.parse::<u64>().unwrap_or(1024)
    };

    // ── 步骤 3：监控间隔 ──
    println!();
    println!("[步骤 3/3] 监控轮询间隔 (秒) [默认 10]");
    print!("  请输入: ");
    stdout().flush().ok();
    let mut line = String::new();
    stdin().read_line(&mut line).ok();
    let line = line.trim();
    let monitor_interval = if line.is_empty() {
        10
    } else {
        line.parse::<u64>().unwrap_or(10)
    };

    // ── 写入配置 ──
    let mut cfg = config::LauncherConfig {
        qwen_path: None,
        max_memory_mb,
        monitor_interval_sec: monitor_interval,
    };
    if let Some(ref path) = qwen_path {
        cfg.qwen_path = Some(path.clone());
    }

    println!();
    match config::write_config(&cfg) {
        Ok(()) => {
            println!("[OK] 配置已完成，已写入: {:?}", config::config_file_path());
            println!("{}", serde_json::to_string_pretty(&cfg).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[ERR] 写入配置失败: {}", e);
            ExitCode::from(1)
        }
    }
}

/// 程序入口
///
/// 解析 CLI 参数后分发到对应子命令处理函数。
/// 双击 `.exe` 无参数时：`try_parse` 失败（MissingSubcommand）→ 默认走 `launch`。
fn main() -> ExitCode {
    match Cli::try_parse() {
        Ok(Cli::Launch { qwen_args }) => launcher::run(&qwen_args),
        Ok(Cli::Monitor { interval }) => monitor::run(Some(interval)),
        Ok(Cli::InitConfig {
            qwen_path,
            max_memory_mb,
            monitor_interval,
            show,
        }) => cmd_init_config(qwen_path, max_memory_mb, monitor_interval, show),
        Err(e) => {
            // 双击 .exe 无参：缺少子命令 → 默认走 launch
            if e.kind() == clap::error::ErrorKind::MissingSubcommand {
                launcher::run(&[])
            } else {
                // 其他解析错误（如无效参数）正常报错退出
                e.exit()
            }
        }
    }
}

/// 处理 `init-config` 子命令
///
/// 支持以下操作：
/// - `--show`：显示当前配置
/// - `--qwen-path auto`：自动搜索并写入
/// - `--qwen-path <路径>`：手动指定路径
/// - `--max-memory-mb <MB>`：调整内存限制
/// - `--monitor-interval <秒>`：调整轮询间隔
fn cmd_init_config(
    qwen_path: Option<String>,
    max_memory_mb: Option<u64>,
    monitor_interval: Option<u64>,
    show: bool,
) -> ExitCode {
    if show {
        let cfg = config::read_config();
        println!("{}", serde_json::to_string_pretty(&cfg).unwrap());
        return ExitCode::SUCCESS;
    }

    let mut cfg = config::read_config();
    let mut changed = false;

    if let Some(p) = qwen_path {
        let resolved = if p.eq_ignore_ascii_case("auto") {
            // 先尝试自动搜索，搜到就写入配置文件
            match process::find_qwen_command() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => {
                    eprintln!("自动搜索失败，请手动指定路径");
                    return ExitCode::from(1);
                }
            }
        } else {
            let path = std::path::Path::new(&p);
            if !path.exists() {
                eprintln!("路径不存在: {}", p);
                return ExitCode::from(1);
            }
            p
        };
        cfg.qwen_path = Some(resolved);
        changed = true;
    }
    if let Some(m) = max_memory_mb {
        cfg.max_memory_mb = m;
        changed = true;
    }
    if let Some(i) = monitor_interval {
        cfg.monitor_interval_sec = i;
        changed = true;
    }

    if !changed {
        if !config::config_file_path().exists() {
            return interactive_setup();
        }
        eprintln!(
            "未指定任何配置项。使用 --help 查看选项。\n  当前配置: {:?}",
            config::config_file_path()
        );
        return ExitCode::from(1);
    }

    match config::write_config(&cfg) {
        Ok(()) => {
            println!("配置已写入: {:?}", config::config_file_path());
            println!("{}", serde_json::to_string_pretty(&cfg).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写入配置失败: {}", e);
            ExitCode::from(1)
        }
    }
}
