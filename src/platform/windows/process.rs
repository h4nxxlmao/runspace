use crate::core::paths::Paths;
use crate::core::state::Record;
use crate::platform::BackendStarted; // keep the platform API in one place
use crate::{Error, Result};

use super::win32;

pub fn backend_name() -> &'static str {
    "win32"
}

pub fn create_job_for(id: &str) -> Result<windows_sys::Win32::Foundation::HANDLE> {
    let name = win32::job_name(id);
    win32::create_job(&name)
}

pub fn is_alive(record: &Record) -> bool {
    if !win32::process_alive(record.backend_pid as u32) {
        return false;
    }
    record
        .desktop
        .as_deref()
        .is_some_and(|name| win32::open_desktop(name).is_ok())
}

pub fn memory(record: &Record) -> u64 {
    let mut total = win32::process_rss(record.backend_pid as u32);
    if let Some(job) = &record.job {
        let pids = win32::job_pids(job);
        for pid in win32::unique_pids(pids) {
            total += win32::process_rss(pid);
        }
    }
    total
}

pub fn destroy_backend(record: &Record, _paths: &Paths) {
    if let Some(job) = &record.job {
        win32::terminate_job(job);
    }
    let keeper = record.backend_pid as u32;
    if keeper != 0 && win32::process_alive(keeper) {
        win32::terminate_pid(keeper);
        win32::wait_pid(keeper, 500);
    }
}

pub fn destroy_orphan(id: &str, _paths: &Paths) {
    win32::terminate_job(&win32::job_name(id));
}

pub fn cleanup_orphans(_valid_ids: &std::collections::HashSet<String>) {
    // Keeper processes self-exit when their state record disappears; the job
    // kill-on-close flag then reaps every app in the runspace.
}

pub fn ensure_state_dirs(paths: &Paths, id: &str) -> Result<()> {
    for dir in [
        paths.dir(id),
        paths.home_dir(id),
        paths.tmp_dir(id),
        paths.home_dir(id).join("AppData/Local"),
        paths.home_dir(id).join("AppData/Roaming"),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn backend_started(
    id: &str,
    keeper_pid: u32,
    _width: u16,
    _height: u16,
) -> Result<BackendStarted> {
    Ok(BackendStarted {
        display: 0,
        backend_pid: keeper_pid as i32,
        wm_pid: None,
        desktop: Some(win32::desktop_name(id)),
        job: Some(win32::job_name(id)),
        clipboard: crate::core::state::ClipboardMode::Shared,
    })
}

pub fn error(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
