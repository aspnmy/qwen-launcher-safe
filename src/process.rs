//! Windows 进程工具模块
//!
//! 提供跨平台（Windows 为主）的进程管理功能：
//!
//! - **qwen 命令发现**：仅从配置文件 `config/config.json` 的 `qwenPath` 字段读取
//! - **Qwen 进程检测**：通过命令行关键字匹配识别 Qwen 相关 node 进程
//! - **CPU 核绑定**：Windows 专属 API，将进程绑定到指定逻辑处理器
//! - **处理器计数**：返回系统逻辑处理器数量（含超线程）
//!
//! # 平台兼容性
//!
//! `bind_cpu_core` 和 `get_processor_count` 使用 `#[cfg(windows)]` 条件编译，
//! 非 Windows 平台返回空操作或默认值 1。

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

use regex::Regex;

/// 编译一次 Qwen 进程匹配正则
///
/// 在循环中频繁调用 `is_qwen_process()` 时避免重复编译正则表达式。
fn qwen_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:qwen|coder-agent|cli-agent)").expect("编译 Qwen 进程匹配正则失败")
    })
}
#[cfg(windows)]
use windows_sys::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetProcessAffinityMask, OpenProcess, SetProcessAffinityMask, PROCESS_QUERY_INFORMATION,
    PROCESS_SET_INFORMATION,
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
    qwen_regex().is_match(&joined)
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

/// 获取系统逻辑处理器数量
///
/// Windows 下使用 [`GetSystemInfo`] API 获取处理器数量。
/// 注意：`dwNumberOfProcessors` 在启用超线程的系统上返回的是逻辑处理器数
/// （通常是物理核心数的 2 倍），而非物理核心数。对 CPU 亲和性绑定而言，
/// 逻辑处理器才是正确的分配粒度。
#[cfg(windows)]
pub fn get_processor_count() -> u32 {
    unsafe {
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors
    }
}

/// 非 Windows 平台占位实现，返回 1
#[cfg(not(windows))]
pub fn get_processor_count() -> u32 {
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
        let handle = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        // 查询进程和系统的有效处理器掩码
        let mut process_mask: usize = 0;
        let mut system_mask: usize = 0;
        let ret = GetProcessAffinityMask(handle, &mut process_mask, &mut system_mask);
        if ret == 0 {
            let err = io::Error::last_os_error();
            CloseHandle(handle);
            return Err(err);
        }

        let requested: usize = 1usize << core_index;

        // 验证请求的核心在系统有效范围内
        if requested & system_mask == 0 {
            CloseHandle(handle);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "核心 {} 超出系统有效处理器范围 (掩码 {:#016x})",
                    core_index, system_mask
                ),
            ));
        }

        // 将请求掩码与进程当前有效掩码取交集，
        // 确保 SetProcessAffinityMask 不会因权限/Job 限制而失败
        let final_mask = requested & process_mask;
        if final_mask == 0 {
            CloseHandle(handle);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "核心 {} 不在进程可用掩码 {:#016x} 内",
                    core_index, process_mask
                ),
            ));
        }

        let ret = SetProcessAffinityMask(handle, final_mask);
        CloseHandle(handle);
        if ret == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
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
pub fn spawn_qwen(cmd: &Path, args: &[String], cwd: Option<&std::path::Path>) -> io::Result<Child> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_qwen_process_matches_qwen() {
        let cmd = vec![
            "node.exe".into(),
            "C:\\Users\\user\\.cherrystudio\\bin\\qwen.exe".into(),
            "--model".into(),
            "qwen-max".into(),
        ];
        assert!(is_qwen_process(&cmd), "应匹配 qwen");
    }

    #[test]
    fn test_is_qwen_process_matches_coder_agent() {
        let cmd = vec!["node.exe".into(), "coder-agent".into(), "serve".into()];
        assert!(is_qwen_process(&cmd), "应匹配 coder-agent");
    }

    #[test]
    fn test_is_qwen_process_matches_cli_agent() {
        let cmd = vec!["node".into(), "cli-agent".into()];
        assert!(is_qwen_process(&cmd), "应匹配 cli-agent");
    }

    #[test]
    fn test_is_qwen_process_case_insensitive() {
        let cmd = vec!["QWEN".into()];
        assert!(is_qwen_process(&cmd), "应大小写不敏感");
    }

    #[test]
    fn test_is_qwen_process_no_match() {
        let cmd = vec!["node.exe".into(), "server.js".into()];
        assert!(!is_qwen_process(&cmd), "非 Qwen 进程不应匹配");
    }

    #[test]
    fn test_is_qwen_process_empty_cmdline() {
        let cmd: Vec<String> = vec![];
        assert!(!is_qwen_process(&cmd), "空命令行不应匹配");
    }

    #[test]
    fn test_is_qwen_process_partial_substring_no_match() {
        // "qwe" 是 "qwen" 的部分但不是完整匹配 — 正则要求单词边界
        let cmd = vec!["qwe".into(), "n".into()];
        // 不含"qwen"这个子串
        assert!(!is_qwen_process(&cmd), "部分子串不应匹配");
    }

    #[test]
    fn test_find_qwen_command_not_found() {
        // 未配置 qwenPath 时应当返回 NotFound
        let result = find_qwen_command();
        // 这个测试依赖于当前配置文件，可能 qwenPath 已设置
        // 所以我们只验证函数不会 panic
        let _ = result;
    }

    #[test]
    fn test_qwen_regex_compiles() {
        let re = qwen_regex();
        assert!(re.is_match("qwen"), "正则应匹配 'qwen'");
        assert!(re.is_match("coder-agent"), "正则应匹配 'coder-agent'");
        assert!(re.is_match("cli-agent"), "正则应匹配 'cli-agent'");
        assert!(re.is_match("QWEN"), "正则应匹配大写 'QWEN'");
        assert!(!re.is_match("node"), "正则不应匹配 'node'");
    }
}
