//! `server` subcommand: start, stop, restart, view status.
//!
//! Manages server process lifecycle via PID file (`{STORAGE_ROOT_DIR}/raisfast.pid`).

use std::path::PathBuf;

use raisfast::config::app::AppConfig;

use raisfast::server as srv;

fn pid_file_path(storage_root: &str) -> PathBuf {
    PathBuf::from(format!("{storage_root}/raisfast.pid"))
}

fn write_pid(storage_root: &str, pid: u32) -> anyhow::Result<()> {
    let path = pid_file_path(storage_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, pid.to_string())?;
    Ok(())
}

pub fn read_pid(storage_root: &str) -> Option<u32> {
    let path = pid_file_path(storage_root);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

pub fn remove_pid(storage_root: &str) {
    let _ = std::fs::remove_file(pid_file_path(storage_root));
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

// ── Subcommand implementations ───────────────────────────────────

/// `server start` — start the HTTP server.
///
/// Writes PID file and checks if another instance is already running.
pub async fn start(config: &AppConfig) -> anyhow::Result<()> {
    let pid = std::process::id();
    let storage_root = &config.storage_root_dir;
    write_pid(storage_root, pid)?;

    if let Some(old_pid) = read_pid(storage_root)
        && old_pid != pid
        && is_process_running(old_pid)
    {
        anyhow::bail!(
            "server already running (pid={}). Stop it first: raisfast server stop",
            old_pid
        );
    }

    srv::start(config).await?;
    remove_pid(storage_root);
    Ok(())
}

/// `server stop` — stop the running server.
///
/// Sends SIGTERM, waits 3 seconds, then sends SIGKILL if still running.
pub fn stop() {
    let storage_root = AppConfig::init().storage_root_dir;
    match read_pid(&storage_root) {
        Some(pid) if is_process_running(pid) => {
            if send_terminate(pid) {
                println!("sent SIGTERM to process {}", pid);
                for _ in 0..30 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if !is_process_running(pid) {
                        remove_pid(&storage_root);
                        println!("server stopped");
                        return;
                    }
                }
                println!("server did not stop within 3 seconds, killing...");
                send_kill(pid);
                remove_pid(&storage_root);
                println!("server killed");
            } else {
                println!("failed to send signal to process {}", pid);
            }
        }
        Some(pid) => {
            println!("server is not running (stale pid={})", pid);
            remove_pid(&storage_root);
        }
        None => {
            println!("server is not running (no pid file)");
        }
    }
}

/// `server restart` — stop the old instance then start a new one.
pub async fn restart(config: &AppConfig) -> anyhow::Result<()> {
    stop();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(config).await
}

/// `server status` — view server running status.
pub fn status() {
    let storage_root = AppConfig::init().storage_root_dir;
    match read_pid(&storage_root) {
        Some(pid) if is_process_running(pid) => {
            let config = AppConfig::init();
            println!(
                "server is running (pid={}, listening on {}:{})",
                pid, config.host, config.port
            );
        }
        Some(pid) => {
            println!("server is not running (stale pid={})", pid);
            remove_pid(&storage_root);
        }
        None => {
            println!("server is not running (no pid file)");
        }
    }
}
