use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::core::paths::Paths;
use crate::core::state::{self, Mode, Record};
use crate::core::types::LaunchOutcome;
use crate::{Error, Result};

use super::process::{self, SpawnSpec};

/// A display backend knows how to start and describe the X server that hosts
/// a runspace. Keeping this behind a trait makes a future Wayland, macOS or
/// Windows backend additive rather than a rewrite of the lifecycle code.
pub trait DisplayBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn program(&self) -> &'static str;
    fn args(&self, display: u16, width: u16, height: u16, title: &str) -> Vec<String>;
    fn requires_host_display(&self) -> bool;
    /// Whether this backend's own DISPLAY variable should point at the nested
    /// server. Xvfb ignores it; Xephyr must keep the host DISPLAY so it knows
    /// where to draw its window.
    fn runs_on_nested_display(&self) -> bool;
}

pub struct Xvfb;
pub struct Xephyr;

static XVFB: Xvfb = Xvfb;
static XEPHYR: Xephyr = Xephyr;

impl DisplayBackend for Xvfb {
    fn name(&self) -> &'static str {
        "Xvfb"
    }

    fn program(&self) -> &'static str {
        "Xvfb"
    }

    fn args(&self, display: u16, width: u16, height: u16, _title: &str) -> Vec<String> {
        vec![
            format!(":{display}"),
            "-screen".into(),
            "0".into(),
            format!("{width}x{height}x24"),
            "-nolisten".into(),
            "tcp".into(),
            "-ac".into(),
            "-noreset".into(),
        ]
    }

    fn requires_host_display(&self) -> bool {
        false
    }

    fn runs_on_nested_display(&self) -> bool {
        true
    }
}

impl DisplayBackend for Xephyr {
    fn name(&self) -> &'static str {
        "Xephyr"
    }

    fn program(&self) -> &'static str {
        "Xephyr"
    }

    fn args(&self, display: u16, width: u16, height: u16, title: &str) -> Vec<String> {
        vec![
            format!(":{display}"),
            "-screen".into(),
            format!("{width}x{height}x24"),
            "-title".into(),
            title.to_string(),
            "-resizeable".into(),
            "-no-host-grab".into(),
            "-nolisten".into(),
            "tcp".into(),
            "-ac".into(),
            "-noreset".into(),
        ]
    }

    fn requires_host_display(&self) -> bool {
        true
    }

    fn runs_on_nested_display(&self) -> bool {
        false
    }
}

pub fn backend_for(mode: Mode) -> &'static dyn DisplayBackend {
    match mode {
        Mode::Invisible => &XVFB,
        Mode::Visible => &XEPHYR,
    }
}

pub struct StartOptions<'a> {
    pub id: &'a str,
    pub display: u16,
    pub width: u16,
    pub height: u16,
    pub mode: Mode,
    pub paths: &'a Paths,
}

pub struct StartedDisplay {
    pub pid: i32,
}

pub fn start_display(options: &StartOptions<'_>) -> Result<StartedDisplay> {
    let backend = backend_for(options.mode);
    process::require_tool(backend.program(), process::X11_INSTALL)?;

    if backend.requires_host_display() {
        ensure_host_display()?;
    }

    let title = format!("Runspace {}", options.id);
    let args = backend.args(options.display, options.width, options.height, &title);

    let mut envs = base_env(options.id, options.display);
    if backend.runs_on_nested_display() {
        envs.push(("DISPLAY".into(), format!(":{}", options.display)));
    }

    let log_path = options.paths.log_path(options.id);
    let pid = process::spawn_detached(&SpawnSpec {
        program: backend.program(),
        args,
        envs,
        env_remove: Vec::new(),
        cwd: None,
        log_path: &log_path,
    })?;

    if let Err(error) = wait_ready(options.display, pid, &log_path, Duration::from_secs(5)) {
        process::send_signal(pid, libc::SIGTERM);
        thread::sleep(Duration::from_millis(150));
        process::send_signal(pid, libc::SIGKILL);
        process::remove_display_artifacts(options.display);
        return Err(error);
    }

    Ok(StartedDisplay { pid })
}

fn ensure_host_display() -> Result<()> {
    let display = std::env::var("DISPLAY").unwrap_or_default();
    if display.trim().is_empty() {
        return Err(Error::NoHostDisplay {
            display: "(unset)".into(),
        });
    }
    if x11rb::connect(Some(display.trim())).is_err() {
        return Err(Error::NoHostDisplay { display });
    }
    Ok(())
}

