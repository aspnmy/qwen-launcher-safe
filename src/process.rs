//! Windows 进程工具模块
//!
//! 提供跨平台（Windows 为主）的进程管理功能：
//!
//! - **qwen 命令发现**：PATH → 常见安装位置 → 节点模块 → 配置文件兜底
//! - **Qwen 进程检测**：通过命令行关键字匹配识别 Qwen 相关 node 进程
//! - **CPU 核绑定**：Windows 专属 API，将进程绑定到指定物理核心
//! - **进程启动**：继承当前控制台启动子进程
//!
//! # 平台兼容性
//!
//! `bind_cpu_core` 和 `get_physical_core_count` 使用 `#[cfg(windows)]` 条件编译，
//! 非 Windows 平台返回空操作或默认值 1。

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use regex::Regex;
#[cfg(windows)]
use windows_sys::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION,
};

use crate::config;

/// 搜索 qwen 命令的候选名称列表
const SEARCH_CANDIDATES: &[&str] = &[
    "qwen.cmd", // npm global bin
    "qwen.exe", // standalone exe
    "qwen",     // Unix / PATH without .exe
];

/// 查找 qwen 命令
///
/// 搜索优先级：
/// 1. 配置文件 `<exe 同级>/config/config.json` 中的 `qwenPath`（用户显式配置优先）
/// 2. 常见全局安装位置（npm / localappdata 等）
/// 3. `PATH` 环境变量（过滤临时包装器）
/// 4. 当前目录向上遍历 `node_modules/.bin/`
///
/// 全部失败时返回 [`io::ErrorKind::NotFound`]。
pub fn find_qwen_command() -> io::Result<PathBuf> {
    let cfg = config::read_config();

    // ── 链路 A：配置文件优先（用户显式配置） ──
    if let Some(ref path_str) = cfg.qwen_path {
        let path = PathBuf::from(path_str);
        if path.exists() {
            log::info!("配置文件指定 qwen: {:?}", path);
            return Ok(path);
        }
        log::warn!("配置文件 qwenPath {:?} 不存在，回退到自动搜索", path);
    }

    // ── 链路 B：自动搜索（通用降级） ──
    let auto = auto_search();
    if let Some(path) = auto {
        log::info!("自动搜索到 qwen: {:?}", path);
        return Ok(path);
    }

    // ── 全部失败 ──
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "找不到 qwen 命令：配置文件未指定且自动搜索无结果。\n  \
         请先运行 `qwen-launcher-safe init-config --qwen-path <路径>`",
    ))
}

/// 判断路径是否为临时/转发的包装器（fnm、volta、nvm 等）
fn is_transient_wrapper(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy().to_lowercase();
    s.contains("fnm_multishells")
        || s.contains("fnm\\multishells")
        || s.contains("volta")
        || s.contains("\\.nvm\\")
        || s.contains("_nvm\\")
}

/// 自动搜索 qwen（通用方法：PATH → node_modules/.bin）
fn auto_search() -> Option<PathBuf> {
    // 1. PATH 搜索（过滤掉 fnm/volta/nvm 等临时包装器路径）
    for name in SEARCH_CANDIDATES {
        if let Ok(path) = which(name) {
            if !is_transient_wrapper(&path) {
                return Some(path);
            }
        }
    }

    // 2. 从当前目录向上找 node_modules/.bin/
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = Some(cwd.as_path());
        while let Some(d) = dir {
            let bin_dir = d.join("node_modules").join(".bin");
            for name in SEARCH_CANDIDATES {
                let candidate = bin_dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            dir = d.parent();
        }
    }

    None
}

/// 在 `PATH` 中查找可执行文件（简化版 which）
fn which(name: &str) -> io::Result<PathBuf> {
    let path_vals = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_vals) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "不在 PATH 中"))
}

/// 检测是否为 Qwen 相关进程
///
/// 通过匹配命令行中是否包含 `qwen`、`coder-agent` 或 `cli-agent` 关键字
/// （大小写不敏感）来识别 Qwen 相关的 node 进程。
fn is_qwen_process(cmdline: &[String]) -> bool {
    let joined = cmdline.join(" ");
    if joined.is_empty() {
        return false;
    }
    let re = Regex::new(r"(?i)qwen|coder-agent|cli-agent").unwrap();
    re.is_match(&joined)
}

/// 返回当前所有 Qwen 相关进程的 PID 集合
///
/// 遍历系统进程表，对每个进程调用 [`is_qwen_process`] 检测命令行，
/// 收集匹配的进程 ID。
pub fn get_qwen_pids(sys: &sysinfo::System) -> HashSet<u32> {
    let mut pids = HashSet::new();
    for process in sys.processes().values() {
        let cmd = process.cmd();
        if is_qwen_process(cmd) {
            pids.insert(process.pid().as_u32());
        }
    }
    pids
}

/// 获取物理 CPU 核心数
///
/// Windows 下使用 [`GetSystemInfo`] API 获取处理器数量。
#[cfg(windows)]
pub fn get_physical_core_count() -> u32 {
    unsafe {
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors
    }
}

/// 非 Windows 平台占位实现，返回 1
#[cfg(not(windows))]
pub fn get_physical_core_count() -> u32 {
    1
}

/// 将指定进程绑定到指定 CPU 核
///
/// 使用 [`SetProcessAffinityMask`] API 设置进程的 CPU 亲和性掩码。
/// 掩码为 `1 << core_index`，即第 `core_index` 位为 1。
///
/// # 平台限制
///
/// 仅 Windows 平台有效。非 Windows 平台返回 `Ok(())`。
#[cfg(windows)]
pub fn bind_cpu_core(pid: u32, core_index: u32) -> io::Result<()> {
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mask: usize = 1usize << core_index;
        let ret = SetProcessAffinityMask(handle, mask);
        CloseHandle(handle);
        if ret == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// 非 Windows 平台占位实现（空操作）
#[cfg(not(windows))]
pub fn bind_cpu_core(_pid: u32, _core_index: u32) -> io::Result<()> {
    Ok(())
}

/// 启动 Qwen 进程，继承当前控制台的 stdin/stdout/stderr
pub fn spawn_qwen(cmd: &PathBuf, args: &[String]) -> io::Result<Child> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

/// 返回当前可执行文件路径（用于自调用生成 monitor 子进程）
pub fn self_exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}
