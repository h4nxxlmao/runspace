#![allow(non_snake_case)]

use std::collections::HashSet;
use std::ffi::{OsStr, c_void};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, RECT, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, EnumDesktopWindows, HDESK, OpenDesktopW, SetThreadDesktop,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateEventW, CreateProcessW, GetCurrentThreadId,
    GetExitCodeProcess, OpenEventW, OpenProcess, PROCESS_INFORMATION, ResumeThread, STARTUPINFOW,
    SetEvent, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardState, IsWindowEnabled, SetKeyboardState,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GA_ROOT, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetDlgCtrlID,
    GetParent, GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindow, IsWindowVisible, SMTO_ABORTIFHUNG,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageTimeoutW, SetProcessDPIAware,
    SetWindowPos, ShowWindow, WM_KEYDOWN, WM_KEYUP, WS_EX_TOOLWINDOW,
};

use crate::core::types::{RawImage, UiControl, WindowInfo};
use crate::{Error, Result};

const DESKTOP_ALL_ACCESS: u32 = 0x000F_01FF;
const JOB_OBJECT_ALL_ACCESS: u32 = 0x001F_001F;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
const PW_RENDERFULLCONTENT: u32 = 2;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_READ: u32 = 0x0010;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const PROCESS_TERMINATE: u32 = 0x0001;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>()
}

pub fn last_error(context: &str) -> Error {
    Error::Invalid(format!("{context}: {}", std::io::Error::last_os_error()))
}

pub fn init_dpi() {
    unsafe {
        SetProcessDPIAware();
    }
}

pub struct DesktopHandle(HDESK);

impl DesktopHandle {
    pub fn raw(&self) -> HDESK {
        self.0
    }
}

impl Drop for DesktopHandle {
    fn drop(&mut self) {
        unsafe {
            CloseDesktop(self.0);
        }
    }
}

pub fn open_desktop(name: &str) -> Result<DesktopHandle> {
    let name = wide(name);
    let handle = unsafe { OpenDesktopW(name.as_ptr(), 0, 0, DESKTOP_ALL_ACCESS) };
    if handle.is_null() {
        return Err(last_error("OpenDesktopW"));
    }
    Ok(DesktopHandle(handle))
}

pub fn create_desktop(name: &str) -> Result<DesktopHandle> {
    let name = wide(name);
    let handle =
        unsafe { CreateDesktopW(name.as_ptr(), null(), null(), 0, DESKTOP_ALL_ACCESS, null()) };
    if handle.is_null() {
        return Err(last_error("CreateDesktopW"));
    }
    Ok(DesktopHandle(handle))
}

#[derive(Debug, Clone)]
pub struct WindowEntry {
    pub hwnd: u64,
    pub title: String,
    pub class: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub pid: Option<u32>,
    pub visible: bool,
    pub iconic: bool,
    pub tool: bool,
    pub cloaked: bool,
}

impl WindowEntry {
    pub fn is_candidate(&self) -> bool {
        self.visible
            && !self.iconic
            && !self.tool
            && !self.cloaked
            && self.width > 1
            && self.height > 1
            && !is_system_class(&self.class)
    }

    pub fn to_info(&self) -> WindowInfo {
        WindowInfo {
            id: self.hwnd,
            title: self.title.clone(),
            class: self.class.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            pid: self.pid,
        }
    }
}

fn is_system_class(class: &str) -> bool {
    matches!(
        class,
        "IME"
            | "MSCTFIME UI"
            | "CiceroUIWndFrame"
            | "UAC Input Indicator"
            | "Touch Tooltip Window"
            | "CicLoaderWndClass"
            | "Shell_TrayWnd"
            | "TaskListThumbnailWnd"
            | "Progman"
            | "WorkerW"
    )
}

pub fn enumerate_desktop(desktop: &DesktopHandle) -> Vec<WindowEntry> {
    let mut entries = Vec::new();
    unsafe {
        EnumDesktopWindows(
            desktop.raw(),
            Some(enum_window),
            &mut entries as *mut _ as isize,
        );
    }
    entries
}

