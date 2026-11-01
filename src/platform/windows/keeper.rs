use std::io::Write;
use std::time::Duration;

use windows_sys::Win32::Foundation::CloseHandle;

use crate::Result;
use crate::core::paths::Paths;

use super::win32;

pub fn run(id: &str) -> Result<i32> {
    let paths = Paths::discover()?;
    let log_path = paths.log_path(id);
    append(&log_path, "keeper: starting");

    let job_name = win32::job_name(id);
    let job = match win32::create_job(&job_name) {
        Ok(job) => job,
        Err(error) => {
            append(&log_path, &format!("keeper: {error}"));
            return Err(error);
        }
    };
    // A stale job with the same name can only come from an interrupted run.
    unsafe {
        windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
    }

    let desktop_name = win32::desktop_name(id);
    let desktop = match win32::open_desktop(&desktop_name) {
        Ok(handle) => handle,
        Err(_) => match win32::create_desktop(&desktop_name) {
            Ok(handle) => handle,
            Err(error) => {
                append(&log_path, &format!("keeper: {error}"));
                unsafe {
                    CloseHandle(job);
                }
                return Err(error);
            }
        },
    };

    append(&log_path, "keeper: ready");
    let record_path = paths.record_path(id);
    let creating_path = paths.dir(id).join("creating");
    let mut seen_state = false;
    let grace_deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if record_path.exists() {
            seen_state = true;
        } else if seen_state
            || !creating_path.exists()
            || std::time::Instant::now() >= grace_deadline
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    append(&log_path, "keeper: state removed, exiting");
    unsafe {
        CloseHandle(job);
    }
    drop(desktop);
    Ok(0)
}

fn append(path: &std::path::Path, message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}
