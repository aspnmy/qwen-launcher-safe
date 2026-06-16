//! Qwen Code 资源保护启动器 — CLI 入口和子命令分发
//!
//! 提供三个子命令：
//! - `launch` — 启动 Qwen 并自动注册资源监控
//! - `monitor` — 后台资源监控循环
//! - `init-config` — 初始化或更新配置文件
//!
//! 双击 `.exe` 无参数时默认进入 `launch` 模式。
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
//! # 首次使用：交互式配置向导
//! qwen-launcher-safe init
//!
//! # 配置 qwen 路径
//! qwen-launcher-safe init-config --qwen-path "C:\path\to\qwen.exe"
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

/// 全角字符 → 半角字符转换
///
/// 覆盖：字母、数字、符号（引号、逗号、空格等）。
/// 用户输入法常输出全角字符，统一归一化避免路径匹配失败。
fn normalize_input(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // 全角字母数字符号 U+FF01..U+FF5E → 半角 U+21..U+7E（偏移 0xFEE0）
            '\u{FF01}'..='\u{FF5E}' => {
                out.push(char::from_u32(c as u32 - 0xFEE0).unwrap_or(c));
            }
            // 全角引号（双引号 + 单引号）
            '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' | '\u{300C}' | '\u{300D}' => {
                out.push('"')
            }
            // 全角空格
            '\u{3000}' => out.push(' '),
            // 保留原字符
            _ => out.push(c),
        }
    }
    out
}

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
        /// qwen 可执行文件完整路径
        #[arg(long)]
        qwen_path: Option<String>,
        /// 最大内存限制（MB）
        #[arg(long)]
        max_memory_mb: Option<u64>,
        /// 监控轮询间隔（秒）
        #[arg(long)]
        monitor_interval: Option<u64>,
        /// Qwen 工作目录（使子进程加载该目录下的 .qwen/skills/ 技能）
        #[arg(long)]
        working_dir: Option<String>,
        /// 显示当前配置
        #[arg(long)]
        show: bool,
    },
}

/// 交互式配置向导
///
/// 当配置文件不存在或 qwenPath 未设置时自动进入。
/// 引导用户配置 qwen 路径、内存限制和监控间隔。
/// 不进行任何自动搜索——完全由用户输入决定。
fn interactive_setup() -> ExitCode {
    println!("+----------------------------------------------+");
    println!("|  qwenPath 未配置，进入交互式配置向导          |");
    println!("|  （直接回车使用默认值，留空则跳过）           |");
    println!("+----------------------------------------------+");
    println!();

    // ── 步骤 1：Qwen 路径 ──
    println!("[步骤 1/4] Qwen 可执行文件路径");
    let line = read_line_normalized();
    let qwen_path = if line.is_empty() {
        println!("  [跳过] qwenPath 未设置，将使用系统 PATH 查找");
        None
    } else {
        let path = std::path::Path::new(&line);
        if path.exists() {
            Some(line)
        } else {
            eprintln!("  [警告] 路径不存在，将跳过 qwenPath 配置");
            None
        }
    };

    // ── 步骤 2：内存限制 ──
    println!();
    println!("[步骤 2/4] 最大内存限制 (MB) [默认 1024]");
    let line = read_line_raw();
    let max_memory_mb = if line.is_empty() {
        1024
    } else {
        match line.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("  [提示] 输入无效，使用默认值 1024 MB");
                1024
            }
        }
    };

    // ── 步骤 3：监控间隔 ──
    println!();
    println!("[步骤 3/4] 监控轮询间隔 (秒) [默认 10]");
    let line = read_line_raw();
    let monitor_interval = if line.is_empty() {
        10
    } else {
        match line.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("  [提示] 输入无效，使用默认值 10 秒");
                10
            }
        }
    };

    // ── 步骤 4：工作目录（可选） ──
    println!();
    println!("[步骤 4/4] Qwen 工作目录（使子进程加载该目录下的 .qwen/skills/ 技能）");
    let line = read_line_normalized();
    let working_dir = if line.is_empty() {
        None
    } else {
        let path = std::path::Path::new(&line);
        if path.is_dir() {
            Some(line)
        } else {
            eprintln!("  [警告] 目录不存在，将跳过 workingDir 配置");
            None
        }
    };

    // ── 写入配置 ──
    let mut cfg = config::LauncherConfig {
        qwen_path: None,
        max_memory_mb,
        monitor_interval_sec: monitor_interval,
        working_dir: None,
    };
    if let Some(ref path) = qwen_path {
        cfg.qwen_path = Some(path.clone());
    }
    if let Some(ref wd) = working_dir {
        cfg.working_dir = Some(wd.clone());
    }

    println!();
    match config::write_config(&cfg) {
        Ok(()) => {
            println!("[OK] 配置已完成，已写入: {:?}", config::config_file_path());
            match serde_json::to_string_pretty(&cfg) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("[WARN] 序列化配置失败: {}", e),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[ERR] 写入配置失败: {}", e);
            ExitCode::from(1)
        }
    }
}