pub fn enumerate_host() -> Vec<WindowEntry> {
    let mut entries = Vec::new();
    unsafe {
        EnumWindows(Some(enum_window), &mut entries as *mut _ as isize);
    }
    entries
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let entries = unsafe { &mut *(lparam as *mut Vec<WindowEntry>) };
    entries.push(unsafe { read_window(hwnd) });
    1
}

unsafe fn read_window(hwnd: HWND) -> WindowEntry {
    let mut title_buffer = [0u16; 1024];
    let mut class_buffer = [0u16; 256];
    let title_len =
        unsafe { GetWindowTextW(hwnd, title_buffer.as_mut_ptr(), title_buffer.len() as i32) };
    let class_len =
        unsafe { GetClassNameW(hwnd, class_buffer.as_mut_ptr(), class_buffer.len() as i32) };
    let mut rect: RECT = unsafe { zeroed() };
    unsafe {
        GetWindowRect(hwnd, &mut rect);
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    let mut cloaked = 0u32;
    let cloaked_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut _ as *mut c_void,
            size_of::<u32>() as u32,
        ) == 0
    };
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;

    WindowEntry {
        hwnd: hwnd as usize as u64,
        title: String::from_utf16_lossy(&title_buffer[..title_len.max(0) as usize]),
        class: String::from_utf16_lossy(&class_buffer[..class_len.max(0) as usize]),
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left).max(0) as u32,
        height: (rect.bottom - rect.top).max(0) as u32,
        pid: (pid != 0).then_some(pid),
        visible: unsafe { IsWindowVisible(hwnd) } != 0,
        iconic: unsafe { IsIconic(hwnd) } != 0,
        tool: ex_style & WS_EX_TOOLWINDOW != 0,
        cloaked: cloaked_ok && cloaked != 0,
    }
}

pub fn window_rect(hwnd: u64) -> Option<(i32, i32, u32, u32)> {
    let mut rect: RECT = unsafe { zeroed() };
    if unsafe { GetWindowRect(hwnd as usize as HWND, &mut rect) } == 0 {
        return None;
    }
    Some((
        rect.left,
        rect.top,
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    ))
}

pub fn window_exists(hwnd: u64) -> bool {
    unsafe { IsWindow(hwnd as usize as HWND) != 0 }
}

