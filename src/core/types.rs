use serde::Serialize;

/// A visible top-level window, shared by every platform backend.
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub class: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub pid: Option<u32>,
}

/// A native control inside a window. Windows exposes these; Linux currently
/// exposes top-level windows only, so controls will be empty there.
#[derive(Debug, Clone, Serialize)]
pub struct UiControl {
    pub id: u64,
    pub parent: Option<u64>,
    pub class: String,
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
    pub visible: bool,
    pub focused: bool,
    pub control_id: i32,
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiWindow {
    #[serde(flatten)]
    pub window: WindowInfo,
    pub focused: bool,
    pub controls: Vec<UiControl>,
    pub controls_truncated: bool,
}

/// Text representation of the desktop for agents that cannot see screenshots.
#[derive(Debug, Clone, Serialize)]
pub struct UiTree {
    pub backend: String,
    pub windows: Vec<UiWindow>,
    pub focused_window: Option<u64>,
    pub cursor_x: i32,
    pub cursor_y: i32,
}

/// A matching window or native control with an absolute click point.
#[derive(Debug, Clone, Serialize)]
pub struct UiMatch {
    pub window: WindowInfo,
    pub control: Option<UiControl>,
    pub x: i32,
    pub y: i32,
}

/// Result of querying the UI tree for text, class, or window filters.
#[derive(Debug, Clone, Serialize)]
pub struct UiQueryResult {
    pub backend: String,
    pub matches: Vec<UiMatch>,
    pub windows_scanned: usize,
    pub controls_scanned: usize,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct LaunchOutcome {
    pub pid: u32,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub name: &'static str,
    pub required: bool,
    pub purpose: &'static str,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Default, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    pub notes: Vec<String>,
    pub ok: bool,
    pub install_hint: Option<String>,
}
