//! `server` 子命令：启动、停止、重启、查看状态。
//!
//! 通过 PID 文件（`hello-axum.pid`）管理服务器进程生命周期。

use std::path::PathBuf;

use rust_blog::config::app::AppConfig;

use rust_blog::server as srv;

fn pid_file_path() -> PathBuf {
    PathBuf::from("./hello-axum.pid")
}

fn write_pid(pid: u32) -> anyhow::Result<()> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, pid.to_string())?;
    Ok(())
}

pub fn read_pid() -> Option<u32> {
    let path = pid_file_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

pub fn remove_pid() {
    let _ = std::fs::remove_file(pid_file_path());
}

fn is_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(nix::errno::Errno::ESRCH) => false,
            Err(_) => true,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn send_terminate(pid: u32) -> bool {
    #[cfg(unix)]
    {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGTERM,
        )
        .is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn send_kill(pid: u32) -> bool {
    #[cfg(unix)]
    {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

// ── 子命令实现 ───────────────────────────────────────────────────

/// `server start` — 启动 HTTP 服务器。
///
/// 写入 PID 文件并检查是否已有实例在运行。
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    let pid = std::process::id();
    write_pid(pid)?;

    if let Some(old_pid) = read_pid()
        && old_pid != pid
        && is_process_running(old_pid)
    {
        anyhow::bail!(
            "server already running (pid={}). Stop it first: hello-axum server stop",
            old_pid
        );
    }

    srv::start(config).await?;
    remove_pid();
    Ok(())
}

/// `server stop` — 停止运行中的服务器。
///
/// 发送 SIGTERM，等待 3 秒后若仍未退出则 SIGKILL。
pub fn stop() {
    match read_pid() {
        Some(pid) if is_process_running(pid) => {
            if send_terminate(pid) {
                println!("sent SIGTERM to process {}", pid);
                for _ in 0..30 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if !is_process_running(pid) {
                        remove_pid();
                        println!("server stopped");
                        return;
                    }
                }
                println!("server did not stop within 3 seconds, killing...");
                send_kill(pid);
                remove_pid();
                println!("server killed");
            } else {
                println!("failed to send signal to process {}", pid);
            }
        }
        Some(pid) => {
            println!("server is not running (stale pid={})", pid);
            remove_pid();
        }
        None => {
            println!("server is not running (no pid file)");
        }
    }
}

/// `server restart` — 停止旧实例后启动新实例。
pub async fn restart(config: &AppConfig) -> anyhow::Result<()> {
    stop();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(config).await
}

/// `server status` — 查看服务器运行状态。
pub fn status() {
    match read_pid() {
        Some(pid) if is_process_running(pid) => {
            let config = AppConfig::init();
            println!(
                "server is running (pid={}, listening on {}:{})",
                pid, config.host, config.port
            );
        }
        Some(pid) => {
            println!("server is not running (stale pid={})", pid);
            remove_pid();
        }
        None => {
            println!("server is not running (no pid file)");
        }
    }
}
