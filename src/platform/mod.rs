use crate::core::paths::Paths;
use crate::core::state::{ClipboardMode, Record};

pub use crate::core::types::{
    DoctorCheck, DoctorReport, LaunchOutcome, RawImage, Screenshot, UiControl, UiTree, UiWindow,
    WindowInfo,
};

/// Everything a platform backend needs in order to bring a runspace up.
pub struct CreateContext<'a> {
    pub id: &'a str,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
    pub active: &'a [Record],
    pub paths: &'a Paths,
}

/// What a platform backend hands back after the display/desktop is ready.
pub struct BackendStarted {
    pub display: u16,
    pub backend_pid: i32,
    pub wm_pid: Option<i32>,
    pub desktop: Option<String>,
    pub job: Option<String>,
    pub clipboard: ClipboardMode,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("Runspace currently supports Linux (X11) and Windows.");
