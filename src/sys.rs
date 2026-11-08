//! Small Windows helpers: time, battery, fullscreen detection, autostart, volume keys, sounds.
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Power::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Time::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct Now {
    pub year: i32,
    pub month: u32, // 1-12
    pub day: u32,
    pub dow: u32, // 0 = Sunday
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub ms: u32,
}

impl Now {
    pub fn minutes(&self) -> u32 {
        self.hour * 60 + self.min
    }
    pub fn ymd(&self) -> u32 {
        (self.year as u32) * 10000 + self.month * 100 + self.day
    }
}

pub fn now() -> Now {
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetLocalTime();
        Now {
            year: st.wYear as i32,
            month: st.wMonth as u32,
            day: st.wDay as u32,
            dow: st.wDayOfWeek as u32,
            hour: st.wHour as u32,
            min: st.wMinute as u32,
            sec: st.wSecond as u32,
            ms: st.wMilliseconds as u32,
        }
    }
}

pub const DOW: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MON: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
pub const MON_FULL: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// Day of week (0 = Sunday) for a Gregorian date.
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    ((yy + yy / 4 - yy / 100 + yy / 400 + t[(m - 1) as usize] + d as i32) % 7) as u32
}

/// Days since 1970-01-01 (civil-from-days algorithm).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

#[derive(Clone, Copy, Debug)]
pub struct Battery {
    pub percent: u32,
    pub charging: bool,
    pub ac: bool,
    pub present: bool,
}

pub fn battery() -> Battery {
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut s).is_ok() {
            let present = s.BatteryFlag != 128 && s.BatteryLifePercent != 255;
            return Battery {
                percent: if s.BatteryLifePercent == 255 { 100 } else { s.BatteryLifePercent as u32 },
                charging: (s.BatteryFlag & 8) != 0 || (s.ACLineStatus == 1 && present && s.BatteryLifePercent < 100),
                ac: s.ACLineStatus == 1,
                present,
            };
        }
    }
    Battery { percent: 100, charging: false, ac: true, present: false }
}

/// True only when a game / full-screen video / slideshow owns the whole of the notch's monitor.
///
/// The shell's own state flag (`SHQueryUserNotificationState`) is a hint, not the verdict: it is also
/// raised for things that should never hide the notch. So the foreground window itself must cover the
/// notch's monitor edge to edge, and must not be the desktop, the taskbar or one of our own windows.
/// A merely maximised window never qualifies (it has a title bar and stops at the taskbar).
pub fn fullscreen_busy(own: HWND, notch_monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> bool {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || fg == own || !IsWindowVisible(fg).as_bool() || IsIconic(fg).as_bool() {
            return false;
        }
        // desktop / taskbar / our own process never count
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls) as usize;
        let cls = String::from_utf16_lossy(&cls[..n]);
        if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NookNotchWindow" | "NookSettings") {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == std::process::id() {
            return false;
        }
        // must be on the monitor the notch lives on
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if mon != notch_monitor {
            return false;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let m = mi.rcMonitor;
        let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        if !covers {
            return false;
        }
        let state = SHQueryUserNotificationState().ok();
        let shell_says_busy = matches!(state, Some(s) if s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE);
        // borderless / exclusive full screen has no title bar; a title bar means "just maximised"
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        let has_caption = style & WS_CAPTION.0 == WS_CAPTION.0;
        shell_says_busy || !has_caption
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let v: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
                let _ = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key,
                    w!("Nook"),
                    REG_SZ.0,
                    Some(v.as_ptr() as *const _),
                    (v.len() * 2) as u32,
                );
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("Nook"));
        }
    }
}

