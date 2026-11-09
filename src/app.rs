//! The notch window: lifecycle, animation state machine, input, and the shared chrome
//! (silhouette, header clock/ring, page dots). Page bodies live in `pages.rs`.
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Settings;
use crate::gfx::{Align, Color, Gfx};
use crate::lyrics::LyricsHandle;
use crate::media::{self, MediaCmd, MediaHandle};
use crate::netmon::{self, NetEvent, NetMon};
use crate::spring::*;
use crate::store::{Phase, Store};
use crate::sys;
use crate::update::{self, UpdateHandle, UpdateState};

pub const WIN_W: f32 = 740.0;
pub const WIN_H: f32 = 360.0;
pub const EXP_W: f32 = 456.0;
pub const EXP_H: f32 = 154.0;
/// Pages are laid out on a 420 px canvas; this centres that canvas inside the wider panel so the
/// side arrows have their own gutter and never sit on top of page content.
pub const PAGE_PAD: f32 = 18.0;
/// Height of the always-visible page dock at the bottom of the panel.
pub const DOCK_H: f32 = 30.0;
const SPLIT_MAIN: f32 = 150.0;
const SPLIT_GAP: f32 = 9.0;

pub const WM_TRAY: u32 = WM_APP + 1;
pub const WM_HUD: u32 = WM_APP + 2;
pub const WM_CLIPBOARDUPDATE_: u32 = 0x031D;

pub const BLACK: Color = Color::rgb(0, 0, 0);
pub const WHITE: Color = Color::rgb(255, 255, 255);
pub const GREEN: Color = Color::rgb(48, 209, 88);
pub const AMBER: Color = Color::rgb(255, 159, 10);
pub const RED: Color = Color::rgb(255, 69, 58);
pub const BLUE: Color = Color::rgb(10, 132, 255);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    PageLeft,
    PageRight,
    WaterCfg,
    WaterToggle,
    WaterBottle(i32),
    WaterGoal(u32),
    WaterFrom(i32),
    WaterTo(i32),
    WaterReset,
    TourNext,
    TourBack,
    TourSkip,
    PrayOpen,
    PrayCountry(usize),
    PrayPick(usize, usize),
    PrayWorld(usize, usize),
    PrayBack,
    WhatsNewClose,
    VolSlider,
    ChatFocus,
    ChatMic,
    ChatSend,
    ChatClear,
    Dot(usize),
    PlayPause,
    Prev,
    Next,
    Seek,
    Lyrics,
    Source,
    Volume,
    Drink,
    Undo,
    TodoToggle(usize),
    TodoDelete(usize),
    TodoAdd,
    TodoSelect(usize),
    TodoEdit,
    FocusToggle,
    FocusReset,
    FocusSkip,
    ShelfOpen(usize),
    ShelfRemove(usize),
    ShelfClear,
    ClipCopy(usize),
    CalPrev,
    CalNext,
    CalToday,
    ToastA,
    ToastB,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageKind {
    Media,
    Assistant,
    Widgets,
    Calendar,
    Focus,
    Water,
    Todo,
    Prayer,
    Shelf,
}

#[derive(Clone)]
pub struct Toast {
    pub icon: char,
    pub color: Color,
    pub title: String,
    pub body: String,
    pub a: Option<(String, Act)>,
    pub b: Option<(String, Act)>,
    pub until: Instant,
    pub tag: u32,
    pub idx: usize,
}

pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub scale: f32,
    pub win_px: (i32, i32),
    pub settings: Settings,
    pub store: Store,
    pub media: MediaHandle,
    pub lyr: LyricsHandle,
    // animation
    pub w: Spring,
    pub h: Spring,
    pub y_off: Spring,
    pub page_pos: Spring,
    pub lyric_slide: Spring,
    pub expanded: bool,
    pub hover_since: Option<Instant>,
    pub leave_since: Option<Instant>,
    pub last: Instant,
    pub start: Instant,
    pub page: usize,
    pub vis: Ease,
    pub bars: [f32; 5],
    // input
    pub mouse: (f32, f32),
    pub mouse_in: bool,
    pub hits: RefCell<Vec<(Rect, Act)>>,
    pub seek_drag: Option<f32>,
    pub last_wheel: Instant,
    // media view
    pub art: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>,
    pub art_ver: u64,
    pub lyrics_view: bool,
    pub lyric_idx: Option<usize>,
    pub track_ver: u64,
    // misc state
    pub toast: Option<Toast>,
    pub toast_q: Vec<Toast>,
    pub battery: sys::Battery,
    pub battery_at: Instant,
    pub hidden: bool,
    pub fs_check: Instant,
    pub last_render: Instant,
    pub dirty: Cell<bool>,
    pub cal_month: (i32, u32),
    pub todo_edit: Option<String>,
    pub minute_seen: u32,
    pub prev_playing: bool,
    pub quit: bool,
    pub drag_hover: bool,
    pub hud: Option<(u32, Instant, f32)>, // kind, until, value
    pub shelf_tab: usize,
    pub hold: bool,
    pub theme_color: Color,
    pub cur_lyric: String,
    pub lyric_bar_w: Option<f32>,
    pub todo_sel: Option<usize>,
    pub net: NetMon,
    pub hud_pending: Option<(u32, Instant)>,
    pub svc_at: Instant,
    pub water_snooze_until: u32,
    pub fs_hits: u32,
    /// Current window rectangle inside the canvas, in pixels (x, y, w, h). The window is kept
    /// exactly as big as the visible notch so it never blocks clicks meant for other apps.
    pub view: (i32, i32, i32, i32),
    /// 0..1: the island has split into two blobs (music pill + timer dot).
    pub split: Ease,
    pub water_cfg: bool,
    pub updater: UpdateHandle,
    pub tour: Option<crate::tour::Tour>,
    pub last_play: Cell<Instant>,
    pub pray_pick: bool,
    pub pray_country: usize,
    pub pray_level: u8,
    pub assistant: crate::assistant::Assistant,
    pub chat_input: String,
    pub chat_focus: bool,
    pub whats_new: bool,
    pub last_bright: i32,
    pub vol_drag: bool,
    pub vol_cache: Cell<(Instant, f32)>,
    pub blank_since: Option<Instant>,
    pub chat_scroll: Cell<f32>,
    pub pray_scroll: Ease,
    pub dock_hover: Spring,
}

static mut APP: *mut App = std::ptr::null_mut();

pub fn install(_hwnd: HWND, a: App) {
    unsafe {
        APP = Box::into_raw(Box::new(a));
    }
}
pub fn app() -> Option<&'static mut App> {
    unsafe {
        if APP.is_null() {
            None
        } else {
            Some(&mut *APP)
        }
    }
}

// ---------------------------------------------------------------- geometry
fn monitor_dpi(m: HMONITOR) -> f32 {
    unsafe {
        let (mut x, mut y) = (0u32, 0u32);
        if GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() && x > 0 {
            x as f32
        } else {
            96.0
        }
    }
}

fn monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
        let v = &mut *(l.0 as *mut Vec<HMONITOR>);
        v.push(m);
        TRUE
    }
    let mut v: Vec<HMONITOR> = vec![];
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

pub fn monitor_count() -> usize {
    monitors().len().max(1)
}

/// The monitor the notch lives on (0 = primary).
pub fn window_monitor(monitor: u32) -> HMONITOR {
    unsafe {
        let ms = monitors();
        if monitor == 0 || ms.is_empty() {
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
        } else {
            *ms.get((monitor as usize - 1).min(ms.len() - 1)).unwrap()
        }
    }
}

/// (x, y, w_px, h_px, scale) of the overlay window for the chosen monitor.
pub fn window_geometry(monitor: u32) -> (i32, i32, i32, i32, f32) {
    unsafe {
        let m = window_monitor(monitor);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(m, &mut mi);
        let st = Settings::load();
        let user = if st.notch_scale == 0 { 1.0 } else { st.notch_scale as f32 / 100.0 };
        let s = monitor_dpi(m) / 96.0 * user;
        let wpx = (WIN_W * s).round() as i32;
        let hpx = (WIN_H * s).round() as i32;
        let cx = (mi.rcMonitor.left + mi.rcMonitor.right) / 2;
        let dy: i32 = std::env::var("NOOK_DEBUG_Y").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        (cx - wpx / 2, mi.rcMonitor.top + dy, wpx, hpx, s)
    }
}