pub fn window_class(hwnd: u64) -> String {
    let mut buffer = [0u16; 256];
    let len = unsafe {
        GetClassNameW(
            hwnd as usize as HWND,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

pub fn window_text(hwnd: u64) -> String {
    let mut buffer = [0u16; 2048];
    let len = unsafe {
        GetWindowTextW(
            hwnd as usize as HWND,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn control_text(hwnd: u64, class: &str) -> String {
    let text = window_text(hwnd);
    if !text.is_empty() {
        return text;
    }
    let class = class.to_ascii_lowercase();
    if class.starts_with("edit") || class.starts_with("richedit") || class.starts_with("scintilla")
    {
        let mut buffer = [0u16; 4096];
        let mut copied = 0usize;
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd as usize as HWND,
                0x000D, // WM_GETTEXT
                buffer.len(),
                buffer.as_mut_ptr() as isize,
                SMTO_ABORTIFHUNG,
                500,
                &mut copied,
            )
        };
        if ok != 0 {
            return String::from_utf16_lossy(&buffer[..copied.min(buffer.len())]);
        }
    }
    String::new()
}

pub struct WindowUi {
    pub focused: bool,
    pub controls: Vec<UiControl>,
    pub truncated: bool,
}

struct ControlCollect {
    top: HWND,
    focus: HWND,
    limit: usize,
    items: Vec<UiControl>,
    truncated: bool,
}

pub fn window_ui(top: u64, limit: usize) -> WindowUi {
    let hwnd = top as usize as HWND;
    let mut thread = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut thread);
    }
    let mut info: windows_sys::Win32::UI::WindowsAndMessaging::GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<windows_sys::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32;
    let ok = thread != 0
        && unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetGUIThreadInfo(thread, &mut info)
        } != 0;
    let focus = if ok { info.hwndFocus } else { null_mut() };
    let active = if ok { info.hwndActive } else { null_mut() };

    let mut collect = ControlCollect {
        top: hwnd,
        focus,
        limit: limit.max(1),
        items: Vec::new(),
        truncated: false,
    };
    unsafe {
        EnumChildWindows(hwnd, Some(enum_controls), &mut collect as *mut _ as isize);
    }
    WindowUi {
        focused: active == hwnd,
        controls: collect.items,
        truncated: collect.truncated,
    }
}

unsafe extern "system" fn enum_controls(hwnd: HWND, lparam: LPARAM) -> i32 {
    let collect = unsafe { &mut *(lparam as *mut ControlCollect) };
    if collect.items.len() >= collect.limit {
        collect.truncated = true;
        return 0;
    }
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    let mut rect: RECT = unsafe { zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return 1;
    }
    let width = (rect.right - rect.left).max(0) as u32;
    let height = (rect.bottom - rect.top).max(0) as u32;
    if width == 0 || height == 0 {
        return 1;
    }
    let parent = unsafe { GetParent(hwnd) };
    let mut depth = 1u32;
    let mut current = parent;
    while !current.is_null() && current != collect.top && depth < 16 {
        depth += 1;
        current = unsafe { GetParent(current) };
    }
    let class = window_class(hwnd as usize as u64);
    let text = control_text(hwnd as usize as u64, &class);
    collect.items.push(UiControl {
        id: hwnd as usize as u64,
        parent: (!parent.is_null()).then_some(parent as usize as u64),
        class,
        text,
        x: rect.left,
        y: rect.top,
        width,
        height,
        enabled: unsafe { IsWindowEnabled(hwnd) } != 0,
        visible: true,
        focused: hwnd == collect.focus,
        control_id: unsafe { GetDlgCtrlID(hwnd) },
        depth,
    });
    1
}

pub fn window_pid(hwnd: u64) -> Option<u32> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd as usize as HWND, &mut pid);
    }
    (pid != 0).then_some(pid)
}

pub fn focused_child(hwnd: u64) -> Option<u64> {
    let mut thread = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd as usize as HWND, &mut thread);
    }
    if thread == 0 {
        return None;
    }
    let mut info: windows_sys::Win32::UI::WindowsAndMessaging::GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<windows_sys::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32;
    let ok =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetGUIThreadInfo(thread, &mut info) };
    if ok == 0 || info.hwndFocus.is_null() {
        return None;
    }
    Some(info.hwndFocus as usize as u64)
}

pub fn first_editable_child(hwnd: u64) -> Option<u64> {
    let mut found = 0u64;
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
            hwnd as usize as HWND,
            Some(find_editable),
            &mut found as *mut _ as isize,
        );
    }
    (found != 0).then_some(found)
}

unsafe extern "system" fn find_editable(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = unsafe { &mut *(lparam as *mut u64) };
    if *found != 0 {
        return 1;
    }
    let mut buffer = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    let class = String::from_utf16_lossy(&buffer[..len.max(0) as usize]).to_ascii_lowercase();
    if class.starts_with("edit")
        || class.starts_with("richedit")
        || class == "tkchild"
        || class.contains("renderwidgethost")
    {
        *found = hwnd as usize as u64;
    }
    1
}