pub fn key_tap(vk: VIRTUAL_KEY) {
    unsafe {
        let mk = |flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [mk(KEYBD_EVENT_FLAGS(0)), mk(KEYEVENTF_KEYUP)];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

pub fn chime(kind: u32) {
    // Short system chimes without shipping audio assets.
    std::thread::spawn(move || unsafe {
        let _ = kind;
        let _ = windows::Win32::Media::Audio::PlaySoundW(w!("SystemNotification"), None, windows::Win32::Media::Audio::SND_ALIAS | windows::Win32::Media::Audio::SND_ASYNC);
    });
}

pub fn open_url(url: &str) {
    unsafe {
        let u: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

pub fn open_path(path: &std::path::Path) {
    unsafe {
        let u: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

pub fn keyboard_layout_name() -> String {
    unsafe {
        let fg = GetForegroundWindow();
        let tid = windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let lang = (hkl.0 as usize & 0xFFFF) as u16;
        match lang & 0x3FF {
            0x09 => "EN",
            0x21 => "ID",
            0x01 => "AR",
            0x0C => "FR",
            0x07 => "DE",
            0x0A => "ES",
            0x11 => "JA",
            0x12 => "KO",
            0x04 => "ZH",
            0x19 => "RU",
            0x16 => "PT",
            0x10 => "IT",
            0x1F => "TR",
            _ => "??",
        }
        .to_string()
    }
}

pub fn caps_on() -> bool {
    unsafe { (GetKeyState(VK_CAPITAL.0 as i32) & 1) != 0 }
}

/// Local UTC offset in minutes (for prayer maths when the city table lacks one).
pub fn utc_offset_minutes() -> i32 {
    unsafe {
        let mut tz = TIME_ZONE_INFORMATION::default();
        let r = GetTimeZoneInformation(&mut tz);
        let bias = tz.Bias + if r == 2 { tz.DaylightBias } else { tz.StandardBias };
        -bias
    }
}

pub fn log(msg: &str) {
    use std::io::Write;
    let p = std::env::temp_dir().join("nook.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "{msg}");
    }
}

// ---------------------------------------------------------------- audio
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

fn endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// (level 0..1, muted)
pub fn volume() -> Option<(f32, bool)> {
    unsafe {
        let v = endpoint()?;
        Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool()))
    }
}

pub fn set_volume(level: f32) {
    unsafe {
        if let Some(v) = endpoint() {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
        }
    }
}


// ---------------------------------------------------------------- brightness (laptop panels, via WMI)
/// Current panel brightness 0..100, or -1 when the PC has no controllable built-in display.
pub static BRIGHT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

pub fn spawn_brightness_poller() {
    std::thread::spawn(|| unsafe {
        use windows::Win32::System::Com::*;
        use windows::Win32::System::Wmi::*;
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let svc: Option<IWbemServices> = (|| {
            let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
            let svc = loc.ConnectServer(&BSTR::from("ROOT\\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None).ok()?;
            let _ = CoSetProxyBlanket(&svc, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE);
            Some(svc)
        })();
        let Some(svc) = svc else { return };
        loop {
            let v = (|| -> Option<i32> {
                let q = svc
                    .ExecQuery(&BSTR::from("WQL"), &BSTR::from("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE"), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None)
                    .ok()?;
                let mut objs: [Option<IWbemClassObject>; 1] = [None];
                let mut ret = 0u32;
                let _ = q.Next(2000, &mut objs, &mut ret);
                let obj = objs[0].take()?;
                let mut var = windows::core::VARIANT::default();
                obj.Get(w!("CurrentBrightness"), 0, &mut var, None, None).ok()?;
                i32::try_from(&var).ok()
            })();
            BRIGHT.store(v.unwrap_or(-1), std::sync::atomic::Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(600));
            if v.is_none() {
                std::thread::sleep(std::time::Duration::from_secs(10)); // no panel brightness here: check rarely
            }
        }
    });
}

#[cfg(test)]
mod brightness_tests {
    #[test]
    fn brightness_poller_runs() {
        super::spawn_brightness_poller();
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let b = super::BRIGHT.load(std::sync::atomic::Ordering::Relaxed);
        println!("BRIGHTNESS {b}");
        assert!((-1..=100).contains(&b));
    }
}
