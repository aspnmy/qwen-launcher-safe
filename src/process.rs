/// Windows 进程工具 — 进程发现、命令查找、CPU 核绑定
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

/// 搜索 qwen 命令的搜索路径（共 9 个备选）
const SEARCH_CANDIDATES: &[&str] = &[
    "qwen.cmd", // npm global bin
    "qwen.exe", // standalone exe
    "qwen",     // Unix / PATH without .exe
];

/// 查找 qwen 命令 — 先自动搜索，再读 .config 配置
///
/// 搜索顺序：
///   1. PATH 环境变量（找 qwen.cmd / qwen.exe / qwen）
///   2. 常见全局安装位置（npm / localappdata / homebrew 等）
///   3. 当前目录及其父目录的 node_modules/.bin/
///   4. 配置文件中手动指定的路径 `~\.qwen-launcher\config.json`
pub fn find_qwen_command() -> io::Result<PathBuf> {
    // ── 链路 A：自动搜索 ──
    let auto = auto_search();
    if let Some(path) = auto {
        log::info!("自动搜索到 qwen: {:?}", path);
        return Ok(path);
    }

    // ── 链路 B：读取配置文件 ──
    let cfg = config::read_config();
    if let Some(ref path_str) = cfg.qwen_path {
        let path = PathBuf::from(path_str);
        if path.exists() {
            log::info!("配置文件指定 qwen: {:?}", path);
            return Ok(path);
        }
        log::warn!("配置文件指定路径 {:?} 不存在，将在创建后写入", path);
        // 配置指定但文件不存在 → 跳过，后续报错
    }

    // ── 全部失败 ──
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "找不到 qwen 命令：自动搜索无结果，且 .config 文件未配置或路径无效。\n  \
         请先运行 `qwen-launcher-safe init-config --qwen-path <路径>`",
    ))
}

/// 自动搜索 qwen（PATH → 常见位置 → node_modules/.bin）
fn auto_search() -> Option<PathBuf> {
    // 1. PATH 搜索（尝试所有候选名）
    for name in SEARCH_CANDIDATES {
        if let Ok(path) = which(name) {
            return Some(path);
        }
    }

    // 2. 常见全局安装位置
    let common_dirs = common_search_dirs();
    for dir in &common_dirs {
        for name in SEARCH_CANDIDATES {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 3. 从当前目录向上找 node_modules/.bin/
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

/// 常见全局安装目录列表
fn common_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // %APPDATA%\npm （npm 全局 bin）
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    // %LOCALAPPDATA%\qwen
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(localappdata).join("qwen").join("bin"));
    }
    // ~\.cherrystudio\bin （原版 fallback）
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        dirs.push(PathBuf::from(home).join(".cherrystudio").join("bin"));
    }
    // %ProgramFiles%\qwen
    if let Ok(pf) = std::env::var("ProgramFiles") {
        dirs.push(PathBuf::from(pf).join("qwen").join("bin"));
    }
    if let Ok(pfx86) = std::env::var("ProgramFiles(x86)") {
        dirs.push(PathBuf::from(pfx86).join("qwen").join("bin"));
    }

    dirs
}

/// 查找 PATH 中的可执行文件（简化版 which）
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

/// 检测是否为 Qwen 相关进程（匹配命令行）
fn is_qwen_process(cmdline: &[String]) -> bool {
    let joined = cmdline.join(" ");
    if joined.is_empty() {
        return false;
    }
    // 匹配 qwen|coder-agent|cli-agent
    let re = Regex::new(r"(?i)qwen|coder-agent|cli-agent").unwrap();
    re.is_match(&joined)
}

/// 获取当前所有 Qwen 相关进程的 PID 集合
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
#[cfg(windows)]
pub fn get_physical_core_count() -> u32 {
    unsafe {
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors
    }
}

/// 没有 cfg(windows) 时的占位
#[cfg(not(windows))]
pub fn get_physical_core_count() -> u32 {
    1
}

/// 绑定进程到指定 CPU 核
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

/// 非 Windows 占位
#[cfg(not(windows))]
pub fn bind_cpu_core(_pid: u32, _core_index: u32) -> io::Result<()> {
    Ok(())
}

/// 启动 Qwen 进程（继承当前控制台）
pub fn spawn_qwen(cmd: &PathBuf, args: &[String]) -> io::Result<Child> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

/// 查找当前可执行文件路径（用于自调用 spawn monitor）
pub fn self_exe_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}
