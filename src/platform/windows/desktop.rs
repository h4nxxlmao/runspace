use std::hash::{DefaultHasher, Hash, Hasher};

use crate::core::image::encode_png;
use crate::core::paths::Paths;
use crate::core::state::{self, Mode, Record};
use crate::core::types::{RawImage, Screenshot, UiTree, UiWindow, WindowInfo};
use crate::{Error, Result};

use super::win32::{self, DesktopHandle, WindowEntry};

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_RBUTTONDBLCLK: u32 = 0x0206;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_MOUSEHWHEEL: u32 = 0x020E;
const WM_CHAR: u32 = 0x0102;
const WM_ACTIVATE: u32 = 0x0006;
const WM_SETFOCUS: u32 = 0x0007;
const WM_CLOSE: u32 = 0x0010;
const WM_COPY: u32 = 0x0301;
const WM_CUT: u32 = 0x0300;
const WM_PASTE: u32 = 0x0302;
const WM_UNDO: u32 = 0x0304;
const WM_SYSCOMMAND: u32 = 0x0112;
const EM_SETSEL: u32 = 0x00B1;
const SC_CLOSE: usize = 0xF060;
const WA_ACTIVE: usize = 1;

const MK_LBUTTON: usize = 0x0001;
const MK_RBUTTON: usize = 0x0002;
const MK_MBUTTON: usize = 0x0010;

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_INSERT: u32 = 0x2D;
const VK_DELETE: u32 = 0x2E;
const VK_LWIN: u32 = 0x5B;

const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;

#[derive(Clone)]
pub struct Desktop {
    id: String,
    desktop_name: String,
    width: u16,
    height: u16,
    mode: Mode,
    paths: Paths,
}

impl Desktop {
    pub fn new(record: &Record, paths: &Paths) -> Result<Self> {
        let desktop_name = record
            .desktop
            .clone()
            .unwrap_or_else(|| win32::desktop_name(&record.id));
        let _handle = win32::open_desktop(&desktop_name)?;
        Ok(Self {
            id: record.id.clone(),
            desktop_name,
            width: record.width,
            height: record.height,
            mode: record.mode,
            paths: paths.clone(),
        })
    }

    pub fn display(&self) -> u16 {
        0
    }

    fn handle(&self) -> Result<DesktopHandle> {
        win32::open_desktop(&self.desktop_name)
    }

    fn record(&self) -> Result<Record> {
        state::load(&self.paths, &self.id)
    }

    fn update_state(&self, update: impl FnOnce(&mut Record)) -> Result<Record> {
        let mut record = self.record()?;
        update(&mut record);
        state::save(&self.paths, &record)?;
        Ok(record)
    }

    pub fn mouse_move(&self, x: i32, y: i32) -> Result<String> {
        self.update_state(|record| {
            record.cursor_x = x;
            record.cursor_y = y;
        })?;
        let handle = self.handle()?;
        if let Some((_top, target, client_x, client_y)) = self.target_under_point(&handle, x, y) {
            win32::post_message(
                target,
                WM_MOUSEMOVE,
                0,
                win32::make_lparam(client_x, client_y),
            );
        }
        Ok(format!("moved cursor to {x},{y} on win32"))
    }

    pub fn click(
        &self,
        button: &str,
        x: Option<i32>,
        y: Option<i32>,
        count: u32,
    ) -> Result<String> {
        let spec = button_spec(button)?;
        if x.is_some() != y.is_some() {
            return Err(Error::invalid("provide both x and y, or neither"));
        }
        let record = self.record()?;
        let x = x.unwrap_or(record.cursor_x);
        let y = y.unwrap_or(record.cursor_y);
        let count = count.clamp(1, 10);
        self.update_state(|record| {
            record.cursor_x = x;
            record.cursor_y = y;
        })?;

        let handle = self.handle()?;
        let Some((_top, target, client_x, client_y)) = self.target_under_point(&handle, x, y)
        else {
            return Ok(format!(
                "clicked at {x},{y} on win32 (no window under the cursor)"
            ));
        };
        self.set_focus(target)?;
        for index in 0..count {
            let message = if count >= 2 && index == 1 {
                spec.double_click
            } else {
                spec.down
            };
            win32::post_message(
                target,
                WM_MOUSEMOVE,
                spec.mask,
                win32::make_lparam(client_x, client_y),
            );
            win32::post_message(
                target,
                message,
                spec.mask,
                win32::make_lparam(client_x, client_y),
            );
            win32::post_message(target, spec.up, 0, win32::make_lparam(client_x, client_y));
        }
        Ok(format!(
            "clicked button {} {count} time(s) at {x},{y} on win32 into {} ({target})",
            spec.name,
            win32::window_class(target)
        ))
    }