/// 读取一行原始输入（无归一化处理，仅 trim）
fn read_line_raw() -> String {
    use std::io::{stdin, stdout, Write};
    print!("  请输入: ");
    stdout().flush().ok();
    let mut line = String::new();
    stdin().read_line(&mut line).ok();
    line.trim().to_string()
}

/// 读取一行输入并做全角→半角归一化、去除引号和空白
fn read_line_normalized() -> String {
    let raw = read_line_raw();
    normalize_input(&raw).trim_matches('"').trim().to_string()
}

/// 程序入口
///
/// 解析 CLI 参数后分发到对应子命令处理函数。
/// 双击 `.exe` 无参数时：`try_parse` 失败（MissingSubcommand）→ 默认走 `launch`。
fn main() -> ExitCode {
    match Cli::try_parse() {
        Ok(Cli::Launch { qwen_args }) => {
            // 启动前检查 qwenPath 是否已配置
            if process::find_qwen_command().is_err() {
                eprintln!("qwenPath 未配置，进入交互式配置向导...");
                if interactive_setup() != ExitCode::SUCCESS {
                    return ExitCode::from(1);
                }
            }
            launcher::run(&qwen_args)
        }
        Ok(Cli::Monitor { interval }) => monitor::run(Some(interval)),
        Ok(Cli::InitConfig {
            qwen_path,
            max_memory_mb,
            monitor_interval,
            working_dir,
            show,
        }) => cmd_init_config(
            qwen_path,
            max_memory_mb,
            monitor_interval,
            working_dir,
            show,
        ),
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
/// - `--qwen-path <路径>`：手动指定 qwen 路径
/// - `--max-memory-mb <MB>`：调整内存限制
/// - `--monitor-interval <秒>`：调整轮询间隔
/// - 无参数且配置文件不存在或 qwenPath 为空：进入交互式配置向导
fn cmd_init_config(
    qwen_path: Option<String>,
    max_memory_mb: Option<u64>,
    monitor_interval: Option<u64>,
    working_dir: Option<String>,
    show: bool,
) -> ExitCode {
    if show {
        let cfg = config::read_config();
        match serde_json::to_string_pretty(&cfg) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("[WARN] 序列化配置失败: {}", e),
        }
        return ExitCode::SUCCESS;
    }

    let mut cfg = config::read_config();
    let mut changed = false;

    if let Some(p) = qwen_path {
        let p = normalize_input(&p).trim_matches('"').to_string();
        let path = std::path::Path::new(&p);
        if !path.exists() {
            eprintln!("路径不存在: {}", p);
            return ExitCode::from(1);
        }
        cfg.qwen_path = Some(p);
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
    if let Some(w) = working_dir {
        let w = normalize_input(&w).trim_matches('"').to_string();
        let path = std::path::Path::new(&w);
        if !path.is_dir() {
            eprintln!("目录不存在: {}", w);
            return ExitCode::from(1);
        }
        cfg.working_dir = Some(w);
        changed = true;
    }

    if !changed {
        // 配置文件不存在或 qwenPath 为空 → 进入交互式向导
        if !config::config_file_path().exists() || cfg.qwen_path.is_none() {
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
            match serde_json::to_string_pretty(&cfg) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("[WARN] 序列化配置失败: {}", e),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写入配置失败: {}", e);
            ExitCode::from(1)
        }
    }
}
