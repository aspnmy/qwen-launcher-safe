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
        /// Qwen 额外参数（透传给 qwen 命令）
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
    InitConfig {
        /// qwen 可执行文件路径
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli {
        Cli::Launch { qwen_args } => launcher::run(&qwen_args),
        Cli::Monitor { interval } => monitor::run(Some(interval)),
        Cli::InitConfig {
            qwen_path,
            max_memory_mb,
            monitor_interval,
            show,
        } => cmd_init_config(qwen_path, max_memory_mb, monitor_interval, show),
    }
}

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
        eprintln!("未指定任何配置项。使用 --help 查看选项。");
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