    pub fn drag(
        &self,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        button: &str,
        duration_ms: u64,
    ) -> Result<String> {
        let spec = button_spec(button)?;
        let duration_ms = duration_ms.clamp(0, 10_000);
        let handle = self.handle()?;
        let Some((_top, target, _, _)) = self.target_under_point(&handle, from_x, from_y) else {
            return Err(Error::invalid(format!(
                "no window at {from_x},{from_y} on win32"
            )));
        };
        self.set_focus(target)?;
        let Some((target_x, target_y, _, _)) = win32::window_rect(target) else {
            return Err(Error::invalid("could not read the drag target geometry"));
        };
        let mut client_x = from_x - target_x;
        let mut client_y = from_y - target_y;
        win32::post_message(
            target,
            spec.down,
            spec.mask,
            win32::make_lparam(client_x, client_y),
        );
        let steps = 12;
        for step in 1..=steps {
            let x = from_x + (to_x - from_x) * step / steps;
            let y = from_y + (to_y - from_y) * step / steps;
            client_x = x - target_x;
            client_y = y - target_y;
            win32::post_message(
                target,
                WM_MOUSEMOVE,
                spec.mask,
                win32::make_lparam(client_x, client_y),
            );
            if duration_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(duration_ms / steps as u64));
            }
        }
        win32::post_message(target, spec.up, 0, win32::make_lparam(client_x, client_y));
        self.update_state(|record| {
            record.cursor_x = to_x;
            record.cursor_y = to_y;
        })?;
        Ok(format!(
            "dragged from {from_x},{from_y} to {to_x},{to_y} on win32"
        ))
    }

    pub fn scroll(
        &self,
        direction: &str,
        amount: u32,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<String> {
        let direction = direction.to_ascii_lowercase();
        let horizontal = matches!(direction.as_str(), "left" | "right");
        if x.is_some() != y.is_some() {
            return Err(Error::invalid("provide both x and y, or neither"));
        }
        let record = self.record()?;
        let x = x.unwrap_or(record.cursor_x);
        let y = y.unwrap_or(record.cursor_y);
        let amount = amount.clamp(1, 100) as i32;
        let delta = match direction.as_str() {
            "up" | "right" => 120 * amount,
            "down" | "left" => -120 * amount,
            other => {
                return Err(Error::invalid(format!(
                    "unknown scroll direction '{other}' (use up, down, left or right)"
                )));
            }
        };
        let handle = self.handle()?;
        let target = self
            .target_under_point(&handle, x, y)
            .map(|(_, target, _, _)| target)
            .or_else(|| self.input_target(&record));
        let Some(target) = target else {
            return Ok(format!("scrolled {direction} {amount} step(s) on win32"));
        };
        self.set_focus(target)?;
        let wparam = ((delta as u32) << 16) as usize;
        win32::post_message(
            target,
            if horizontal {
                WM_MOUSEHWHEEL
            } else {
                WM_MOUSEWHEEL
            },
            wparam,
            win32::make_lparam(x, y),
        );
        Ok(format!(
            "scrolled {direction} {amount} step(s) at {x},{y} on win32"
        ))
    }

    pub fn type_text(&self, text: &str) -> Result<String> {
        if text.is_empty() {
            return Err(Error::invalid("text cannot be empty"));
        }
        if text.len() > 100_000 {
            return Err(Error::invalid("text is too long (limit: 100000 bytes)"));
        }
        let _handle = self.handle()?;
        let record = self.record()?;
        let Some(target) = self.input_target(&record) else {
            return Err(Error::invalid(
                "no focused window to type into; click a window first",
            ));
        };
        for unit in text.encode_utf16() {
            let unit = if unit == b'\n' as u16 {
                b'\r' as u16
            } else {
                unit
            };
            win32::post_message(target, WM_CHAR, unit as usize, 0);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        Ok(format!(
            "typed {} character(s) into {} ({target}) on win32",
            text.chars().count(),
            win32::window_class(target)
        ))
    }

    pub fn key(&self, combos: &[String]) -> Result<String> {
        if combos.is_empty() {
            return Err(Error::invalid("provide at least one key combination"));
        }
        let parsed = combos
            .iter()
            .map(|combo| parse_chord(combo))
            .collect::<Result<Vec<_>>>()?;
        let _handle = self.handle()?;
        let record = self.record()?;
        let Some(target) = self.input_target(&record) else {
            return Err(Error::invalid(
                "no focused window to send keys to; click a window first",
            ));
        };
        for chord in &parsed {
            if apply_shortcut(target, chord).is_none() {
                win32::send_chord(&self.desktop_name, target, &chord.modifiers, chord.key);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Ok(format!("pressed {} on win32", combos.join(", ")))
    }

    pub fn cursor_position(&self) -> Result<(i32, i32)> {
        let record = self.record()?;
        Ok((record.cursor_x, record.cursor_y))
    }

    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        let handle = self.handle()?;
        Ok(win32::enumerate_desktop(&handle)
            .into_iter()
            .filter(WindowEntry::is_candidate)
            .map(|window| window.to_info())
            .collect())
    }

    pub fn ui_tree(&self) -> Result<UiTree> {
        let handle = self.handle()?;
        let record = self.record()?;
        let mut windows = Vec::new();
        let mut focused_window = None;
        for window in win32::enumerate_desktop(&handle)
            .into_iter()
            .filter(WindowEntry::is_candidate)
        {
            let ui = win32::window_ui(window.hwnd, 200);
            if ui.focused {
                focused_window = Some(window.hwnd);
            }
            windows.push(UiWindow {
                window: window.to_info(),
                focused: ui.focused,
                controls: ui.controls,
                controls_truncated: ui.truncated,
            });
        }
        if let Some(focus) = record.focus_window {
            for window in &mut windows {
                let mut contains = window.window.id == focus;
                for control in &mut window.controls {
                    if control.id == focus {
                        control.focused = true;
                        contains = true;
                    }
                }
                if contains {
                    window.focused = true;
                    focused_window = Some(window.window.id);
                }
            }
        }
        Ok(UiTree {
            backend: "win32".into(),
            windows,
            focused_window,
            cursor_x: record.cursor_x,
            cursor_y: record.cursor_y,
        })
    }

    pub fn focus_window(&self, target: &str) -> Result<String> {
        let handle = self.handle()?;
        let window = resolve_window(&handle, target)?;
        let input = win32::focused_child(window.id)
            .or_else(|| win32::first_editable_child(window.id))
            .unwrap_or(window.id);
        self.set_focus(input)?;
        win32::bring_to_front(window.id);
        win32::post_message(window.id, WM_ACTIVATE, WA_ACTIVE, 0);
        win32::post_message(input, WM_SETFOCUS, 0, 0);
        Ok(format!("focused window {} \"{}\"", window.id, window.title))
    }

    pub fn close_window(&self, target: &str, force: bool) -> Result<String> {
        let handle = self.handle()?;
        let window = resolve_window(&handle, target)?;
        win32::post_message(window.id, WM_CLOSE, 0, 0);
        if wait_for_close(window.id, 1000) {
            return Ok(format!("closed window {} \"{}\"", window.id, window.title));
        }
        if force {
            if let Some(pid) = win32::window_pid(window.id) {
                win32::terminate_pid(pid);
            }
            if wait_for_close(window.id, 1000) {
                return Ok(format!(
                    "force-closed window {} \"{}\"",
                    window.id, window.title
                ));
            }
        }
        Err(Error::invalid(format!(
            "window {} \"{}\" is still open; it may be asking to save changes. Retry with force=true to kill it.",
            window.id, window.title
        )))
    }

    pub fn move_window(&self, target: &str, x: i32, y: i32) -> Result<String> {
        let handle = self.handle()?;
        let window = resolve_window(&handle, target)?;
        set_window_rect(window.id, x, y, window.width, window.height);
        let (x, y, width, height) = win32::window_rect(window.id)
            .ok_or_else(|| Error::invalid("could not read window geometry"))?;
        Ok(format!(
            "moved window {} to {x},{y}, size {width}x{height}",
            window.id
        ))
    }

    pub fn resize_window(&self, target: &str, width: u32, height: u32) -> Result<String> {
        if !(1..=10000).contains(&width) || !(1..=10000).contains(&height) {
            return Err(Error::invalid(
                "width and height must be between 1 and 10000",
            ));
        }
        let handle = self.handle()?;
        let window = resolve_window(&handle, target)?;
        set_window_rect(window.id, window.x, window.y, width, height);
        let (x, y, width, height) = win32::window_rect(window.id)
            .ok_or_else(|| Error::invalid("could not read window geometry"))?;
        Ok(format!(
            "resized window {} to {width}x{height} at {x},{y}",
            window.id
        ))
    }

    pub fn set_resolution(&self, width: u16, height: u16) -> Result<String> {
        validate_resolution(width, height)?;
        let _ = self.mode;
        Ok(format!(
            "resolution is now {width}x{height} on win32; the desktop canvas resized"
        ))
    }

    pub fn screen_size(&self) -> Result<(u16, u16)> {
        let record = self.record()?;
        Ok((record.width, record.height))
    }

    pub fn capture_rgba(&self) -> Result<RawImage> {
        let handle = self.handle()?;
        win32::capture_desktop(&handle, u32::from(self.width), u32::from(self.height))
    }

    pub fn screenshot(&self) -> Result<Screenshot> {
        let raw = self.capture_rgba()?;
        let png = encode_png(&raw)?;
        Ok(Screenshot {
            width: raw.width,
            height: raw.height,
            png,
        })
    }

    pub fn fingerprint(&self) -> Result<u64> {
        let raw = self.capture_rgba()?;
        let mut hasher = DefaultHasher::new();
        raw.width.hash(&mut hasher);
        raw.height.hash(&mut hasher);
        raw.rgba.hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn set_focus(&self, hwnd: u64) -> Result<()> {
        self.update_state(|record| {
            record.focus_window = Some(hwnd);
        })?;
        Ok(())
    }

    fn input_target(&self, record: &Record) -> Option<u64> {
        if let Some(hwnd) = record.focus_window
            && win32::window_exists(hwnd)
        {
            return Some(hwnd);
        }
        let handle = self.handle().ok()?;
        let top = win32::enumerate_desktop(&handle)
            .into_iter()
            .find(WindowEntry::is_candidate)?;
        win32::focused_child(top.hwnd)
            .or_else(|| win32::first_editable_child(top.hwnd))
            .or(Some(top.hwnd))
    }

    fn target_under_point(
        &self,
        handle: &DesktopHandle,
        x: i32,
        y: i32,
    ) -> Option<(u64, u64, i32, i32)> {
        let windows = win32::enumerate_desktop(handle);
        let window = windows
            .into_iter()
            .filter(WindowEntry::is_candidate)
            .find(|window| {
                x >= window.x
                    && y >= window.y
                    && x < window.x + window.width as i32
                    && y < window.y + window.height as i32
            })?;
        let (child, client_x, client_y) = win32::child_at_point(window.hwnd, x, y)?;
        Some((window.hwnd, child, client_x, client_y))
    }
}

fn wait_for_close(hwnd: u64, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if !win32::window_exists(hwnd) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    !win32::window_exists(hwnd)
}

fn set_window_rect(hwnd: u64, x: i32, y: i32, width: u32, height: u32) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            hwnd as usize as windows_sys::Win32::Foundation::HWND,
            std::ptr::null_mut(),
            x,
            y,
            width as i32,
            height as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn resolve_window(handle: &DesktopHandle, target: &str) -> Result<WindowInfo> {
    let target = target.trim();
    if target.is_empty() {
        return Err(Error::invalid("window id or title cannot be empty"));
    }
    let windows = win32::enumerate_desktop(handle)
        .into_iter()
        .filter(WindowEntry::is_candidate)
        .collect::<Vec<_>>();

    if let Ok(id) = target.parse::<u64>() {
        return windows
            .iter()
            .find(|window| window.hwnd == id)
            .map(WindowEntry::to_info)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "window {id} does not exist on win32; use list_windows to see what is open"
                ))
            });
    }

    if let Some(window) = windows
        .iter()
        .find(|window| window.title.eq_ignore_ascii_case(target))
    {
        return Ok(window.to_info());
    }
    if let Some(window) = windows.iter().find(|window| {
        window
            .title
            .to_ascii_lowercase()
            .contains(&target.to_ascii_lowercase())
    }) {
        return Ok(window.to_info());
    }
    if let Some(window) = windows
        .iter()
        .find(|window| window.class.eq_ignore_ascii_case(target))
    {
        return Ok(window.to_info());
    }
    Err(Error::invalid(format!(
        "no window matching '{target}' on win32; use list_windows to see what is open"
    )))
}