pub fn print_window_rgba(hwnd: u64) -> Result<RawImage> {
    let Some((_, _, width, height)) = window_rect(hwnd) else {
        return Err(Error::Invalid(format!(
            "could not read window geometry for {hwnd}"
        )));
    };
    if width == 0 || height == 0 {
        return Err(Error::Invalid("window has no area".into()));
    }
    let width_i = width as i32;
    let height_i = height as i32;

    let screen = unsafe { GetDC(null_mut()) };
    if screen.is_null() {
        return Err(last_error("GetDC"));
    }
    let memory = unsafe { CreateCompatibleDC(screen) };
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width_i,
        biHeight: -height_i,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut bits: *mut c_void = null_mut();
    let bitmap =
        unsafe { CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe {
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
        }
        return Err(last_error("CreateDIBSection"));
    }
    let old = unsafe { SelectObject(memory, bitmap) };
    let printed = unsafe { PrintWindow(hwnd as usize as HWND, memory, PW_RENDERFULLCONTENT) };

    let byte_len = width as usize * height as usize * 4;
    let raw = unsafe { std::slice::from_raw_parts(bits as *const u8, byte_len) };
    let mut rgba = vec![0u8; byte_len];
    for (index, pixel) in raw.chunks_exact(4).enumerate() {
        let offset = index * 4;
        rgba[offset] = pixel[2];
        rgba[offset + 1] = pixel[1];
        rgba[offset + 2] = pixel[0];
        rgba[offset + 3] = 255;
    }

    unsafe {
        SelectObject(memory, old);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(null_mut(), screen);
    }

    if printed == 0 {
        return Err(Error::Invalid(format!(
            "PrintWindow failed for window {hwnd} ({}x{})",
            width, height
        )));
    }

    Ok(RawImage {
        width,
        height,
        rgba,
    })
}

pub fn capture_desktop(desktop: &DesktopHandle, width: u32, height: u32) -> Result<RawImage> {
    let mut canvas = RawImage {
        width,
        height,
        rgba: vec![0u8; width as usize * height as usize * 4],
    };
    for pixel in canvas.rgba.chunks_exact_mut(4) {
        pixel[0] = 24;
        pixel[1] = 26;
        pixel[2] = 32;
        pixel[3] = 255;
    }

    let mut windows = enumerate_desktop(desktop);
    windows.retain(WindowEntry::is_candidate);
    // EnumDesktopWindows walks top-to-bottom; composite bottom-up.
    for window in windows.iter().rev() {
        let Ok(image) = print_window_rgba(window.hwnd) else {
            continue;
        };
        blit(&mut canvas, &image, window.x, window.y);
    }
    Ok(canvas)
}

fn blit(canvas: &mut RawImage, image: &RawImage, dx: i32, dy: i32) {
    let canvas_w = canvas.width as i32;
    let canvas_h = canvas.height as i32;
    let image_w = image.width as i32;
    let image_h = image.height as i32;
    let src_x = (-dx).max(0);
    let src_y = (-dy).max(0);
    let dst_x = dx.max(0);
    let dst_y = dy.max(0);
    let copy_w = (image_w - src_x).min(canvas_w - dst_x);
    let copy_h = (image_h - src_y).min(canvas_h - dst_y);
    if copy_w <= 0 || copy_h <= 0 {
        return;
    }
    for row in 0..copy_h {
        let src_start = ((src_y + row) * image_w + src_x) as usize * 4;
        let dst_start = ((dst_y + row) * canvas_w + dst_x) as usize * 4;
        let count = copy_w as usize * 4;
        canvas.rgba[dst_start..dst_start + count]
            .copy_from_slice(&image.rgba[src_start..src_start + count]);
    }
}

/// Find the deepest child window at a screen point, returning its handle and
/// client-relative coordinates. Coordinate conversion is done from window
/// rectangles because ScreenToClient is unreliable across desktops.
pub fn child_at_point(hwnd: u64, screen_x: i32, screen_y: i32) -> Option<(u64, i32, i32)> {
    let mut children: Vec<(u64, i32, i32, u32, u32)> = Vec::new();
    unsafe {
        EnumChildWindows(
            hwnd as usize as HWND,
            Some(collect_child),
            &mut children as *mut _ as isize,
        );
    }
    let best = children
        .into_iter()
        .filter(|(_, x, y, width, height)| {
            screen_x >= *x
                && screen_y >= *y
                && screen_x < x + *width as i32
                && screen_y < y + *height as i32
        })
        .min_by_key(|(_, _, _, width, height)| *width * *height);
    if let Some((child, x, y, _, _)) = best {
        return Some((child, screen_x - x, screen_y - y));
    }

    let (x, y, width, height) = window_rect(hwnd)?;
    if screen_x < x || screen_y < y || screen_x >= x + width as i32 || screen_y >= y + height as i32
    {
        return None;
    }
    let (client_width, client_height) = client_size(hwnd).unwrap_or((width as i32, height as i32));
    let offset_x = ((width as i32 - client_width) / 2).max(0);
    let offset_y = ((height as i32 - client_height) - offset_x).max(0);
    Some((hwnd, screen_x - (x + offset_x), screen_y - (y + offset_y)))
}

