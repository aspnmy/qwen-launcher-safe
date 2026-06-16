//! Windows 进程工具模块
//!
//! 提供跨平台（Windows 为主）的进程管理功能：
//!
//! - **qwen 命令发现**：仅从配置文件 `config/config.json` 的 `qwenPath` 字段读取
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

/// 查找 qwen 命令
///
/// 仅从配置文件读取 `qwenPath` 字段，不进行任何自动搜索。
/// 若配置未设置或路径不存在，返回 [`io::ErrorKind::NotFound`]。
pub fn find_qwen_command() -> io::Result<PathBuf> {
    let cfg = config::read_config();

    if let Some(ref path_str) = cfg.qwen_path {
        let path = PathBuf::from(path_str);
        if path.exists() {
            log::info!("qwen 路径: {:?}（来自配置文件）", path);
            return Ok(path);
        }
        log::warn!("配置文件 qwenPath {:?} 不存在", path);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "qwenPath 未配置。请运行 `qwen-launcher-safe init-config` 进行配置",
    ))
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
///
/// `cwd` 为可选工作目录，设置后子进程在此目录下运行，
/// 确保能找到该目录下的 `.qwen/skills/` 等配置。
pub fn spawn_qwen(cmd: &PathBuf, args: &[String], cwd: Option<&std::path::Path>) -> io::Result<Child> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    command.spawn()
}

/// 返回当前可执行文件路径（用于自调用生成 monitor 子进程）
pub fn self_exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}