fn wait_ready(display: u16, pid: i32, log_path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if display_reachable(display) {
            return Ok(());
        }
        if !process::pid_alive(pid) {
            return Err(start_error(
                display,
                "the server exited during startup",
                log_path,
            ));
        }
        if Instant::now() >= deadline {
            return Err(start_error(
                display,
                &format!("timed out after {}s", timeout.as_secs()),
                log_path,
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn start_error(display: u16, reason: &str, log_path: &Path) -> Error {
    let tail = process::tail_lines(log_path, 20)
        .unwrap_or_default()
        .trim()
        .to_string();
    let details = if tail.is_empty() {
        format!("  {reason}")
    } else {
        format!("  {reason}\n  log: {}\n  {tail}", log_path.display())
    };
    Error::BackendStart {
        backend: "X server".into(),
        display,
        details,
    }
}

pub fn display_reachable(display: u16) -> bool {
    x11rb::connect(Some(&format!(":{display}"))).is_ok()
}

/// Start a lightweight window manager inside the runspace. openbox gives us
/// focus, move/resize and window decorations. It is strongly recommended but
/// not fatal if missing.
pub fn start_window_manager(id: &str, display: u16, paths: &Paths) -> Option<i32> {
    if process::find_in_path("openbox").is_none() {
        let _ = process::append_log(
            &paths.log_path(id),
            "runspace: openbox not found; windows will have no decorations or focus management. Install it with: sudo apt-get install -y openbox",
        );
        return None;
    }

    let envs = app_environment(id, display, paths);
    let log_path = paths.log_path(id);
    let spawned = process::spawn_detached(&SpawnSpec {
        program: "openbox",
        args: vec!["--sm-disable".into()],
        envs,
        env_remove: vec!["WAYLAND_DISPLAY"],
        cwd: Some(paths.home_dir(id)),
        log_path: &log_path,
    });

    match spawned {
        Ok(pid) => {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut ready = false;
            while Instant::now() < deadline {
                if !process::pid_alive(pid) {
                    let _ = process::append_log(
                        &log_path,
                        "runspace: openbox exited during startup; continuing without a window manager",
                    );
                    return None;
                }
                if crate::platform::linux::desktop::window_manager_ready(display) {
                    ready = true;
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
            if !ready {
                let _ = process::append_log(
                    &log_path,
                    "runspace: openbox did not signal readiness within 2s; continuing anyway",
                );
            }
            Some(pid)
        }
        Err(error) => {
            let _ = process::append_log(
                &log_path,
                &format!("runspace: could not start openbox: {error}"),
            );
            None
        }
    }
}

/// Environment shared by every process that belongs to a runspace. The tag is
/// what makes cleanup reliable even for apps that daemonise.
pub fn base_env(id: &str, display: u16) -> Vec<(String, String)> {
    vec![
        ("RUNSPACE_ID".into(), id.to_string()),
        ("RUNSPACE_DISPLAY".into(), format!(":{display}")),
    ]
}

/// Full environment for applications and the window manager: private HOME,
/// TMPDIR and XDG directories under the runspace, and X11 forced over Wayland.
pub fn app_environment(id: &str, display: u16, paths: &Paths) -> Vec<(String, String)> {
    let home = paths.home_dir(id);
    let mut envs = base_env(id, display);
    envs.extend([
        ("DISPLAY".into(), format!(":{display}")),
        ("HOME".into(), home.display().to_string()),
        ("TMPDIR".into(), paths.tmp_dir(id).display().to_string()),
        (
            "XDG_RUNTIME_DIR".into(),
            paths.xdg_dir(id).display().to_string(),
        ),
        (
            "XDG_CONFIG_HOME".into(),
            home.join(".config").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".into(),
            home.join(".cache").display().to_string(),
        ),
        (
            "XDG_DATA_HOME".into(),
            home.join(".local/share").display().to_string(),
        ),
        ("XDG_SESSION_TYPE".into(), "x11".into()),
        ("XDG_CURRENT_DESKTOP".into(), "openbox".into()),
    ]);
    envs
}

/// Launch an application inside a runspace. The command is interpreted by
/// `sh -c`, which is what makes `launch_app("python3 app.py")` behave the way
/// an agent expects. The process is detached from the MCP server so it keeps
/// running between tool calls.
pub fn launch_application(
    record: &Record,
    paths: &Paths,
    command: &str,
    cwd: Option<&Path>,
    wait_ms: u64,
) -> Result<LaunchOutcome> {
    let command = command.trim();
    if command.is_empty() {
        return Err(Error::invalid("command cannot be empty"));
    }
    if command.len() > 8192 {
        return Err(Error::invalid("command is too long (limit: 8192 bytes)"));
    }

    let workdir = match cwd {
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_dir() {
                return Err(Error::invalid(format!(
                    "working directory does not exist: {}",
                    path.display()
                )));
            }
            path
        }
        None => std::env::current_dir().unwrap_or_else(|_| paths.home_dir(&record.id)),
    };

    let log_path = paths.log_path(&record.id);
    let mut child = process::spawn_detached_child(&SpawnSpec {
        program: "/bin/sh",
        args: vec!["-c".into(), command.to_string()],
        envs: app_environment(&record.id, record.display, paths),
        env_remove: vec!["WAYLAND_DISPLAY"],
        cwd: Some(workdir),
        log_path: &log_path,
    })?;
    let pid = child.id();

    let deadline = Instant::now() + Duration::from_millis(wait_ms.min(5000));
    let mut status = None;
    let mut last_touch = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                return Err(Error::Spawn {
                    program: command.to_string(),
                    source: error,
                });
            }
        }
        if last_touch.elapsed() >= Duration::from_secs(1) {
            let _ = state::touch(paths, &record.id);
            last_touch = Instant::now();
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    if let Some(exit) = status {
        let tail = process::tail_lines(&log_path, 25).unwrap_or_default();
        if exit.success() {
            return Ok(LaunchOutcome {
                pid,
                message: format!("'{command}' exited successfully"),
            });
        }
        let code = exit
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".into());
        let tail = tail.trim();
        let details = if tail.is_empty() {
            String::new()
        } else {
            format!("\n{tail}")
        };
        return Err(Error::Command {
            program: command.to_string(),
            message: format!("exited with status {code}{details}"),
        });
    }

    // The process outlived the probe. Reap it from a small thread when it
    // eventually exits so a long-lived MCP session does not accumulate
    // zombies.
    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(LaunchOutcome {
        pid,
        message: format!(
            "launched '{command}' (pid {pid}) on {}",
            record.display_name()
        ),
    })
}