unsafe extern "system" fn collect_child(hwnd: HWND, lparam: LPARAM) -> i32 {
    let children = unsafe { &mut *(lparam as *mut Vec<(u64, i32, i32, u32, u32)>) };
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    let mut rect: RECT = unsafe { zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } != 0 {
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        if width > 0 && height > 0 {
            children.push((hwnd as usize as u64, rect.left, rect.top, width, height));
        }
    }
    1
}

fn client_size(hwnd: u64) -> Option<(i32, i32)> {
    let mut rect: RECT = unsafe { zeroed() };
    if unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd as usize as HWND, &mut rect)
    } == 0
    {
        return None;
    }
    Some((rect.right - rect.left, rect.bottom - rect.top))
}

pub fn post_char(hwnd: u64, unit: u16) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd as usize as HWND,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
            unit as usize,
            0,
        );
    }
}

pub fn post_message(hwnd: u64, message: u32, wparam: usize, lparam: isize) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd as usize as HWND,
            message,
            wparam,
            lparam,
        );
    }
}

pub fn send_message(hwnd: u64, message: u32, wparam: usize, lparam: isize) -> bool {
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            hwnd as usize as HWND,
            message,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG,
            500,
            &mut result,
        ) != 0
    }
}

pub fn top_level(hwnd: u64) -> u64 {
    unsafe { GetAncestor(hwnd as usize as HWND, GA_ROOT) as usize as u64 }
}

/// Send a key chord from a thread attached to the runspace desktop, so the
/// target application sees the modifier state.
pub fn send_chord(desktop_name: &str, target: u64, modifiers: &[u32], key: u32) -> bool {
    let desktop_name = desktop_name.to_string();
    let modifiers = modifiers.to_vec();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let desktop = wide(&desktop_name);
        let handle = unsafe { OpenDesktopW(desktop.as_ptr(), 0, 0, DESKTOP_ALL_ACCESS) };
        if !handle.is_null() {
            unsafe {
                SetThreadDesktop(handle);
            }
        }

        let mut thread_id = 0u32;
        unsafe {
            GetWindowThreadProcessId(target as usize as HWND, &mut thread_id);
        }
        let current = unsafe { GetCurrentThreadId() };
        let attached = thread_id != 0
            && unsafe {
                windows_sys::Win32::System::Threading::AttachThreadInput(current, thread_id, 1)
            } != 0;

        if attached {
            let mut state = [0u8; 256];
            unsafe {
                GetKeyboardState(state.as_mut_ptr());
            }
            for modifier in &modifiers {
                state[*modifier as usize] = 0x80;
            }
            unsafe {
                SetKeyboardState(state.as_ptr());
            }
        }

        for modifier in &modifiers {
            send_message(target, WM_KEYDOWN, *modifier as usize, 1);
        }
        send_message(target, WM_KEYDOWN, key as usize, 1);
        send_message(target, WM_KEYUP, key as usize, 0xC000_0001u32 as isize);
        for modifier in modifiers.iter().rev() {
            send_message(
                target,
                WM_KEYUP,
                *modifier as usize,
                0xC000_0001u32 as isize,
            );
        }

        if attached {
            let empty = [0u8; 256];
            unsafe {
                SetKeyboardState(empty.as_ptr());
                windows_sys::Win32::System::Threading::AttachThreadInput(current, thread_id, 0);
            }
        }
        if !handle.is_null() {
            unsafe {
                CloseDesktop(handle);
            }
        }
        let _ = sender.send(attached);
    });

    receiver
        .recv_timeout(std::time::Duration::from_millis(500))
        .unwrap_or(false)
}

pub fn make_lparam(x: i32, y: i32) -> isize {
    (((y as u32 & 0xffff) << 16) | (x as u32 & 0xffff)) as isize
}