pub fn validate_resolution(width: u16, height: u16) -> Result<()> {
    if !(320..=7680).contains(&width) || !(240..=4320).contains(&height) {
        return Err(Error::invalid(
            "resolution must be between 320x240 and 7680x4320",
        ));
    }
    Ok(())
}

struct ButtonSpec {
    name: &'static str,
    down: u32,
    up: u32,
    double_click: u32,
    mask: usize,
}

fn button_spec(button: &str) -> Result<ButtonSpec> {
    match button.to_ascii_lowercase().as_str() {
        "left" | "1" => Ok(ButtonSpec {
            name: "left",
            down: WM_LBUTTONDOWN,
            up: WM_LBUTTONUP,
            double_click: WM_LBUTTONDBLCLK,
            mask: MK_LBUTTON,
        }),
        "right" | "3" => Ok(ButtonSpec {
            name: "right",
            down: WM_RBUTTONDOWN,
            up: WM_RBUTTONUP,
            double_click: WM_RBUTTONDBLCLK,
            mask: MK_RBUTTON,
        }),
        "middle" | "2" => Ok(ButtonSpec {
            name: "middle",
            down: WM_MBUTTONDOWN,
            up: WM_MBUTTONUP,
            double_click: WM_MBUTTONDOWN,
            mask: MK_MBUTTON,
        }),
        other => Err(Error::invalid(format!(
            "unknown mouse button '{other}' (use left, middle or right)"
        ))),
    }
}

