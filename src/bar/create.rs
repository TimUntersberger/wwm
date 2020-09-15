use super::{get_bar_by_hmonitor, get_windows, redraw::redraw, window_cb, Bar, BARS};
use crate::{event::Event, message_loop, util, CHANNEL, CONFIG, DISPLAYS};
use log::{debug, error, info};
use winapi::shared::minwindef::HINSTANCE;
use winapi::shared::windef::{HBRUSH, HWND, RECT};
use winapi::um::shellapi::{
    SHAppBarMessage, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_SETPOS, APPBARDATA,
};
use winapi::um::wingdi::CreateSolidBrush;
use winapi::um::winuser::{
    GetWindowRect, MoveWindow, RegisterClassA, ShowWindow, SW_SHOW, WNDCLASSA,
};

pub fn create() -> Result<(), util::WinApiResultError> {
    info!("Creating appbar");

    let name = "nog_bar";

    let app_bar_bg = CONFIG.lock().bar.color;
    let height = CONFIG.lock().bar.height;

    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(200));

        if get_windows().is_empty() {
            break;
        }

        CHANNEL
            .sender
            .clone()
            .send(Event::RedrawAppBar)
            .expect("Failed to send redraw-app-bar event");
    });

    for display in DISPLAYS.lock().clone() {
        std::thread::spawn(move || unsafe {
            if get_bar_by_hmonitor(display.hmonitor as i32).is_some() {
                error!(
                    "Appbar for monitor {} already exists. Aborting",
                    display.hmonitor as i32
                );
            }

            debug!("Creating appbar for display {}", display.hmonitor as i32);

            let working_area_width = display.working_area_width();

            let instance = winapi::um::libloaderapi::GetModuleHandleA(std::ptr::null_mut());

            let background_brush = CreateSolidBrush(app_bar_bg as u32);

            let class = WNDCLASSA {
                hInstance: instance as HINSTANCE,
                lpszClassName: name.as_ptr() as *const i8,
                lpfnWndProc: Some(window_cb),
                hbrBackground: background_brush as HBRUSH,
                ..WNDCLASSA::default()
            };

            RegisterClassA(&class);

            let window_handle = winapi::um::winuser::CreateWindowExA(
                winapi::um::winuser::WS_EX_NOACTIVATE | winapi::um::winuser::WS_EX_TOPMOST,
                name.as_ptr() as *const i8,
                name.as_ptr() as *const i8,
                winapi::um::winuser::WS_POPUPWINDOW & !winapi::um::winuser::WS_BORDER,
                display.working_area_left(),
                display.working_area_top(),
                working_area_width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance as HINSTANCE,
                std::ptr::null_mut(),
            );

            let mut bar = Bar::default();

            bar.hmonitor = display.hmonitor as i32;
            bar.window.id = window_handle as i32;

            BARS.lock().push(bar);

            ShowWindow(window_handle, SW_SHOW);
            redraw();

            let mut appbar_data: APPBARDATA = APPBARDATA {
                cbSize: 4 + 4 + 4 + 4 + 16 + 4,
                hWnd: window_handle as HWND,
                uCallbackMessage: window_handle as u32,
                uEdge: ABE_TOP,
                ..Default::default()
            };

            SHAppBarMessage(ABM_NEW, &mut appbar_data as *mut APPBARDATA);

            GetWindowRect(appbar_data.hWnd as HWND, &mut appbar_data.rc as *mut RECT);

            SHAppBarMessage(ABM_QUERYPOS, &mut appbar_data as *mut APPBARDATA);

            SHAppBarMessage(ABM_SETPOS, &mut appbar_data as *mut APPBARDATA);

            MoveWindow(
                appbar_data.hWnd as HWND,
                appbar_data.rc.left,
                appbar_data.rc.top,
                appbar_data.rc.right - appbar_data.rc.left,
                appbar_data.rc.bottom - appbar_data.rc.top,
                true as i32,
            );

            message_loop::start(|_| true);
        });
    }

    Ok(())
}