pub fn bring_to_front(hwnd: u64) {
    unsafe {
        SetWindowPos(
            hwnd as usize as HWND,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn show_host_window(hwnd: u64) {
    unsafe {
        ShowWindow(hwnd as usize as HWND, SW_RESTORE);
        ShowWindow(hwnd as usize as HWND, SW_SHOW);
        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd as usize as HWND);
        SetWindowPos(
            hwnd as usize as HWND,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn find_host_window(title: &str) -> Option<u64> {
    enumerate_host()
        .into_iter()
        .find(|window| window.title == title)
        .map(|window| window.hwnd)
}

pub fn set_window_rect(hwnd: u64, x: i32, y: i32, width: u32, height: u32) {
    unsafe {
        SetWindowPos(
            hwnd as usize as HWND,
            null_mut(),
            x,
            y,
            width as i32,
            height as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn job_name(id: &str) -> String {
    format!("Local\\RunspaceJob-{id}")
}

pub fn desktop_name(id: &str) -> String {
    format!("Runspace-{id}")
}

pub fn viewer_title(id: &str) -> String {
    format!("Runspace {id}")
}

pub fn watchdog_event_name(id: &str) -> String {
    format!("Local\\RunspaceWatch-{id}")
}

pub fn create_watchdog_event(id: &str) -> Result<HANDLE> {
    let name = wide(watchdog_event_name(id));
    let handle = unsafe { CreateEventW(null(), 1, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(last_error("CreateEventW"));
    }
    Ok(handle)
}

pub fn open_watchdog_event(id: &str, modify: bool) -> Option<HANDLE> {
    let name = wide(watchdog_event_name(id));
    let access = if modify {
        EVENT_MODIFY_STATE
    } else {
        SYNCHRONIZE_ACCESS
    };
    let handle = unsafe { OpenEventW(access, 0, name.as_ptr()) };
    (!handle.is_null()).then_some(handle)
}

pub fn signal_watchdog_event(id: &str) -> bool {
    let Some(handle) = open_watchdog_event(id, true) else {
        return false;
    };
    unsafe {
        SetEvent(handle);
        CloseHandle(handle);
    }
    true
}

pub fn watchdog_event_signaled(id: &str) -> bool {
    let Some(handle) = open_watchdog_event(id, false) else {
        return false;
    };
    let signaled = unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 };
    unsafe {
        CloseHandle(handle);
    }
    signaled
}

pub fn host_display_available() -> bool {
    unsafe { GetSystemMetrics(0) > 0 }
}

pub fn create_job(name: &str) -> Result<HANDLE> {
    let name = wide(name);
    let job = unsafe { CreateJobObjectW(null(), name.as_ptr()) };
    if job.is_null() {
        return Err(last_error("CreateJobObjectW"));
    }
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        unsafe {
            CloseHandle(job);
        }
        return Err(last_error("SetInformationJobObject"));
    }
    Ok(job)
}

pub fn open_job(name: &str) -> Result<HANDLE> {
    let name = wide(name);
    let job = unsafe { OpenJobObjectW(JOB_OBJECT_ALL_ACCESS, 0, name.as_ptr()) };
    if job.is_null() {
        return Err(last_error("OpenJobObjectW"));
    }
    Ok(job)
}

pub fn terminate_job(name: &str) -> bool {
    let Ok(job) = open_job(name) else {
        return false;
    };
    let ok = unsafe { TerminateJobObject(job, 1) } != 0;
    unsafe {
        CloseHandle(job);
    }
    ok
}

pub fn job_pids(name: &str) -> Vec<u32> {
    let Ok(job) = open_job(name) else {
        return Vec::new();
    };
    let mut buffer = vec![0u8; 64 * 1024];
    let mut returned = 0u32;
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            buffer.as_mut_ptr() as *mut c_void,
            buffer.len() as u32,
            &mut returned,
        )
    };
    unsafe {
        CloseHandle(job);
    }
    if ok == 0 {
        return Vec::new();
    }
    let list = unsafe { &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
    let count = list.NumberOfProcessIdsInList as usize;
    let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
    ids.iter().map(|pid| *pid as u32).collect()
}

pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let status = unsafe { WaitForSingleObject(handle, 0) };
    unsafe {
        CloseHandle(handle);
    }
    status == WAIT_TIMEOUT
}

pub fn process_rss(pid: u32) -> u64 {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return 0;
    }
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { zeroed() };
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        0
    } else {
        counters.WorkingSetSize as u64
    }
}

pub fn terminate_pid(pid: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return false;
    }
    let ok = unsafe { TerminateProcess(handle, 1) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    ok
}

pub fn wait_pid(pid: u32, milliseconds: u32) -> Option<u32> {
    let handle = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let status = unsafe { WaitForSingleObject(handle, milliseconds) };
    let mut code = 0u32;
    let result = if status == WAIT_OBJECT_0 {
        unsafe {
            GetExitCodeProcess(handle, &mut code);
        }
        Some(code)
    } else {
        None
    };
    unsafe {
        CloseHandle(handle);
    }
    result
}

pub fn quote_arg(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|ch| ch == ' ' || ch == '\t' || ch == '"')
    {
        return argument.to_string();
    }
    let mut result = String::from("\"");
    let mut backslashes = 0usize;
    for ch in argument.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                result.push_str(&"\\".repeat(backslashes * 2 + 1));
                result.push('"');
                backslashes = 0;
            }
            _ => {
                result.push_str(&"\\".repeat(backslashes));
                result.push(ch);
                backslashes = 0;
            }
        }
    }
    result.push_str(&"\\".repeat(backslashes * 2));
    result.push('"');
    result
}