struct Chord {
    modifiers: Vec<u32>,
    key: u32,
}

fn parse_chord(combo: &str) -> Result<Chord> {
    let combo = combo.trim();
    if combo.is_empty() || combo.len() > 128 {
        return Err(Error::invalid("key combination must be 1-128 characters"));
    }
    let parts = combo.split('+').map(str::trim).collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(Error::invalid(format!("invalid key combination '{combo}'")));
    }
    let mut modifiers = Vec::new();
    let mut key = None;
    for (index, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some(VK_CONTROL),
            "shift" => Some(VK_SHIFT),
            "alt" => Some(VK_MENU),
            "win" | "super" | "cmd" | "meta" => Some(VK_LWIN),
            _ => None,
        };
        if let Some(modifier) = modifier
            && index + 1 < parts.len()
        {
            modifiers.push(modifier);
            continue;
        }
        if key.is_some() {
            return Err(Error::invalid(format!(
                "invalid key combination '{combo}' (only one non-modifier key)"
            )));
        }
        key = Some(key_code(&lower)?);
    }
    let key = key.ok_or_else(|| Error::invalid(format!("invalid key combination '{combo}'")))?;
    Ok(Chord { modifiers, key })
}

/// Windows cannot inject modifier state across desktops reliably. Route the
/// common editing shortcuts through the standard control messages instead.
fn apply_shortcut(target: u64, chord: &Chord) -> Option<()> {
    let ctrl = chord.modifiers.contains(&VK_CONTROL);
    let alt = chord.modifiers.contains(&VK_MENU);
    let shift = chord.modifiers.contains(&VK_SHIFT);
    if ctrl && !alt && !shift {
        let (message, wparam, lparam) = match chord.key {
            0x41 => (EM_SETSEL, 0, -1),
            0x43 => (WM_COPY, 0, 0),
            0x58 => (WM_CUT, 0, 0),
            0x56 => (WM_PASTE, 0, 0),
            0x5A => (WM_UNDO, 0, 0),
            _ => (0, 0, 0),
        };
        if message != 0 && win32::send_message(target, message, wparam, lparam) {
            return Some(());
        }
    }
    if alt && chord.key == 0x73 {
        let top = win32::top_level(target);
        if win32::send_message(top, WM_SYSCOMMAND, SC_CLOSE, 0) {
            return Some(());
        }
    }
    None
}

