//! Page bodies for the expanded notch, toasts / HUD, and the background reminder services.
use std::time::{Duration, Instant};

use crate::app::*;
use crate::gfx::{Align, Color};
use crate::lyrics::{self, LyricsStatus};
use crate::media::MediaCmd;
use crate::prayer;
use crate::spring::*;
use crate::store::Phase;

use crate::sys;

const DIM: Color = Color::rgba(255, 255, 255, 0.55);
const INDIGO: Color = Color::rgb(124, 108, 255);
const FAINT: Color = Color::rgba(255, 255, 255, 0.35);
const CARD: Color = Color::rgba(255, 255, 255, 0.075);

pub fn fmt_time(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// "Chrome", "Spotify" … from an AppUserModelId such as "chrome.exe" or "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".
pub fn app_name(id: &str) -> String {
    let l = id.to_lowercase();
    for (k, n) in [("chrome", "Chrome"), ("msedge", "Edge"), ("edge", "Edge"), ("firefox", "Firefox"), ("spotify", "Spotify"), ("brave", "Brave"), ("opera", "Opera"), ("vlc", "VLC"), ("music.ui", "Media Player"), ("zunemusic", "Media Player"), ("apple", "Apple Music")] {
        if l.contains(k) {
            return n.to_string();
        }
    }
    id.trim_end_matches(".exe").to_string()
}

pub const WORLD: [(&str, f32); 10] = [
    ("Tokyo", 9.0), ("London", 0.0), ("New York", -5.0), ("Dubai", 4.0), ("Jakarta", 7.0),
    ("Sydney", 11.0), ("Paris", 1.0), ("Mecca", 3.0), ("Singapore", 8.0), ("Los Angeles", -8.0),
];
pub const WIDGET_NAMES: [&str; 8] = ["This week", "Next prayer", "Water", "Focus today", "World clock", "Battery", "Moon", "Countdown"];
pub const COUNTDOWNS: [&str; 4] = ["Ramadan", "Eid al-Fitr", "Eid al-Adha", "New Year"];

fn countdown_days(which: u32) -> (i64, &'static str) {
    let n = sys::now();
    let today = sys::days_from_civil(n.year, n.month, n.day);
    // Approximate Islamic-calendar dates (moon-sighting can shift them by a day).
    let table: &[(i32, u32, u32)] = match which {
        0 => &[(2026, 2, 18), (2027, 2, 8), (2028, 1, 28), (2029, 1, 16)],
        1 => &[(2026, 3, 20), (2027, 3, 10), (2028, 2, 26), (2029, 2, 14)],
        2 => &[(2026, 5, 27), (2027, 5, 17), (2028, 5, 5), (2029, 4, 24)],
        _ => &[(2026, 1, 1), (2027, 1, 1), (2028, 1, 1), (2029, 1, 1), (2030, 1, 1)],
    };
    for (y, m, d) in table {
        let t = sys::days_from_civil(*y, *m, *d);
        if t >= today {
            return (t - today, COUNTDOWNS[(which as usize).min(3)]);
        }
    }
    (0, COUNTDOWNS[(which as usize).min(3)])
}

fn moon_phase() -> (f32, &'static str) {
    let n = sys::now();
    let days = sys::days_from_civil(n.year, n.month, n.day) as f64 + n.hour as f64 / 24.0;
    // 2000-01-06 18:14 UTC new moon = day 10962.76
    let ph = (((days - 10962.76) / 29.530588853) % 1.0 + 1.0) % 1.0;
    let name = match (ph * 8.0 + 0.5) as u32 % 8 {
        0 => "New Moon",
        1 => "Waxing Crescent",
        2 => "First Quarter",
        3 => "Waxing Gibbous",
        4 => "Full Moon",
        5 => "Waning Gibbous",
        6 => "Last Quarter",
        _ => "Waning Crescent",
    };
    (ph as f32, name)
}

impl App {
    fn t(&self, s: &str, x: f32, cy: f32, w: f32, size: f32, weight: u32, c: Color, al: Align) {
        self.gfx.text(s, x, cy - size, w, size * 2.0, size, weight, c, al);
    }

    pub fn draw_page(&self, k: PageKind, ox: f32, yo: f32, a: f32) {
        match k {
            PageKind::Media => self.page_media(ox, yo, a),
            PageKind::Widgets => self.page_widgets(ox, yo, a),
            PageKind::Calendar => self.page_calendar(ox, yo, a),
            PageKind::Focus => self.page_focus(ox, yo, a),
            PageKind::Water => self.page_water(ox, yo, a),
            PageKind::Todo => self.page_todo(ox, yo, a),
            PageKind::Prayer => self.page_prayer(ox, yo, a),
            PageKind::Assistant => self.page_assistant(ox, yo, a),
            PageKind::Shelf => self.page_shelf(ox, yo, a),
        }
    }

    // ------------------------------------------------------------- media
    /// Two-tone page title in the top-left corner ("Todo" + coloured "List").
    fn title2(&self, a: &str, b: &str, bc: Color, x: f32, y: f32, al: f32) {
        let g = &self.gfx;
        self.t(a, x, y, 120.0, 14.5, 700, WHITE.a(al), Align::Left);
        let w = g.measure(a, 14.5, 700);
        self.t(b, x + w + 1.0, y, 120.0, 14.5, 700, bc.a(al), Align::Left);
    }

    fn page_media(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let m = self.media.snapshot();
        if !m.has_session || m.title.is_empty() {
            g.fill_round(ox + 190.0, yo + 36.0, 40.0, 40.0, 13.0, WHITE.a(0.08 * a));
            g.icon('\u{E8D6}', ox + 210.0, yo + 56.0, 17.0, WHITE.a(0.4 * a));
            self.t("Nothing playing", ox + 110.0, yo + 92.0, 200.0, 14.0, 600, WHITE.a(0.72 * a), Align::Center);
            self.t("Start a song and it shows up here", ox + 60.0, yo + 110.0, 300.0, 10.5, 500, FAINT.a(a), Align::Center);
            return;
        }
        let pos = if let Some(f) = self.seek_drag { m.duration * f as f64 } else { m.pos_now() };
        let split = self.lyrics_view && self.settings.lyrics;
        let frac = if m.duration > 0.0 { (pos / m.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let rem = if m.duration > 0.0 { format!("-{}", fmt_time(m.duration - pos)) } else { "--:--".into() };
        let who = if m.artist.is_empty() { app_name(&m.app) } else { m.artist.clone() };

        if split {
            // ---- compact player on the left, scrolling lyrics on the right
            self.art_tile(ox + 20.0, yo + 32.0, 48.0, 11.0, &m, a);
            self.t(&g.fit(&m.title, 13.5, 700, 124.0), ox + 80.0, yo + 41.0, 130.0, 13.5, 700, WHITE.a(a), Align::Left);
            self.t(&g.fit(&who, 11.5, 500, 124.0), ox + 80.0, yo + 58.0, 130.0, 11.5, 500, DIM.a(a), Align::Left);
            self.seekbar(ox + 20.0, 190.0, yo + 83.0, frac, a);
            self.t(&fmt_time(pos), ox + 20.0, yo + 94.0, 60.0, 10.0, 500, DIM.a(a), Align::Left);
            self.t(&rem, ox + 150.0, yo + 94.0, 60.0, 10.0, 500, DIM.a(a), Align::Right);
            let cy = yo + 111.0;
            self.lyrics_toggle(ox + 30.0, cy, true, a);
            self.transport(0, ox + 92.0, cy, a, Act::Prev);
            self.play_btn(m.playing, ox + 126.0, cy, 14.0, a);
            self.transport(1, ox + 160.0, cy, a, Act::Next);

            g.fill_rect(ox + 226.0, yo + 30.0, 1.0, 92.0, WHITE.a(0.10 * a));
            let (px, pw) = (ox + 242.0, 160.0);
            let (st, _synced, w5, _idx) = self.lyr.window(m.pos_now(), m.duration);
            use crate::lyrics::LyricsStatus as S;
            g.push_clip(ox + 232.0, yo + 26.0, 176.0, 98.0);
            match st {
                S::Ready => {
                    let cur_t = if w5[2].is_empty() { "\u{266A}".to_string() } else { w5[2].clone() };
                    let slide = self.lyric_slide.value;
                    let ch = g.measure_wrap(&cur_t, 16.5, 700, pw);
                    let cy0 = yo + 74.0 - ch / 2.0 + slide * 22.0;
                    let ph = g.measure_wrap(&w5[1], 12.5, 600, pw);
                    let pph = g.measure_wrap(&w5[0], 11.5, 600, pw);
                    g.text_wrap(&w5[0], px, cy0 - 8.0 - ph - 6.0 - pph, pw, pph + 4.0, 11.5, 600, WHITE.a(0.16 * a), Align::Left);
                    g.text_wrap(&w5[1], px, cy0 - 8.0 - ph, pw, ph + 4.0, 12.5, 600, WHITE.a(0.36 * a), Align::Left);
                    g.text_wrap(&cur_t, px, cy0, pw, ch + 4.0, 16.5, 700, WHITE.a(a), Align::Left);
                    let nh = g.measure_wrap(&w5[3], 12.5, 600, pw);
                    g.text_wrap(&w5[3], px, cy0 + ch + 8.0, pw, nh + 4.0, 12.5, 600, WHITE.a(0.36 * a), Align::Left);
                    g.text_wrap(&w5[4], px, cy0 + ch + 8.0 + nh + 6.0, pw, 40.0, 11.5, 600, WHITE.a(0.16 * a), Align::Left);
                }
                S::Loading => self.t("Finding lyrics…", px, yo + 74.0, pw, 12.5, 500, DIM.a(a), Align::Left),
                S::Offline => self.t("No internet access", px, yo + 74.0, pw, 12.5, 500, DIM.a(a), Align::Left),
                _ => self.t("No lyrics for this song", px, yo + 74.0, pw, 12.5, 500, DIM.a(a), Align::Left),
            }
            g.pop_clip();
            return;
        }

        // ---- full player: artwork with an ambient glow, big title, thin progress, round play button
        if let Some(t) = self.theme_tint() {
            g.glow(ox + 52.0, yo + 64.0, 56.0, t.a(0.20 * a));
        }
        self.art_tile(ox + 20.0, yo + 32.0, 64.0, 16.0, &m, a);
        let (x, cw) = (ox + 100.0, 300.0);
        self.t(&g.fit(&m.title, 16.0, 700, cw), x, yo + 42.0, cw + 4.0, 16.0, 700, WHITE.a(a), Align::Left);
        self.t(&g.fit(&who, 12.0, 500, cw), x, yo + 59.0, cw + 4.0, 12.0, 500, DIM.a(a), Align::Left);
        self.seekbar(x, cw, yo + 75.0, frac, a);
        self.t(&fmt_time(pos), x, yo + 87.0, 60.0, 10.0, 500, DIM.a(a), Align::Left);
        self.t(&rem, x + cw - 60.0, yo + 87.0, 60.0, 10.0, 500, DIM.a(a), Align::Right);
        let cy = yo + 108.0;
        if self.settings.lyrics {
            self.lyrics_toggle(x + 10.0, cy, false, a);
        }
        self.transport(0, x + cw / 2.0 - 54.0, cy, a, Act::Prev);
        self.play_btn(m.playing, x + cw / 2.0, cy, 15.0, a);
        self.transport(1, x + cw / 2.0 + 54.0, cy, a, Act::Next);
        self.icon_btn('\u{E767}', x + cw - 12.0, cy, 15.0, a, Act::Volume);
        // volume slider (drag), next to the speaker / mute button
        let vr = Rect { x: x + cw - 66.0, y: cy - 10.0, w: 42.0, h: 20.0 };
        let vol = self.volume_cached();
        let hot = self.hovered(vr) || self.vol_drag;
        g.fill_round(vr.x, cy - 2.0, vr.w, 4.0, 2.0, WHITE.a(0.22 * a));
        g.fill_round(vr.x, cy - 2.0, (vr.w * vol).max(4.0), 4.0, 2.0, WHITE.a(a));
        if hot {
            g.fill_circle(vr.x + vr.w * vol, cy, 4.5, WHITE.a(a));
        }
        self.hit(vr, Act::VolSlider);
    }

    /// Artwork (or a placeholder) with the source-switch badge.
    fn art_tile(&self, x: f32, y: f32, s: f32, r: f32, m: &crate::media::MediaState, a: f32) {
        let g = &self.gfx;
        if let Some(b) = &self.art {
            g.draw_cover(b, x, y, s, r, a);
        } else {
            g.fill_round(x, y, s, s, r, Color::rgba(255, 255, 255, 0.10 * a));
            g.icon('\u{E8D6}', x + s / 2.0, y + s / 2.0, s * 0.24, WHITE.a(0.45 * a));
        }
        self.source_badge(x, y, s, m.sources, a);
    }

    fn lyrics_toggle(&self, cx: f32, cy: f32, on: bool, a: f32) {
        let g = &self.gfx;
        let r = Rect { x: cx - 15.0, y: cy - 14.0, w: 30.0, h: 28.0 };
        let hot = self.hovered(r);
        if on {
            g.fill_round(cx - 13.0, cy - 11.0, 26.0, 22.0, 11.0, Color::rgb(255, 99, 130).a(0.22 * a));
        }
        let col = if on { Color::rgb(255, 99, 130) } else { WHITE.a(if hot { 1.0 } else { 0.75 }) };
        g.icon('\u{E8BD}', cx, cy, 14.5, col.a(a));
        self.hit(r, Act::Lyrics);
    }

    /// Round filled play / pause button.
    fn play_btn(&self, playing: bool, cx: f32, cy: f32, r: f32, a: f32) {
        let g = &self.gfx;
        let hot = self.hovered(Rect { x: cx - r, y: cy - r, w: r * 2.0, h: r * 2.0 });
        let rr = r * if hot { 1.07 } else { 1.0 };
        g.fill_circle(cx, cy, rr, WHITE.a((if hot { 1.0 } else { 0.94 }) * a));
        let k = rr / 16.0;
        if playing {
            g.fill_round(cx - 6.0 * k, cy - 7.0 * k, 4.2 * k, 14.0 * k, 1.5, BLACK.a(a));
            g.fill_round(cx + 1.8 * k, cy - 7.0 * k, 4.2 * k, 14.0 * k, 1.5, BLACK.a(a));
        } else {
            g.poly(&[(cx - 4.5 * k, cy - 7.0 * k), (cx - 4.5 * k, cy + 7.0 * k), (cx + 7.5 * k, cy)], BLACK.a(a), 1.8 * k);
        }
        self.hit(Rect { x: cx - r - 2.0, y: cy - r - 2.0, w: r * 2.0 + 4.0, h: r * 2.0 + 4.0 }, Act::PlayPause);
    }

    /// Clicking the artwork switches between media sources (several players / browser tabs at once);
    /// a small badge with the source count shows when there is more than one.
    fn source_badge(&self, x: f32, y: f32, s: f32, sources: usize, a: f32) {
        if sources < 2 {
            return;
        }
        let g = &self.gfx;
        let hot = self.hovered(Rect { x, y, w: s, h: s });
        let (bx, by) = (x + s - 4.0, y + s - 4.0);
        g.fill_circle(bx, by, 8.0, Color::rgba(0, 0, 0, 0.78 * a));
        g.stroke_circle(bx, by, 8.0, 1.0, WHITE.a((if hot { 0.7 } else { 0.35 }) * a));
        self.t(&sources.to_string(), bx - 8.0, by, 16.0, 9.5, 700, WHITE.a(a), Align::Center);
        self.hit(Rect { x, y, w: s, h: s }, Act::Source);
    }

    fn seekbar(&self, bx: f32, bw: f32, by: f32, frac: f32, a: f32) {
        let g = &self.gfx;
        let hot = self.hovered(Rect { x: bx - 4.0, y: by - 9.0, w: bw + 8.0, h: 18.0 }) || self.seek_drag.is_some();
        let th = if hot { 5.0 } else { 4.0 };
        g.fill_round(bx, by - th / 2.0, bw, th, th / 2.0, WHITE.a(0.22 * a));
        g.fill_round(bx, by - th / 2.0, (bw * frac).max(th), th, th / 2.0, WHITE.a(a));
        if hot {
            g.fill_circle(bx + bw * frac, by, 5.5, WHITE.a(a));
        }
        self.hit(Rect { x: bx - 4.0, y: by - 9.0, w: bw + 8.0, h: 18.0 }, Act::Seek);
    }

    /// Vector transport glyphs: 0 = rewind, 1 = forward, 2 = play, 3 = pause.
    fn transport(&self, kind: u8, cx: f32, cy: f32, a: f32, act: Act) {
        let g = &self.gfx;
        let hot = self.hovered(Rect { x: cx - 16.0, y: cy - 15.0, w: 32.0, h: 30.0 });
        let k = if hot { 1.1 } else { 1.0 } * 1.22;
        let c = WHITE.a(if hot { 1.0 } else { 0.94 } * a);
        let p = |x: f32, y: f32| (cx + x * k, cy + y * k);
        match kind {
            0 => {
                g.poly(&[p(-9.5, 0.0), p(-1.0, -6.0), p(-1.0, 6.0)], c, 1.6);
                g.poly(&[p(-0.5, 0.0), p(8.0, -6.0), p(8.0, 6.0)], c, 1.6);
            }
            1 => {
                g.poly(&[p(9.5, 0.0), p(1.0, -6.0), p(1.0, 6.0)], c, 1.6);
                g.poly(&[p(0.5, 0.0), p(-8.0, -6.0), p(-8.0, 6.0)], c, 1.6);
            }
            2 => g.poly(&[p(-6.0, -8.0), p(-6.0, 8.0), p(8.5, 0.0)], c, 2.2),
            _ => {
                g.fill_round(cx - 7.5 * k, cy - 8.5 * k, 5.0 * k, 17.0 * k, 1.8, c);
                g.fill_round(cx + 2.5 * k, cy - 8.5 * k, 5.0 * k, 17.0 * k, 1.8, c);
            }
        }
        self.hit(Rect { x: cx - 18.0, y: cy - 16.0, w: 36.0, h: 32.0 }, act);
    }

    // ----------------------------------------------------------- widgets
    fn page_widgets(&self, ox: f32, yo: f32, a: f32) {
        self.widget(self.settings.widget_left, ox + 22.0, yo + 36.0, 180.0, 84.0, a);
        self.widget(self.settings.widget_right, ox + 218.0, yo + 36.0, 180.0, 84.0, a);
    }

    fn widget(&self, kind: u32, x: f32, y: f32, w: f32, h: f32, a: f32) {
        let g = &self.gfx;
        g.fill_round(x, y, w, h, 14.0, CARD.a(a));
        let label = WIDGET_NAMES[(kind as usize).min(7)].to_uppercase();
        self.t(&label, x + 12.0, y + 14.0, w - 20.0, 8.5, 700, FAINT.a(a), Align::Left);
        let n = sys::now();
        match kind {
            0 => {
                // week strip
                let start = sys::days_from_civil(n.year, n.month, n.day) - n.dow as i64;
                for i in 0..7 {
                    let (yy, mm, dd) = sys::civil_from_days(start + i);
                    let cx = x + 18.0 + i as f32 * 24.0;
                    self.t(&sys::DOW[i as usize][..1], cx - 8.0, y + 34.0, 16.0, 9.0, 600, FAINT.a(a), Align::Center);
                    let today = yy == n.year && mm == n.month && dd == n.day;
                    if today {
                        g.fill_circle(cx, y + 52.0, 9.0, BLUE.a(a));
                    }
                    self.t(&dd.to_string(), cx - 10.0, y + 52.0, 20.0, 10.5, 600, WHITE.a(a), Align::Center);
                }
                let open = self.store.todos.iter().filter(|t| !t.done).count();
                self.t(
                    &if open == 0 { "All done today".to_string() } else { format!("{open} tasks left") },
                    x + 12.0,
                    y + 72.0,
                    w - 20.0,
                    10.0,
                    500,
                    DIM.a(a),
                    Align::Left,
                );
            }
            1 => {
                let (t, _, city) = prayer::today(&self.settings);
                let nm = prayer::minutes_now(&self.settings);
                let (i, at, tomorrow) = prayer::next(&t, nm);
                self.t(prayer::names_s(&self.settings)[i], x + 12.0, y + 38.0, 90.0, 15.0, 700, WHITE.a(a), Align::Left);
                self.t(&prayer::fmt(at), x + 12.0, y + 56.0, 80.0, 12.0, 500, DIM.a(a), Align::Left);
                let mins = (at + if tomorrow { 1440.0 } else { 0.0 } - nm).max(0.0) as u32;
                let rem = if mins >= 60 { format!("in {}h {}m", mins / 60, mins % 60) } else { format!("in {mins} min") };
                self.t(&rem, x + 12.0, y + 72.0, 100.0, 9.5, 500, FAINT.a(a), Align::Left);
                // sun arc
                let (cx, cy, r) = (x + w - 48.0, y + 68.0, 30.0);
                g.arc(cx, cy, r, 0.5, 1.5, Color::rgba(255, 255, 255, 0.0));
                let prog = ((nm - t[0]) / (t[3] - t[0])).clamp(0.0, 1.0) as f32;
                let ang = std::f32::consts::PI * (1.0 - prog);
                g.line(cx - r, cy, cx + r, cy, 1.0, Color::rgba(255, 255, 255, 0.15 * a));
                g.fill_circle(cx + r * ang.cos(), cy - r * ang.sin() * 0.9, 3.2, AMBER.a(a));
                let _ = city;
            }
            2 => {
                let taken = self.store.water_taken;
                self.t(&format!("{taken} ml"), x + 12.0, y + 44.0, 100.0, 17.0, 700, WHITE.a(a), Align::Left);
                self.t(&format!("of {:.1} L today", self.settings.water_goal as f32 / 1000.0), x + 12.0, y + 64.0, 120.0, 9.5, 500, DIM.a(a), Align::Left);
                let f = (taken as f32 / self.settings.water_goal.max(1) as f32).clamp(0.0, 1.0);
                g.stroke_round(x + w - 38.0, y + 28.0, 20.0, 44.0, 6.0, 1.5, WHITE.a(0.25 * a));
                g.fill_round(x + w - 36.5, y + 28.0 + 1.5 + (41.0 * (1.0 - f)), 17.0, 41.0 * f, 5.0, BLUE.a(0.9 * a));
            }
            3 => {
                self.t(&format!("{} min", self.store.focus_minutes), x + 12.0, y + 44.0, 120.0, 17.0, 700, WHITE.a(a), Align::Left);
                self.t(&format!("{} sessions", self.store.focus_sessions), x + 12.0, y + 64.0, 120.0, 9.5, 500, DIM.a(a), Align::Left);
                for i in 0..8 {
                    let on = (i as u32) < self.store.focus_sessions;
                    g.fill_circle(x + 14.0 + i as f32 * 12.0, y + 74.0, 3.0, if on { AMBER.a(a) } else { WHITE.a(0.15 * a) });
                }
            }
            4 => {
                let utc_min = (n.minutes() as i32 - sys::utc_offset_minutes()).rem_euclid(1440);
                for (row, idx) in [self.settings.clock_a, self.settings.clock_b].iter().enumerate() {
                    let (name, off) = WORLD[(*idx as usize).min(WORLD.len() - 1)];
                    let m = (utc_min + (off * 60.0) as i32).rem_euclid(1440);
                    let cy = y + 38.0 + row as f32 * 22.0;
                    self.t(name, x + 12.0, cy, 90.0, 11.5, 600, WHITE.a(a), Align::Left);
                    self.t(&format!("{:02}:{:02}", m / 60, m % 60), x + w - 82.0, cy, 70.0, 15.0, 700, WHITE.a(a), Align::Right);
                }
            }
            5 => {
                let b = self.battery;
                let col = if b.percent <= 20 && !b.charging { RED } else { GREEN };
                self.t(&format!("{}%", b.percent), x + 12.0, y + 48.0, 80.0, 20.0, 700, WHITE.a(a), Align::Left);
                self.t(if b.charging { "Charging" } else if b.ac { "Plugged in" } else { "On battery" }, x + 12.0, y + 68.0, 100.0, 9.5, 500, DIM.a(a), Align::Left);
                g.stroke_circle(x + w - 34.0, y + 50.0, 17.0, 3.5, WHITE.a(0.12 * a));
                g.arc(x + w - 34.0, y + 50.0, 17.0, b.percent as f32 / 100.0, 3.5, col.a(a));
            }
            6 => {
                let (ph, name) = moon_phase();
                let (cx, cy, r) = (x + w - 34.0, y + 50.0, 17.0);
                g.fill_circle(cx, cy, r, Color::rgba(235, 235, 225, 0.95 * a));
                // shadow disc: offset by phase to carve the terminator
                let k = (ph * 2.0 - 1.0) * 2.0; // -2..2
                let off = if ph < 0.5 { -r * (1.0 - ph * 2.0) * 2.0 } else { r * (ph * 2.0 - 1.0) * 2.0 };
                let _ = k;
                g.push_round_clip(cx - r, cy - r, r * 2.0, r * 2.0, r);
                g.fill_circle(cx + off, cy, r, Color::rgba(18, 18, 22, 0.96 * a));
                g.pop_layer();
                self.t(name, x + 12.0, y + 46.0, 110.0, 12.0, 700, WHITE.a(a), Align::Left);
                let illum = (1.0 - (ph * std::f32::consts::TAU).cos()) / 2.0;
                self.t(&format!("{:.0}% illuminated", illum * 100.0), x + 12.0, y + 66.0, 120.0, 9.5, 500, DIM.a(a), Align::Left);
            }
            _ => {
                let (d, name) = countdown_days(self.settings.countdown);
                self.t(&d.to_string(), x + 12.0, y + 48.0, 80.0, 24.0, 700, WHITE.a(a), Align::Left);
                self.t(&format!("days to {name}"), x + 12.0, y + 70.0, 160.0, 9.5, 500, DIM.a(a), Align::Left);
                let f = 1.0 - (d as f32 / 365.0).clamp(0.0, 1.0);
                g.fill_round(x + 12.0, y + h - 10.0, w - 24.0, 3.0, 1.5, WHITE.a(0.15 * a));
                g.fill_round(x + 12.0, y + h - 10.0, (w - 24.0) * f, 3.0, 1.5, AMBER.a(a));
            }
        }
    }

    // ---------------------------------------------------------- calendar
    fn page_calendar(&self, ox: f32, yo: f32, a: f32) {
        if self.settings.cal_week_view {
            return self.page_week(ox, yo, a);
        }
        let g = &self.gfx;
        let n = sys::now();
        let (cy, cm) = self.cal_month;
        let x0 = ox + 30.0;
        let top = yo + 36.0;
        self.t(&format!("{} {}", sys::MON_FULL[(cm - 1) as usize], cy), x0, top, 130.0, 12.0, 700, WHITE.a(a), Align::Left);
        self.icon_btn('\u{E76B}', ox + 150.0, top, 9.0, a, Act::CalPrev);
        self.icon_btn('\u{E76C}', ox + 176.0, top, 9.0, a, Act::CalNext);
        let start_mon = self.settings.week_monday;
        let first = sys::weekday(cy, cm, 1);
        let lead = if start_mon { (first + 6) % 7 } else { first } as i32;
        let dim = sys::days_in_month(cy, cm) as i32;
        let cell = 24.5;
        for i in 0..7 {
            let d = if start_mon { (i + 1) % 7 } else { i } as usize;
            self.t(&sys::DOW[d][..1], x0 + i as f32 * cell - 6.0, top + 16.0, 20.0, 8.5, 700, FAINT.a(a), Align::Center);
        }
        for day in 1..=dim {
            let idx = lead + day - 1;
            let (r, c) = (idx / 7, idx % 7);
            let cx = x0 + c as f32 * cell + 4.0;
            let cyy = top + 28.0 + r as f32 * 11.0;
            let today = cy == n.year && cm == n.month && day as u32 == n.day;
            if today {
                g.fill_round(cx - 8.0, cyy - 5.0, 18.0, 10.0, 5.0, GREEN.a(a));
            }
            self.t(&day.to_string(), cx - 8.0, cyy, 18.0, 9.0, if today { 700 } else { 500 }, WHITE.a(if today { 1.0 } else { 0.8 } * a), Align::Center);
        }
        self.hit(Rect { x: x0 - 6.0, y: top - 8.0, w: 120.0, h: 16.0 }, Act::CalToday);
        // today panel
        let px = ox + 236.0;
        g.fill_round(px - 14.0, yo + 34.0, 1.0, 84.0, 0.5, WHITE.a(0.1 * a));
        self.t(&format!("{} {}", sys::DOW[n.dow as usize], n.day), px, yo + 46.0, 120.0, 17.0, 700, WHITE.a(a), Align::Left);
        let todays: Vec<_> = self.store.todos.iter().filter(|t| !t.done).take(3).collect();
        if todays.is_empty() {
            self.t("Nothing scheduled", px, yo + 72.0, 150.0, 11.0, 500, DIM.a(a), Align::Left);
        } else {
            for (i, t) in todays.iter().enumerate() {
                let y = yo + 70.0 + i as f32 * 17.0;
                g.fill_circle(px + 3.0, y, 2.5, AMBER.a(a));
                let s = g.fit(&t.title, 11.0, 500, 118.0);
                self.t(&s, px + 12.0, y, 120.0, 11.0, 500, WHITE.a(0.9 * a), Align::Left);
                if let Some(m) = t.time {
                    self.t(&format!("{:02}:{:02}", m / 60, m % 60), px + 128.0, y, 40.0, 9.5, 500, FAINT.a(a), Align::Right);
                }
            }
        }
    }

    /// Week strip view of the calendar (setting: Default view = Week).
    fn page_week(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let n = sys::now();
        let today = sys::days_from_civil(n.year, n.month, n.day);
        let lead = if self.settings.week_monday { (n.dow + 6) % 7 } else { n.dow } as i64;
        let start = today - lead;
        let (y0, m0, _) = sys::civil_from_days(start);
        self.t(&format!("{} {}", sys::MON_FULL[(m0 - 1) as usize], y0), ox + 34.0, yo + 40.0, 200.0, 12.0, 700, WHITE.a(a), Align::Left);
        let open = self.store.todos.iter().filter(|t| !t.done).count();
        self.t(&if open == 0 { "Nothing scheduled".to_string() } else { format!("{open} tasks open") }, ox + 200.0, yo + 40.0, 186.0, 10.5, 500, DIM.a(a), Align::Right);
        for i in 0..7i64 {
            let (yy, mm, dd) = sys::civil_from_days(start + i);
            let dow = sys::weekday(yy, mm, dd);
            let cx = ox + 58.0 + i as f32 * 51.0;
            let is_today = start + i == today;
            let col = if is_today { 1.0 } else { 0.0 };
            g.fill_round(cx - 20.0, yo + 56.0, 40.0, 58.0, 16.0, Color::rgba(255, 255, 255, 0.07 + 0.0 * col).a(a));
            self.t(&sys::DOW[dow as usize].to_uppercase(), cx - 20.0, yo + 69.0, 40.0, 8.5, 700, FAINT.a(a), Align::Center);
            if is_today {
                g.fill_circle(cx, yo + 91.0, 13.0, GREEN.a(a));
            }
            self.t(&dd.to_string(), cx - 20.0, yo + 91.0, 40.0, 13.0, if is_today { 700 } else { 600 }, WHITE.a(a), Align::Center);
        }
    }

    // ------------------------------------------------------------- focus
    fn page_focus(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let s = &self.store;
        self.t("Pomodoro", ox + 20.0, yo + 18.0, 120.0, 14.5, 700, WHITE.a(a), Align::Left);

        let (label, col) = match s.phase {
            Phase::Idle | Phase::Focus => (format!("Focus · {} of 4", s.cycle % 4 + 1), INDIGO),
            Phase::Short => ("Short break".to_string(), GREEN),
            Phase::Long => ("Long break".to_string(), BLUE),
        };
        let total = if s.phase == Phase::Idle { self.settings.pomo_focus as f64 * 60.0 } else { s.phase_total };
        let remaining = if s.phase == Phase::Idle { total } else { s.remaining };
        let frac = if total > 0.0 { (1.0 - remaining / total).clamp(0.0, 1.0) as f32 } else { 0.0 };

        // left: phase, big time, status, progress
        self.t(&label, ox + 20.0, yo + 44.0, 180.0, 11.5, 600, DIM.a(a), Align::Left);
        self.t(&fmt_time(remaining), ox + 20.0, yo + 74.0, 190.0, 34.0, 700, WHITE.a(a), Align::Left);
        let mins = format!("{} min", (total / 60.0).round() as u32);
        self.t(&mins, ox + 20.0, yo + 99.0, 60.0, 12.0, 600, WHITE.a(0.78 * a), Align::Left);
        let (st, stc) = if s.phase == Phase::Idle { ("Ready", INDIGO) } else if s.running { ("Running", GREEN) } else { ("Paused", AMBER) };
        self.t(st, ox + 28.0 + g.measure(&mins, 12.0, 600), yo + 99.0, 80.0, 12.0, 700, stc.a(a), Align::Left);
        g.fill_round(ox + 20.0, yo + 113.0, 184.0, 4.0, 2.0, WHITE.a(0.14 * a));
        if frac > 0.0 {
            g.fill_round(ox + 20.0, yo + 113.0, (184.0 * frac).max(4.0), 4.0, 2.0, col.a(a));
        }
        g.fill_rect(ox + 216.0, yo + 36.0, 1.0, 82.0, WHITE.a(0.10 * a));

        // right: this round
        let x = ox + 232.0;
        self.t("This round", x, yo + 44.0, 70.0, 11.5, 600, DIM.a(a), Align::Left);
        let pill = Rect { x: ox + 296.0, y: yo + 31.0, w: 64.0, h: 26.0 };
        let hot = self.hovered(pill);
        let k = if hot { 1.0 } else { 0.9 };
        g.fill_round_hgrad(pill.x, pill.y, pill.w, pill.h, 13.0, Color::rgb(86, 120, 255).a(k * a), Color::rgb(168, 92, 255).a(k * a));
        self.t(if s.running { "Pause" } else { "Start" }, pill.x, pill.y + 13.0, pill.w, 12.0, 700, WHITE.a(a), Align::Center);
        self.hit(pill, Act::FocusToggle);
        for (cx, glyph, act) in [(ox + 372.0, '\u{E72C}', Act::FocusReset), (ox + 392.0, '\u{E893}', Act::FocusSkip)] {
            let r = Rect { x: cx - 10.0, y: yo + 34.0, w: 20.0, h: 20.0 };
            g.fill_circle(cx, yo + 44.0, 10.0, WHITE.a((if self.hovered(r) { 0.20 } else { 0.10 }) * a));
            g.icon(glyph, cx, yo + 44.0, 10.0, WHITE.a(0.9 * a));
            self.hit(r, act);
        }
        // round timeline: 1 2 3 4 + long break
        let done = (s.cycle % 4) as usize;
        let widths = [28.0f32, 28.0, 28.0, 28.0, 36.0];
        let mut sx = x;
        for i in 0..5usize {
            let w = widths[i];
            g.fill_round(sx, yo + 76.0, w, 6.0, 3.0, WHITE.a(0.14 * a));
            let (fill, c) = if i < 4 {
                let f = if s.phase == Phase::Long || i < done { 1.0 } else if i == done && s.phase == Phase::Focus { frac } else { 0.0 };
                (f, INDIGO)
            } else {
                (if s.phase == Phase::Long { frac } else { 0.0 }, BLUE)
            };
            if fill > 0.0 {
                g.fill_round(sx, yo + 76.0, (w * fill).max(6.0), 6.0, 3.0, c.a(a));
            }
            let lab = if i < 4 { (i + 1).to_string() } else { "Long".to_string() };
            let on = (i == done && matches!(s.phase, Phase::Focus | Phase::Idle)) || (i == 4 && s.phase == Phase::Long);
            self.t(&lab, sx, yo + 93.0, w, 9.5, if on { 700 } else { 500 }, WHITE.a((if on { 0.95 } else { 0.45 }) * a), Align::Center);
            sx += w + 5.0;
        }
        let round = 4 * self.settings.pomo_focus + 3 * self.settings.pomo_short + self.settings.pomo_long;
        let rt = if round >= 60 { format!("{} h {} min", round / 60, round % 60) } else { format!("{round} min") };
        self.t(&format!("{rt} · {} sessions", s.focus_sessions), x, yo + 111.0, 170.0, 10.5, 500, DIM.a(a), Align::Left);
    }

    pub fn pill_btn(&self, label: &str, x: f32, y: f32, w: f32, a: f32, act: Act, primary: bool) {
        let g = &self.gfx;
        let hot = self.hovered(Rect { x, y, w, h: 22.0 });
        let bg = if primary { Color::rgba(255, 255, 255, if hot { 1.0 } else { 0.92 }) } else { Color::rgba(255, 255, 255, if hot { 0.2 } else { 0.12 }) };
        g.fill_round(x, y, w, 22.0, 11.0, bg.a(a));
        self.t(label, x, y + 11.0, w, 11.0, 600, if primary { BLACK.a(a) } else { WHITE.a(a) }, Align::Center);
        self.hit(Rect { x, y, w, h: 22.0 }, act);
    }

    // ------------------------------------------------------------- water
    /// Reminder times between "from" and "to": one bottle per reminder, spread evenly.
    pub fn water_plan(&self) -> (u32, u32) {
        let st = &self.settings;
        let count = ((st.water_goal as f32 / st.bottle.max(100) as f32).ceil() as u32).max(1);
        let span = st.water_to.saturating_sub(st.water_from).max(1);
        (count, (span / count).max(15))
    }

    /// Next reminder time (minute of day) today, if reminders are on and any are left.
    pub fn water_next(&self) -> Option<u32> {
        if !self.settings.water {
            return None;
        }
        let (count, step) = self.water_plan();
        let now = sys::now().minutes();
        (0..count).map(|k| self.settings.water_from + k * step).find(|t| *t > now && *t <= self.settings.water_to)
    }

    fn drop_btn(&self, label: &str, x: f32, y: f32, w: f32, a: f32, act: Act, primary: bool) {
        let g = &self.gfx;
        let r = Rect { x, y, w, h: 26.0 };
        let hot = self.hovered(r);
        let bg = if primary { BLUE.a((if hot { 0.95 } else { 0.78 }) * a) } else { Color::rgba(255, 255, 255, if hot { 0.18 } else { 0.10 }).a(a) };
        g.fill_round(x, y, w, 26.0, 13.0, bg);
        let tw = g.measure(label, 12.0, 600);
        let ix = x + (w - tw - 18.0) / 2.0 + 6.0;
        if primary {
            self.drop_icon(ix, y + 13.0, WHITE.a(a));
        } else {
            g.icon('\u{E713}', ix, y + 13.0, 11.0, WHITE.a(0.85 * a));
        }
        self.t(label, ix + 11.0, y + 13.0, tw + 4.0, 12.0, 600, WHITE.a(a), Align::Left);
        self.hit(r, act);
    }

    fn round_btn(&self, glyph: char, cx: f32, cy: f32, a: f32, act: Act) {
        let g = &self.gfx;
        let r = Rect { x: cx - 11.0, y: cy - 11.0, w: 22.0, h: 22.0 };
        g.fill_circle(cx, cy, 10.5, WHITE.a((if self.hovered(r) { 0.22 } else { 0.11 }) * a));
        g.icon(glyph, cx, cy, 8.5, WHITE.a(0.92 * a));
        self.hit(r, act);
    }

    fn page_water(&self, ox: f32, yo: f32, a: f32) {
        if self.water_cfg {
            return self.page_water_cfg(ox, yo, a);
        }
        let g = &self.gfx;
        let s = &self.store;
        let st = &self.settings;
        self.title2("Water", "Reminder", BLUE, ox + 20.0, yo + 18.0, a);
        let goal = st.water_goal.max(1) as f32;
        let f = (s.water_taken as f32 / goal).clamp(0.0, 1.0);

        // amount, reminder switch, status line, progress
        let amt = format!("{:.1}", s.water_taken as f32 / 1000.0);
        self.t(&amt, ox + 20.0, yo + 58.0, 90.0, 30.0, 700, WHITE.a(a), Align::Left);
        let aw = g.measure(&amt, 30.0, 700);
        self.t(&format!("/ {:.1} L", goal / 1000.0), ox + 25.0 + aw, yo + 64.0, 70.0, 12.5, 600, DIM.a(a), Align::Left);
        let sw = Rect { x: ox + 164.0, y: yo + 48.0, w: 38.0, h: 20.0 };
        g.fill_round(sw.x, sw.y, sw.w, sw.h, 10.0, if st.water { BLUE.a(a) } else { WHITE.a(0.20 * a) });
        let kx = if st.water { sw.x + 28.0 } else { sw.x + 10.0 };
        g.fill_circle(kx, sw.y + 10.0, 8.0, WHITE.a(a));
        if st.water {
            self.drop_icon(kx, sw.y + 10.0, BLUE.a(a));
        }
        self.hit(sw, Act::WaterToggle);
        let next = match self.water_next() {
            Some(m) => format!("Next {:02}:{:02}", m / 60, m % 60),
            None if st.water => "Done for today".to_string(),
            None => "Reminders off".to_string(),
        };
        self.t(&format!("Today · {}% · {}", (f * 100.0).round() as u32, next), ox + 20.0, yo + 80.0, 190.0, 10.5, 500, DIM.a(a), Align::Left);
        g.fill_round(ox + 20.0, yo + 91.0, 182.0, 4.0, 2.0, WHITE.a(0.14 * a));
        if f > 0.0 {
            g.fill_round(ox + 20.0, yo + 91.0, (182.0 * f).max(4.0), 4.0, 2.0, BLUE.a(a));
        }
        self.drop_btn("Drink", ox + 20.0, yo + 101.0, 82.0, a, Act::Drink, true);
        self.drop_btn("Settings", ox + 108.0, yo + 101.0, 94.0, a, Act::WaterCfg, false);

        // hourly chart with a 0 / 200 / 400 ml scale
        let (cx0, cw, base, ch) = (ox + 226.0, 140.0, yo + 112.0, 70.0);
        for (i, lbl) in ["400", "200", "0 ml"].iter().enumerate() {
            let y = base - ch + i as f32 * ch / 2.0;
            g.fill_rect(cx0, y, cw, 1.0, WHITE.a(0.07 * a));
            self.t(lbl, ox + 372.0, y, 30.0, 9.0, 500, FAINT.a(a), Align::Right);
        }
        let from = ((st.water_from / 60) as usize).min(23);
        let to = ((st.water_to / 60) as usize).clamp(from, 23);
        let hours = to - from + 1;
        let slot = cw / hours as f32;
        for i in 0..hours {
            let h24 = from + i;
            let v = s.water_hours[h24] as f32;
            let bh = (v / 400.0 * ch).clamp(0.0, ch);
            let bx = cx0 + i as f32 * slot + slot * 0.2;
            g.fill_round(bx, base - bh.max(2.0), (slot * 0.6).max(2.0), bh.max(2.0), 1.5, if v > 0.0 { BLUE.a(a) } else { WHITE.a(0.10 * a) });
            if h24 % 3 == 0 {
                self.t(&format!("{h24:02}"), bx - 6.0, base + 8.0, (slot * 0.6).max(2.0) + 12.0, 9.0, 500, FAINT.a(a), Align::Center);
            }
        }
    }

    /// The Settings sub-page of the water tracker: bottle, goal, active hours, plan summary.
    fn page_water_cfg(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let st = &self.settings;
        // back button + title
        let back = Rect { x: ox + 14.0, y: yo + 6.0, w: 90.0, h: 24.0 };
        g.fill_circle(ox + 28.0, yo + 18.0, 10.0, WHITE.a((if self.hovered(back) { 0.22 } else { 0.12 }) * a));
        g.icon('\u{E76B}', ox + 28.0, yo + 18.0, 9.0, WHITE.a(a));
        self.t("Settings", ox + 44.0, yo + 18.0, 80.0, 14.5, 700, WHITE.a(a), Align::Left);
        self.hit(back, Act::WaterCfg);

        let lab = |y: f32, t: &str| self.t(t, ox + 20.0, y, 50.0, 11.5, 600, WHITE.a(0.85 * a), Align::Left);
        let hm = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        // bottle
        lab(yo + 50.0, "Bottle");
        self.round_btn('\u{E108}', ox + 84.0, yo + 50.0, a, Act::WaterBottle(-1));
        self.t(&format!("{} ml", st.bottle), ox + 98.0, yo + 50.0, 58.0, 12.5, 700, WHITE.a(a), Align::Center);
        self.round_btn('\u{E109}', ox + 170.0, yo + 50.0, a, Act::WaterBottle(1));
        let (count, step) = self.water_plan();
        let every = if step >= 60 && step % 60 == 0 { format!("every {} h", step / 60) } else if step >= 60 { format!("every {} h {} min", step / 60, step % 60) } else { format!("every {step} min") };
        self.t(&every, ox + 188.0, yo + 50.0, 100.0, 10.0, 500, FAINT.a(a), Align::Left);
        // goal
        lab(yo + 80.0, "Goal");
        for (i, ml) in [1500u32, 2000, 2500, 3000].iter().enumerate() {
            let r = Rect { x: ox + 72.0 + i as f32 * 52.0, y: yo + 67.0, w: 48.0, h: 26.0 };
            let on = st.water_goal == *ml;
            if on {
                g.fill_round_hgrad(r.x, r.y, r.w, r.h, 13.0, Color::rgb(0, 150, 255).a(a), Color::rgb(70, 130, 255).a(a));
            } else {
                g.fill_round(r.x, r.y, r.w, r.h, 13.0, WHITE.a((if self.hovered(r) { 0.14 } else { 0.07 }) * a));
            }
            self.t(&format!("{:.1} L", *ml as f32 / 1000.0), r.x, r.y + 13.0, r.w, 11.5, 600, WHITE.a((if on { 1.0 } else { 0.55 }) * a), Align::Center);
            self.hit(r, Act::WaterGoal(*ml));
        }
        // hours
        lab(yo + 110.0, "Hours");
        self.round_btn('\u{E108}', ox + 84.0, yo + 110.0, a, Act::WaterFrom(-1));
        self.t(&hm(st.water_from), ox + 98.0, yo + 110.0, 44.0, 12.5, 700, WHITE.a(a), Align::Center);
        self.round_btn('\u{E109}', ox + 156.0, yo + 110.0, a, Act::WaterFrom(1));
        self.t("to", ox + 170.0, yo + 110.0, 20.0, 11.0, 500, FAINT.a(a), Align::Center);
        self.round_btn('\u{E108}', ox + 204.0, yo + 110.0, a, Act::WaterTo(-1));
        self.t(&hm(st.water_to), ox + 218.0, yo + 110.0, 44.0, 12.5, 700, WHITE.a(a), Align::Center);
        self.round_btn('\u{E109}', ox + 276.0, yo + 110.0, a, Act::WaterTo(1));

        // plan summary card
        let c = Rect { x: ox + 306.0, y: yo + 36.0, w: 96.0, h: 86.0 };
        g.fill_round(c.x, c.y, c.w, c.h, 16.0, WHITE.a(0.07 * a));
        self.t(&count.to_string(), c.x, c.y + 16.0, c.w, 18.0, 700, BLUE.a(a), Align::Center);
        self.t("reminders", c.x, c.y + 30.0, c.w, 9.5, 500, DIM.a(a), Align::Center);
        self.t(&count.saturating_sub(1).to_string(), c.x, c.y + 45.0, c.w, 15.0, 700, BLUE.a(a), Align::Center);
        self.t("refills", c.x, c.y + 58.0, c.w, 9.5, 500, DIM.a(a), Align::Center);
        let reset = Rect { x: c.x + 12.0, y: c.y + 67.0, w: c.w - 24.0, h: 15.0 };
        g.fill_round(reset.x, reset.y, reset.w, reset.h, 7.5, WHITE.a((if self.hovered(reset) { 0.20 } else { 0.10 }) * a));
        self.t("Reset today", reset.x, reset.y + 7.5, reset.w, 9.0, 600, WHITE.a(0.85 * a), Align::Center);
        self.hit(reset, Act::WaterReset);
    }

    // -------------------------------------------------------------- todo
    fn page_todo(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        self.title2("Todo", "List", GREEN, ox + 20.0, yo + 18.0, a);
        let todos = &self.store.todos;
        let total = todos.len();
        let done = todos.iter().filter(|t| t.done).count();
        self.t(&format!("{done}/{total}"), ox + 22.0, yo + 53.0, 130.0, 36.0, 700, WHITE.a(a), Align::Left);
        // calendar shortcut with an "open tasks" dot
        let (bx, by) = (ox + 140.0, yo + 52.0);
        let cal_idx = self.pages().iter().position(|k| *k == PageKind::Calendar);
        let hot = self.hovered(Rect { x: bx - 14.0, y: by - 14.0, w: 28.0, h: 28.0 });
        g.fill_circle(bx, by, 14.0, WHITE.a(if hot { 0.16 } else { 0.10 } * a));
        g.icon('\u{E787}', bx, by, 13.0, WHITE.a(0.9 * a));
        if todos.iter().any(|t| !t.done) {
            g.fill_circle(bx + 9.0, by - 9.0, 3.2, BLUE.a(a));
        }
        if let Some(i) = cal_idx {
            self.hit(Rect { x: bx - 14.0, y: by - 14.0, w: 28.0, h: 28.0 }, Act::Dot(i));
        }
        self.t("Task Done Today", ox + 22.0, yo + 78.0, 160.0, 12.0, 500, DIM.a(a), Align::Left);

        // buttons
        let add = Rect { x: ox + 22.0, y: yo + 92.0, w: 98.0, h: 26.0 };
        let hot = self.hovered(add);
        g.fill_round(add.x, add.y, add.w, add.h, 13.5, Color::rgba(255, 255, 255, if hot { 0.18 } else { 0.12 } * a));
        g.fill_circle(add.x + 14.0, add.y + 13.5, 8.0, WHITE.a(a));
        g.icon('\u{E710}', add.x + 14.0, add.y + 13.5, 10.0, BLACK.a(a));
        self.t("Add Task", add.x + 26.0, add.y + 13.5, 72.0, 12.5, 600, WHITE.a(a), Align::Center);
        self.hit(add, Act::TodoAdd);
        let edit = Rect { x: ox + 124.0, y: yo + 92.0, w: 80.0, h: 26.0 };
        let can = self.todo_sel.map(|i| i < total).unwrap_or(false);
        let hot = can && self.hovered(edit);
        g.fill_round(edit.x, edit.y, edit.w, edit.h, 13.5, Color::rgba(255, 255, 255, if hot { 0.14 } else { 0.06 } * a));
        g.icon('\u{E70F}', edit.x + 14.0, edit.y + 13.5, 12.0, WHITE.a(if can { 0.9 } else { 0.35 } * a));
        self.t("Edit Task", edit.x + 22.0, edit.y + 13.5, 56.0, 12.0, 600, WHITE.a(if can { 0.95 } else { 0.4 } * a), Align::Center);
        if can {
            self.hit(edit, Act::TodoEdit);
        }

        // task panel
        let (px, py, pw, ph) = (ox + 212.0, yo + 32.0, 180.0, 90.0);
        g.fill_round(px, py, pw, ph, 18.0, WHITE.a(0.085 * a));
        if total == 0 {
            g.fill_round(px + pw / 2.0 - 20.0, py + 16.0, 40.0, 40.0, 12.0, WHITE.a(0.12 * a));
            g.icon('\u{E73A}', px + pw / 2.0, py + 36.0, 20.0, WHITE.a(0.35 * a));
            self.t("No tasks.", px, py + 74.0, pw, 13.0, 600, WHITE.a(0.5 * a), Align::Center);
        } else {
            let mut order: Vec<usize> = (0..total).collect();
            order.sort_by_key(|i| todos[*i].done);
            for (row, i) in order.iter().take(3).enumerate() {
                let t = &todos[*i];
                let y = py + 20.0 + row as f32 * 23.0;
                if self.todo_sel == Some(*i) {
                    g.fill_round(px + 6.0, y - 10.0, pw - 12.0, 20.0, 10.0, WHITE.a(0.10 * a));
                }
                if t.done {
                    g.fill_circle(px + 20.0, y, 7.0, GREEN.a(a));
                    g.icon('\u{E73E}', px + 20.0, y, 9.0, BLACK.a(a));
                } else {
                    g.stroke_circle(px + 20.0, y, 6.5, 1.5, WHITE.a(0.55 * a));
                }
                let title = g.fit(&t.title, 12.5, 500, 100.0);
                self.t(&title, px + 34.0, y, 108.0, 12.5, 500, WHITE.a(if t.done { 0.4 } else { 0.95 } * a), Align::Left);
                if let Some(m) = t.time {
                    self.t(&format!("{:02}:{:02}", m / 60, m % 60), px + pw - 52.0, y, 40.0, 10.5, 500, DIM.a(a), Align::Right);
                }
                self.hit(Rect { x: px + 30.0, y: y - 10.0, w: pw - 36.0, h: 20.0 }, Act::TodoSelect(*i));
                self.hit(Rect { x: px + 8.0, y: y - 10.0, w: 24.0, h: 20.0 }, Act::TodoToggle(*i));
            }
            if total > 3 {
                self.t(&format!("+{} more", total - 3), px, py + ph - 9.0, pw, 9.5, 500, DIM.a(a), Align::Center);
            }
        }
    }

    // ------------------------------------------------------------ prayer
    fn page_prayer(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let s = &self.settings;
        let (t, _rise, city) = prayer::today(s);
        let nm = prayer::minutes_now(s);
        let (i, at, tomorrow) = prayer::next(&t, nm);

        if self.pray_pick {
            return self.page_pray_pick(ox, yo, a);
        }
        self.title2("Prayer", "", WHITE, ox + 20.0, yo + 18.0, a);
        let tw = g.measure("Prayer", 14.5, 700);
        // tappable city chip: opens the picker
        let cw = g.measure(city.name, 11.5, 600) + 32.0;
        let chip = Rect { x: ox + 20.0 + tw + 8.0, y: yo + 8.0, w: cw, h: 21.0 };
        g.fill_round(chip.x, chip.y, chip.w, chip.h, 10.5, WHITE.a((if self.hovered(chip) { 0.18 } else { 0.09 }) * a));
        self.t(city.name, chip.x + 10.0, yo + 18.5, cw, 11.5, 600, WHITE.a(0.85 * a), Align::Left);
        g.icon('\u{E70D}', chip.x + cw - 11.0, yo + 18.5, 7.5, WHITE.a(0.6 * a));
        self.hit(chip, Act::PrayOpen);

        self.t("Next", ox + 22.0, yo + 50.0, 100.0, 12.0, 500, DIM.a(a), Align::Left);
        self.t(prayer::names_s(&self.settings)[i], ox + 22.0, yo + 76.0, 170.0, 30.0, 700, WHITE.a(a), Align::Left);
        let mins = (at + if tomorrow { 1440.0 } else { 0.0 } - nm).max(0.0) as u32;
        let rem = if mins >= 60 { format!("in {} h {} min", mins / 60, mins % 60) } else { format!("in {mins} min") };
        let ts = prayer::fmt(at);
        self.t(&ts, ox + 22.0, yo + 100.0, 70.0, 15.0, 700, WHITE.a(a), Align::Left);
        let tsw = g.measure(&ts, 15.0, 700);
        self.t(&rem, ox + 22.0 + tsw + 8.0, yo + 100.0, 120.0, 12.5, 600, Color::rgb(90, 200, 250).a(a), Align::Left);
        // progress since the previous prayer
        let prev = if i == 0 { t[4] - 1440.0 } else { t[i - 1] };
        let span = (at + if tomorrow { 1440.0 } else { 0.0 } - prev).max(1.0);
        let f = ((nm - prev) / span).clamp(0.0, 1.0) as f32;
        g.fill_round(ox + 22.0, yo + 116.0, 158.0, 3.0, 1.5, WHITE.a(0.14 * a));
        g.fill_round(ox + 22.0, yo + 116.0, (158.0 * f).max(3.0), 3.0, 1.5, WHITE.a(0.9 * a));

        // sun arc, Subuh -> Isya
        let (x0, x1, base, hh) = (ox + 208.0, ox + 396.0, yo + 90.0, 42.0);
        let u_of = |m: f64| ((m - t[0]) / (t[4] - t[0])).clamp(0.0, 1.0) as f32;
        let pt = |u: f32| (x0 + u * (x1 - x0), base - (u * std::f32::consts::PI).sin() * hh);
        let pts: Vec<(f32, f32)> = (0..=48).map(|k| pt(k as f32 / 48.0)).collect();
        g.polyline(&pts, 1.2, WHITE.a(0.30 * a));
        for k in 1..5 {
            let (x, y) = pt(u_of(t[k]));
            g.fill_circle(x, y, 2.6, if k == i { WHITE.a(a) } else { WHITE.a(0.45 * a) });
        }
        let day = nm >= t[0] && nm <= t[4];
        let (sx, sy) = pt(if day { u_of(nm) } else { 0.0 });
        g.glow(sx, sy, 15.0, Color::rgb(255, 170, 60).a((if day { 0.85 } else { 0.45 }) * a));
        g.fill_circle(sx, sy, 4.6, Color::rgb(255, 205, 110).a((if day { 1.0 } else { 0.7 }) * a));
        // labels, evenly spaced
        for k in 0..5 {
            let cx = ox + 224.0 + k as f32 * 44.0;
            let hl = k == i;
            self.t(prayer::names_s(&self.settings)[k], cx - 24.0, yo + 104.0, 48.0, 11.0, if hl { 700 } else { 500 }, WHITE.a(if hl { 1.0 } else { 0.62 } * a), Align::Center);
            self.t(&prayer::fmt(t[k]), cx - 24.0, yo + 117.0, 48.0, 10.0, 500, WHITE.a(if hl { 0.8 } else { 0.42 } * a), Align::Center);
        }
    }

    // ------------------------------------------------------------- shelf
    /// Rows of the notch's prayer picker: all countries (level 0) or the cities of one country (level 1).
    pub fn pray_items(&self) -> Vec<(String, String, bool, Act)> {
        let cl = prayer::countries();
        if self.pray_level == 0 {
            let cur = prayer::current_country(&self.settings);
            return cl
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let n = match c.2 {
                        prayer::CRef::Builtin(k) => prayer::all_cities(k).len(),
                        prayer::CRef::World(w) => prayer::WORLD[w].cities.len(),
                    };
                    (c.0.to_string(), format!("{n} cities"), Some(i) == cur, Act::PrayCountry(i))
                })
                .collect();
        }
        let c = cl.get(self.pray_country).copied().unwrap_or(cl[0]);
        prayer::cities_of(c.2)
            .into_iter()
            .map(|pc| {
                let sel = prayer::is_selected(&self.settings, &pc.target);
                let act = match pc.target {
                    prayer::Target::Builtin(p, ci) => Act::PrayPick(p, ci),
                    prayer::Target::World(w, ci) => Act::PrayWorld(w, ci),
                };
                (pc.name, pc.sub, sel, act)
            })
            .collect()
    }

    /// Prayer picker: pick a country, then a city. The mouse wheel scrolls the list.
    fn page_pray_pick(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let cl = prayer::countries();
        let back = Rect { x: ox + 14.0, y: yo + 6.0, w: 190.0, h: 24.0 };
        g.fill_circle(ox + 28.0, yo + 18.0, 10.0, WHITE.a((if self.hovered(back) { 0.22 } else { 0.12 }) * a));
        g.icon('\u{E76B}', ox + 28.0, yo + 18.0, 9.0, WHITE.a(a));
        let title = if self.pray_level == 0 { "Choose country".to_string() } else { cl.get(self.pray_country).map(|c| c.0.to_string()).unwrap_or_default() };
        self.t(&g.fit(&title, 14.5, 700, 150.0), ox + 44.0, yo + 18.0, 160.0, 14.5, 700, WHITE.a(a), Align::Left);
        self.hit(back, if self.pray_level == 0 { Act::PrayOpen } else { Act::PrayBack });

        let items = self.pray_items();
        self.t(&format!("{} {} · scroll", items.len(), if self.pray_level == 0 { "countries" } else { "cities" }), ox + 200.0, yo + 42.0, 200.0, 9.5, 500, FAINT.a(a), Align::Right);
        let (top, row_h, rows) = (yo + 50.0, 22.0, 3.4);
        let scroll = self.pray_scroll.value;
        let view_h = row_h * 3.0 + 6.0;
        g.push_clip(ox + 14.0, top - 2.0, 392.0, view_h + 4.0);
        for (k, (title, sub, sel, act)) in items.iter().enumerate() {
            let y = top + k as f32 * row_h - scroll;
            if y + 20.0 < top - 2.0 || y > top + view_h {
                continue;
            }
            let r = Rect { x: ox + 20.0, y, w: 372.0, h: 20.0 };
            let hot = self.hovered(r) && y >= top - 4.0 && y + 20.0 <= top + view_h + 2.0;
            if *sel || hot {
                g.fill_round(r.x, r.y, r.w, r.h, 10.0, WHITE.a((if *sel { 0.16 } else { 0.09 }) * a));
            }
            self.t(title, r.x + 12.0, y + 10.0, 190.0, 12.0, if *sel { 700 } else { 600 }, WHITE.a(a), Align::Left);
            self.t(&g.fit(sub, 10.0, 500, 140.0), r.x + 200.0, y + 10.0, 150.0, 10.0, 500, FAINT.a(a), Align::Right);
            if *sel {
                g.icon('\u{E73E}', r.x + r.w - 12.0, y + 10.0, 9.0, Color::rgb(32, 201, 166).a(a));
            }
            if y >= top - 6.0 && y + 20.0 <= top + view_h + 2.0 {
                self.hit(r, *act);
            }
        }
        g.pop_clip();
        let _ = rows;
        let max = ((items.len() as f32 - 3.0) * row_h).max(1.0);
        let th = (view_h * 3.0 / items.len().max(3) as f32).clamp(10.0, view_h);
        let ty = top + (view_h - th) * (scroll / max).clamp(0.0, 1.0);
        g.fill_round(ox + 398.0, top, 2.5, view_h, 1.25, WHITE.a(0.08 * a));
        g.fill_round(ox + 398.0, ty, 2.5, th, 1.25, WHITE.a(0.4 * a));
    }

    fn page_shelf(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let s = &self.store;
        let clip_mode = self.settings.clip_history && self.page_aux() == 1;
        self.t("Shelf", ox + 20.0, yo + 18.0, 60.0, 14.5, 700, WHITE.a(a), Align::Left);
        if self.settings.clip_history {
            self.pill_btn_tab("Files", ox + 70.0, yo + 8.0, 50.0, a, !clip_mode, 0);
            self.pill_btn_tab("Clipboard", ox + 124.0, yo + 8.0, 74.0, a, clip_mode, 1);
        }
        if !clip_mode {
            if s.shelf.is_empty() {
                // dotted drop zone with a tray icon
                let z = Rect { x: ox + 20.0, y: yo + 36.0, w: 380.0, h: 86.0 };
                let hot = self.drag_hover || self.hovered(z);
                g.stroke_round_dashed(z.x + 1.0, z.y + 1.0, z.w - 2.0, z.h - 2.0, 18.0, 1.6, WHITE.a((if hot { 0.42 } else { 0.24 }) * a));
                let (cx, cy) = (z.x + z.w / 2.0, z.y + 26.0);
                let ic = WHITE.a(0.88 * a);
                g.line(cx, cy - 11.0, cx, cy + 1.0, 2.4, ic);
                g.poly(&[(cx - 6.0, cy - 3.0), (cx + 6.0, cy - 3.0), (cx, cy + 5.0)], ic, 1.2);
                g.stroke_round(cx - 14.0, cy + 2.0, 28.0, 15.0, 5.0, 2.2, ic);
                g.fill_rect(cx - 14.0, cy + 9.0, 8.0, 2.2, ic);
                g.fill_rect(cx + 6.0, cy + 9.0, 8.0, 2.2, ic);
                self.t("Drop files here", z.x, z.y + 56.0, z.w, 13.5, 700, WHITE.a(a), Align::Center);
                self.t("Drag out anytime · click to open or copy", z.x, z.y + 73.0, z.w, 10.0, 500, FAINT.a(a), Align::Center);
            }
            for (i, p) in s.shelf.iter().take(4).enumerate() {
                let x = ox + 20.0 + i as f32 * 96.0;
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let r = Rect { x, y: yo + 38.0, w: 90.0, h: 82.0 };
                let hot = self.hovered(r);
                g.fill_round(x, r.y, r.w, r.h, 16.0, WHITE.a(if hot { 0.13 } else { 0.07 } * a));
                g.icon(if p.is_dir() { '\u{E8B7}' } else { '\u{E8A5}' }, x + 45.0, r.y + 30.0, 18.0, WHITE.a(0.9 * a));
                let nm = g.fit(&name, 10.0, 500, 78.0);
                self.t(&nm, x + 6.0, r.y + 64.0, 78.0, 10.0, 500, DIM.a(a), Align::Center);
                self.hit(r, Act::ShelfOpen(i));
                if hot {
                    self.icon_btn('\u{E711}', x + 78.0, r.y + 10.0, 6.0, a, Act::ShelfRemove(i));
                }
            }
            if !s.shelf.is_empty() {
                self.pill_btn("Clear", ox + 348.0, yo + 5.0, 52.0, a, Act::ShelfClear, false);
            }
        } else if s.clips.is_empty() {
            self.t("Copy some text", ox + 20.0, yo + 78.0, 380.0, 13.0, 700, WHITE.a(a), Align::Center);
            self.t("It will wait here, ready to paste again", ox + 20.0, yo + 98.0, 380.0, 10.5, 500, DIM.a(a), Align::Center);
        } else {
            for (i, c) in s.clips.iter().take(3).enumerate() {
                let y = yo + 52.0 + i as f32 * 25.0;
                let hot = self.hovered(Rect { x: ox + 20.0, y: y - 11.0, w: 380.0, h: 22.0 });
                g.fill_round(ox + 20.0, y - 11.0, 380.0, 22.0, 11.0, WHITE.a(if hot { 0.13 } else { 0.06 } * a));
                let one: String = c.replace('\n', " ");
                let t = g.fit(&one, 11.0, 500, 350.0);
                self.t(&t, ox + 34.0, y, 350.0, 11.0, 500, WHITE.a(0.9 * a), Align::Left);
                self.hit(Rect { x: ox + 20.0, y: y - 11.0, w: 380.0, h: 22.0 }, Act::ClipCopy(i));
            }
        }
    }

    fn pill_btn_tab(&self, label: &str, x: f32, y: f32, w: f32, a: f32, on: bool, tab: usize) {
        let g = &self.gfx;
        g.fill_round(x, y, w, 20.0, 10.0, WHITE.a(if on { 0.2 } else { 0.0 } * a));
        self.t(label, x, y + 10.0, w, 10.5, 600, WHITE.a(if on { 1.0 } else { 0.55 } * a), Align::Center);
        self.hit(Rect { x, y, w, h: 20.0 }, if tab == 0 { Act::CalToday } else { Act::CalNext });
        // tab acts are routed through shelf_tab in page_action via Dot-like ids
        self.hits.borrow_mut().pop();
        self.hit(Rect { x, y, w, h: 20.0 }, if tab == 0 { Act::ClipCopy(usize::MAX - 1) } else { Act::ClipCopy(usize::MAX) });
    }

    pub fn page_aux(&self) -> usize {
        self.shelf_tab
    }

    // ---------------------------------------------------- page actions
    pub fn page_action(&mut self, act: Act, _lx: f32, _r: Rect) {
        match act {
            Act::Drink => {
                let b = self.settings.bottle;
                self.store.drink(b);
            }
            Act::WaterCfg => self.water_cfg = !self.water_cfg,
            Act::WaterToggle => {
                self.settings.water = !self.settings.water;
                self.settings.save();
            }
            Act::WaterBottle(d) => {
                self.settings.bottle = (self.settings.bottle as i32 + d * 50).clamp(100, 1000) as u32;
                self.settings.save();
            }
            Act::WaterGoal(ml) => {
                self.settings.water_goal = ml;
                self.settings.save();
            }
            Act::WaterFrom(d) => {
                let v = (self.settings.water_from as i32 + d * 30).clamp(0, self.settings.water_to as i32 - 60);
                self.settings.water_from = v.max(0) as u32;
                self.settings.save();
            }
            Act::WaterTo(d) => {
                let v = (self.settings.water_to as i32 + d * 30).clamp(self.settings.water_from as i32 + 60, 1439);
                self.settings.water_to = v as u32;
                self.settings.save();
            }
            Act::WaterReset => {
                self.store.water_taken = 0;
                self.store.water_hours = [0; 24];
                self.store.save_water();
            }
            Act::Undo => {
                let b = self.settings.bottle.min(self.store.water_taken);
                self.store.water_taken -= b;
                let h = sys::now().hour as usize;
                self.store.water_hours[h] = self.store.water_hours[h].saturating_sub(b);
                self.store.save_water();
            }
            Act::TodoToggle(i) => {
                if let Some(t) = self.store.todos.get_mut(i) {
                    t.done = !t.done;
                }
                self.store.save_todos();
            }
            Act::TodoDelete(i) => {
                if i < self.store.todos.len() {
                    self.store.todos.remove(i);
                    self.store.save_todos();
                }
            }
            Act::TodoAdd => crate::settings_ui::open_task_dialog(),
            Act::TodoSelect(i) => self.todo_sel = if self.todo_sel == Some(i) { None } else { Some(i) },
            Act::TodoEdit => {
                if let Some(i) = self.todo_sel {
                    crate::settings_ui::open_task_edit(i);
                }
            }
            Act::FocusToggle => self.focus_toggle(),
            Act::FocusReset => {
                self.store.phase = Phase::Idle;
                self.store.running = false;
            }
            Act::FocusSkip => self.focus_next_phase(false),
            Act::ShelfOpen(i) => {
                if let Some(p) = self.store.shelf.get(i) {
                    sys::open_path(p);
                }
            }
            Act::ShelfRemove(i) => {
                if i < self.store.shelf.len() {
                    self.store.shelf.remove(i);
                    self.store.save_shelf();
                }
            }
            Act::ShelfClear => {
                self.store.shelf.clear();
                self.store.save_shelf();
            }
            Act::ClipCopy(i) => {
                if i == usize::MAX - 1 {
                    self.shelf_tab = 0;
                } else if i == usize::MAX {
                    self.shelf_tab = 1;
                } else if let Some(c) = self.store.clips.get(i).cloned() {
                    set_clipboard_text(&c);
                }
            }
            Act::CalPrev => {
                let (y, m) = self.cal_month;
                self.cal_month = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
            }
            Act::CalNext => {
                let (y, m) = self.cal_month;
                self.cal_month = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            }
            Act::CalToday => {
                let n = sys::now();
                self.cal_month = (n.year, n.month);
            }
            Act::ToastA | Act::ToastB => self.toast_click(act == Act::ToastA),
            _ => {}
        }
        self.dirty.set(true);
    }

    // ----------------------------------------------------------- pomodoro
    pub fn focus_toggle(&mut self) {
        let s = &mut self.store;
        if s.phase == Phase::Idle {
            s.phase = Phase::Focus;
            s.phase_total = self.settings.pomo_focus as f64 * 60.0;
            s.remaining = s.phase_total;
            s.running = true;
        } else {
            s.running = !s.running;
        }
    }

    pub fn focus_next_phase(&mut self, completed: bool) {
        let (f, sh, lg) = (self.settings.pomo_focus, self.settings.pomo_short, self.settings.pomo_long);
        let s = &mut self.store;
        match s.phase {
            Phase::Focus => {
                if completed {
                    s.focus_minutes += f;
                    s.focus_sessions += 1;
                    s.cycle += 1;
                    s.save_focus();
                }
                let long = s.cycle % 4 == 0 && s.cycle > 0;
                s.phase = if long { Phase::Long } else { Phase::Short };
                s.phase_total = (if long { lg } else { sh }) as f64 * 60.0;
            }
            _ => {
                s.phase = Phase::Focus;
                s.phase_total = f as f64 * 60.0;
            }
        }
        s.remaining = s.phase_total;
        s.running = completed || s.running;
    }

    // ------------------------------------------------------------ toasts
    pub fn push_toast(&mut self, t: Toast) {
        if self.tour.is_some() {
            return;
        }
        if self.toast.is_none() {
            self.expanded = false;
            self.toast = Some(t);
        } else {
            self.toast_q.push(t);
        }
        if self.settings.sounds {
            sys::chime(0);
        }
        self.dirty.set(true);
    }

    fn toast_click(&mut self, first: bool) {
        let Some(t) = self.toast.take() else { return };
        match (t.tag, first) {
            (1, true) => {
                let b = self.settings.bottle;
                self.store.drink(b);
            }
            (1, false) => self.water_snooze_until = sys::now().minutes() + 15,
            (2, true) => {
                // Later: re-arm in `snooze` minutes
                if let Some(td) = self.store.todos.get_mut(t.idx) {
                    td.time = Some((sys::now().minutes() + self.settings.snooze) % 1440);
                    td.fired_start = false;
                    td.fired_end = false;
                }
                self.store.save_todos();
            }
            (2, false) => {
                if let Some(td) = self.store.todos.get_mut(t.idx) {
                    td.done = true;
                }
                self.store.save_todos();
            }
            _ => {}
        }
        self.toast = self.toast_q.pop();
    }

    pub fn draw_toast(&self, l: f32, yo: f32, w: f32, h: f32) {
        let g = &self.gfx;
        let Some(t) = &self.toast else { return };
        let cy = yo + h / 2.0;
        g.fill_circle(l + 34.0, cy, 17.0, t.color.a(0.2));
        g.icon(t.icon, l + 34.0, cy, 15.0, t.color);
        self.t(&t.title, l + 62.0, cy - 10.0, w - 190.0, 13.0, 700, WHITE, Align::Left);
        let b = g.fit(&t.body, 11.0, 500, w - 190.0);
        self.t(&b, l + 62.0, cy + 9.0, w - 190.0, 11.0, 500, DIM, Align::Left);
        let mut x = l + w - 24.0;
        if let Some((s, _)) = &t.b {
            let bw = g.measure(s, 11.0, 600) + 22.0;
            x -= bw;
            self.pill_btn(s, x, cy - 11.0, bw, 1.0, Act::ToastB, true);
            x -= 6.0;
        }
        if let Some((s, _)) = &t.a {
            let bw = g.measure(s, 11.0, 600) + 22.0;
            x -= bw;
            self.pill_btn(s, x, cy - 11.0, bw, 1.0, Act::ToastA, false);
        }
    }

    pub fn draw_hud(&self, l: f32, yo: f32, w: f32, h: f32) {
        let g = &self.gfx;
        let Some((kind, _, v)) = self.hud else { return };
        let cy = yo + h / 2.0;
        match kind {
            0 => {
                g.icon(if v <= 0.01 { '\u{E74F}' } else { '\u{E767}' }, l + 30.0, cy, 14.0, WHITE);
                g.fill_round(l + 54.0, cy - 2.5, w - 108.0, 5.0, 2.5, WHITE.a(0.2));
                g.fill_round(l + 54.0, cy - 2.5, ((w - 108.0) * v).max(5.0), 5.0, 2.5, WHITE);
                self.t(&format!("{:.0}", v * 100.0), l + w - 46.0, cy, 30.0, 11.0, 600, WHITE, Align::Right);
            }
            1 => {
                g.icon('\u{E72E}', l + 34.0, cy, 14.0, if v > 0.5 { AMBER } else { WHITE });
                self.t(if v > 0.5 { "Caps Lock On" } else { "Caps Lock Off" }, l + 58.0, cy, 170.0, 13.0, 600, WHITE, Align::Left);
            }
            3 => {
                g.icon('\u{E706}', l + 30.0, cy, 14.0, Color::rgb(255, 214, 10));
                g.fill_round(l + 54.0, cy - 2.5, w - 108.0, 5.0, 2.5, WHITE.a(0.2));
                g.fill_round(l + 54.0, cy - 2.5, ((w - 108.0) * v).max(5.0), 5.0, 2.5, Color::rgb(255, 214, 10));
                self.t(&format!("{:.0}", v * 100.0), l + w - 46.0, cy, 30.0, 11.0, 600, WHITE, Align::Right);
            }
            _ => {
                g.icon('\u{E774}', l + 34.0, cy, 14.0, WHITE);
                self.t(&sys::keyboard_layout_name(), l + 58.0, cy, 170.0, 13.0, 600, WHITE, Align::Left);
            }
        }
    }

    pub fn battery_events(&mut self, b: &sys::Battery) {
        let prev = self.battery;
        if !self.settings.pop_battery {
            return;
        }
        if b.ac != prev.ac {
            self.push_toast(Toast {
                icon: '\u{E945}',
                color: GREEN,
                title: if b.ac { "Charging".into() } else { "On battery".into() },
                body: format!("{}%", b.percent),
                a: None,
                b: None,
                until: Instant::now() + Duration::from_secs(3),
                tag: 5,
                idx: 0,
            });
        } else if b.percent <= 10 && !b.ac && prev.percent > 10 {
            self.push_toast(Toast { icon: '\u{EBAA}', color: RED, title: "Battery critical".into(), body: "Plug in now".into(), a: None, b: None, until: Instant::now() + Duration::from_secs(5), tag: 5, idx: 0 });
        } else if b.percent <= 20 && !b.ac && prev.percent > 20 {
            self.push_toast(Toast { icon: '\u{EBA0}', color: AMBER, title: "Low battery".into(), body: "Plug in soon".into(), a: None, b: None, until: Instant::now() + Duration::from_secs(5), tag: 5, idx: 0 });
        } else if b.percent >= 100 && b.ac && prev.percent < 100 {
            self.push_toast(Toast { icon: '\u{EBA7}', color: GREEN, title: "Fully charged".into(), body: "You can unplug".into(), a: None, b: None, until: Instant::now() + Duration::from_secs(4), tag: 5, idx: 0 });
        }
    }

    // --------------------------------------------------------- services
    pub fn tick_services(&mut self, now: Instant) {
        // toast expiry
        if let Some(t) = &self.toast {
            if now >= t.until && t.tag != 2 && t.tag != 1 || (now >= t.until + Duration::from_secs(25)) {
                self.toast = self.toast_q.pop();
                self.dirty.set(true);
            }
        }
        if let Some((_, until, _)) = self.hud {
            if now >= until {
                self.hud = None;
                self.dirty.set(true);
            }
        }
        // one-second services
        if now.duration_since(self.svc_at) < Duration::from_millis(500) {
            return;
        }
        let dt = now.duration_since(self.svc_at).as_secs_f64();
        self.svc_at = now;
        self.updater.set_auto(self.settings.auto_update);
        {
            let b = sys::BRIGHT.load(std::sync::atomic::Ordering::Relaxed);
            if b >= 0 && self.last_bright >= 0 && b != self.last_bright && self.settings.hud_brightness {
                self.show_hud(3, b as f32 / 100.0, 1.6);
            }
            self.last_bright = b;
        }
        if self.settings.auto_update {
            self.install_update(false);
        }
        self.store.roll_day();
        let n = sys::now();
        let minute = n.minutes();

        // pomodoro countdown
        if self.store.running {
            self.store.remaining -= dt;
            if self.store.remaining <= 0.0 {
                let was = self.store.phase;
                self.focus_next_phase(true);
                let (icon, col, title, body) = match was {
                    Phase::Focus => ('\u{E916}', GREEN, "Time's Up!", "Take a break"),
                    _ => ('\u{E916}', AMBER, "Break over", "Back to focus"),
                };
                self.push_toast(Toast { icon, color: col, title: title.into(), body: body.into(), a: None, b: Some(("OK".into(), Act::ToastB)), until: now + Duration::from_secs(8), tag: 4, idx: 0 });
            }
        }

        if minute == self.minute_seen {
            return;
        }
        self.minute_seen = minute;
        self.dirty.set(true);

        // to-do reminders
        if self.settings.todo_start {
            let mut fire = None;
            for (i, t) in self.store.todos.iter_mut().enumerate() {
                if !t.done && !t.fired_start && t.time == Some(minute) {
                    t.fired_start = true;
                    fire = Some((i, t.title.clone()));
                    break;
                }
            }
            if let Some((i, title)) = fire {
                self.push_toast(Toast { icon: '\u{E73E}', color: GREEN, title: "Task Time!".into(), body: title, a: Some(("Later".into(), Act::ToastA)), b: Some(("Done".into(), Act::ToastB)), until: now + Duration::from_secs(60), tag: 2, idx: i });
            }
        }

        // water reminders: spread across active hours
        if self.settings.water && minute >= self.settings.water_from && minute <= self.settings.water_to {
            let bottle = self.settings.bottle.max(100);
            let count = ((self.settings.water_goal as f32 / bottle as f32).ceil() as u32).max(1);
            let span = (self.settings.water_to - self.settings.water_from).max(1);
            let step = (span / count).max(15);
            let due = (minute - self.settings.water_from) % step == 0 && minute > self.settings.water_snooze_floor(self.water_snooze_until);
            let behind = {
                let slots = ((minute - self.settings.water_from) / step + 1).min(count);
                self.store.water_taken < slots * bottle
            };
            if due && behind && self.store.water_taken < self.settings.water_goal {
                self.push_toast(Toast { icon: '\u{E81B}', color: BLUE, title: "Drink".into(), body: format!("{} of {} ml so far", self.store.water_taken, self.settings.water_goal), a: Some(("Snooze".into(), Act::ToastA)), b: Some(("Drank".into(), Act::ToastB)), until: now + Duration::from_secs(40), tag: 1, idx: 0 });
            }
        }

        // prayer reminders
        if self.settings.prayer {
            prayer::ensure(&self.settings);
            let (t, _, city) = prayer::today(&self.settings);
            let pminute = prayer::minutes_now(&self.settings) as u32;
            for i in 0..5 {
                let at = t[i] as u32;
                if self.settings.pray_before && pminute + 10 == at {
                    self.push_toast(Toast { icon: '\u{E708}', color: AMBER, title: format!("{} in 10 minutes", prayer::names_s(&self.settings)[i]), body: city.name.into(), a: None, b: None, until: now + Duration::from_secs(8), tag: 3, idx: i });
                }
                if self.settings.pray_at && pminute == at {
                    self.push_toast(Toast { icon: '\u{E708}', color: AMBER, title: prayer::time_title_s(&self.settings, prayer::names_s(&self.settings)[i]), body: format!("{} · {}", city.name, prayer::fmt(t[i])), a: None, b: None, until: now + Duration::from_secs(10), tag: 3, idx: i });
                    if self.settings.pray_pause && self.is_playing() {
                        self.media.send(MediaCmd::PlayPause);
                    }
                }
            }
        }
    }
}

impl crate::config::Settings {
    pub fn water_snooze_floor(&self, until: u32) -> u32 {
        until
    }
}