pub struct SpawnRequest<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub desktop: Option<&'a str>,
    pub job: Option<&'a str>,
    pub cwd: Option<&'a Path>,
}

pub struct ChildProcess {
    pub pid: u32,
    process: HANDLE,
    thread: HANDLE,
}

impl ChildProcess {
    pub fn wait(&self, milliseconds: u32) -> Option<u32> {
        let status = unsafe { WaitForSingleObject(self.process, milliseconds) };
        if status != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0u32;
        unsafe {
            GetExitCodeProcess(self.process, &mut code);
        }
        Some(code)
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

pub fn spawn_process(request: &SpawnRequest<'_>) -> Result<ChildProcess> {
    let program_wide = wide(request.program);
    let mut command_line = quote_arg(&request.program.display().to_string());
    for argument in request.args {
        command_line.push(' ');
        command_line.push_str(&quote_arg(argument));
    }
    let mut command_line = wide(command_line);
    let desktop_wide = request.desktop.map(wide);
    let cwd_wide = request.cwd.map(wide);

    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    if let Some(desktop) = &desktop_wide {
        startup.lpDesktop = desktop.as_ptr() as *mut u16;
    }

    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessW(
            program_wide.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NO_WINDOW | CREATE_SUSPENDED,
            null(),
            cwd_wide
                .as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(null()),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessW"));
    }

    if let Some(job_name) = request.job {
        let job = match open_job(job_name) {
            Ok(job) => job,
            Err(error) => {
                unsafe {
                    TerminateProcess(process.hProcess, 1);
                    CloseHandle(process.hThread);
                    CloseHandle(process.hProcess);
                }
                return Err(error);
            }
        };
        let assigned = unsafe { AssignProcessToJobObject(job, process.hProcess) };
        unsafe {
            CloseHandle(job);
        }
        if assigned == 0 {
            unsafe {
                TerminateProcess(process.hProcess, 1);
                CloseHandle(process.hThread);
                CloseHandle(process.hProcess);
            }
            return Err(last_error("AssignProcessToJobObject"));
        }
    }

    unsafe {
        ResumeThread(process.hThread);
    }

    Ok(ChildProcess {
        pid: process.dwProcessId,
        process: process.hProcess,
        thread: process.hThread,
    })
}

pub fn wait_for_desktop(name: &str, timeout_ms: u32) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
    loop {
        if let Ok(handle) = open_desktop(name) {
            drop(handle);
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

pub fn unique_pids(pids: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut seen = HashSet::new();
    pids.into_iter()
        .filter(|pid| *pid != 0 && seen.insert(*pid))
        .collect()
}
