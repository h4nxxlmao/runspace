#![allow(non_snake_case)]

use std::cell::Cell;
use std::ffi::c_void;
use std::io::Write;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_LEFT, ES_MULTILINE, GetMessageW, GetWindowRect,
    GetWindowTextW, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW,
    SW_SHOWNORMAL, SendMessageW, SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use crate::Result;

const WM_GEOMETRY_TIMER: usize = 1;
const BUTTON_ID: usize = 100;
const EDIT_ID: usize = 101;
const LABEL_ID: usize = 102;
const WM_MOUSEMOVE_MSG: u32 = 0x0200;
const WM_LBUTTONDOWN_MSG: u32 = 0x0201;
const WM_LBUTTONUP_MSG: u32 = 0x0202;
const WM_MOUSEWHEEL_MSG: u32 = 0x020A;
const MK_LBUTTON: usize = 0x0001;

static LOG_PATH: OnceLock<String> = OnceLock::new();

thread_local! {
    static COUNT: Cell<i32> = const { Cell::new(0) };
    static EDIT: Cell<isize> = const { Cell::new(0) };
    static LABEL: Cell<isize> = const { Cell::new(0) };
}

pub fn run(log_path: &str) -> Result<i32> {
    let _ = LOG_PATH.set(log_path.to_string());
    let instance = unsafe { GetModuleHandleW(null()) };
    let class_name = crate::platform::windows::win32::wide("RunspaceTestWindow");
    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as *mut c_void,
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };
    unsafe {
        RegisterClassW(&window_class);
    }

    let title = crate::platform::windows::win32::wide("Runspace Test App");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            100,
            100,
            460,
            300,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        )
    };
    if hwnd.is_null() {
        return Err(crate::Error::invalid("could not create the test window"));
    }

    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) as usize };
    let edit_class = crate::platform::windows::win32::wide("EDIT");
    let edit = unsafe {
        CreateWindowExW(
            0,
            edit_class.as_ptr(),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WS_TABSTOP
                | ES_LEFT as u32
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_AUTOHSCROLL as u32,
            20,
            20,
            400,
            100,
            hwnd,
            EDIT_ID as *mut c_void,
            instance,
            null_mut(),
        )
    };
    let static_class = crate::platform::windows::win32::wide("STATIC");
    let label = unsafe {
        CreateWindowExW(
            0,
            static_class.as_ptr(),
            crate::platform::windows::win32::wide("Count: 0").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            140,
            180,
            30,
            hwnd,
            LABEL_ID as *mut c_void,
            instance,
            null_mut(),
        )
    };
    let button_class = crate::platform::windows::win32::wide("BUTTON");
    let button = unsafe {
        CreateWindowExW(
            0,
            button_class.as_ptr(),
            crate::platform::windows::win32::wide("Increment").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            220,
            135,
            200,
            40,
            hwnd,
            BUTTON_ID as *mut c_void,
            instance,
            null_mut(),
        )
    };
    for control in [edit, label, button] {
        unsafe {
            SendMessageW(control, WM_SETFONT, font, 1);
        }
    }
    EDIT.with(|cell| cell.set(edit as isize));
    LABEL.with(|cell| cell.set(label as isize));

    unsafe {
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);
        SetTimer(hwnd, WM_GEOMETRY_TIMER, 300, None);
    }
    log_line("testapp started");

    let mut message: MSG = unsafe { zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    log_line("testapp exited");
    Ok(0)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER => {
            if wparam == WM_GEOMETRY_TIMER {
                unsafe {
                    KillTimer(hwnd, WM_GEOMETRY_TIMER);
                }
                log_geometry(hwnd);
            }
            0
        }
        WM_COMMAND => {
            let id = wparam & 0xffff;
            let code = ((wparam >> 16) & 0xffff) as u32;
            if id == BUTTON_ID && code == 0 {
                let count = COUNT.with(|cell| {
                    let value = cell.get() + 1;
                    cell.set(value);
                    value
                });
                let label = LABEL.with(|cell| cell.get()) as HWND;
                if !label.is_null() {
                    let text = crate::platform::windows::win32::wide(format!("Count: {count}"));
                    unsafe {
                        SetWindowTextW(label, text.as_ptr());
                    }
                }
                let edit = EDIT.with(|cell| cell.get()) as HWND;
                let text = window_text(edit);
                log_line(&format!("button clicked count={count} edit={text:?}"));
            }
            0
        }
        WM_MOUSEMOVE_MSG => {
            if wparam & MK_LBUTTON != 0 {
                log_line("drag move");
            }
            0
        }
        WM_LBUTTONDOWN_MSG => {
            log_line("mouse down");
            0
        }
        WM_LBUTTONUP_MSG => {
            log_line("mouse up");
            0
        }
        WM_MOUSEWHEEL_MSG => {
            let delta = ((wparam >> 16) & 0xffff) as u16 as i16;
            log_line(&format!("wheel delta={delta}"));
            0
        }
        WM_CLOSE => {
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn log_geometry(hwnd: HWND) {
    let mut rect: RECT = unsafe { zeroed() };
    unsafe {
        GetWindowRect(hwnd, &mut rect);
    }
    log_line(&format!(
        "geometry window={},{} size={}x{}",
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top
    ));
    let edit = EDIT.with(|cell| cell.get()) as HWND;
    let button =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetDlgItem(hwnd, BUTTON_ID as i32) };
    if !edit.is_null() {
        let mut rect: RECT = unsafe { zeroed() };
        unsafe {
            GetWindowRect(edit, &mut rect);
        }
        log_line(&format!(
            "edit_center x={} y={}",
            (rect.left + rect.right) / 2,
            (rect.top + rect.bottom) / 2
        ));
    }
    if !button.is_null() {
        let mut rect: RECT = unsafe { zeroed() };
        unsafe {
            GetWindowRect(button, &mut rect);
        }
        log_line(&format!(
            "button_center x={} y={}",
            (rect.left + rect.right) / 2,
            (rect.top + rect.bottom) / 2
        ));
    }
}

fn window_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn log_line(message: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}
