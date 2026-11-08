#![windows_subsystem = "windows"]

mod app;
mod assistant;
mod chat;
mod config;
mod gfx;
mod lyrics;
mod media;
mod netmon;
mod pages;
mod prayer;
mod settings_ui;
mod spring;
mod store;
mod sys;
mod tour;
mod update;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|info| {
        sys::log(&format!("PANIC: {info}"));
    }));
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = OleInitialize(None);

        // single instance
        // After a self-update the new process starts while the old one is still shutting down: wait for its lock.
        let updated = std::env::args().any(|a| a == "--updated" || a == "--restarted");
        let mut tries = 0;
        let mtx = loop {
            let m = CreateMutexW(None, true, w!("Local\\NookNotch.SingleInstance"))?;
            if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(m);
                if updated && tries < 80 {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                return Ok(());
            }
            break m;
        };
        let _keep = mtx;

        let hinst = GetModuleHandleW(None)?;
        let icon = LoadIconW(hinst, PCWSTR(1 as _)).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let class = w!("NookNotchWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(app::wndproc),
            hInstance: hinst.into(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let (x, y, wpx, hpx, scale) = app::window_geometry(0);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_ACCEPTFILES,
            class,
            w!("Nook"),
            WS_POPUP,
            x,
            y,
            wpx,
            hpx,
            None,
            None,
            hinst,
            None,
        )?;

        let app = app::App::new(hwnd, scale)?;
        app::install(hwnd, app);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if std::env::var("NOOK_DEBUG_SETTINGS").is_ok() {
            settings_ui::open();
        }

        app::run_loop(hwnd);
    }
    Ok(())
}
