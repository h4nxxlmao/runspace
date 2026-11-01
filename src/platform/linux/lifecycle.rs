use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use super::{backend, desktop, process};
use crate::core::paths::Paths;
use crate::core::state::{ClipboardMode, Mode, Record};
use crate::core::types::{DoctorCheck, DoctorReport};
use crate::platform::{BackendStarted, CreateContext};
use crate::{Error, Result};

pub fn backend_name() -> &'static str {
    "x11"
}

pub fn create_backend(ctx: &CreateContext<'_>) -> Result<BackendStarted> {
    let mode = if ctx.visible {
        Mode::Visible
    } else {
        Mode::Invisible
    };
    let display = allocate_display(ctx.active)?;
    let started = backend::start_display(&backend::StartOptions {
        id: ctx.id,
        display,
        width: ctx.width,
        height: ctx.height,
        mode,
        paths: ctx.paths,
    })?;
    let wm_pid = backend::start_window_manager(ctx.id, display, ctx.paths);
    if let Err(error) = desktop::set_root_background(display, 24, 26, 32) {
        tracing::warn!("could not set the desktop background: {error}");
    }

    Ok(BackendStarted {
        display,
        backend_pid: started.pid,
        wm_pid,
        desktop: None,
        job: None,
        clipboard: ClipboardMode::Private,
    })
}

fn allocate_display(active: &[Record]) -> Result<u16> {
    for display in 90u16..=199 {
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{display}"));
        let lock = PathBuf::from(format!("/tmp/.X{display}-lock"));
        if socket.exists() || lock.exists() {
            continue;
        }
        if active.iter().any(|record| record.display == display) {
            continue;
        }
        if backend::display_reachable(display) {
            continue;
        }
        return Ok(display);
    }
    Err(Error::invalid(
        "no free display numbers in the 90-199 range; destroy stale runspaces with `runspace destroy --all`",
    ))
}

pub fn post_create(_record: &Record, _paths: &Paths) -> Result<()> {
    Ok(())
}

pub fn is_alive(record: &Record) -> bool {
    process::pid_alive(record.backend_pid) && backend::display_reachable(record.display)
}

pub fn memory(record: &Record) -> u64 {
    let mut pids = vec![record.backend_pid];
    if let Some(wm_pid) = record.wm_pid {
        pids.push(wm_pid);
    }
    process::memory_bytes(&record.id, &pids)
}

pub fn destroy_backend(record: &Record, _paths: &Paths) {
    process::terminate_tagged(&record.id, Duration::from_millis(1500));

    if process::is_display_server(record.backend_pid, record.display) {
        process::send_signal(record.backend_pid, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(250));
        if process::pid_alive(record.backend_pid) {
            process::send_signal(record.backend_pid, libc::SIGKILL);
        }
    }

    if backend::display_reachable(record.display) {
        tracing::warn!(
            "display :{} is still reachable after destroying {}; leaving sockets in place",
            record.display,
            record.id
        );
    } else {
        process::remove_display_artifacts(record.display);
    }
}

pub fn destroy_orphan(id: &str, _paths: &Paths) {
    process::terminate_tagged(id, Duration::from_millis(500));
}

pub fn cleanup_orphans(valid_ids: &std::collections::HashSet<String>) {
    let swept = process::sweep_orphans(valid_ids);
    if swept > 0 {
        tracing::debug!("swept {swept} orphaned process(es)");
    }
}

pub fn attach(record: &Record) -> Result<String> {
    match record.mode {
        Mode::Visible => {
            let host = std::env::var("DISPLAY").unwrap_or_default();
            if host.trim().is_empty() {
                return Err(Error::NoHostDisplay {
                    display: "(unset)".into(),
                });
            }
            desktop::focus_host_window(host.trim(), &record.title())?;
            Ok(format!("focused the {} window on {host}", record.label()))
        }
        Mode::Invisible => Err(Error::Unsupported(format!(
            "{} is headless (Xvfb), so it cannot be promoted live.\n  Create it with --visible to watch it from the start, or take screenshots through MCP while it runs.",
            record.label()
        ))),
    }
}

pub fn doctor() -> DoctorReport {
    let checks = [
        ("Xvfb", true, "invisible desktops"),
        ("xdotool", true, "mouse, keyboard and window control"),
        ("openbox", false, "window management"),
        ("Xephyr", false, "visible desktops"),
        ("xrandr", false, "live resolution changes"),
    ]
    .into_iter()
    .map(|(name, required, purpose)| {
        let path = process::find_in_path(name);
        DoctorCheck {
            name,
            required,
            purpose,
            ok: path.is_some(),
            detail: path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".into()),
        }
    })
    .collect::<Vec<_>>();

    let host_display = std::env::var("DISPLAY")
        .ok()
        .filter(|display| !display.trim().is_empty());
    let host_display_ok = host_display
        .as_deref()
        .is_some_and(|display| x11rb::connect(Some(display.trim())).is_ok());

    let (runtime, runtime_error) = match Paths::discover().and_then(|paths| {
        paths.ensure()?;
        Ok(paths.root().to_path_buf())
    }) {
        Ok(path) => (Some(path), None),
        Err(error) => (None, Some(error.to_string())),
    };

    let mut notes = Vec::new();
    match (&host_display, host_display_ok) {
        (Some(display), true) => notes.push(format!("host display  {display} (ok)")),
        (Some(display), false) => notes.push(format!("host display  {display} (unreachable)")),
        (None, _) => notes.push(
            "host display  not set (visible mode unavailable; invisible mode still works)".into(),
        ),
    }
    match &runtime {
        Some(path) => notes.push(format!("runtime dir   {}", path.display())),
        None => notes.push(format!(
            "runtime dir   {}",
            runtime_error.as_deref().unwrap_or("unavailable")
        )),
    }

    let ok = checks
        .iter()
        .filter(|check| check.required)
        .all(|check| check.ok)
        && runtime_error.is_none();

    DoctorReport {
        checks,
        notes,
        ok,
        install_hint: Some(process::X11_INSTALL.to_string()),
    }
}

pub fn spawn_watchdog(id: &str) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg("__watch")
        .arg(id)
        .env_remove("RUNSPACE_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    Ok(child.id() as i32)
}

pub fn stop_watchdog(record: &Record) {
    let Some(pid) = record.watcher_pid else {
        return;
    };
    let me = std::process::id() as i32;
    if pid == me || !process::is_watchdog(pid, &record.id) {
        return;
    }
    process::send_signal(pid, libc::SIGTERM);
    std::thread::sleep(Duration::from_millis(100));
    if process::pid_alive(pid) {
        process::send_signal(pid, libc::SIGKILL);
    }
}

pub fn watchdog_should_stop(_id: &str) -> bool {
    false
}

pub fn internal_mode(_args: &[String]) -> Option<ExitCode> {
    None
}

pub fn ensure_state_dirs(paths: &Paths, id: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for dir in [
        paths.dir(id),
        paths.home_dir(id),
        paths.tmp_dir(id),
        paths.xdg_dir(id),
    ] {
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn cleanup_helpers(_paths: &Paths) {}
