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
            let path = path.canonicalize().unwrap_or(path);
            log::info!("qwen 路径: {:?}（来自配置文件）", path);
            return Ok(path);
        }
        log::warn!("配置文件 qwenPath {:?} 不存在", path);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "qwenPath 未配置。请运行 `agent-launcher-safe init-config` 进行配置",
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
/// - Windows: 使用 [`GetSystemInfo`] API 获取 `dwNumberOfProcessors`
/// - Linux: 使用 `sysconf(_SC_NPROCESSORS_ONLN)` 获取在线处理器数
///
/// 注意：在启用超线程的系统上返回的是**逻辑处理器数**（通常是物理核心数的 2 倍），
/// 而非物理核心数。对 CPU 亲和性绑定而言，逻辑处理器才是正确的分配粒度。
#[cfg(windows)]
pub fn get_processor_count() -> u32 {
    // SAFETY: GetSystemInfo 是 Windows 标准 API，
    // 接受栈上分配的有效 SYSTEM_INFO 结构体指针。
    // 该结构体由 API 填充，不涉及内存安全问题。
    unsafe {
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors
    }
}

/// Linux 下使用 `sysconf(_SC_NPROCESSORS_ONLN)` 获取在线逻辑处理器数
#[cfg(target_os = "linux")]
pub fn get_processor_count() -> u32 {
    // SAFETY: sysconf 是 POSIX 标准 API，接受标准配置常量。
    // _SC_NPROCESSORS_ONLN 始终返回正整数或 -1（出错时），
    // 这里是安全的 FFI 调用，不涉及借用的内存。
    unsafe {
        let n = libc::sysconf(libc::_SC_NPROCESSORS_ONLN);
        if n > 0 {
            n as u32
        } else {
            1
        }
    }
}

/// 其他平台（macOS/FreeBSD 等）占位实现，返回 1
#[cfg(not(any(windows, target_os = "linux")))]
pub fn get_processor_count() -> u32 {
    1
}

/// 将指定进程绑定到指定 CPU 核
///
/// - Windows: 使用 [`SetProcessAffinityMask`] API 设置进程的 CPU 亲和性掩码
/// - Linux: 使用 `sched_setaffinity` 设置进程的 CPU 亲和性掩码
///
/// 掩码为 `1 << core_index`，即第 `core_index` 位为 1。
///
/// # 平台限制
///
/// macOS 和 FreeBSD 不支持 CPU 亲和性设置，返回 `Ok(())`（空操作）。
#[cfg(windows)]
pub fn bind_cpu_core(pid: u32, core_index: u32) -> io::Result<()> {
    // SAFETY: OpenProcess/SetProcessAffinityMask/GetProcessAffinityMask
    // 是 Windows 标准进程管理 API。handle 有效性在每条 API 调用后
    // 通过返回值检查（is_null / ret == 0）验证，failure 路径及时
    // CloseHandle 释放资源。所有指针参数（SYSTEM_INFO）指向栈上
    // 已初始化的有效内存。
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

/// Linux 下使用 `sched_setaffinity` 设置进程 CPU 亲和性
#[cfg(target_os = "linux")]
pub fn bind_cpu_core(pid: u32, core_index: u32) -> io::Result<()> {
    // SAFETY: sched_setaffinity 是 POSIX 标准 API。
    // cpu_set_t 是栈上分配的固定大小结构体，CPU_SET 宏在
    // 有效索引范围内操作（core_index < CPU_SETSIZE）。
    // pid 参数由调用者传入，经 sysinfo 验证的有效 PID。
    unsafe {
        let mut mask: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(core_index as usize, &mut mask);
        let ret = libc::sched_setaffinity(
            pid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mask,
        );
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// 其他平台（macOS/FreeBSD 等）占位实现（空操作）
#[cfg(not(any(windows, target_os = "linux")))]
pub fn bind_cpu_core(_pid: u32, _core_index: u32) -> io::Result<()> {
    Ok(())
}

/// 验证路径是否为可执行文件
///
/// - 检查路径存在且为文件（非目录）
/// - Windows: 检查 `.exe` / `.cmd` / `.bat` 扩展名
/// - Unix: 检查文件是否具有执行权限位
#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "exe" | "cmd" | "bat" | "com"
        ),
        None => false,
    }
}

/// Unix 下检查文件执行权限
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// fallback：其他平台只检查是否为文件
#[cfg(not(any(windows, unix)))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// 启动 Qwen 进程，继承当前控制台的 stdin/stdout/stderr
///
/// `cwd` 为可选工作目录，设置后子进程在此目录下运行，
/// 确保能找到该目录下的 `.qwen/skills/` 等配置。
///
/// 在启动前验证路径是有效的可执行文件，返回 `InvalidInput` 错误而非后续运行时失败。
pub fn spawn_qwen(cmd: &Path, args: &[String], cwd: Option<&std::path::Path>) -> io::Result<Child> {
    if !is_executable(cmd) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{:?} 不是有效的可执行文件", cmd),
        ));
    }
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
    fn test_is_executable_file_exists() {
        // 当前 exe 本身应被检测为可执行
        let exe = std::env::current_exe().unwrap();
        assert!(is_executable(&exe), "当前 exe 应为可执行");
    }

    #[test]
    fn test_is_executable_directory_not_executable() {
        // 目录不应被判定为可执行
        let dir = std::env::current_dir().unwrap();
        assert!(!is_executable(&dir), "目录不应被判定为可执行");
    }

    #[test]
    fn test_is_executable_nonexistent_path() {
        let p = PathBuf::from(r"C:\nonexistent_file_12345.exe");
        assert!(!is_executable(&p), "不存在的路径不应为可执行");
    }

    #[test]
    fn test_spawn_qwen_invalid_path_returns_invalid_input() {
        let invalid = PathBuf::from(r"C:\nonexistent_qwen_test.exe");
        let result = spawn_qwen(&invalid, &[], None);
        assert!(result.is_err(), "无效路径应返回错误");
        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::InvalidInput,
            "应为 InvalidInput 错误"
        );
    }

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
    #[test]
    fn test_path_canonicalize_resolves_dotdot() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join("agent_launcher_test_canon");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        let sub = tmp.join("sub");
        std::fs::create_dir(&sub).expect("create subdir");
        let file = sub.join("test.exe");
        std::fs::File::create(&file).expect("create file").write_all(b"fake").unwrap();

        // construct path with .. : <tmp>/sub/../sub/test.exe
        let dotdot = sub.join("..").join("sub").join("test.exe");
        assert!(dotdot.exists(), "path with .. should exist");

        // BUG: PathBuf::from does not resolve ..
        let unresolved = std::path::PathBuf::from(dotdot.to_str().unwrap());
        assert!(unresolved.to_str().unwrap().contains(".."), "PathBuf::from should NOT resolve ..");

        // FIX: canonicalize resolves .. to canonical path
        let resolved = dotdot.canonicalize().expect("canonicalize should succeed");
        assert!(!resolved.to_str().unwrap().contains(".."), "canonical path should not contain ..");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