impl App {
    pub fn new(hwnd: HWND, scale: f32) -> Result<App> {
        let mut settings = Settings::load();
        if std::env::var("NOOK_DEBUG_STYLE").map(|v| v == "island").unwrap_or(false) {
            settings.island = true;
            settings.floating = true;
        }
        if std::env::var_os("NOOK_DEBUG_DOT").is_some() {
            settings.dot_mode = true;
        }
        if let Some(m) = std::env::var("NOOK_DEBUG_CIRCLE").ok().and_then(|v| v.parse().ok()) {
            settings.circle_mode = m;
        }
        let (x, y, wpx, hpx, s) = window_geometry(settings.monitor);
        let _ = scale;
        let gfx = Gfx::new_overlay(hwnd, wpx as u32, hpx as u32, 96.0 * s)?;
        let auto_update = settings.auto_update;
        sys::spawn_brightness_poller();
        let store = Store::load(settings.shelf_empty, settings.clip_persist);
        let now = Instant::now();
        sys::set_autostart(settings.autostart);
        let mut a = App {
            hwnd,
            gfx,
            scale: s,
            win_px: (x, y),
            settings,
            store,
            media: media::spawn(),
            lyr: LyricsHandle::new(),
            w: Spring::with(154.0, 17.5, 0.74),
            h: Spring::with(30.0, 17.5, 0.74),
            y_off: Spring::new(0.0),
            page_pos: Spring::with(0.0, 14.0, 0.9),
            lyric_slide: Spring::with(0.0, 16.0, 0.85),
            expanded: false,
            hover_since: None,
            leave_since: None,
            last: now,
            start: now,
            page: 0,
            vis: Ease::new(1.0, 12.0),
            bars: [0.3; 5],
            mouse: (-1.0, -1.0),
            mouse_in: false,
            hits: RefCell::new(vec![]),
            seek_drag: None,
            last_wheel: now,
            art: None,
            art_ver: 0,
            lyrics_view: false,
            lyric_idx: None,
            track_ver: 0,
            toast: None,
            toast_q: vec![],
            battery: sys::battery(),
            battery_at: now,
            hidden: false,
            fs_check: now,
            last_render: now,
            dirty: Cell::new(true),
            cal_month: (sys::now().year, sys::now().month),
            todo_edit: None,
            minute_seen: sys::now().minutes(),
            prev_playing: false,
            quit: false,
            drag_hover: false,
            hud: None,
            shelf_tab: 0,
            hold: false,
            theme_color: Color::rgb(139, 92, 246),
            cur_lyric: String::new(),
            lyric_bar_w: None,
            todo_sel: None,
            net: netmon::spawn(),
            hud_pending: None,
            svc_at: now,
            water_snooze_until: 0,
            fs_hits: 0,
            view: (-1, -1, -1, -1),
            split: Ease::new(0.0, 9.0),
            water_cfg: false,
            updater: update::spawn(auto_update),
            tour: None,
            last_play: Cell::new(now - Duration::from_secs(60)),
            pray_pick: false,
            pray_country: 0,
            pray_level: 1,
            assistant: crate::assistant::Assistant::new(),
            chat_input: String::new(),
            chat_focus: false,
            whats_new: false,
            last_bright: -1,
            vol_drag: false,
            vol_cache: Cell::new((now - Duration::from_secs(5), 0.5)),
            blank_since: None,
            chat_scroll: Cell::new(0.0),
            pray_scroll: Ease::new(0.0, 16.0),
            dock_hover: Spring::with(0.0, 16.0, 0.76),
        };
        unsafe {
            let _ = AddClipboardFormatListener(hwnd);
        }
        a.add_tray();
        install_hook(hwnd);
        let (w, h) = a.collapsed_size();
        a.w.snap(w);
        a.h.snap(h);
        // first start after a self-update: say so
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|x| x == "--updated") {
            if let Some(v) = args.get(i + 1) {
                a.push_toast(Toast { icon: '\u{E73E}', color: GREEN, title: "Nook updated".into(), body: format!("Now running v{v}"), a: None, b: None, until: Instant::now() + Duration::from_secs(5), tag: 7, idx: 0 });
            }
        }
        if let Some(n) = std::env::var("NOOK_DEBUG_TOUR").ok().and_then(|v| v.parse::<usize>().ok()) {
            a.start_tour();
            if let Some(t) = &mut a.tour {
                t.step = n.min(t.steps.len() - 1);
            }
            a.tour_enter();
        }
        if std::env::var("NOOK_DEBUG_TIMER").is_ok() {
            a.store.phase = Phase::Focus;
            a.store.running = true;
            a.store.phase_total = 1500.0;
            a.store.remaining = 1342.0;
            a.store.cycle = 1;
        }
        a.water_cfg = std::env::var("NOOK_DEBUG_WATERCFG").is_ok();
        if let Some((p, c)) = std::env::var("NOOK_DEBUG_CITY").ok().and_then(|x| x.split_once(',').map(|(a, b)| (a.parse().unwrap_or(0), b.parse().unwrap_or(0)))) {
            a.settings.prayer_prov = p;
            a.settings.prayer_city = c;
        }
        if let Ok(v) = std::env::var("NOOK_DEBUG_PICK") {
            a.pray_pick = true;
            a.pray_country = v.parse().unwrap_or(0);
            a.pray_level = 1;
            a.pray_scroll_to_current();
        }
        if let Some(pg) = std::env::var("NOOK_DEBUG_PAGE").ok().and_then(|v| v.parse::<usize>().ok()) {
            let n = a.pages().len();
            a.page = pg.min(n - 1);
            a.page_pos.snap(a.page as f32);
            a.expanded = true;
            a.hold = true;
            a.lyrics_view = std::env::var("NOOK_DEBUG_LYRICS").is_ok();
            a.w.snap(EXP_W);
            let eh = a.exp_h();
            a.h.snap(eh);
        }
        Ok(a)
    }

    pub fn pages(&self) -> Vec<PageKind> {
        let mut v = vec![PageKind::Media];
        if self.settings.ai_enabled {
            v.push(PageKind::Assistant);
        }
        if self.settings.widgets_page {
            v.push(PageKind::Widgets);
        }
        v.push(PageKind::Calendar);
        v.push(PageKind::Focus);
        if self.settings.water {
            v.push(PageKind::Water);
        }
        v.push(PageKind::Shelf);
        v.push(PageKind::Todo);
        if self.settings.prayer {
            v.push(PageKind::Prayer);
        }
        v
    }

    fn add_tray(&self) {
        unsafe {
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: WM_TRAY,
                hIcon: { let h = GetModuleHandleW(None).unwrap_or_default(); LoadIconW(h, PCWSTR(1 as _)).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default()) },
                ..Default::default()
            };
            let tip: Vec<u16> = "Nook".encode_utf16().collect();
            nid.szTip[..tip.len()].copy_from_slice(&tip);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    pub fn remove_tray(&self) {
        unsafe {
            let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    pub fn is_playing(&self) -> bool {
        if std::env::var_os("NOOK_DEBUG_IDLE").is_some() {
            return false;
        }
        let m = self.media.state.lock().unwrap();
        let raw = m.has_session && m.playing && !m.title.is_empty();
        if raw {
            self.last_play.set(Instant::now());
            return true;
        }
        // a pause shorter than ~1.5 s (buffering, track change) must not collapse / re-open the pill
        m.has_session && !m.title.is_empty() && self.last_play.get().elapsed() < Duration::from_millis(1500)
    }
    pub fn has_track(&self) -> bool {
        let m = self.media.state.lock().unwrap();
        m.has_session && !m.title.is_empty()
    }

    pub fn exp_h(&self) -> f32 {
        EXP_H
    }

    /// Glow / theme colour for the lyric surfaces, or None when theming is off.
    pub fn theme_tint(&self) -> Option<Color> {
        match self.settings.theme {
            0 => {
                let c = self.theme_color;
                let mx = c.0.max(c.1).max(c.2).max(0.01);
                let sat = (mx - c.0.min(c.1).min(c.2)) / mx;
                Some(if sat < 0.45 { c.mix(Color::rgb(139, 92, 246), 0.6) } else { c })
            }
            1 => Some(Color::rgb(139, 92, 246)),
            2 => Some(Color::rgb(10, 132, 255)),
            3 => Some(Color::rgb(255, 55, 95)),
            4 => Some(Color::rgb(48, 209, 88)),
            _ => None,
        }
    }

    pub fn collapsed_size(&self) -> (f32, f32) {
        if self.toast.is_some() {
            return (340.0, 76.0);
        }
        if let Some((k, _, _)) = self.hud {
            let _ = k;
            return (250.0, 46.0);
        }
        if self.ai_busy() {
            return if self.settings.island { (190.0, 36.0) } else { (200.0, 32.0) };
        }
        if self.settings.dot_mode && !self.ai_busy() && self.tour.is_none() {
            return if self.settings.island { (8.0, 8.0) } else { (12.0, 4.0) }; // sleek tiny dot until hovered / clicked
        }
        let playing = self.is_playing();
        let isl = self.settings.island;
        let timer = self.timer_active();
        if playing {
            if isl && self.circle_content().is_some() {
                return (SPLIT_MAIN + SPLIT_GAP + 36.0, 36.0); // music pill + detached timer dot
            }
            if let Some(w) = self.lyric_bar_w {
                return (w.min(520.0), 36.0);
            }
            if isl { (206.0, 36.0) } else { (236.0, 32.0) }
        } else if timer {
            if isl { (178.0, 36.0) } else { (190.0, 32.0) } // ring + remaining time
        } else if self.settings.minimal {
            (100.0, 26.0)
        } else if isl {
            (128.0, 36.0)
        } else {
            (154.0, 30.0)
        }
    }

    /// Install a downloaded update now (`force`) or only when nothing is going on (idle notch, no timer
    /// running, no settings window), then quit so the new version takes over.
    pub fn install_update(&mut self, force: bool) {
        let UpdateState::Ready { version, path } = self.updater.snapshot() else { return };
        if !force {
            let idle = self.tour.is_none() && !self.chat_focus && !self.ai_busy() && !self.expanded && self.hover_since.is_none() && !self.mouse_in && self.toast.is_none() && self.hud.is_none() && self.seek_drag.is_none() && !self.timer_active() && !crate::settings_ui::is_open();
            if !idle {
                return;
            }
        }
        match update::apply(&path, &version) {
            Ok(()) => {
                sys::log(&format!("updated to {version}, restarting"));
                self.remove_tray();
                unsafe {
                    PostQuitMessage(0);
                }
            }
            Err(e) => {
                sys::log(&format!("update install failed: {e}"));
                self.updater.fail(e);
            }
        }
    }

    /// Scroll the picker list so the current country / city sits in view.
    pub fn pray_scroll_to_current(&mut self) {
        let items = self.pray_items();
        let max = ((items.len() as f32 - 3.0) * 22.0).max(0.0);
        let to = items.iter().position(|i| i.2).map(|i| ((i as f32 - 1.0) * 22.0).clamp(0.0, max)).unwrap_or(0.0);
        self.pray_scroll.value = to;
        self.pray_scroll.target = to;
    }

    /// System volume 0..1, refreshed at most every 400 ms (the audio endpoint is costly to query per frame).
    pub fn volume_cached(&self) -> f32 {
        let (t, v) = self.vol_cache.get();
        if t.elapsed() > Duration::from_millis(400) {
            let nv = sys::volume().map(|x| if x.1 { 0.0 } else { x.0 }).unwrap_or(v);
            self.vol_cache.set((Instant::now(), nv));
            return nv;
        }
        v
    }

    /// What the detached circle of the island shows: (ring fill 0..1, short label, colour).
    /// Modes: 0 auto (timer, else battery), 1 battery, 2 volume, 3 timer, 4 next prayer, 5 water, 6 off.
    pub fn circle_content(&self) -> Option<(f32, String, Color)> {
        let batt = || {
            if !self.battery.present {
                return None;
            }
            let b = self.battery;
            let c = if b.percent <= 15 && !b.charging { RED } else if b.percent <= 30 && !b.charging { AMBER } else { GREEN };
            Some((b.percent as f32 / 100.0, b.percent.to_string(), c))
        };
        let timer = || {
            if !self.timer_active() {
                return None;
            }
            let (rem, frac, tc) = self.timer_info();
            Some((frac, ((rem / 60.0).ceil() as u32).to_string(), tc))
        };
        match self.settings.circle_mode {
            0 => {
                let secs = self.last.duration_since(self.start).as_secs_f32();
                if self.timer_active() && self.battery.present {
                    if secs % 8.0 < 4.0 { timer() } else { batt() }
                } else {
                    timer().or_else(batt)
                }
            }
            1 => batt(),
            2 => {
                let v = self.volume_cached();
                Some((v, ((v * 100.0).round() as u32).to_string(), BLUE))
            }
            3 => timer(),
            4 => {
                let s = &self.settings;
                let (t, _, _) = crate::prayer::today(s);
                let nm = crate::prayer::minutes_now(s);
                let (i, at, tomorrow) = crate::prayer::next(&t, nm);
                let until = (at + if tomorrow { 1440.0 } else { 0.0 } - nm).max(0.0);
                let prev = if i == 0 { t[4] - 1440.0 } else { t[i - 1] };
                let span = (at + if tomorrow { 1440.0 } else { 0.0 } - prev).max(1.0);
                let label = if until >= 60.0 { format!("{}h", (until / 60.0) as u32) } else { format!("{}m", until as u32) };
                Some((((nm - prev) / span) as f32, label, Color::rgb(32, 201, 166)))
            }
            6 => {
                let b = sys::BRIGHT.load(std::sync::atomic::Ordering::Relaxed);
                if b < 0 { None } else { Some((b as f32 / 100.0, b.to_string(), Color::rgb(255, 214, 10))) }
            }
            5 => {
                let goal = self.settings.water_goal.max(1) as f32;
                let f = (self.store.water_taken as f32 / goal).clamp(0.0, 1.0);
                Some((f, format!("{}", (f * 100.0).round() as u32), BLUE))
            }
            _ => None,
        }
    }

    pub fn timer_active(&self) -> bool {
        self.store.phase != Phase::Idle && self.store.running
    }

    /// (remaining seconds, progress 0..1, phase colour)
    pub fn timer_info(&self) -> (f64, f32, Color) {
        let s = &self.store;
        let col = match s.phase {
            Phase::Short => GREEN,
            Phase::Long => BLUE,
            _ => Color::rgb(124, 108, 255),
        };
        let frac = if s.phase_total > 0.0 { (1.0 - s.remaining / s.phase_total).clamp(0.0, 1.0) as f32 } else { 0.0 };
        (s.remaining.max(0.0), frac, col)
    }

    pub fn reposition(&mut self) {
        let (x, y, wpx, hpx, s) = window_geometry(self.settings.monitor);
        self.win_px = (x, y);
        self.view = (-1, -1, -1, -1); // force `fit_window` to re-place the window
        if wpx as u32 != self.gfx.width || hpx as u32 != self.gfx.height || (s - self.scale).abs() > 0.001 {
            if let Ok(g) = Gfx::new_overlay(self.hwnd, wpx as u32, hpx as u32, 96.0 * s) {
                self.gfx = g;
                self.art = None;
                self.art_ver = 0;
                self.scale = s;
            }
        }
        self.dirty.set(true);
    }

    /// Keep the overlay window exactly as large as what is visible (notch + a little shadow margin).
    /// The swap chain stays canvas-sized; a visual offset makes the right slice show. A transparent
    /// 740x250 window would swallow every click aimed at Chrome's tab strip, so this is the fix.
    fn fit_window(&mut self) {
        let s = self.scale;
        let mut r = self.shape_rect();
        if self.tour.is_none() && !self.whats_new {
            // while the notch grows or shrinks, size the window for the larger of now/target once instead of
            // resizing it every frame (per-frame window resizes are a source of stutter)
            let cx = WIN_W / 2.0;
            let (tw, th) = (self.w.target.max(self.w.value), self.h.target.max(self.h.value));
            if tw + 32.0 > r.w {
                r.x = cx - tw / 2.0 - 16.0;
                r.w = tw + 32.0;
            }
            r.h = r.h.max(th + self.y_off.value);
        }
        let margin = if self.expanded || self.toast.is_some() || self.h.value > 60.0 { 20.0 } else { 4.0 };
        let (cw, ch) = (self.gfx.width as i32, self.gfx.height as i32);
        let x0 = (((r.x - margin) * s).floor() as i32).clamp(0, cw);
        let x1 = (((r.x + r.w + margin) * s).ceil() as i32).clamp(0, cw);
        let y1 = (((r.y + r.h + margin) * s).ceil() as i32).clamp(0, ch);
        let v = (x0, 0, (x1 - x0).max(1), y1.max(1));
        if v == self.view {
            return;
        }
        self.view = v;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, self.win_px.0 + v.0, self.win_px.1 + v.1, v.2, v.3, SWP_NOACTIVATE);
        }
        self.gfx.set_view_offset(v.0, v.1);
    }

    pub fn apply_settings(&mut self) {
        self.settings.save();
        sys::set_autostart(self.settings.autostart);
        self.reposition();
        let n = self.pages().len();
        if self.page >= n {
            self.page = 0;
            self.page_pos.snap(0.0);
        }
        self.dirty.set(true);
    }

    // ------------------------------------------------------------ main tick
    /// Returns true while something is animating (so the loop keeps vsync pacing).
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;

        // Hide only while a game / full-screen video / slideshow owns the notch's monitor.
        // Two positive checks in a row to hide (no flicker on alt-tab); one negative check to show again.
        if now.duration_since(self.fs_check) > Duration::from_millis(500) {
            self.fs_check = now;
            let enabled = self.settings.hide_fullscreen && std::env::var("NOOK_DEBUG_NOHIDE").is_err();
            let hit = enabled && sys::fullscreen_busy(self.hwnd, window_monitor(self.settings.monitor));
            self.fs_hits = if hit { (self.fs_hits + 1).min(3) } else { 0 };
            let busy = if self.hidden { hit } else { self.fs_hits >= 2 };
            if busy != self.hidden {
                sys::log(&format!("fullscreen hide -> {busy}"));
                self.hidden = busy;
                unsafe {
                    let _ = ShowWindow(self.hwnd, if busy { SW_HIDE } else { SW_SHOWNOACTIVATE });
                }
                if !busy {
                    self.dirty.set(true);
                }
            }
        }
        if self.hidden {
            return false;
        }

        if now.duration_since(self.battery_at) > Duration::from_secs(5) {
            self.battery_at = now;
            let b = sys::battery();
            if b.present && self.settings.pop_battery {
                self.battery_events(&b);
            }
            self.battery = b;
        }

        self.poll_media();
        self.poll_input(now);
        self.tick_services(now);

        // target sizes
        let (wc, hc) = self.collapsed_size();
        let (mut tw, mut th) = if self.expanded { (EXP_W, self.exp_h()) } else { (wc, hc) };
        if !self.expanded && self.toast.is_none() && self.hud.is_none() {
            if let Some(since) = self.hover_since {
                // hover-intent anticipation: the pill swells slightly before it opens
                let t = (now.duration_since(since).as_secs_f32() / 0.11).clamp(0.0, 1.0);
                tw += 10.0 * t;
                th += 2.0 * t;
            }
        }
        self.poll_events(now);
        if self.w.target != tw {
            self.w.set(tw);
        }
        if self.h.target != th {
            self.h.set(th);
        }
        // iOS-style island: bouncy spring when it grows, quick critically-damped spring when it shrinks
        let isl = self.settings.island;
        let (om, ze) = if isl { if self.expanded { (13.0, 0.66) } else { (24.0, 0.95) } } else { (17.5, 0.74) };
        self.w.retune(om, ze);
        self.h.retune(om, ze);
        let want_split = isl && self.is_playing() && self.circle_content().is_some() && !self.expanded && self.toast.is_none() && self.hud.is_none();
        self.split.target = if want_split { 1.0 } else { 0.0 };
        let want_y = if isl { 10.0 } else if self.settings.floating { 8.0 } else { 0.0 };
        if self.y_off.target != want_y {
            self.y_off.set(want_y);
        }
        let n = self.pages().len() as f32;
        let tp = (self.page as f32).min(n - 1.0);
        if self.page_pos.target != tp {
            self.page_pos.set(tp);
        }

        let speed = if self.settings.fast_anim { 1.45 } else { 1.0 };
        let dts = dt * speed;
        self.w.step(dts);
        self.h.step(dts);
        self.y_off.step(dts);
        self.page_pos.step(dts);
        self.lyric_slide.step(dt);
        self.split.step(dt);
        self.pray_scroll.step(dt);

        let kinds = self.pages();
        let total = (kinds.len() as f32) * 33.0 + 10.0 + 33.0;
        let cx = WIN_W / 2.0;
        let yo = self.y_off.value;
        let cy = yo + EXP_H - 17.0;
        let dock_zone = Rect {
            x: cx - total / 2.0 - 16.0,
            y: cy - 14.0,
            w: total + 32.0,
            h: 34.0,
        };
        let dock_hot = self.expanded && kinds.len() > 1 && !self.vol_drag && self.seek_drag.is_none() && dock_zone.contains(self.mouse.0, self.mouse.1);
        let tgt = if dock_hot { 1.0 } else { 0.0 };
        if (self.dock_hover.target - tgt).abs() > 0.01 {
            self.dock_hover.set(tgt);
            self.dirty.set(true);
        }
        if !self.expanded {
            self.dock_hover.snap(0.0);
        }
        self.dock_hover.step(dts);
        if !self.expanded {
            self.pray_pick = false;
        }
        let touring = self.tour_tick(dt);
        self.fit_window();

        // visualiser bars
        let t = now.duration_since(self.start).as_secs_f32();
        let playing = self.is_playing();
        for i in 0..5 {
            let tgt = if playing { 0.35 + 0.65 * ((t * (3.1 + i as f32 * 1.3) + i as f32 * 1.7).sin() * 0.5 + 0.5) } else { 0.15 };
            self.bars[i] += (tgt - self.bars[i]) * (1.0 - (-14.0 * dt).exp());
        }

        let animating = !(self.w.settled() && self.h.settled() && self.page_pos.settled() && self.y_off.settled() && self.lyric_slide.settled() && self.split.settled() && self.pray_scroll.settled() && self.dock_hover.settled() && !touring);
        let visual_live = self.ai_busy() || self.tour.is_some() || self.expanded || playing || self.toast.is_some() || self.hud.is_some() || self.drag_hover;
        let idle_anim = self.settings.idle_scenes && !self.expanded && !playing && !self.timer_active();
        let fading = idle_anim && {
            let ph = now.duration_since(self.start).as_secs_f32() % 6.0;
            ph < 0.5 || ph > 5.5
        };
        let since = now.duration_since(self.last_render);
        let due = animating
            || (fading && since > Duration::from_millis(16))
            || (idle_anim && since > Duration::from_millis(200))
            || (visual_live && since > Duration::from_millis(33))
            || since > Duration::from_millis(1000)
            || self.dirty.get();
        if due {
            if self.render().is_err() {
                // device lost etc.: rebuild on next tick
                self.reposition();
            }
            self.last_render = now;
            self.dirty.set(false);
        }
        animating
    }

    fn poll_media(&mut self) {
        let (ver, art_ver, title, artist, dur) = {
            let m = self.media.state.lock().unwrap();
            (m.track_ver, m.art_ver, m.title.clone(), m.artist.clone(), m.duration)
        };
        if ver != self.track_ver {
            self.track_ver = ver;
            self.lyric_idx = None;
            self.dirty.set(true);
            if self.settings.lyrics {
                self.lyr.request(&title, &artist, dur);
            }
        }
        if self.settings.lyrics && !title.is_empty() {
            self.lyr.retry_stale(&title, &artist, dur);
        }
        if art_ver != self.art_ver {
            self.art_ver = art_ver;
            let bytes = self.media.state.lock().unwrap().art.clone();
            if let Some(c) = bytes.as_ref().and_then(|b| self.gfx.dominant(&b[..])) {
                self.theme_color = c;
            }
            self.art = bytes.and_then(|b| self.gfx.bitmap_from_bytes(&b[..]));
            self.dirty.set(true);
        }
        let playing = self.is_playing();
        if playing != self.prev_playing {
            self.prev_playing = playing;
            self.dirty.set(true);
        }
        // debug: pretend a lyric line is active (NOOK_DEBUG_LYRIC="text")
        static FAKE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        if let Some(t) = FAKE.get_or_init(|| std::env::var("NOOK_DEBUG_LYRIC").ok()) {
            if self.cur_lyric != *t {
                self.cur_lyric = t.clone();
                self.lyric_bar_w = Some((self.gfx.measure(t, 14.0, 600) + 96.0).clamp(236.0, 640.0));
            }
            return;
        }
        // current lyric line (drives the collapsed lyric bar and the lyrics view)
        if self.settings.lyrics && playing {
            let (pos, dur) = {
                let m = self.media.state.lock().unwrap();
                (m.pos_now(), m.duration)
            };
            let (st, synced, _p, cur, _n, idx) = self.lyr.around(pos, dur);
            let ready = st == crate::lyrics::LyricsStatus::Ready && synced;
            // instrumental gaps are blank lines: show a note instead of dropping the bar (that made the pill flash)
            // blank line (instrumental gap): keep the previous line for a moment, then fall back to the normal
            // pill instead of sitting on a bare note
            let raw = if ready { cur.trim().to_string() } else { String::new() };
            let line = if !raw.is_empty() {
                self.blank_since = None;
                raw
            } else if ready {
                let since = *self.blank_since.get_or_insert_with(Instant::now);
                if since.elapsed() < Duration::from_millis(1200) { self.cur_lyric.clone() } else { title.clone() }
            } else {
                self.blank_since = None;
                String::new()
            };
            let want_bar = self.settings.lyric_bar && self.lyrics_view;
            let idx_changed = idx.is_some() && idx != self.lyric_idx;
            if line != self.cur_lyric || idx_changed || (want_bar != self.lyric_bar_w.is_some()) {
                self.cur_lyric = line;
                self.lyric_idx = idx;
                self.lyric_slide.snap(1.0);
                self.lyric_slide.set(0.0);
                if want_bar && !self.cur_lyric.is_empty() {
                    // coarse steps + hysteresis so the pill does not twitch on every line
                    let want = (((self.gfx.measure(&self.cur_lyric, 14.0, 600) + 96.0) / 24.0).ceil() * 24.0).clamp(236.0, 520.0);
                    let blank_now = self.blank_since.is_some();
                    let keep = self.lyric_bar_w.map(|w| blank_now || (w >= want && w - want < 72.0)).unwrap_or(false);
                    if !keep {
                        self.lyric_bar_w = Some(want);
                    }
                } else {
                    self.lyric_bar_w = None;
                }
                self.dirty.set(true);
            }
        } else if !self.cur_lyric.is_empty() || self.lyric_bar_w.is_some() {
            self.cur_lyric.clear();
            self.lyric_idx = None;
            self.lyric_bar_w = None;
            self.dirty.set(true);
        }
    }

    pub fn shape_rect(&self) -> Rect {
        let cx = WIN_W / 2.0;
        if self.tour.is_some() {
            // panel + tooltip below it
            return Rect { x: cx - 235.0, y: 0.0, w: 470.0, h: self.y_off.target + EXP_H + 12.0 + 180.0 };
        }
        if self.whats_new && !self.expanded {
            let b = self.whats_new_box();
            return Rect { x: cx - 215.0, y: 0.0, w: 430.0, h: b.y + b.h + 24.0 };
        }
        let dot = self.settings.dot_mode && !self.expanded;
        let (w, h) = (self.w.value.max(if dot { 14.0 } else { 0.0 }), self.h.value.max(if dot { 5.0 } else { 0.0 }));
        Rect { x: cx - w / 2.0 - 16.0, y: 0.0, w: w + 32.0, h: h + self.y_off.value }
    }

    fn poll_input(&mut self, now: Instant) {
        unsafe {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let lx = (p.x - self.win_px.0) as f32 / self.scale;
            let ly = (p.y - self.win_px.1) as f32 / self.scale;
            self.mouse = (lx, ly);
            let cx = WIN_W / 2.0;
            // a dot is tiny: give it a generous hover target
            let (min_w, min_h) = if self.settings.dot_mode && !self.expanded { (50.0, 24.0) } else { (0.0, 0.0) };
            let (hw, hh) = (self.w.target.max(self.w.value).max(min_w), self.h.target.max(self.h.value).max(min_h));
            let r = Rect { x: cx - hw / 2.0 - 4.0, y: -400.0, w: hw + 8.0, h: hh + 400.0 + self.y_off.value + 4.0 };
            let inside = r.contains(lx, ly);
            self.mouse_in = inside;

            // dragging files over the notch opens the shelf
            let lbtn = (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0;
            self.drag_hover = lbtn && inside && !self.expanded && self.seek_drag.is_none() && false;

            if self.tour.is_some() || self.chat_focus {
                self.expanded = true; // the tour / typing keeps the panel open
                self.leave_since = None;
                self.hover_since = None;
            } else if self.settings.open_on_click {
                if !inside && self.expanded {
                    let since = *self.leave_since.get_or_insert(now);
                    if now.duration_since(since) > Duration::from_millis(350) {
                        self.expanded = false;
                    }
                } else {
                    self.leave_since = None;
                }
            } else if inside {
                self.leave_since = None;
                if !self.expanded && self.toast.is_none() {
                    let since = *self.hover_since.get_or_insert(now);
                    if now.duration_since(since) > Duration::from_millis(110) {
                        self.expanded = true;
                        self.hover_since = None;
                    }
                }
            } else {
                self.hover_since = None;
                if self.expanded && self.seek_drag.is_none() && !self.hold {
                    let since = *self.leave_since.get_or_insert(now);
                    if now.duration_since(since) > Duration::from_millis(260) {
                        self.expanded = false;
                        self.leave_since = None;
                    }
                }
            }
        }
    }

    // --------------------------------------------------------------- render
    fn render(&self) -> Result<()> {
        self.hits.borrow_mut().clear();
        let g = &self.gfx;
        g.begin_overlay()?;
        self.draw_all();
        g.end_overlay()
    }

    fn draw_all(&self) {
        let g = &self.gfx;
        let cx = WIN_W / 2.0;
        let dot = self.settings.dot_mode && !self.expanded;
        let w = self.w.value.max(if dot { 12.0 } else { 40.0 });
        let h = self.h.value.max(if dot { 4.0 } else { 18.0 });
        let (wc, _) = self.collapsed_size();
        let p_raw = (w - wc) / (EXP_W - wc);
        let p = p_raw.clamp(0.0, 1.0);
        let l = cx - w / 2.0;
        let r = cx + w / 2.0;
        let yo = self.y_off.value;
        let floating = self.settings.floating;
        let rt = if floating { 0.0 } else if dot { 2.0 } else { lerp(7.0, 15.0, p) };
        let isl = self.settings.island;
        let rb = if isl { lerp(h / 2.0, 36.0, p) } else if dot { h / 2.0 } else { lerp(12.0, 26.0, p) };
        let sp = if isl && !self.expanded { self.split.value } else { 0.0 };
        let wm = w - sp * (h + SPLIT_GAP); // width of the main blob when split
        let mut geo_keep: Option<windows::Win32::Graphics::Direct2D::ID2D1PathGeometry> = None;

        // soft shadow
        let glass = self.settings.glass;
        let sh = (p * 0.9).max(if self.toast.is_some() { 0.6 } else { 0.0 });
        if sh > 0.01 {
            for i in 0..7 {
                let k = i as f32 * 1.25;
                let a = 0.07 * (1.0 - k / 9.0) * sh;
                g.fill_round(l - k * 1.8, yo - 6.0, w + k * 3.6, h + 8.0 + k * 2.2, rb + k * 1.6, Color::rgba(0, 0, 0, a));
            }
        }

        // body: the original solid black (optional plain translucent tint)
        let body = if glass { Color::rgba(16, 16, 20, 0.35 + 0.6 * (self.settings.glass_tint as f32 / 100.0)) } else { BLACK };
        let geo = if floating { g.pill_geometry(l, yo, wm, h, rb.min(h / 2.0)) } else { g.notch_geometry(l, r, h, rt, rb) };
        if let Ok(geo) = geo {
            g.fill_geometry(&geo, body);
            if glass && floating {
                g.stroke_geo_grad(&geo, (l, yo), (r, yo + h), &[(0.0, Color::rgba(255, 255, 255, 0.16)), (1.0, Color::rgba(255, 255, 255, 0.06))], 1.0);
            }
            geo_keep = Some(geo);
        }
        if sp > 0.01 {
            // the detached blob: grows out of the main pill's right end
            let rad = h / 2.0 * sp;
            g.fill_circle(r - rad, yo + h / 2.0, rad, body);
        }

        // theme glow along the bottom edge (lyric bar when collapsed, lyrics view when expanded)
        if let Some(t) = self.theme_tint() {
            let calm = self.toast.is_none() && self.hud.is_none();
            let bar = if self.lyric_bar_w.is_some() && calm && !self.settings.dot_mode { (1.0 - p * 2.0).clamp(0.0, 1.0) } else { 0.0 };
            let lyr_view = if self.lyrics_view { (1.0 - self.page_pos.value).clamp(0.0, 1.0) * ((p_raw - 0.4) / 0.5).clamp(0.0, 1.0) } else { 0.0 };
            let glow = bar.max(lyr_view);
            if glow > 0.01 {
                match &geo_keep {
                    Some(geo) => g.push_geo_clip(geo),
                    None => g.push_round_clip(l, yo, w, h, rb),
                }
                let top = yo + h * if lyr_view > bar { 0.42 } else { 0.18 };
                g.vgradient(l - 20.0, top, w + 40.0, yo + h - top + 2.0, t.a(0.0), t.mix(BLACK, 0.18).a(0.92 * glow));
                g.pop_layer();
            }
        }

        let ex = ((p_raw - 0.55) / 0.40).clamp(0.0, 1.0);
        let ca = ex * ex * (3.0 - 2.0 * ex); // expanded content alpha
        let col = (1.0 - (p * 3.0)).clamp(0.0, 1.0); // collapsed content alpha

        g.push_clip(l - 1.0, yo, w + 2.0, h + 1.0);
        if self.toast.is_some() && !self.expanded {
            self.draw_toast(l, yo, w, h);
        } else if self.hud.is_some() && !self.expanded {
            self.draw_hud(l, yo, w, h);
        } else {
            if col > 0.01 && !(self.settings.dot_mode && !self.ai_busy()) {
                self.draw_collapsed(l, yo, wm, h, col);
                if sp > 0.3 {
                    let rad = h / 2.0;
                    let (dx, dy) = (r - rad, yo + h / 2.0);
                    let da = col * ((sp - 0.3) / 0.7).clamp(0.0, 1.0);
                    if let Some((frac, label, cc)) = self.circle_content() {
                        g.stroke_circle(dx, dy, rad - 6.0, 2.2, WHITE.a(0.16 * da));
                        g.arc(dx, dy, rad - 6.0, frac, 2.2, cc.a(da));
                        g.text(&label, dx - 12.0, dy - 8.0, 24.0, 16.0, 10.0, 700, WHITE.a(da), Align::Center);
                    }
                }
            }
            if ca > 0.01 {
                if isl {
                    // content settles in: scale 0.93 -> 1 as it fades up (the island's "blur-in")
                    g.scale_about(cx, yo + h * 0.45, lerp(0.93, 1.0, ca));
                }
                self.draw_expanded(l, yo, w, h, ca, p);
                if isl {
                    g.reset_transform();
                }
            }
        }
        g.pop_clip();
        if self.whats_new && !self.expanded && self.tour.is_none() {
            self.draw_whats_new();
        }
        if self.tour.is_some() {
            self.draw_tour(geo_keep.as_ref(), l, yo, w, h);
        }
    }

    pub fn clock_text(&self) -> String {
        let n = sys::now();
        format!("{:02}:{:02}", n.hour, n.min)
    }

    fn draw_collapsed(&self, l: f32, yo: f32, w: f32, h: f32, a: f32) {
        let g = &self.gfx;
        let cx = l + w / 2.0;
        let cy = yo + h / 2.0;
        let playing = self.is_playing();
        let clock = self.clock_text();
        if self.ai_busy() {
            // assistant state in the closed pill: sparkle | label | moving bars
            let (label, col) = match self.assistant.status() {
                crate::assistant::Status::Listening => ("Listening…", Color::rgb(255, 99, 130)),
                crate::assistant::Status::Thinking => ("Thinking…", crate::chat::PURPLE),
                _ => ("Speaking…", GREEN),
            };
            let t = self.last.duration_since(self.start).as_secs_f32();
            self.sparkle(l + 24.0, cy, 7.5, col.a(a));
            g.text(label, l + 42.0, cy - 10.0, w - 90.0, 20.0, 13.5, 600, WHITE.a(a), Align::Left);
            for i in 0..5 {
                let hh = 4.0 + 9.0 * ((t * (4.0 + i as f32 * 1.3) + i as f32).sin() * 0.5 + 0.5);
                g.fill_round(l + w - 30.0 + i as f32 * 4.0, cy - hh / 2.0, 2.0, hh, 1.0, col.a(a));
            }
        } else if playing && self.lyric_bar_w.is_some() && !self.cur_lyric.is_empty() {
            // note icon | current lyric line (width follows the text)
            let slide = self.lyric_slide.value;
            let la = a * (1.0 - slide.abs() * 0.9).clamp(0.0, 1.0);
            g.icon('\u{E8D6}', l + 28.0, cy, 14.0, WHITE.a(0.8 * a));
            let line = g.fit(&self.cur_lyric, 14.0, 600, w - 76.0);
            g.text(&line, l + 50.0, cy - 12.0 + slide * 6.0, w - 62.0, 24.0, 14.0, 600, WHITE.a(la), Align::Left);
        } else if playing {
            // artwork thumb | clock | visualiser
            let s = 20.0;
            if let Some(b) = &self.art {
                g.draw_cover(b, l + 16.0, cy - s / 2.0, s, 6.0, a);
            } else {
                g.fill_round(l + 16.0, cy - s / 2.0, s, s, 6.0, Color::rgba(255, 255, 255, 0.18 * a));
            }
            g.text(&clock, cx - 30.0, cy - 10.0, 60.0, 20.0, 14.0, 600, WHITE.a(a), Align::Center);
            self.draw_bars(r_of(l, w) - 30.0, cy, a);
        } else if self.timer_active() {
            // stretched pill: progress ring | remaining time | phase
            let (rem, frac, tc) = self.timer_info();
            g.stroke_circle(l + 24.0, cy, 8.5, 2.4, WHITE.a(0.16 * a));
            g.arc(l + 24.0, cy, 8.5, frac, 2.4, tc.a(a));
            g.text(&crate::pages::fmt_time(rem), l + 42.0, cy - 10.0, 70.0, 20.0, 14.5, 700, WHITE.a(a), Align::Left);
            let lbl = match self.store.phase {
                Phase::Short => "Break",
                Phase::Long => "Long break",
                _ => "Focus",
            };
            g.text(lbl, l + w - 78.0, cy - 8.0, 64.0, 16.0, 10.5, 600, tc.a(a), Align::Right);
        } else {
            if self.settings.minimal {
                g.text(&clock, cx - 30.0, cy - 10.0, 60.0, 20.0, 13.0, 600, WHITE.a(a), Align::Center);
            } else {
                let scenes = self.idle_scenes();
                let t = self.last.duration_since(self.start).as_secs_f32();
                let (idx, fade) = if scenes.len() > 1 {
                    let period = 6.0;
                    let ph = t % period;
                    let f = (ph / 0.45).min(1.0).min(((period - ph) / 0.45).min(1.0));
                    (((t / period) as usize) % scenes.len(), f * f * (3.0 - 2.0 * f))
                } else {
                    (0, 1.0)
                };
                let sa = a * fade;
                match scenes[idx] {
                    0 => {
                        if self.settings.status_ring {
                            self.draw_ring(l + 20.0, cy, sa);
                        }
                        let off = if self.settings.status_ring { 6.0 } else { 0.0 };
                        g.text(&clock, cx - 30.0 + off, cy - 10.0, 60.0, 20.0, 14.0, 600, WHITE.a(sa), Align::Center);
                        if self.settings.mascot {
                            self.draw_mascot(l + w - 18.0, cy, sa);
                        }
                    }
                    kind => self.draw_idle_scene(kind, l, cy, w, sa),
                }
            }
        }
    }

    /// Scene ids for the idle pill: 0 clock + mascot, 1 date, 2 next prayer, 3 water, 4 tasks, 5 battery.
    fn idle_scenes(&self) -> Vec<u8> {
        let mut v = vec![0u8];
        if !self.settings.idle_scenes || self.settings.minimal {
            return v;
        }
        v.push(1);
        if self.settings.prayer {
            v.push(2);
        }
        if self.settings.water {
            v.push(3);
        }
        if self.store.todos.iter().any(|t| !t.done) {
            v.push(4);
        }
        if self.battery.present {
            v.push(5);
        }
        v
    }

    fn draw_idle_scene(&self, kind: u8, l: f32, cy: f32, w: f32, a: f32) {
        let g = &self.gfx;
        let n = sys::now();
        let (glyph, col, text): (Option<char>, Color, String) = match kind {
            1 => (Some('\u{E787}'), RED, format!("{} {} {}", sys::DOW[n.dow as usize], n.day, sys::MON[(n.month - 1) as usize])),
            2 => {
                let s = &self.settings;
                let (t, _, _) = crate::prayer::today(s);
                let nm = crate::prayer::minutes_now(s);
                let (i, at, _) = crate::prayer::next(&t, nm);
                (Some('\u{E708}'), Color::rgb(32, 201, 166), format!("{} {}", crate::prayer::names_s(s)[i], crate::prayer::fmt(at)))
            }
            3 => (None, BLUE, format!("{:.1} / {:.1} L", self.store.water_taken as f32 / 1000.0, self.settings.water_goal as f32 / 1000.0)),
            4 => {
                let open = self.store.todos.iter().filter(|t| !t.done).count();
                (Some('\u{E73A}'), GREEN, format!("{open} task{}", if open == 1 { "" } else { "s" }))
            }
            _ => (None, GREEN, format!("{}%{}", self.battery.percent, if self.battery.charging { " ⚡" } else { "" })),
        };
        let cx = l + w / 2.0;
        match (kind, glyph) {
            (3, _) => self.drop_icon(l + 22.0, cy, col.a(a)),
            (5, _) => self.draw_ring(l + 22.0, cy, a),
            (_, Some(ch)) => g.icon(ch, l + 22.0, cy, 13.0, col.a(a)),
            _ => {}
        }
        let shown = g.fit(&text, 13.5, 600, w - 60.0);
        g.text(&shown, cx - w / 2.0 + 38.0, cy - 10.0, w - 46.0, 20.0, 13.5, 600, WHITE.a(a), Align::Center);
    }

    /// A tiny blob that blinks and glances toward the cursor.
    pub fn draw_mascot(&self, cx: f32, cy: f32, a: f32) {
        let g = &self.gfx;
        let t = self.last.duration_since(self.start).as_secs_f32();
        let blink = (t % 4.3) > 4.15;
        let bob = (t * 1.6).sin() * 0.6;
        let cy = cy + bob;
        g.fill_round(cx - 8.0, cy - 6.5, 16.0, 13.0, 6.5, Color::rgba(170, 142, 255, a));
        g.fill_circle(cx - 6.5, cy - 6.0, 2.2, Color::rgba(170, 142, 255, a));
        g.fill_circle(cx + 6.5, cy - 6.0, 2.2, Color::rgba(170, 142, 255, a));
        let dx = ((self.mouse.0 - cx) / 220.0).clamp(-1.0, 1.0) * 0.9;
        let dy = ((self.mouse.1 - cy) / 120.0).clamp(-1.0, 1.0) * 0.7;
        for ex in [-3.2f32, 3.2] {
            if blink {
                g.fill_round(cx + ex - 2.0, cy - 0.3, 4.0, 1.0, 0.5, BLACK.a(0.8 * a));
            } else {
                g.fill_circle(cx + ex, cy - 0.5, 2.3, WHITE.a(a));
                g.fill_circle(cx + ex + dx, cy - 0.5 + dy, 1.1, BLACK.a(a));
            }
        }
    }

    fn poll_events(&mut self, now: Instant) {
        // HUD triggers coming from the keyboard hook
        if let Some((k, at)) = self.hud_pending {
            if now >= at {
                self.hud_pending = None;
                match k {
                    0 if self.settings.hud_volume => {
                        if let Some((v, m)) = sys::volume() {
                            self.show_hud(0, if m { 0.0 } else { v }, 1.6);
                        }
                    }
                    1 if self.settings.hud_caps => self.show_hud(1, if sys::caps_on() { 1.0 } else { 0.0 }, 1.4),
                    2 if self.settings.hud_lang => self.show_hud(2, 0.0, 1.4),
                    _ => {}
                }
            }
        }
        // Wi-Fi / Bluetooth popups
        for e in self.net.drain() {
            let secs = Duration::from_secs(4);
            let t = match e {
                NetEvent::WifiUp(ssid) if self.settings.pop_wifi => Some(Toast { icon: '\u{E701}', color: BLUE, title: "Wi-Fi connected".into(), body: ssid, a: None, b: None, until: now + secs, tag: 6, idx: 0 }),
                NetEvent::WifiDown if self.settings.pop_wifi => Some(Toast { icon: '\u{F384}', color: AMBER, title: "Wi-Fi disconnected".into(), body: "No wireless connection".into(), a: None, b: None, until: now + secs, tag: 6, idx: 0 }),
                NetEvent::BtUp(n) if self.settings.pop_bt => Some(Toast { icon: '\u{E7F6}', color: BLUE, title: "Connected".into(), body: n, a: None, b: None, until: now + secs, tag: 6, idx: 0 }),
                NetEvent::BtDown(n) if self.settings.pop_bt => Some(Toast { icon: '\u{E7F6}', color: AMBER, title: "Disconnected".into(), body: n, a: None, b: None, until: now + secs, tag: 6, idx: 0 }),
                _ => None,
            };
            if let Some(t) = t {
                self.push_toast(t);
            }
        }
    }

    pub fn show_hud(&mut self, kind: u32, value: f32, secs: f32) {
        if self.toast.is_none() && !self.expanded {
            self.hud = Some((kind, Instant::now() + Duration::from_secs_f32(secs), value));
            self.dirty.set(true);
        }
    }

    pub fn draw_bars(&self, x: f32, cy: f32, a: f32) {
        let g = &self.gfx;
        for i in 0..5 {
            let hh = 3.0 + self.bars[i] * 11.0;
            g.fill_round(x + i as f32 * 4.0, cy - hh / 2.0, 2.0, hh, 1.0, WHITE.a(0.9 * a));
        }
    }

    pub fn draw_ring(&self, cx: f32, cy: f32, a: f32) {
        let g = &self.gfx;
        let b = self.battery;
        let col = if b.percent <= 15 && !b.charging { RED } else if b.percent <= 30 && !b.charging { AMBER } else { GREEN };
        g.stroke_circle(cx, cy, 6.5, 2.0, Color::rgba(255, 255, 255, 0.16 * a));
        g.arc(cx, cy, 6.5, b.percent as f32 / 100.0, 2.0, col.a(a));
        if b.charging {
            g.icon('\u{E945}', cx, cy, 7.0, col.a(a));
        }
    }

    fn draw_expanded(&self, l: f32, yo: f32, w: f32, _h: f32, a: f32, _p: f32) {
        let g = &self.gfx;
        let kinds = self.pages();
        let lv = if self.lyrics_view && self.settings.lyrics { (1.0 - self.page_pos.value).clamp(0.0, 1.0) } else { 0.0 };

        // header, right aligned to the content edge: ring | date | clock
        // On the Assistant page, the assistant renders its own header (title, status, clock, clear)
        let is_ai = kinds.get(self.page) == Some(&PageKind::Assistant);
        let ha = a * (1.0 - lv);
        if ha > 0.01 && !is_ai {
            let hy = yo + 18.0;
            let right = l + w - 68.0;
            self.draw_bars(l + w - 58.0, hy, ha * if self.is_playing() { 1.0 } else { 0.5 });
            let clock = self.clock_text();
            g.text(&clock, right - 52.0, hy - 10.0, 52.0, 20.0, 14.5, 700, WHITE.a(ha), Align::Right);
            let n = sys::now();
            let date = format!("{} {} {} {}", sys::DOW[n.dow as usize], n.day, sys::MON[(n.month - 1) as usize], n.year);
            let dr = right - 52.0 - 8.0;
            g.text(&date, dr - 150.0, hy - 10.0, 150.0, 20.0, 12.5, 500, WHITE.a(0.62 * ha), Align::Right);
            if self.settings.status_ring {
                self.draw_ring(dr - g.measure(&date, 12.5, 500) - 14.0, hy, ha);
            }
        }

        // pages (slide + fade); clipped above the dock so a sliding page never covers it
        let pp = self.page_pos.value;
        for (i, k) in kinds.iter().enumerate() {
            let d = i as f32 - pp;
            if d.abs() >= 1.0 {
                continue;
            }
            let pa = a * (1.0 - d.abs()).clamp(0.0, 1.0);
            let ox = l + d * 60.0 + PAGE_PAD;
            g.push_clip(l + 8.0, yo, w - 16.0, EXP_H - DOCK_H + 2.0);
            self.draw_page(*k, ox, yo, pa);
            g.pop_clip();
        }
        if kinds.len() > 1 {
            let aa = a * (1.0 - lv);
            if aa > 0.02 && self.settings.page_icons {
                self.arrow(l + 17.0, yo + 70.0, true, aa, Act::PageLeft);
                self.arrow(l + w - 17.0, yo + 70.0, false, aa, Act::PageRight);
            }
            self.draw_dock(&kinds, l + w / 2.0, yo + EXP_H - 17.0, a);
        }
    }

    pub fn tab_color(k: PageKind) -> Color {
        match k {
            PageKind::Media => Color::rgb(255, 55, 95),
            PageKind::Widgets => BLUE,
            PageKind::Calendar => RED,
            PageKind::Focus => Color::rgb(124, 108, 255),
            PageKind::Water => BLUE,
            PageKind::Shelf => Color::rgb(255, 189, 46),
            PageKind::Todo => GREEN,
            PageKind::Prayer => Color::rgb(32, 201, 166),
            PageKind::Assistant => Color::rgb(167, 139, 250),
        }
    }

    /// Bottom page indicator: minimal dots + active dash by default (Image 1),
    /// smoothly animating and morphing into the full dock bar on hover (Image 2).
    fn draw_dock(&self, kinds: &[PageKind], cx: f32, cy: f32, a: f32) {
        let g = &self.gfx;
        let dock_t = self.dock_hover.value.clamp(0.0, 1.0);
        let n = kinds.len();
        let pp = self.page_pos.value.clamp(0.0, (n - 1) as f32);

        // 1. Calculate dots dimensions and positions
        let gap = 5.5;
        let mut dot_widths = vec![3.5f32; n];
        for i in 0..n {
            let diff = (pp - i as f32).abs();
            let active_weight = (1.0 - diff).clamp(0.0, 1.0);
            dot_widths[i] = 3.5 + (16.0 - 3.5) * active_weight;
        }
        let total_dots_w: f32 = dot_widths.iter().sum::<f32>() + (n as f32 - 1.0) * gap;

        // 2. Calculate full dock dimensions
        let slot = 33.0;
        let n_f = n as f32;
        let total = n_f * slot + 10.0 + slot;
        let full_w = total + 14.0;
        let x0 = cx - total / 2.0;

        // 3. Dynamic morphing pill container (expands horizontally and vertically from dots)
        let cur_w = lerp(total_dots_w + 14.0, full_w, smoothstep(0.0, 1.0, dock_t));
        let cur_h = lerp(16.0, 26.0, smoothstep(0.0, 1.0, dock_t));
        let cur_r = cur_h / 2.0;
        let cur_x = cx - cur_w / 2.0;
        let cur_y = cy - cur_h / 2.0;
        let bg_a = smoothstep(0.06, 0.70, dock_t) * a;
        if bg_a > 0.01 {
            g.fill_round(cur_x, cur_y, cur_w, cur_h, cur_r, Color::rgba(18, 18, 22, 0.90 * bg_a));
            g.fill_round(cur_x, cur_y, cur_w, cur_h, cur_r, WHITE.a(0.05 * bg_a));
            g.stroke_round(cur_x, cur_y, cur_w, cur_h, cur_r, 1.0, WHITE.a(0.12 * bg_a));
        }

        // 4. Compact dots + active dash indicator (fades out as dock opens)
        let dot_a = (1.0 - smoothstep(0.0, 0.40, dock_t)) * a;
        if dot_a > 0.01 {
            let mut cur_dx = cx - total_dots_w / 2.0;
            for (i, &w_i) in dot_widths.iter().enumerate() {
                let diff = (pp - i as f32).abs();
                let active_weight = (1.0 - diff).clamp(0.0, 1.0);
                let col = WHITE.a((0.38 + 0.57 * active_weight) * dot_a);
                g.fill_round(cur_dx, cy - 1.75, w_i, 3.5, 1.75, col);
                if dock_t <= 0.5 {
                    let r = Rect { x: cur_dx - 3.0, y: cy - 10.0, w: w_i + 6.0, h: 20.0 };
                    self.hit(r, Act::Dot(i));
                }
                cur_dx += w_i + gap;
            }
        }

        // 5. Full dock icons, selection capsule, and settings gear (animates in)
        let full_a = smoothstep(0.20, 0.90, dock_t) * a;
        if full_a > 0.01 {
            let enter_scale = lerp(0.70, 1.0, smoothstep(0.15, 1.0, dock_t));
            let enter_dy = (1.0 - smoothstep(0.15, 1.0, dock_t)) * 4.0;
            let icon_cy = cy + enter_dy;

            // Sliding selection capsule (colour blends between neighbouring pages)
            let (i0, i1) = (pp.floor() as usize, (pp.ceil() as usize).min(kinds.len() - 1));
            let col = Self::tab_color(kinds[i0]).mix(Self::tab_color(kinds[i1]), pp - pp.floor());
            let ccx = x0 + pp * slot + slot / 2.0;
            let cap_w = 30.0 * enter_scale;
            let cap_h = 21.0 * enter_scale;
            let cap_r = cap_h / 2.0;
            g.fill_round(ccx - cap_w / 2.0, icon_cy - cap_h / 2.0, cap_w, cap_h, cap_r, col.a(0.20 * full_a));
            g.stroke_round(ccx - cap_w / 2.0, icon_cy - cap_h / 2.0, cap_w, cap_h, cap_r, 1.0, col.a(0.60 * full_a));

            for (i, k) in kinds.iter().enumerate() {
                let c = x0 + i as f32 * slot + slot / 2.0;
                let active = (self.page_pos.value - i as f32).abs() < 0.5;
                let r = Rect { x: c - 15.0, y: cy - 13.0, w: 30.0, h: 26.0 };
                let hot = self.hovered(r);
                if hot && !active && dock_t > 0.6 {
                    g.fill_round(c - 13.0, cy - 10.5, 26.0, 21.0, 10.5, WHITE.a(0.08 * a));
                }
                let icon_size = 13.0 * enter_scale * if hot { 1.12 } else { 1.0 };
                let ic = if active { Self::tab_color(*k) } else { WHITE.a(if hot { 1.0 } else { 0.55 }) }.a(full_a);
                match k {
                    PageKind::Water => self.drop_icon(c, icon_cy, ic),
                    PageKind::Media => g.icon('\u{E8D6}', c, icon_cy, icon_size, ic),
                    PageKind::Widgets => g.icon('\u{E8A9}', c, icon_cy, icon_size, ic),
                    PageKind::Calendar => g.icon('\u{E787}', c, icon_cy, icon_size, ic),
                    PageKind::Focus => g.icon('\u{E916}', c, icon_cy, icon_size, ic),
                    PageKind::Shelf => g.icon('\u{E8B7}', c, icon_cy, icon_size, ic),
                    PageKind::Todo => g.icon('\u{E73A}', c, icon_cy, icon_size, ic),
                    PageKind::Prayer => g.icon('\u{E708}', c, icon_cy, icon_size, ic),
                    PageKind::Assistant => self.sparkle(c, icon_cy, 7.5 * enter_scale * if hot { 1.12 } else { 1.0 }, ic),
                }
                if dock_t > 0.5 {
                    self.hit(r, Act::Dot(i));
                }
            }

            // Divider line
            let div_h = 14.0 * smoothstep(0.25, 1.0, dock_t);
            g.fill_rect(x0 + n_f * slot + 4.0, cy - div_h / 2.0, 1.0, div_h, WHITE.a(0.16 * full_a));

            // Settings button
            let gx = x0 + n_f * slot + 10.0 + slot / 2.0;
            let r_set = Rect { x: gx - 15.0, y: cy - 13.0, w: 30.0, h: 26.0 };
            let set_hot = self.hovered(r_set);
            if set_hot && dock_t > 0.6 {
                g.fill_round(gx - 13.0, cy - 10.5, 26.0, 21.0, 10.5, WHITE.a(0.08 * a));
            }
            let gear_size = 13.0 * enter_scale * if set_hot { 1.12 } else { 1.0 };
            g.icon('\u{E713}', gx, icon_cy, gear_size, WHITE.a(if set_hot { 1.0 } else { 0.6 } * full_a));
            if dock_t > 0.5 {
                self.hit(r_set, Act::Settings);
            }
        }
    }

    pub fn drop_icon(&self, c: f32, cy: f32, col: Color) {
        let g = &self.gfx;
        g.fill_circle(c, cy + 2.2, 4.6, col);
        g.poly(&[(c, cy - 6.6), (c - 4.0, cy + 0.2), (c + 4.0, cy + 0.2)], col, 0.8);
    }

    fn arrow(&self, cx: f32, cy: f32, left: bool, a: f32, act: Act) {
        let g = &self.gfx;
        let hot = self.hovered(Rect { x: cx - 10.0, y: cy - 10.0, w: 20.0, h: 20.0 });
        g.fill_circle(cx, cy, 8.5, Color::rgba(255, 255, 255, if hot { 0.30 } else { 0.18 } * a));
        g.icon(if left { '\u{E76B}' } else { '\u{E76C}' }, cx, cy, 8.0, WHITE.a(0.9 * a));
        self.hit(Rect { x: cx - 12.0, y: cy - 14.0, w: 24.0, h: 28.0 }, act);
    }

    pub fn hit(&self, r: Rect, a: Act) {
        self.hits.borrow_mut().push((r, a));
    }
    pub fn hovered(&self, r: Rect) -> bool {
        r.contains(self.mouse.0, self.mouse.1)
    }

    /// Icon button with hover feedback; registers its hit rect.
    pub fn icon_btn(&self, glyph: char, cx: f32, cy: f32, size: f32, a: f32, act: Act) {
        let hot = self.hovered(Rect { x: cx - 14.0, y: cy - 13.0, w: 28.0, h: 26.0 });
        self.gfx.icon(glyph, cx, cy, size * if hot { 1.08 } else { 1.0 }, WHITE.a(if hot { 1.0 } else { 0.82 } * a));
        self.hit(Rect { x: cx - 15.0, y: cy - 14.0, w: 30.0, h: 28.0 }, act);
    }

    // --------------------------------------------------------------- input
    pub fn on_click(&mut self, lx: f32, ly: f32) {
        if std::env::var("NOOK_DEBUG_CLICK").is_ok() {
            let n = self.hits.borrow().len();
            sys::log(&format!("click at {lx:.1},{ly:.1} view={:?} hits={n} page={}", self.view, self.page));
        }
        let hit = {
            let hs = self.hits.borrow();
            hs.iter().rev().find(|(r, _)| r.contains(lx, ly)).map(|(r, a)| (*r, *a))
        };
        if self.tour.is_some() {
            // while the tour runs only its own buttons react
            if let Some((r, act)) = hit {
                if matches!(act, Act::TourNext | Act::TourBack | Act::TourSkip) {
                    self.perform(act, lx, r);
                }
            }
            self.dirty.set(true);
            return;
        }
        if let Some((r, act)) = hit {
            self.perform(act, lx, r);
        } else if self.settings.open_on_click && !self.expanded {
            self.expanded = true;
        }
        self.dirty.set(true);
    }

    pub fn perform(&mut self, act: Act, lx: f32, r: Rect) {
        let n = self.pages().len();
        match act {
            Act::PageLeft => self.page = (self.page + n - 1) % n,
            Act::PageRight => self.page = (self.page + 1) % n,
            Act::Dot(i) => self.page = i,
            Act::PlayPause => self.media.send(MediaCmd::PlayPause),
            Act::Prev => self.media.send(MediaCmd::Prev),
            Act::Next => self.media.send(MediaCmd::Next),
            Act::Seek => {
                let f = ((lx - r.x) / r.w).clamp(0.0, 1.0);
                self.seek_drag = Some(f);
            }
            Act::Lyrics => {
                self.lyrics_view = !self.lyrics_view;
                if !self.lyrics_view {
                    self.lyric_bar_w = None;
                }
                self.dirty.set(true);
            }
            Act::PrayOpen => {
                self.pray_pick = !self.pray_pick;
                if self.pray_pick {
                    match crate::prayer::current_country(&self.settings) {
                        Some(i) => {
                            self.pray_country = i;
                            self.pray_level = 1;
                        }
                        None => self.pray_level = 0,
                    }
                    self.pray_scroll_to_current();
                }
            }
            Act::PrayCountry(c) => {
                self.pray_country = c;
                self.pray_level = 1;
                self.pray_scroll_to_current();
            }
            Act::PrayBack => {
                self.pray_level = 0;
                self.pray_scroll_to_current();
            }
            Act::PrayPick(p, c) => {
                crate::prayer::apply_target(&mut self.settings, &crate::prayer::Target::Builtin(p, c));
                self.settings.save();
                self.pray_pick = false;
            }
            Act::PrayWorld(w, c) => {
                crate::prayer::apply_target(&mut self.settings, &crate::prayer::Target::World(w, c));
                self.settings.save();
                self.pray_pick = false;
            }
            Act::ChatFocus | Act::ChatMic | Act::ChatSend | Act::ChatClear => self.chat_act(act),
            Act::WhatsNewClose => self.whats_new_close(),
            Act::TourNext => self.tour_next(),
            Act::TourBack => self.tour_back(),
            Act::TourSkip => self.tour_finish(),
            Act::Source => self.media.send(MediaCmd::NextSource),
            Act::Volume => sys::key_tap(VK_VOLUME_MUTE),
            Act::VolSlider => {
                sys::set_volume(((lx - r.x) / r.w).clamp(0.0, 1.0));
                self.vol_cache.set((Instant::now() - Duration::from_secs(5), 0.0));
                self.vol_drag = true;
            }
            Act::Settings => crate::settings_ui::open(),
            other => self.page_action(other, lx, r),
        }
    }

    pub fn on_move(&mut self, lx: f32) {
        if self.vol_drag {
            let r = self.hits.borrow().iter().find(|(_, a)| *a == Act::VolSlider).map(|(r, _)| *r);
            if let Some(r) = r {
                sys::set_volume(((lx - r.x) / r.w).clamp(0.0, 1.0));
                self.vol_cache.set((Instant::now() - Duration::from_secs(5), 0.0));
                self.dirty.set(true);
            }
        }
        if self.seek_drag.is_some() {
            let hs = self.hits.borrow();
            if let Some((r, _)) = hs.iter().find(|(_, a)| *a == Act::Seek) {
                let f = ((lx - r.x) / r.w).clamp(0.0, 1.0);
                drop(hs);
                self.seek_drag = Some(f);
                self.dirty.set(true);
            }
        }
    }

    pub fn on_release(&mut self) {
        self.vol_drag = false;
        if let Some(f) = self.seek_drag.take() {
            let dur = self.media.state.lock().unwrap().duration;
            if dur > 0.0 {
                self.media.send(MediaCmd::Seek(dur * f as f64));
            }
        }
        self.dirty.set(true);
    }

    pub fn on_wheel(&mut self, delta: i16) {
        let now = Instant::now();
        // wheel over the closed pill = volume (every style)
        if !self.expanded && self.mouse_in && self.tour.is_none() && !self.ai_busy() {
            if let Some((v, _)) = sys::volume() {
                let nv = (v + if delta > 0 { 0.04 } else { -0.04 }).clamp(0.0, 1.0);
                sys::set_volume(nv);
                self.show_hud(0, nv, 1.6);
            }
            return;
        }
        // the prayer city list scrolls with the wheel instead of changing page
        if self.expanded && self.pray_pick && self.tour.is_none() && self.pages().get(self.page) == Some(&PageKind::Prayer) {
            let n = self.pray_items().len();
            let max = ((n as f32 - 3.0) * 22.0).max(0.0);
            self.pray_scroll.target = (self.pray_scroll.target - delta as f32 / 120.0 * 44.0).clamp(0.0, max);
            self.dirty.set(true);
            return;
        }
        // a long assistant answer scrolls under the wheel
        if self.expanded && self.tour.is_none() && self.pages().get(self.page) == Some(&PageKind::Assistant) {
            let (mx, my) = self.mouse;
            let l = WIN_W / 2.0 - EXP_W / 2.0 + PAGE_PAD;
            if mx > l + 18.0 && mx < l + 402.0 && my > self.y_off.value + 16.0 && my < self.y_off.value + 76.0 {
                let reply = self.assistant.view().reply;
                let h = self.gfx.measure_wrap(&reply, 12.0, 500, 370.0);
                let max = (h - 36.0).max(0.0);
                if max > 0.0 {
                    self.chat_scroll.set((self.chat_scroll.get() - delta as f32 / 120.0 * 16.0).clamp(0.0, max));
                    self.dirty.set(true);
                    return;
                }
            }
        }
        // wheel over the speaker icon adjusts system volume
        if self.expanded && self.pages().get(self.page) == Some(&PageKind::Media) {
            let (mx, my) = self.mouse;
            let l = WIN_W / 2.0 - EXP_W / 2.0;
            if (mx - (l + PAGE_PAD + 388.0)).abs() < 18.0 && (my - 108.0 - self.y_off.value).abs() < 16.0 {
                if let Some((v, _)) = sys::volume() {
                    sys::set_volume(v + if delta > 0 { 0.04 } else { -0.04 });
                }
                return;
            }
        }
        if now.duration_since(self.last_wheel) < Duration::from_millis(220) || !self.expanded || self.tour.is_some() {
            return;
        }
        self.last_wheel = now;
        let n = self.pages().len();
        if delta < 0 {
            self.page = (self.page + 1) % n;
        } else {
            self.page = (self.page + n - 1) % n;
        }
        self.dirty.set(true);
    }

    pub fn on_clipboard(&mut self) {
        if !self.settings.clip_history {
            return;
        }
        if let Some(t) = read_clipboard_text() {
            let max = self.settings.clip_max as usize;
            self.store.add_clip(t, max);
            self.store.save_clips(self.settings.clip_persist);
            self.dirty.set(true);
        }
    }

    pub fn on_drop_files(&mut self, files: Vec<std::path::PathBuf>) {
        for f in files {
            if !self.store.shelf.contains(&f) {
                self.store.shelf.insert(0, f);
            }
        }
        self.store.shelf.truncate(30);
        self.store.save_shelf();
        // jump to the shelf page
        let pk = self.pages();
        if let Some(i) = pk.iter().position(|k| *k == PageKind::Shelf) {
            self.page = i;
            self.expanded = true;
        }
        self.dirty.set(true);
    }

    pub fn context_menu(&mut self) {
        unsafe {
            let m = CreatePopupMenu().unwrap();
            let _ = AppendMenuW(m, MF_STRING, 1, w!("Settings…"));
            let _ = AppendMenuW(m, MF_STRING, 4, w!("Show tutorial"));
            let flags = if self.settings.autostart { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(m, flags, 2, w!("Start with Windows"));
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(m, MF_STRING, 3, w!("Quit Nook"));
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, p.x, p.y, 0, self.hwnd, None);
            let _ = DestroyMenu(m);
            match cmd.0 {
                1 => crate::settings_ui::open(),
                4 => self.start_tour(),
                2 => {
                    self.settings.autostart = !self.settings.autostart;
                    self.apply_settings();
                }
                3 => {
                    self.remove_tray();
                    PostQuitMessage(0);
                }
                _ => {}
            }
        }
    }
}