fn key_code(name: &str) -> Result<u32> {
    if name.chars().count() == 1 {
        let ch = name.chars().next().unwrap();
        if ch.is_ascii_alphanumeric() {
            return Ok(ch.to_ascii_uppercase() as u32);
        }
        return match ch {
            ' ' => Ok(VK_SPACE),
            ',' => Ok(0xBC),
            '.' => Ok(0xBE),
            '/' => Ok(0xBF),
            ';' => Ok(0xBA),
            '=' => Ok(0xBB),
            '-' => Ok(0xBD),
            '[' => Ok(0xDB),
            ']' => Ok(0xDD),
            '\\' => Ok(0xDC),
            '\'' => Ok(0xDE),
            '`' => Ok(0xC0),
            _ => Err(Error::invalid(format!("unsupported key '{name}'"))),
        };
    }
    match name {
        "enter" | "return" => Ok(VK_RETURN),
        "esc" | "escape" => Ok(VK_ESCAPE),
        "tab" => Ok(VK_TAB),
        "space" => Ok(VK_SPACE),
        "backspace" => Ok(VK_BACK),
        "delete" | "del" => Ok(VK_DELETE),
        "insert" | "ins" => Ok(VK_INSERT),
        "home" => Ok(VK_HOME),
        "end" => Ok(VK_END),
        "pageup" | "page_up" | "prior" => Ok(VK_PRIOR),
        "pagedown" | "page_down" | "next" => Ok(VK_NEXT),
        "up" => Ok(VK_UP),
        "down" => Ok(VK_DOWN),
        "left" => Ok(VK_LEFT),
        "right" => Ok(VK_RIGHT),
        "ctrl" | "control" => Ok(VK_CONTROL),
        "shift" => Ok(VK_SHIFT),
        "alt" => Ok(VK_MENU),
        "win" | "super" | "cmd" | "meta" => Ok(VK_LWIN),
        other if other.starts_with('f') => {
            let number: u32 = other[1..]
                .parse()
                .map_err(|_| Error::invalid(format!("unknown key '{other}'")))?;
            if (1..=24).contains(&number) {
                Ok(0x70 + number - 1)
            } else {
                Err(Error::invalid(format!("unknown function key '{other}'")))
            }
        }
        other => Err(Error::invalid(format!("unsupported key '{other}'"))),
    }
}
