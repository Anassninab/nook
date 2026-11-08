//! Plain key=value settings + small data files under %APPDATA%\Nook.
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let p = PathBuf::from(base).join("Nook");
    let _ = fs::create_dir_all(&p);
    p
}

pub fn local_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    let p = PathBuf::from(base).join("Nook");
    let _ = fs::create_dir_all(&p);
    p
}

pub fn read_kv(name: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(s) = fs::read_to_string(dir().join(name)) {
        for l in s.lines() {
            if let Some((k, v)) = l.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

pub fn write_kv(name: &str, m: &BTreeMap<String, String>) {
    let mut s = String::new();
    for (k, v) in m {
        s.push_str(&format!("{k}={v}\n"));
    }
    let _ = fs::write(dir().join(name), s);
}

#[derive(Clone, Debug)]
pub struct Settings {
    // General
    pub autostart: bool,
    pub notch_scale: u32, // 0 = auto, else percent
    pub minimal: bool,
    pub mascot: bool,
    pub sounds: bool,
    pub hide_fullscreen: bool,
    pub auto_update: bool,
    pub tour_done: bool,
    pub idle_scenes: bool,
    // assistant
    pub ai_enabled: bool,
    pub ai_provider: u32,
    pub ai_key: String,
    pub ai_model: String,
    pub ai_url: String,
    pub ai_voice: bool,
    pub ai_voice_name: String,
    pub ai_neural: bool,
    pub last_seen_version: String,
    pub hud_brightness: bool,
    pub ai_rate: u32,
    pub circle_mode: u32,
    pub dot_mode: bool,
    // prayer: any city in the world (used instead of the built-in Indonesia / Morocco lists when `pray_custom`)
    pub pray_custom: bool,
    pub pray_name: String,
    pub pray_lat: f64,
    pub pray_lon: f64,
    pub pray_tz: String,
    pub pray_cc: String,
    // Status ring
    pub status_ring: bool,
    // Display
    pub monitor: u32,
    // Appearance
    pub floating: bool,
    pub island: bool,
    pub glass: bool,
    pub glass_tint: u32,
    pub open_on_click: bool,
    pub page_icons: bool,
    pub fast_anim: bool,
    // Music
    pub lyrics: bool,
    pub lyric_bar: bool,
    pub theme: u32,
    // Widgets
    pub widgets_page: bool,
    pub widget_left: u32,
    pub widget_right: u32,
    pub clock_a: u32,
    pub clock_b: u32,
    pub countdown: u32,
    // To-do
    pub todo_start: bool,
    pub todo_end: bool,
    pub snooze: u32,
    pub todo_ordered: bool,
    // Calendar
    pub week_monday: bool,
    pub cal_week_view: bool,
    // Pomodoro
    pub pomo_focus: u32,
    pub pomo_short: u32,
    pub pomo_long: u32,
    // Water
    pub water: bool,
    pub water_goal: u32,
    pub bottle: u32,
    pub water_from: u32,
    pub water_to: u32,
    // Prayer
    pub prayer: bool,
    pub prayer_prov: usize,
    pub prayer_city: usize,
    pub pray_at: bool,
    pub pray_before: bool,
    pub pray_sound: bool,
    pub pray_pause: bool,
    pub pray_adjust: [i32; 5],
    // Shelf
    pub shelf_empty: bool,
    pub clip_history: bool,
    pub clip_max: u32,
    pub clip_persist: bool,
    // HUD
    pub hud_volume: bool,
    pub hud_caps: bool,
    pub hud_lang: bool,
    pub pop_battery: bool,
    pub pop_wifi: bool,
    pub pop_bt: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false, notch_scale: 0, minimal: false, mascot: true, sounds: true, hide_fullscreen: true, auto_update: true, tour_done: false, idle_scenes: true,
            ai_enabled: true, ai_provider: 0, ai_key: String::new(), ai_model: String::new(), ai_url: String::new(), ai_voice: true, ai_voice_name: String::new(), ai_neural: true, last_seen_version: String::new(), hud_brightness: true, ai_rate: 100, circle_mode: 0, dot_mode: false,
            pray_custom: false, pray_name: String::new(), pray_lat: 0.0, pray_lon: 0.0, pray_tz: String::new(), pray_cc: String::new(),
            status_ring: true, monitor: 0, floating: false, island: false, glass: false, glass_tint: 40, open_on_click: false,
            page_icons: true, fast_anim: false, lyrics: true, lyric_bar: true, theme: 0, widgets_page: false, widget_left: 0, widget_right: 1,
            clock_a: 0, clock_b: 1, countdown: 0, todo_start: true, todo_end: true, snooze: 10, todo_ordered: true,
            week_monday: false, cal_week_view: false, pomo_focus: 25, pomo_short: 5, pomo_long: 15, water: true,
            water_goal: 1500, bottle: 500, water_from: 480, water_to: 1020, prayer: true, prayer_prov: 0,
            prayer_city: 0, pray_at: true, pray_before: false, pray_sound: false, pray_pause: false,
            pray_adjust: [0; 5], shelf_empty: false, clip_history: false, clip_max: 5, clip_persist: false,
            hud_volume: true, hud_caps: true, hud_lang: true, pop_battery: true, pop_wifi: true, pop_bt: true,
        }
    }
}

fn b(m: &BTreeMap<String, String>, k: &str, d: bool) -> bool {
    m.get(k).map(|v| v == "1").unwrap_or(d)
}
fn n<T: std::str::FromStr>(m: &BTreeMap<String, String>, k: &str, d: T) -> T {
    m.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}

impl Settings {
    pub fn load() -> Self {
        let m = read_kv("settings.txt");
        let d = Settings::default();
        let mut adj = [0i32; 5];
        if let Some(v) = m.get("pray_adjust") {
            for (i, p) in v.split(',').take(5).enumerate() {
                adj[i] = p.parse().unwrap_or(0);
            }
        }
        Settings {
            autostart: b(&m, "autostart", d.autostart), notch_scale: n(&m, "notch_scale", d.notch_scale),
            minimal: b(&m, "minimal", d.minimal), mascot: b(&m, "mascot", d.mascot), sounds: b(&m, "sounds", d.sounds),
            hide_fullscreen: b(&m, "hide_fullscreen", d.hide_fullscreen), auto_update: b(&m, "auto_update", d.auto_update), tour_done: b(&m, "tour_done", d.tour_done), idle_scenes: b(&m, "idle_scenes", d.idle_scenes),
            ai_enabled: b(&m, "ai_enabled", d.ai_enabled), ai_provider: n(&m, "ai_provider", d.ai_provider), ai_voice: b(&m, "ai_voice", d.ai_voice),
            ai_key: m.get("ai_key").cloned().unwrap_or_default(), ai_model: m.get("ai_model").cloned().unwrap_or_default(), ai_url: m.get("ai_url").cloned().unwrap_or_default(),
            ai_voice_name: m.get("ai_voice_name").cloned().unwrap_or_default(), ai_neural: b(&m, "ai_neural", d.ai_neural), last_seen_version: m.get("last_seen_version").cloned().unwrap_or_default(), hud_brightness: b(&m, "hud_brightness", d.hud_brightness), ai_rate: n(&m, "ai_rate", d.ai_rate), circle_mode: n(&m, "circle_mode", d.circle_mode), dot_mode: b(&m, "dot_mode", d.dot_mode),
            pray_custom: b(&m, "pray_custom", d.pray_custom), pray_name: m.get("pray_name").cloned().unwrap_or_default(),
            pray_lat: n(&m, "pray_lat", d.pray_lat), pray_lon: n(&m, "pray_lon", d.pray_lon),
            pray_tz: m.get("pray_tz").cloned().unwrap_or_default(), pray_cc: m.get("pray_cc").cloned().unwrap_or_default(), status_ring: b(&m, "status_ring", d.status_ring),
            monitor: n(&m, "monitor", d.monitor), floating: b(&m, "floating", d.floating), island: b(&m, "island", d.island), glass: b(&m, "glass", d.glass),
            glass_tint: n(&m, "glass_tint", d.glass_tint), open_on_click: b(&m, "open_on_click", d.open_on_click),
            page_icons: b(&m, "page_icons", d.page_icons), fast_anim: b(&m, "fast_anim", d.fast_anim),
            lyrics: b(&m, "lyrics", d.lyrics), lyric_bar: b(&m, "lyric_bar", d.lyric_bar), theme: n(&m, "theme", d.theme), widgets_page: b(&m, "widgets_page", d.widgets_page),
            widget_left: n(&m, "widget_left", d.widget_left), widget_right: n(&m, "widget_right", d.widget_right),
            clock_a: n(&m, "clock_a", d.clock_a), clock_b: n(&m, "clock_b", d.clock_b), countdown: n(&m, "countdown", d.countdown),
            todo_start: b(&m, "todo_start", d.todo_start), todo_end: b(&m, "todo_end", d.todo_end),
            snooze: n(&m, "snooze", d.snooze), todo_ordered: b(&m, "todo_ordered", d.todo_ordered),
            week_monday: b(&m, "week_monday", d.week_monday), cal_week_view: b(&m, "cal_week_view", d.cal_week_view),
            pomo_focus: n(&m, "pomo_focus", d.pomo_focus), pomo_short: n(&m, "pomo_short", d.pomo_short),
            pomo_long: n(&m, "pomo_long", d.pomo_long), water: b(&m, "water", d.water), water_goal: n(&m, "water_goal", d.water_goal),
            bottle: n(&m, "bottle", d.bottle), water_from: n(&m, "water_from", d.water_from), water_to: n(&m, "water_to", d.water_to),
            prayer: b(&m, "prayer", d.prayer), prayer_prov: n(&m, "prayer_prov", d.prayer_prov),
            prayer_city: n(&m, "prayer_city", d.prayer_city), pray_at: b(&m, "pray_at", d.pray_at),
            pray_before: b(&m, "pray_before", d.pray_before), pray_sound: b(&m, "pray_sound", d.pray_sound),
            pray_pause: b(&m, "pray_pause", d.pray_pause), pray_adjust: adj,
            shelf_empty: b(&m, "shelf_empty", d.shelf_empty), clip_history: b(&m, "clip_history", d.clip_history),
            clip_max: n(&m, "clip_max", d.clip_max), clip_persist: b(&m, "clip_persist", d.clip_persist),
            hud_volume: b(&m, "hud_volume", d.hud_volume), hud_caps: b(&m, "hud_caps", d.hud_caps),
            hud_lang: b(&m, "hud_lang", d.hud_lang), pop_battery: b(&m, "pop_battery", d.pop_battery),
            pop_wifi: b(&m, "pop_wifi", d.pop_wifi), pop_bt: b(&m, "pop_bt", d.pop_bt),
        }
    }

    pub fn save(&self) {
        let mut m = BTreeMap::new();
        let mut put = |k: &str, v: String| {
            m.insert(k.to_string(), v);
        };
        let f = |x: bool| (x as u8).to_string();
        put("autostart", f(self.autostart)); put("notch_scale", self.notch_scale.to_string());
        put("minimal", f(self.minimal)); put("mascot", f(self.mascot)); put("sounds", f(self.sounds));
        put("hide_fullscreen", f(self.hide_fullscreen)); put("auto_update", f(self.auto_update)); put("tour_done", f(self.tour_done)); put("idle_scenes", f(self.idle_scenes));
        put("ai_enabled", f(self.ai_enabled)); put("ai_provider", self.ai_provider.to_string()); put("ai_voice", f(self.ai_voice));
        put("ai_key", self.ai_key.clone()); put("ai_model", self.ai_model.clone()); put("ai_url", self.ai_url.clone());
        put("ai_voice_name", self.ai_voice_name.clone()); put("ai_neural", f(self.ai_neural)); put("last_seen_version", self.last_seen_version.clone()); put("hud_brightness", f(self.hud_brightness)); put("ai_rate", self.ai_rate.to_string()); put("circle_mode", self.circle_mode.to_string()); put("dot_mode", f(self.dot_mode));
        put("pray_custom", f(self.pray_custom)); put("pray_name", self.pray_name.clone()); put("pray_lat", self.pray_lat.to_string());
        put("pray_lon", self.pray_lon.to_string()); put("pray_tz", self.pray_tz.clone()); put("pray_cc", self.pray_cc.clone()); put("status_ring", f(self.status_ring));
        put("monitor", self.monitor.to_string()); put("floating", f(self.floating)); put("island", f(self.island)); put("glass", f(self.glass));
        put("glass_tint", self.glass_tint.to_string()); put("open_on_click", f(self.open_on_click));
        put("page_icons", f(self.page_icons)); put("fast_anim", f(self.fast_anim)); put("lyrics", f(self.lyrics)); put("lyric_bar", f(self.lyric_bar)); put("theme", self.theme.to_string());
        put("widgets_page", f(self.widgets_page)); put("widget_left", self.widget_left.to_string());
        put("widget_right", self.widget_right.to_string()); put("clock_a", self.clock_a.to_string());
        put("clock_b", self.clock_b.to_string()); put("countdown", self.countdown.to_string());
        put("todo_start", f(self.todo_start)); put("todo_end", f(self.todo_end)); put("snooze", self.snooze.to_string());
        put("todo_ordered", f(self.todo_ordered)); put("week_monday", f(self.week_monday));
        put("cal_week_view", f(self.cal_week_view)); put("pomo_focus", self.pomo_focus.to_string());
        put("pomo_short", self.pomo_short.to_string()); put("pomo_long", self.pomo_long.to_string());
        put("water", f(self.water)); put("water_goal", self.water_goal.to_string()); put("bottle", self.bottle.to_string());
        put("water_from", self.water_from.to_string()); put("water_to", self.water_to.to_string());
        put("prayer", f(self.prayer)); put("prayer_prov", self.prayer_prov.to_string());
        put("prayer_city", self.prayer_city.to_string()); put("pray_at", f(self.pray_at));
        put("pray_before", f(self.pray_before)); put("pray_sound", f(self.pray_sound)); put("pray_pause", f(self.pray_pause));
        put("pray_adjust", self.pray_adjust.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
        put("shelf_empty", f(self.shelf_empty)); put("clip_history", f(self.clip_history));
        put("clip_max", self.clip_max.to_string()); put("clip_persist", f(self.clip_persist));
        put("hud_volume", f(self.hud_volume)); put("hud_caps", f(self.hud_caps)); put("hud_lang", f(self.hud_lang));
        put("pop_battery", f(self.pop_battery)); put("pop_wifi", f(self.pop_wifi)); put("pop_bt", f(self.pop_bt));
        write_kv("settings.txt", &m);
    }
}
