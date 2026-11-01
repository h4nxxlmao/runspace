use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::json;
use windows_sys::Win32::Foundation::CloseHandle;

use super::{exec, keeper, process, testapp, viewer, win32};
use crate::core::paths::Paths;
use crate::core::state::{Mode, Record};
use crate::core::types::{DoctorCheck, DoctorReport, LaunchOutcome};
use crate::platform::{BackendStarted, CreateContext};
use crate::{Error, Result, ops};

pub fn backend_name() -> &'static str {
    "win32"
}

pub fn create_backend(ctx: &CreateContext<'_>) -> Result<BackendStarted> {
    let desktop_name = win32::desktop_name(ctx.id);
    let exe = helper_exe(ctx.paths)?;
    let args = vec!["__keeper".to_string(), ctx.id.to_string()];
    let keeper_process = win32::spawn_process(&win32::SpawnRequest {
        program: &exe,
        args: &args,
        desktop: None,
        job: None,
        cwd: None,
    })?;

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if win32::open_desktop(&desktop_name).is_ok() {
            break;
        }
        if let Some(code) = keeper_process.wait(0) {
            cleanup_failed_create(ctx.id, keeper_process.pid);
            return Err(Error::Invalid(format!(
                "the Windows desktop keeper exited with status {code}\n{}",
                log_tail(ctx.paths, ctx.id)
            )));
        }
        if Instant::now() >= deadline {
            cleanup_failed_create(ctx.id, keeper_process.pid);
            return Err(Error::Invalid(format!(
                "the Windows desktop did not become ready within 3s\n{}",
                log_tail(ctx.paths, ctx.id)
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let keeper_pid = keeper_process.pid;
    drop(keeper_process);
    process::backend_started(ctx.id, keeper_pid, ctx.width, ctx.height)
}

fn cleanup_failed_create(id: &str, keeper_pid: u32) {
    win32::terminate_job(&win32::job_name(id));
    win32::terminate_pid(keeper_pid);
}

fn log_tail(paths: &Paths, id: &str) -> String {
    ops::tail_lines(&paths.log_path(id), 20)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn post_create(record: &Record, paths: &Paths) -> Result<()> {
    if record.mode == Mode::Visible && !launch_viewer(record) {
        append_log(
            paths,
            &record.id,
            "viewer: did not appear within 2s; the runspace is still usable through MCP",
        );
    }
    Ok(())
}

pub fn launch_application(
    record: &Record,
    paths: &Paths,
    command: &str,
    cwd: Option<&std::path::Path>,
    wait_ms: u64,
) -> Result<LaunchOutcome> {
    let payload = json!({
        "command": command,
        "cwd": cwd.map(|path| path.display().to_string()),
    });
    let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&payload)?);
    let exe = helper_exe(paths)?;
    let args = vec!["__exec".to_string(), record.id.clone(), encoded];
    let child = win32::spawn_process(&win32::SpawnRequest {
        program: &exe,
        args: &args,
        desktop: record.desktop.as_deref(),
        job: record.job.as_deref(),
        cwd: None,
    })?;
    let wait = wait_ms.min(5000) as u32;
    if let Some(code) = child.wait(wait) {
        let tail = ops::tail_lines(&paths.log_path(&record.id), 30).unwrap_or_default();
        let tail = tail.trim();
        if code == 0 {
            return Ok(LaunchOutcome {
                pid: child.pid,
                message: format!("'{command}' exited successfully"),
            });
        }
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

    Ok(LaunchOutcome {
        pid: child.pid,
        message: format!("launched '{command}' (helper pid {}) on win32", child.pid),
    })
}

pub fn is_alive(record: &Record) -> bool {
    process::is_alive(record)
}

pub fn memory(record: &Record) -> u64 {
    process::memory(record)
}

pub fn destroy_backend(record: &Record, paths: &Paths) {
    process::destroy_backend(record, paths);
}

pub fn destroy_orphan(id: &str, paths: &Paths) {
    process::destroy_orphan(id, paths);
}

pub fn cleanup_orphans(valid_ids: &HashSet<String>) {
    process::cleanup_orphans(valid_ids);
}

pub fn ensure_state_dirs(paths: &Paths, id: &str) -> Result<()> {
    process::ensure_state_dirs(paths, id)
}

pub fn attach(record: &Record) -> Result<String> {
    let title = win32::viewer_title(&record.id);
    if let Some(hwnd) = win32::find_host_window(&title) {
        win32::show_host_window(hwnd);
        return Ok(format!(
            "focused the viewer window for {} on the Windows desktop",
            record.label()
        ));
    }
    if record.job.is_none() {
        return Err(Error::Invalid(
            "this runspace has no Windows job; it may have been created by an older version".into(),
        ));
    }
    if !launch_viewer(record) {
        return Err(Error::Invalid(
            "the viewer window did not appear; check `runspace logs` for details".into(),
        ));
    }
    if let Some(hwnd) = win32::find_host_window(&title) {
        win32::show_host_window(hwnd);
        Ok(format!(
            "opened the viewer window for {} on the Windows desktop",
            record.label()
        ))
    } else {
        Err(Error::Invalid(
            "the viewer window could not be created".into(),
        ))
    }
}

fn launch_viewer(record: &Record) -> bool {
    let Some(job) = record.job.as_deref() else {
        return false;
    };
    let Ok(paths) = Paths::discover() else {
        return false;
    };
    let Ok(exe) = helper_exe(&paths) else {
        return false;
    };
    let args = vec!["__view".to_string(), record.id.clone()];
    let spawned = win32::spawn_process(&win32::SpawnRequest {
        program: &exe,
        args: &args,
        desktop: None,
        job: Some(job),
        cwd: None,
    });
    let Ok(_child) = spawned else {
        return false;
    };
    let title = win32::viewer_title(&record.id);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if win32::find_host_window(&title).is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub fn doctor() -> DoctorReport {
    let stamp = std::process::id();
    let desktop_name = format!("RunspaceDoctor-{stamp}");
    let desktop_ok = win32::create_desktop(&desktop_name).is_ok();
    let job_name = format!("Local\\RunspaceDoctor-{stamp}");
    let job_ok = match win32::create_job(&job_name) {
        Ok(job) => {
            unsafe {
                CloseHandle(job);
            }
            true
        }
        Err(_) => false,
    };
    let session_ok = win32::host_display_available();

    let checks = vec![
        DoctorCheck {
            name: "Win32 desktops",
            required: true,
            purpose: "hidden desktops and window capture",
            ok: desktop_ok,
            detail: if desktop_ok {
                "available".into()
            } else {
                "could not create a desktop".into()
            },
        },
        DoctorCheck {
            name: "Job objects",
            required: true,
            purpose: "process isolation and cleanup",
            ok: job_ok,
            detail: if job_ok {
                "available".into()
            } else {
                "could not create a job".into()
            },
        },
        DoctorCheck {
            name: "Interactive session",
            required: true,
            purpose: "viewer windows and desktop capture",
            ok: session_ok,
            detail: if session_ok {
                "available".into()
            } else {
                "no host display detected".into()
            },
        },
    ];

    let (runtime, runtime_error) = match Paths::discover().and_then(|paths| {
        paths.ensure()?;
        Ok(paths.root().to_path_buf())
    }) {
        Ok(path) => (Some(path), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let mut notes = vec![
        "backend       native Win32 (no WSL required)".to_string(),
        match &runtime {
            Some(path) => format!("runtime dir   {}", path.display()),
            None => format!(
                "runtime dir   {}",
                runtime_error.as_deref().unwrap_or("unavailable")
            ),
        },
    ];
    notes.push("capture       PrintWindow (per-window composition)".into());

    let ok = checks
        .iter()
        .filter(|check| check.required)
        .all(|check| check.ok)
        && runtime_error.is_none();
    DoctorReport {
        checks,
        notes,
        ok,
        install_hint: None,
    }
}

pub fn spawn_watchdog(id: &str) -> Result<i32> {
    let event = win32::create_watchdog_event(id)?;
    unsafe {
        CloseHandle(event);
    }
    let paths = Paths::discover()?;
    let exe = helper_exe(&paths)?;
    let args = vec!["__watch".to_string(), id.to_string()];
    let child = win32::spawn_process(&win32::SpawnRequest {
        program: &exe,
        args: &args,
        desktop: None,
        job: None,
        cwd: None,
    })?;
    Ok(child.pid as i32)
}

pub fn stop_watchdog(record: &Record) {
    let Some(pid) = record.watcher_pid else {
        return;
    };
    if pid == std::process::id() as i32 {
        return;
    }
    win32::signal_watchdog_event(&record.id);
    win32::wait_pid(pid as u32, 1200);
}

pub fn watchdog_should_stop(id: &str) -> bool {
    win32::watchdog_event_signaled(id)
}

pub fn internal_mode(args: &[String]) -> Option<ExitCode> {
    match args.get(1).map(String::as_str) {
        Some("__keeper") => {
            let id = args.get(2)?;
            Some(exit_code(keeper::run(id)))
        }
        Some("__view") => {
            let id = args.get(2)?;
            let result = viewer::run(id);
            if let Err(error) = &result
                && let Ok(paths) = Paths::discover()
            {
                append_log(&paths, id, &format!("viewer: {error}"));
            }
            Some(exit_code(result))
        }
        Some("__exec") => Some(exit_code(exec::run(&args[2..]))),
        Some("__testapp") => {
            let log_path = args.get(2)?;
            Some(exit_code(testapp::run(log_path)))
        }
        _ => None,
    }
}

fn exit_code(result: Result<i32>) -> ExitCode {
    match result {
        Ok(code) => ExitCode::from(code.rem_euclid(256) as u8),
        Err(error) => {
            eprintln!("runspace internal helper failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn append_log(paths: &Paths, id: &str, message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.log_path(id))
    {
        let _ = writeln!(file, "{message}");
    }
}

/// Long-lived helpers run from a versioned copy so the installed binary can be
/// replaced while runspaces are alive. The copy lives in the ephemeral runtime
/// root and is removed once no runspaces remain.
pub fn helper_exe(paths: &Paths) -> Result<PathBuf> {
    let bin_dir = paths.root().join("bin");
    std::fs::create_dir_all(&bin_dir)?;
    let current = std::env::current_exe()?;
    if current.starts_with(&bin_dir) {
        return Ok(current);
    }
    let path = bin_dir.join(format!(
        "runspace-helper-{}-{}-{}.exe",
        env!("CARGO_PKG_VERSION"),
        std::fs::metadata(&current)?.len(),
        std::fs::metadata(&current)?
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    if path.exists() {
        return Ok(path);
    }
    for stale in std::fs::read_dir(&bin_dir).into_iter().flatten().flatten() {
        let stale = stale.path();
        if stale != path
            && stale
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("runspace-helper-"))
        {
            let _ = std::fs::remove_file(stale);
        }
    }
    let tmp = bin_dir.join(format!(".helper-{}.tmp", std::process::id()));
    std::fs::copy(&current, &tmp)?;
    match std::fs::rename(&tmp, &path) {
        Ok(()) => {}
        Err(error) if path.exists() => {
            let _ = std::fs::remove_file(&tmp);
            tracing::debug!("helper copy raced with another process: {error}");
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

pub fn cleanup_helpers(paths: &Paths) {
    let dir = paths.root().join("bin");
    if let Err(error) = std::fs::remove_dir_all(&dir)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!("could not remove helper directory: {error}");
    }
}
