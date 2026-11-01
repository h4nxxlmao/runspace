use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, COLOR_WINDOW, CreateSolidBrush,
    DIB_RGB_COLORS, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT, SRCCOPY,
    StretchDIBits, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetMessageW, GetSystemMetrics, IDC_ARROW, KillTimer, LoadCursorW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNORMAL, SetTimer, SetWindowPos,
    ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

use crate::Result;
use crate::core::paths::Paths;
use crate::core::state;

use super::win32::{self, DesktopHandle};

const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
const TIMER_ID: usize = 1;

struct ViewerContext {
    id: String,
    paths: Paths,
    desktop: DesktopHandle,
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

thread_local! {
    static CONTEXT: RefCell<Option<ViewerContext>> = const { RefCell::new(None) };
}

pub fn run(id: &str) -> Result<i32> {
    let paths = Paths::discover()?;
    let record = state::load(&paths, id)?;
    let desktop_name = record
        .desktop
        .clone()
        .unwrap_or_else(|| win32::desktop_name(id));
    let desktop = win32::open_desktop(&desktop_name)?;
    let width = u32::from(record.width);
    let height = u32::from(record.height);
    let raw = win32::capture_desktop(&desktop, width, height)?;
    let bgra = to_bgra(raw.rgba);

    CONTEXT.with(|context| {
        *context.borrow_mut() = Some(ViewerContext {
            id: id.to_string(),
            paths: paths.clone(),
            desktop,
            width,
            height,
            bgra,
        });
    });

    let instance = unsafe { GetModuleHandleW(null()) };
    let class_name = win32::wide("RunspaceViewerWindow");
    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as usize as *mut c_void,
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
    };
    unsafe {
        RegisterClassW(&window_class);
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
    };
    unsafe {
        AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0);
    }
    let window_width = (rect.right - rect.left).max(200);
    let window_height = (rect.bottom - rect.top).max(150);
    let screen_width = unsafe { GetSystemMetrics(0) };
    let screen_height = unsafe { GetSystemMetrics(1) };
    let x = ((screen_width - window_width) / 2).max(0);
    let y = ((screen_height - window_height) / 2).max(0);
    let title = win32::wide(win32::viewer_title(id));

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            window_width,
            window_height,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        )
    };
    if hwnd.is_null() {
        return Err(win32::last_error("CreateWindowExW"));
    }

    unsafe {
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);
        SetTimer(hwnd, TIMER_ID, 200, None);
    }

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

    CONTEXT.with(|context| {
        let mut context = context.borrow_mut();
        *context = None;
    });
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
            let alive = CONTEXT.with(|context| {
                let mut context = context.borrow_mut();
                let Some(context) = context.as_mut() else {
                    return false;
                };
                let Ok(record) = state::load(&context.paths, &context.id) else {
                    return false;
                };
                let width = u32::from(record.width);
                let height = u32::from(record.height);
                if width != context.width || height != context.height {
                    context.width = width;
                    context.height = height;
                    let mut rect = RECT {
                        left: 0,
                        top: 0,
                        right: width as i32,
                        bottom: height as i32,
                    };
                    unsafe {
                        AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, 0);
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            0,
                            0,
                            rect.right - rect.left,
                            rect.bottom - rect.top,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                let Ok(raw) = win32::capture_desktop(&context.desktop, width, height) else {
                    return true;
                };
                context.bgra = to_bgra(raw.rgba);
                true
            });
            if !alive {
                unsafe {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            } else {
                unsafe {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = unsafe { zeroed() };
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut client: RECT = unsafe { zeroed() };
            unsafe {
                GetClientRect(hwnd, &mut client);
            }
            let client_width = (client.right - client.left).max(1);
            let client_height = (client.bottom - client.top).max(1);
            let drawn = CONTEXT.with(|context| {
                let context = context.borrow();
                let Some(context) = context.as_ref() else {
                    return false;
                };
                if context.bgra.is_empty() || context.width == 0 || context.height == 0 {
                    return false;
                }
                let scale_x = client_width as f64 / context.width as f64;
                let scale_y = client_height as f64 / context.height as f64;
                let scale = scale_x.min(scale_y);
                let draw_width = (context.width as f64 * scale).max(1.0) as i32;
                let draw_height = (context.height as f64 * scale).max(1.0) as i32;
                let draw_x = (client_width - draw_width) / 2;
                let draw_y = (client_height - draw_height) / 2;
                let mut info: BITMAPINFO = unsafe { zeroed() };
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: context.width as i32,
                    biHeight: -(context.height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                };
                unsafe {
                    StretchDIBits(
                        hdc,
                        draw_x,
                        draw_y,
                        draw_width,
                        draw_height,
                        0,
                        0,
                        context.width as i32,
                        context.height as i32,
                        context.bgra.as_ptr() as *const c_void,
                        &info,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                }
                true
            });
            if !drawn {
                let brush = unsafe { CreateSolidBrush(0x0020_1A18) };
                unsafe {
                    FillRect(hdc, &client, brush);
                    DeleteObject(brush);
                }
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
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
                KillTimer(hwnd, TIMER_ID);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn to_bgra(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    rgba
}