fn r_of(l: f32, w: f32) -> f32 {
    l + w
}

static mut HOOK_WND: isize = 0;

unsafe extern "system" fn kb_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && (wp.0 as u32 == WM_KEYDOWN || wp.0 as u32 == WM_SYSKEYDOWN) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let down = |vk: VIRTUAL_KEY| (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0;
        let mut kind: Option<u32> = None;
        match k.vkCode {
            0xAD | 0xAE | 0xAF => kind = Some(0),
            0x14 => kind = Some(1),
            0x20 if down(VK_LWIN) || down(VK_RWIN) => kind = Some(2),
            0xA0 | 0xA1 | 0x10 if down(VK_MENU) || down(VK_CONTROL) => kind = Some(2),
            0xA4 | 0xA5 | 0x12 if down(VK_SHIFT) => kind = Some(2),
            _ => {}
        }
        if let Some(kd) = kind {
            let _ = PostMessageW(HWND(HOOK_WND as *mut _), WM_HUD, WPARAM(kd as usize), LPARAM(0));
        }
    }
    // never swallow keys: the hook only observes
    CallNextHookEx(None, code, wp, lp)
}

fn install_hook(hwnd: HWND) {
    unsafe {
        HOOK_WND = hwnd.0 as isize;
        let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), hinst, 0);
    }
}

pub fn read_clipboard_text() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(h) = GetClipboardData(13u32) {
            // CF_UNICODETEXT
            let hg = HGLOBAL(h.0 as *mut _);
            let p = GlobalLock(hg) as *const u16;
            if !p.is_null() {
                let mut len = 0;
                while *p.add(len) != 0 {
                    len += 1;
                    if len > 4000 {
                        break;
                    }
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

pub fn set_clipboard_text(s: &str) {
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        let wide: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let p = GlobalLock(hg) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(hg);
                let _ = SetClipboardData(13u32, HANDLE(hg.0 as *mut _));
            }
        }
        let _ = CloseClipboard();
    }
}

// ------------------------------------------------------------------ window
pub unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(a) = app() else { return DefWindowProcW(hwnd, msg, wp, lp) };
    let to_local = |lp: LPARAM| -> (f32, f32) {
        // client coordinates are relative to the (shrunken) window; add its offset inside the canvas
        let x = ((lp.0 & 0xFFFF) as i16 as i32 + a.view.0.max(0)) as f32 / a_scale();
        let y = (((lp.0 >> 16) & 0xFFFF) as i16 as i32 + a.view.1.max(0)) as f32 / a_scale();
        (x, y)
    };
    match msg {
        WM_NCHITTEST => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            let lx = (x - a.win_px.0) as f32 / a.scale;
            let ly = (y - a.win_px.1) as f32 / a.scale;
            if a.shape_rect().contains(lx, ly) {
                LRESULT(1) // HTCLIENT
            } else {
                LRESULT(-1) // HTTRANSPARENT
            }
        }
        WM_MOUSEACTIVATE => LRESULT(if a.chat_focus { 1 } else { 3 }), // MA_ACTIVATE while typing, else MA_NOACTIVATE
        WM_CHAR => {
            if a.chat_focus {
                a.chat_char(wp.0 as u32);
            }
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            if a.chat_focus {
                a.chat_set_focus(false);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let _ = SetCursor(LoadCursorW(None, IDC_ARROW).unwrap_or_default());
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = to_local(lp);
            let _ = SetCapture(hwnd);
            a.on_click(x, y);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, _) = to_local(lp);
            a.on_move(x);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            a.on_release();
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            a.context_menu();
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let d = ((wp.0 >> 16) & 0xFFFF) as i16;
            a.on_wheel(d);
            LRESULT(0)
        }
        WM_HUD => {
            let delay = if wp.0 == 1 { 40 } else { 90 };
            a.hud_pending = Some((wp.0 as u32, Instant::now() + Duration::from_millis(delay)));
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE_ => {
            a.on_clipboard();
            LRESULT(0)
        }
        WM_DROPFILES => {
            let hd = HDROP(wp.0 as *mut _);
            let n = DragQueryFileW(hd, 0xFFFFFFFF, None);
            let mut v = vec![];
            for i in 0..n {
                let len = DragQueryFileW(hd, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hd, i, Some(&mut buf));
                v.push(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])));
            }
            DragFinish(hd);
            a.on_drop_files(v);
            LRESULT(0)
        }
        WM_TRAY => {
            let ev = (lp.0 & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                a.context_menu();
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
            a.reposition();
            LRESULT(0)
        }
        WM_DESTROY => {
            a.remove_tray();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn a_scale() -> f32 {
    app().map(|a| a.scale).unwrap_or(1.0)
}

pub fn run_loop(_hwnd: HWND) {
    unsafe {
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let animating = match app() {
                Some(a) => a.tick(),
                None => false,
            };
            if animating {
                // spring motion: pace to the compositor so we render exactly once per frame
                let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            } else {
                windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjects(None, false, 12, QS_ALLINPUT);
            }
        }
    }
}
