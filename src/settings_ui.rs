//! Settings window and the add-task dialog, drawn with a small immediate-mode toolkit.
//! Visual language is deliberately its own: a narrow icon rail, warm amber accent, soft cards.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU8, Ordering as AOrd};
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{app, monitor_count, Rect, BLACK, WHITE};
use crate::config::{self, Settings};
use crate::gfx::{Align, Color, Gfx};
use crate::prayer;
use crate::store::Todo;
use crate::sys;

const BG: Color = Color::rgba(3, 3, 6, 0.80);
const RAIL: Color = Color::rgba(0, 0, 0, 0.34);
const CARD: Color = Color::rgba(255, 255, 255, 0.045);
const CARD_HI: Color = Color::rgba(255, 255, 255, 0.12);
const WELL: Color = Color::rgba(0, 0, 0, 0.42);
const ACCENT: Color = Color::rgb(255, 159, 10);
const TEXT: Color = Color::rgba(255, 255, 255, 0.93);
const DIM: Color = Color::rgba(255, 255, 255, 0.52);
const LINE: Color = Color::rgba(255, 255, 255, 0.06);
const ROW: f32 = 58.0;

const W: f32 = 1300.0;
const H: f32 = 800.0;
const RAIL_W: f32 = 76.0;
const COL2_W: f32 = 364.0;
const MAIN_X: f32 = RAIL_W + COL2_W + 28.0;
const MAIN_W: f32 = W - MAIN_X - 28.0;
const SIDE_W: f32 = 300.0;
const RAIL_TOP: f32 = 96.0;
const ITEM_H: f32 = 42.0;

// ------------------------------------------------------------------ toolkit
struct Ui<'a> {
    g: &'a Gfx,
    x: f32,
    w: f32,
    y: f32,
    top: f32,
    bottom: f32,
    mouse: (f32, f32),
    click: Option<(f32, f32)>,
    changed: bool,
    card_top: f32,
    row: usize,
    anim: &'a RefCell<HashMap<u64, f32>>,
    dt: f32,
    moving: &'a Cell<bool>,
    /// Only the section with this index is shown (usize::MAX = show everything).
    sel: usize,
    sec: usize,
    sup: bool,
    sup_y: f32,
    titles: &'a RefCell<Vec<String>>,
}

impl<'a> Ui<'a> {
    /// Smoothly chase `target`; first sight snaps so nothing animates on page load.
    fn ease<K: Hash>(&self, key: K, target: f32) -> f32 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        ((self.card_top * 10.0) as i32).hash(&mut h);
        let k = h.finish();
        let mut m = self.anim.borrow_mut();
        let v = m.entry(k).or_insert(target);
        let f = 1.0 - (-16.0 * self.dt).exp();
        *v += (target - *v) * f;
        if (target - *v).abs() < 0.003 {
            *v = target;
        } else {
            self.moving.set(true);
        }
        *v
    }

    fn visible(&self, r: Rect) -> bool {
        r.y + r.h > self.top && r.y < self.bottom
    }
    fn hot(&self, r: Rect) -> bool {
        !self.sup && r.contains(self.mouse.0, self.mouse.1) && self.mouse.1 > self.top && self.mouse.1 < self.bottom
    }
    fn clicked(&mut self, r: Rect) -> bool {
        if self.sup {
            return false;
        }
        if let Some(c) = self.click {
            if r.contains(c.0, c.1) && c.1 > self.top && c.1 < self.bottom {
                self.click = None;
                return true;
            }
        }
        false
    }
    /// Starts a section. Sections other than the selected one are laid out but clipped away (nothing drawn,
    /// no clicks) and take no space; the banner shows the selected section's title.
    fn section(&mut self, t: &str) {
        self.finish();
        let idx = self.sec;
        self.sec += 1;
        {
            let mut v = self.titles.borrow_mut();
            if v.len() <= idx {
                v.push(t.to_string());
                self.moving.set(true);
            } else if v[idx] != t {
                v[idx] = t.to_string();
                self.moving.set(true);
            }
        }
        if self.sel != usize::MAX && idx != self.sel {
            self.sup = true;
            self.sup_y = self.y;
            self.g.push_clip(0.0, 0.0, 0.0, 0.0);
            return;
        }
        if self.sel == usize::MAX {
            self.g.icon(section_icon(t), self.x + 14.0, self.y + 9.0, 14.0, ACCENT);
            self.g.text(t, self.x + 36.0, self.y - 1.0, self.w - 40.0, 20.0, 15.0, 700, TEXT, Align::Left);
            self.y += 32.0;
        } else {
            self.y += 4.0;
        }
    }
    /// Ends a hidden section: restore the layout cursor and the clip.
    fn finish(&mut self) {
        if self.sup {
            self.g.pop_clip();
            self.sup = false;
            self.y = self.sup_y;
        }
    }
    /// A read-only row: label on the left, value on the right.
    fn info(&mut self, label: &str, value: &str) {
        let cy = self.next_row(label, "");
        self.g.text(value, self.x + self.w - 220.0, cy - 12.0, 200.0, 24.0, 13.0, 600, DIM, Align::Right);
    }
    fn note(&mut self, t: &str) {
        self.g.text(t, self.x + 6.0, self.y - 4.0, self.w - 12.0, 18.0, 11.5, 500, DIM, Align::Left);
        self.y += 22.0;
    }
    fn card(&mut self, rows: usize) {
        self.card_top = self.y;
        self.row = 0;
        let h = rows as f32 * ROW;
        self.glass_card(self.y, h);
        self.y += h + 22.0;
    }
    fn glass_card(&self, y: f32, h: f32) {
        let g = self.g;
        g.fill_round(self.x, y, self.w, h, 18.0, CARD);
        // soft top sheen clipped to the card, then a rim that is bright at the top-left and fades
        g.push_round_clip(self.x, y, self.w, h, 18.0);
        g.vgradient(self.x, y, self.w, (h * 0.5).min(54.0), Color::rgba(255, 255, 255, 0.06), Color::rgba(255, 255, 255, 0.0));
        g.pop_layer();
        if let Ok(geo) = g.pill_geometry(self.x + 0.5, y + 0.5, self.w - 1.0, h - 1.0, 18.0) {
            g.stroke_geo_grad(&geo, (self.x, y), (self.x + self.w, y + h), &[(0.0, Color::rgba(255, 255, 255, 0.24)), (0.5, Color::rgba(255, 255, 255, 0.06)), (1.0, Color::rgba(255, 255, 255, 0.13))], 1.0);
        }
    }

        fn card_h(&mut self, h: f32) {
        self.card_top = self.y;
        self.row = 0;
        self.glass_card(self.y, h);
        self.y += h + 22.0;
    }
    fn next_row(&mut self, label: &str, sub: &str) -> f32 {
        self.next_row_max(label, sub, self.w * 0.58)
    }

    fn next_row_max(&mut self, label: &str, sub: &str, max_w: f32) -> f32 {
        let ry = self.card_top + self.row as f32 * ROW;
        if self.row > 0 {
            self.g.fill_rect(self.x + 18.0, ry, self.w - 36.0, 1.0, LINE);
        }
        self.row += 1;
        let cy = ry + ROW / 2.0;
        let mw = max_w.max(80.0);
        if sub.is_empty() {
            self.g.text(label, self.x + 20.0, cy - 12.0, mw, 24.0, 14.0, 600, TEXT, Align::Left);
        } else {
            self.g.text(label, self.x + 20.0, cy - 19.0, mw, 20.0, 14.0, 600, TEXT, Align::Left);
            let s = self.g.fit(sub, 11.5, 500, mw);
            self.g.text(&s, self.x + 20.0, cy + 1.0, mw, 18.0, 11.5, 500, DIM, Align::Left);
        }
        cy
    }
    fn right(&self) -> f32 {
        self.x + self.w - 20.0
    }

    fn toggle(&mut self, label: &str, sub: &str, v: &mut bool) {
        let cy = self.next_row(label, sub);
        let r = Rect { x: self.right() - 46.0, y: cy - 13.0, w: 46.0, h: 26.0 };
        if self.clicked(Rect { x: self.x, y: cy - ROW / 2.0, w: self.w, h: ROW }) {
            *v = !*v;
            self.changed = true;
        }
        let t = self.ease(label, *v as u8 as f32);
        let g = self.g;
        g.fill_round(r.x, r.y, r.w, r.h, 13.0, Color::rgba(255, 255, 255, 0.20).mix(ACCENT, t));
        g.stroke_round(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 13.0, 1.0, Color::rgba(255, 255, 255, 0.14));
        // knob squashes slightly while travelling (liquid feel)
        let stretch = 1.0 + 0.22 * (4.0 * t * (1.0 - t));
        let kx = (r.x + 13.0) + (r.w - 26.0) * t;
        g.fill_round(kx - 10.0 * stretch, cy - 10.0, 20.0 * stretch, 20.0, 10.0, WHITE_C);
    }

    fn seg(&mut self, label: &str, sub: &str, opts: &[&str], cur: usize) -> Option<usize> {
        let cy = self.next_row(label, sub);
        let g = self.g;
        let widths: Vec<f32> = opts.iter().map(|o| (g.measure(o, 12.5, 600) + 26.0).max(52.0)).collect();
        let total: f32 = widths.iter().sum::<f32>() + 6.0;
        let mut x = self.right() - total;
        g.fill_round(x, cy - 16.0, total, 32.0, 16.0, WELL);
        x += 3.0;
        let mut out = None;
        let fi = self.ease(label, cur as f32);
        let (lo, hi) = (fi.floor() as usize, (fi.ceil() as usize).min(opts.len() - 1));
        let frac = fi - fi.floor();
        let start = |i: usize| x + widths[..i].iter().sum::<f32>();
        let px = start(lo) + (start(hi) - start(lo)) * frac;
        let pw = widths[lo] + (widths[hi] - widths[lo]) * frac;
        g.fill_round(px, cy - 13.0, pw, 26.0, 13.0, ACCENT);
        for (i, o) in opts.iter().enumerate() {
            let r = Rect { x, y: cy - 13.0, w: widths[i], h: 26.0 };
            if self.clicked(r) && i != cur {
                out = Some(i);
                self.changed = true;
            }
            if i != cur && self.hot(r) {
                g.fill_round(r.x, r.y, r.w, r.h, 13.0, Color::rgba(255, 255, 255, 0.08));
            }
            let near = (1.0 - (fi - i as f32).abs()).clamp(0.0, 1.0);
            g.text(o, r.x, r.y, r.w, r.h, 12.5, 600, TEXT.mix(Color::rgb(30, 20, 0), near), Align::Center);
            x += widths[i];
        }
        out
    }

    /// Returns -1 / 0 / +1.
    fn stepper(&mut self, label: &str, sub: &str, text: &str, width: f32) -> i32 {
        let cy = self.next_row(label, sub);
        let g = self.g;
        let rr = self.right();
        let (m, p) = (Rect { x: rr - 30.0, y: cy - 14.0, w: 30.0, h: 28.0 }, Rect { x: rr - 30.0 - width - 36.0, y: cy - 14.0, w: 30.0, h: 28.0 });
        let _ = (m, p);
        // [-] value [+]  (minus left, plus right)
        let plus = Rect { x: rr - 30.0, y: cy - 14.0, w: 30.0, h: 28.0 };
        let minus = Rect { x: rr - 30.0 - width - 12.0 - 30.0, y: cy - 14.0, w: 30.0, h: 28.0 };
        let mut d = 0;
        for (r, glyph, delta) in [(minus, '\u{E108}', -1), (plus, '\u{E109}', 1)] {
            if self.clicked(r) {
                d = delta;
                self.changed = true;
            }
            g.fill_circle(r.x + 15.0, cy, 14.0, if self.hot(r) { CARD_HI } else { WELL });
            g.icon(glyph, r.x + 15.0, cy, 9.0, TEXT);
        }
        g.text(text, minus.x + 36.0, cy - 12.0, width + 0.0, 24.0, 13.5, 600, TEXT, Align::Center);
        d
    }

    fn button(&mut self, label: &str, sub: &str, btn: &str, danger: bool) -> bool {
        let cy = self.next_row(label, sub);
        let g = self.g;
        let w = g.measure(btn, 12.5, 600) + 32.0;
        let r = Rect { x: self.right() - w, y: cy - 15.0, w, h: 30.0 };
        let hot = self.hot(r);
        let click = self.clicked(r);
        g.fill_round(r.x, r.y, r.w, r.h, 15.0, if hot { CARD_HI } else { WELL });
        g.text(btn, r.x, r.y, r.w, r.h, 12.5, 600, if danger { Color::rgb(255, 99, 88) } else { TEXT }, Align::Center);
        if click {
            self.changed = true;
        }
        click
    }
}

const WHITE_C: Color = Color::rgb(255, 255, 255);

impl<'a> Ui<'a> {
    /// Wrapped chips in one card; returns the index clicked. Selected chips are amber.
    fn chips(&mut self, items: &[(String, bool)]) -> Option<usize> {
        let g = self.g;
        let (pad, gap, ch) = (14.0, 8.0, 30.0);
        let max_x = self.w - pad;
        let widths: Vec<f32> = items.iter().map(|(t, _)| g.measure(t, 12.5, 600) + 26.0).collect();
        // lay out rows first so the card gets the right height
        let mut pos: Vec<(f32, f32)> = vec![];
        let (mut x, mut y) = (pad, pad);
        for w in &widths {
            if x + w > max_x && x > pad {
                x = pad;
                y += ch + gap;
            }
            pos.push((x, y));
            x += w + gap;
        }
        let h = y + ch + pad;
        self.card_h(h);
        let (cx, cy) = (self.x, self.card_top);
        let mut out = None;
        for (i, (t, sel)) in items.iter().enumerate() {
            let r = Rect { x: cx + pos[i].0, y: cy + pos[i].1, w: widths[i], h: ch };
            let hot = self.hot(r);
            if *sel {
                g.fill_round(r.x, r.y, r.w, r.h, 15.0, ACCENT);
            } else {
                g.fill_round(r.x, r.y, r.w, r.h, 15.0, if hot { CARD_HI } else { WELL });
            }
            g.text(t, r.x, r.y, r.w, r.h, 12.5, 600, if *sel { Color::rgb(30, 20, 0) } else { TEXT }, Align::Center);
            if self.clicked(r) {
                out = Some(i);
            }
        }
        out
    }
}

/// Which text box of the settings window receives typing (0 = none).
static FOCUS: AtomicU8 = AtomicU8::new(0);

fn masked(s: &str) -> String {
    let n = s.chars().count();
    if n <= 4 {
        "•".repeat(n)
    } else {
        format!("{}{}", "•".repeat((n - 4).min(24)), s.chars().skip(n - 4).collect::<String>())
    }
}

impl<'a> Ui<'a> {
    /// A row with a text box. Typing is routed by `settings_proc` through FOCUS.
    fn field(&mut self, id: u8, label: &str, sub: &str, val: &str, secret: bool, hint: &str) {
        let cy = self.next_row_max(label, sub, self.w - 345.0);
        let g = self.g;
        let r = Rect { x: self.right() - 290.0, y: cy - 17.0, w: 290.0, h: 34.0 };
        let on = FOCUS.load(AOrd::Relaxed) == id;
        g.fill_round(r.x, r.y, r.w, r.h, 11.0, WELL);
        g.stroke_round(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 11.0, 1.2, if on { ACCENT } else { Color::rgba(255, 255, 255, 0.10) });
        let shown = if secret { masked(val) } else { val.to_string() };
        if shown.is_empty() {
            g.text(hint, r.x + 12.0, r.y, r.w - 20.0, r.h, 12.5, 500, DIM, Align::Left);
        } else {
            let mut s: Vec<char> = shown.chars().collect();
            while s.len() > 1 && g.measure(&s.iter().collect::<String>(), 12.5, 500) > r.w - 28.0 {
                s.remove(0);
            }
            let t: String = s.iter().collect();
            g.text(&t, r.x + 12.0, r.y, r.w - 20.0, r.h, 12.5, 500, TEXT, Align::Left);
            if on {
                g.fill_rect(r.x + 13.0 + g.measure(&t, 12.5, 500), r.y + 8.0, 1.5, 18.0, ACCENT);
            }
        }
        if on && shown.is_empty() {
            g.fill_rect(r.x + 12.0, r.y + 8.0, 1.5, 18.0, ACCENT);
        }
        if self.clicked(r) {
            FOCUS.store(id, AOrd::Relaxed);
        }
    }
}

/// One-line description of a section for the second column.
fn section_sub(t: &str) -> &'static str {
    let l = t.to_lowercase();
    for (k, s) in [
        ("startup", "Launch, auto start and background"), ("update", "Version, auto updates and notes"), ("notch style", "Look of the island and glass"),
        ("notch", "Size, position and behavior"), ("sound", "Reminder sounds"), ("full screen", "Hide during games and videos"),
        ("performance", "Resource use and tools"), ("theme", "Colours and glow"), ("behaviour", "How it opens and animates"),
        ("lyric", "Synced lyrics and sources"), ("find a city", "Search any city in the world"), ("country", "Pick your country"),
        ("cities", "Choose your city"), ("prayer", "Method and place"), ("reminder", "Alerts and notifications"), ("fine-tune", "Shift times by minutes"),
        ("assistant", "Voice and text AI"), ("ai provider", "Which AI answers"), ("configuration", "Key, model and server"), ("detected", "Pick a model"),
        ("things you can say", "Voice commands"), ("info", "About and links"), ("indicator", "Volume, brightness and more"), ("popup", "System popups"),
        ("shelf", "Files and clipboard"), ("widget", "Cards on the widgets page"), ("pomodoro", "Focus and break lengths"), ("water", "Bottle, goal and hours"),
    ] {
        if l.contains(k) {
            return s;
        }
    }
    ""
}

static SYSINFO: std::sync::Mutex<(f32, f32)> = std::sync::Mutex::new((0.0, 0.0));
static RESET_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

fn section_icon(t: &str) -> char {
    if t.to_lowercase().contains("performance") {
        return '\u{E9D9}';
    }
    if t.to_lowercase().contains("assistant") || t.to_lowercase().contains("ai engine") {
        return '\u{E720}';
    }
    let l = t.to_lowercase();
    for (k, ic) in [
        ("startup", '\u{E7E8}'), ("update", '\u{E895}'), ("notch", '\u{E7F4}'), ("sound", '\u{E767}'), ("full screen", '\u{E740}'),
        ("theme", '\u{E790}'), ("behaviour", '\u{E713}'), ("lyric", '\u{E8D6}'), ("reminder", '\u{EA8F}'), ("fine-tune", '\u{E9E9}'),
        ("info", '\u{E946}'), ("find", '\u{E721}'), ("prayer", '\u{E708}'), ("shelf", '\u{E8B7}'), ("water", '\u{E81B}'),
        ("pomodoro", '\u{E916}'), ("calendar", '\u{E787}'), ("to-do", '\u{E73A}'), ("widget", '\u{E8A9}'), ("display", '\u{E7F4}'),
        ("popup", '\u{E7ED}'), ("indicator", '\u{E7ED}'),
    ] {
        if l.contains(k) {
            return ic;
        }
    }
    '\u{E713}'
}

/// Card with a gradient hairline, used by the right-hand column.
fn glass_box(g: &Gfx, x: f32, y: f32, w: f32, h: f32) {
    g.fill_round(x, y, w, h, 18.0, CARD);
    g.push_round_clip(x, y, w, h, 18.0);
    g.vgradient(x, y, w, (h * 0.4).min(46.0), Color::rgba(255, 255, 255, 0.055), Color::rgba(255, 255, 255, 0.0));
    g.pop_layer();
    if let Ok(geo) = g.pill_geometry(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 18.0) {
        g.stroke_geo_grad(&geo, (x, y), (x + w, y + h), &[(0.0, Color::rgba(255, 255, 255, 0.24)), (0.5, Color::rgba(255, 255, 255, 0.06)), (1.0, Color::rgba(255, 255, 255, 0.13))], 1.0);
    }
}

/// The window body: near-black tint over the system acrylic, a top sheen, a faint light bloom and a
/// specular rim around the whole window. `w`/`h` are the client size in DIPs.
fn glass_window(g: &Gfx, w: f32, h: f32) {
    g.fill_rect(0.0, 0.0, w + 400.0, h + 400.0, BG);
    g.vgradient(0.0, 0.0, w, h * 0.40, Color::rgba(255, 255, 255, 0.05), Color::rgba(255, 255, 255, 0.0));
    g.glow(w * 0.30, -30.0, 380.0, Color::rgba(255, 255, 255, 0.045));
    if let Ok(geo) = g.pill_geometry(0.5, 0.5, w - 1.0, h - 1.0, 8.0) {
        g.stroke_geo_grad(&geo, (0.0, 0.0), (w, h), &[(0.0, Color::rgba(255, 255, 255, 0.30)), (0.35, Color::rgba(255, 255, 255, 0.07)), (1.0, Color::rgba(255, 255, 255, 0.18))], 1.0);
    }
}

// --------------------------------------------------------------------- tabs
struct Tab {
    name: &'static str,
    sub: &'static str,
    icon: char,
    color: Color,
}
const TABS: [Tab; 14] = [
    Tab { name: "General", sub: "Startup, size and sounds", icon: '\u{E713}', color: Color::rgb(142, 142, 147) },
    Tab { name: "Appearance", sub: "How the notch looks and moves", icon: '\u{E790}', color: Color::rgb(191, 90, 242) },
    Tab { name: "Display", sub: "Where the notch appears", icon: '\u{E7F4}', color: Color::rgb(94, 92, 230) },
    Tab { name: "Music & lyrics", sub: "Now playing and synced lyrics", icon: '\u{E8D6}', color: Color::rgb(255, 55, 95) },
    Tab { name: "Widgets", sub: "Two glanceable cards, one swipe away", icon: '\u{E8A9}', color: Color::rgb(10, 132, 255) },
    Tab { name: "To-do", sub: "Tasks and their reminders", icon: '\u{E73A}', color: Color::rgb(48, 209, 88) },
    Tab { name: "Calendar", sub: "Month overview", icon: '\u{E787}', color: Color::rgb(255, 69, 58) },
    Tab { name: "Focus", sub: "Pomodoro timer", icon: '\u{E916}', color: Color::rgb(124, 108, 255) },
    Tab { name: "Water", sub: "Hydration plan and reminders", icon: '\u{EA3A}', color: Color::rgb(0, 160, 255) },
    Tab { name: "Prayer", sub: "Prayer times for any city in the world", icon: '\u{E708}', color: Color::rgb(32, 201, 166) },
    Tab { name: "Shelf", sub: "Files and clipboard history", icon: '\u{E8B7}', color: Color::rgb(255, 189, 46) },
    Tab { name: "HUD & popups", sub: "Indicators and system popups", icon: '\u{E7ED}', color: Color::rgb(50, 173, 230) },
    Tab { name: "Assistant", sub: "Voice and text AI inside the island", icon: '\u{E720}', color: Color::rgb(167, 139, 250) },
    Tab { name: "About", sub: "", icon: '\u{E946}', color: Color::rgb(120, 120, 128) },
];

pub struct Panel {
    hwnd: HWND,
    gfx: Gfx,
    scale: f32,
    tab: usize,
    scroll: f32,
    content_h: f32,
    mouse: (f32, f32),
    click: Option<(f32, f32)>,
    side: usize,
    pray_sel: usize,
    anim: RefCell<HashMap<u64, f32>>,
    moving: Cell<bool>,
    last: Instant,
    tab_t: f32,
    rail_y: f32,
    logo: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>,
    search: String,
    pray_view: usize,
    sel_sec: usize,
    titles: RefCell<Vec<String>>,
    confirm_reset: Option<Instant>,
    cpu_at: Instant,
    cpu_ms: f64,
    cpu_pct: f32,
}

static mut PANEL: *mut Panel = std::ptr::null_mut();
static mut TASKP: *mut TaskPanel = std::ptr::null_mut();

fn dpi_scale(hwnd: HWND) -> f32 {
    unsafe {
        let d = GetDpiForWindow(hwnd);
        if d == 0 {
            1.0
        } else {
            d as f32 / 96.0
        }
    }
}

fn dark_frame(hwnd: HWND) {
    unsafe {
        // Windows 11 acrylic backdrop behind a transparent client area = liquid-glass window
        let backdrop: i32 = 3; // DWMSBT_TRANSIENTWINDOW
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop as *const _ as *const _, 4);
        let m = windows::Win32::UI::Controls::MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        let on: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const _ as *const _, 4);

    }
}

pub fn is_open() -> bool {
    unsafe { !PANEL.is_null() }
}

pub fn open() {
    unsafe {
        if !PANEL.is_null() {
            let p = &*PANEL;
            let _ = ShowWindow(p.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(p.hwnd);
            return;
        }
        let hinst = GetModuleHandleW(None).unwrap();
        let icon = LoadIconW(hinst, PCWSTR(1 as _)).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let class = w!("NookSettings");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst.into(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            lpszClassName: class,
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let s = app().map(|a| a.scale).unwrap_or(1.0).max(1.0).min(2.0);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: (W * s) as i32, bottom: (H * s) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, WINDOW_EX_STYLE(0));
        let (ow, oh) = (r.right - r.left, r.bottom - r.top);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            class,
            w!("Nook Settings"),
            style,
            (sw - ow) / 2,
            (sh - oh) / 2,
            ow,
            oh,
            None,
            None,
            hinst,
            None,
        );
        let hwnd = match hwnd {
            Ok(h) => h,
            Err(e) => {
                sys::log(&format!("settings window failed: {e:?}"));
                return;
            }
        };
        let s = dpi_scale(hwnd);
        // resize to the real DPI of the monitor the window landed on
        let mut r = RECT { left: 0, top: 0, right: (W * s) as i32, bottom: (H * s) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, WINDOW_EX_STYLE(0));
        let _ = SetWindowPos(hwnd, None, 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER);
        dark_frame(hwnd);
        let mut cr = RECT::default();
        let _ = GetClientRect(hwnd, &mut cr);
        let gfx = match Gfx::new_overlay(hwnd, (cr.right - cr.left) as u32, (cr.bottom - cr.top) as u32, 96.0 * s) {
            Ok(g) => g,
            Err(e) => {
                sys::log(&format!("settings gfx failed: {e:?}"));
                let _ = DestroyWindow(hwnd);
                return;
            }
        };
        let logo = gfx.bitmap_from_bytes(include_bytes!("../assets/logo128.png"));
        let dbg_tab: usize = std::env::var("NOOK_DEBUG_TAB").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let p = Box::new(Panel { hwnd, gfx, scale: s, tab: dbg_tab.min(13), scroll: std::env::var("NOOK_DEBUG_SCROLL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0), content_h: 0.0, mouse: (-1.0, -1.0), click: None, side: 0, pray_sel: 0, anim: RefCell::new(HashMap::new()), moving: Cell::new(false), last: Instant::now(), tab_t: 1.0, rail_y: RAIL_TOP + ITEM_H * dbg_tab.min(13) as f32, logo, search: std::env::var("NOOK_DEBUG_SEARCH").unwrap_or_default(), pray_view: usize::MAX, sel_sec: 0, titles: RefCell::new(vec![]), confirm_reset: None, cpu_at: Instant::now(), cpu_ms: 0.0, cpu_pct: 0.0 });
        PANEL = Box::into_raw(p);
        SetTimer(hwnd, 7, 16, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        if std::env::var("NOOK_DEBUG_TAB").is_ok() {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        }
        let _ = SetForegroundWindow(hwnd);
        let _ = InvalidateRect(hwnd, None, false);
    }
}

unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if PANEL.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let p = &mut *PANEL;
    let pt = |lp: LPARAM| -> (f32, f32) {
        (((lp.0 & 0xFFFF) as i16) as f32 / p.scale, (((lp.0 >> 16) & 0xFFFF) as i16) as f32 / p.scale)
    };
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            p.paint();
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CHAR => {
            // a focused settings text box (assistant key / model / URL)
            let f = FOCUS.load(AOrd::Relaxed);
            if f != 0 {
                if let Some(a) = app() {
                    let ch = wp.0 as u32;
                    let field: &mut String = match f {
                        1 => &mut a.settings.ai_key,
                        2 => &mut a.settings.ai_model,
                        _ => &mut a.settings.ai_url,
                    };
                    match ch {
                        8 => {
                            field.pop();
                        }
                        27 | 13 => FOCUS.store(0, AOrd::Relaxed),
                        22 => {
                            if let Some(t) = crate::app::read_clipboard_text() {
                                field.push_str(t.trim());
                            }
                        }
                        c if c >= 32 && c != 127 => {
                            if let Some(c) = char::from_u32(c) {
                                if field.chars().count() < 200 {
                                    field.push(c);
                                }
                            }
                        }
                        _ => {}
                    }
                    a.settings.save();
                }
                let _ = InvalidateRect(hwnd, None, false);
                return LRESULT(0);
            }
            // typing in the prayer tab fills the city finder
            if p.tab == 9 {
                let ch = wp.0 as u32;
                match ch {
                    8 => {
                        p.search.pop();
                    }
                    27 => p.search.clear(),
                    c if c >= 32 && c != 127 => {
                        if let Some(c) = char::from_u32(c) {
                            if p.search.chars().count() < 32 {
                                p.search.push(c);
                            }
                        }
                    }
                    _ => {}
                }
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if p.moving.get() || p.cpu_at.elapsed() > Duration::from_millis(1100) {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let w = (lp.0 & 0xFFFF) as u32;
            let h = ((lp.0 >> 16) & 0xFFFF) as u32;
            if w > 0 && h > 0 {
                let _ = p.gfx.resize(w, h);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            p.mouse = pt(lp);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            p.click = Some(pt(lp));
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let d = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
            let max = (p.content_h - (H - 4.0)).max(0.0);
            p.scroll = (p.scroll - d / 120.0 * 48.0).clamp(0.0, max);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            p.scale = dpi_scale(hwnd);
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            drop(Box::from_raw(PANEL));
            PANEL = std::ptr::null_mut();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

impl Panel {
    fn paint(&mut self) {
        let Some(a) = app() else { return };
        if self.gfx.begin_overlay().is_err() {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.moving.set(false);
        let sysinfo = self.sample_sys();
        *SYSINFO.lock().unwrap() = sysinfo;
        let g = &self.gfx;
        // ---- body glass + narrow icon rail
        glass_window(g, W, H);
        g.fill_rect(0.0, 0.0, RAIL_W, H + 400.0, RAIL);
        g.fill_rect(RAIL_W, 0.0, 1.0, H + 400.0, Color::rgba(255, 255, 255, 0.07));
        g.fill_rect(RAIL_W + COL2_W, 0.0, 1.0, H + 400.0, Color::rgba(255, 255, 255, 0.06));
        if let Some(b) = &self.logo {
            g.draw_bitmap(b, 16.0, 18.0, 44.0, 44.0, 1.0);
        } else {
            g.fill_round(16.0, 18.0, 44.0, 44.0, 12.0, ACCENT);
        }
        g.fill_circle(38.0, 76.0, 2.0, ACCENT);

        let target_y = RAIL_TOP + ITEM_H * self.tab as f32;
        self.rail_y += (target_y - self.rail_y) * (1.0 - (-18.0 * dt).exp());
        if (target_y - self.rail_y).abs() > 0.3 {
            self.moving.set(true);
        } else {
            self.rail_y = target_y;
        }
        g.fill_round(12.0, self.rail_y, 52.0, 38.0, 13.0, ACCENT.a(0.14));
        g.stroke_round(12.5, self.rail_y + 0.5, 51.0, 37.0, 13.0, 1.3, ACCENT.a(0.85));
        let mut tip: Option<(&str, f32)> = None;
        let mut y = RAIL_TOP;
        for (i, t) in TABS.iter().enumerate() {
            let r = Rect { x: 12.0, y, w: 52.0, h: 38.0 };
            let hot = r.contains(self.mouse.0, self.mouse.1);
            let sel = i == self.tab;
            if hot && !sel {
                g.fill_round(r.x, r.y, r.w, r.h, 13.0, Color::rgba(255, 255, 255, 0.06));
                tip = Some((t.name, y));
            }
            g.icon(t.icon, 38.0, y + 19.0, 17.0, if sel { ACCENT } else { Color::rgba(255, 255, 255, if hot { 0.95 } else { 0.70 }) });
            if let Some(c) = self.click {
                if r.contains(c.0, c.1) {
                    if self.tab != i {
                        self.tab_t = 0.0;
                        self.sel_sec = 0;
                        self.scroll = 0.0;
                        self.titles.borrow_mut().clear();
                        FOCUS.store(0, AOrd::Relaxed);
                    }
                    self.tab = i;
                    self.click = None;
                }
            }
            y += ITEM_H;
        }
        // version pill
        g.fill_round(8.0, H - 92.0, 60.0, 74.0, 18.0, WHITE.a(0.05));
        g.stroke_round(8.5, H - 91.5, 59.0, 73.0, 18.0, 1.0, Color::rgba(255, 255, 255, 0.10));
        g.fill_circle(38.0, H - 70.0, 5.0, Color::rgb(48, 209, 88));
        g.text(&format!("v{}", crate::update::VERSION), 8.0, H - 56.0, 60.0, 22.0, 11.0, 600, TEXT, Align::Center);
        self.tab_t += (1.0 - self.tab_t) * (1.0 - (-12.0 * dt).exp());
        if self.tab_t < 0.995 {
            self.moving.set(true);
        } else {
            self.tab_t = 1.0;
        }

        // ---- second column: tab header + its sections
        let t = &TABS[self.tab];
        g.fill_round(RAIL_W + 20.0, 24.0, 58.0, 58.0, 16.0, WHITE.a(0.045));
        g.stroke_round(RAIL_W + 20.5, 24.5, 57.0, 57.0, 16.0, 1.2, ACCENT.a(0.40 * self.tab_t));
        g.icon(t.icon, RAIL_W + 49.0, 53.0, 24.0, ACCENT.a(self.tab_t));
        g.text(t.name, RAIL_W + 92.0, 20.0, COL2_W - 110.0, 36.0, 24.0, 700, TEXT.a(self.tab_t), Align::Left);
        g.text_wrap(t.sub, RAIL_W + 92.0, 56.0, COL2_W - 108.0, 40.0, 12.0, 500, DIM, Align::Left);
        let titles: Vec<String> = self.titles.borrow().clone();
        let sel_eff = if titles.is_empty() { 0 } else { self.sel_sec.min(titles.len() - 1) };
        for (i, st) in titles.iter().enumerate() {
            let r = Rect { x: RAIL_W + 16.0, y: 116.0 + i as f32 * 76.0, w: COL2_W - 32.0, h: 66.0 };
            let sel = i == sel_eff;
            let hot = r.contains(self.mouse.0, self.mouse.1);
            g.fill_round(r.x, r.y, r.w, r.h, 16.0, if sel { ACCENT.a(0.12) } else { WHITE.a(if hot { 0.07 } else { 0.04 }) });
            if sel {
                g.stroke_round(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 16.0, 1.5, ACCENT.a(0.9));
            } else {
                g.stroke_round(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 16.0, 1.0, Color::rgba(255, 255, 255, 0.07));
            }
            g.fill_round(r.x + 14.0, r.y + 13.0, 40.0, 40.0, 12.0, WHITE.a(0.06));
            g.icon(section_icon(st), r.x + 34.0, r.y + 33.0, 17.0, if sel { ACCENT } else { Color::rgba(255, 255, 255, 0.85) });
            g.text(&g.fit(st, 14.0, 700, r.w - 90.0), r.x + 68.0, r.y + 10.0, r.w - 80.0, 24.0, 14.0, 700, TEXT, Align::Left);
            g.text(&g.fit(section_sub(st), 11.5, 500, r.w - 90.0), r.x + 68.0, r.y + 33.0, r.w - 80.0, 22.0, 11.5, 500, DIM, Align::Left);
            if let Some(c) = self.click {
                if r.contains(c.0, c.1) {
                    if self.sel_sec != i {
                        self.sel_sec = i;
                        self.scroll = 0.0;
                        self.tab_t = 0.0;
                        FOCUS.store(0, AOrd::Relaxed);
                    }
                    self.click = None;
                }
            }
        }

        // ---- banner with the section's name, a soft wave and the logo
        let hero = Rect { x: MAIN_X, y: 24.0, w: MAIN_W, h: 132.0 };
        g.push_round_clip(hero.x, hero.y, hero.w, hero.h, 20.0);
        g.fill_rect(hero.x, hero.y, hero.w, hero.h, Color::rgb(20, 18, 14));
        g.glow(hero.x + hero.w - 120.0, hero.y + 60.0, 260.0, ACCENT.a(0.30));
        for k in 0..2 {
            let pts: Vec<(f32, f32)> = (0..=48)
                .map(|i| {
                    let u = i as f32 / 48.0;
                    (hero.x + u * hero.w, hero.y + 86.0 + k as f32 * 10.0 - (u * 5.0 + k as f32).sin() * 22.0 * u - (u * 11.0).sin() * 3.0)
                })
                .collect();
            g.polyline(&pts, 1.4, ACCENT.a(0.30 - k as f32 * 0.12));
        }
        g.pop_layer();
        g.stroke_round(hero.x + 0.5, hero.y + 0.5, hero.w - 1.0, hero.h - 1.0, 20.0, 1.0, ACCENT.a(0.28));
        if let Some(b) = &self.logo {
            g.draw_bitmap(b, hero.x + hero.w - 150.0, hero.y + 16.0, 100.0, 100.0, 0.95);
        }
        let (htitle, hsub) = if titles.is_empty() { (t.name.to_string(), t.sub) } else { (titles[sel_eff].clone(), section_sub(&titles[sel_eff])) };
        g.icon(section_icon(&htitle), hero.x + 40.0, hero.y + 46.0, 17.0, ACCENT);
        g.text(&g.fit(&htitle, 26.0, 700, 440.0), hero.x + 64.0, hero.y + 28.0 + (1.0 - self.tab_t) * 6.0, 450.0, 36.0, 26.0, 700, TEXT.a(self.tab_t), Align::Left);
        g.text(if hsub.is_empty() { t.sub } else { hsub }, hero.x + 40.0, hero.y + 78.0, 460.0, 22.0, 13.0, 500, DIM, Align::Left);

        // hover label for the rail icons
        if let Some((name, ty)) = tip {
            let w = g.measure(name, 12.0, 600) + 24.0;
            g.fill_round(RAIL_W + 6.0, ty + 5.0, w, 28.0, 10.0, Color::rgba(34, 34, 40, 0.98));
            g.stroke_round(RAIL_W + 6.5, ty + 5.5, w - 1.0, 27.0, 10.0, 1.0, Color::rgba(255, 255, 255, 0.14));
            g.text(name, RAIL_W + 6.0, ty + 5.0, w, 28.0, 12.0, 600, TEXT, Align::Center);
        }

        // scrolling body
        let top = 176.0;
        let bottom = H;
        g.push_clip(MAIN_X - 14.0, top, MAIN_W + 28.0, bottom - top);
        let mut ui = Ui {
            g,
            x: MAIN_X,
            w: MAIN_W,
            y: top + 4.0 - self.scroll + (1.0 - self.tab_t) * 18.0,
            top,
            bottom,
            mouse: self.mouse,
            click: self.click,
            changed: false,
            card_top: 0.0,
            row: 0,
            anim: &self.anim,
            dt,
            moving: &self.moving,
            sel: sel_eff,
            sec: 0,
            sup: false,
            sup_y: 0.0,
            titles: &self.titles,
        };
        let start_y = ui.y;
        let side = &mut self.side;
        let psel = &mut self.pray_sel;
        let search = &mut self.search;
        let pview = &mut self.pray_view;
        page(self.tab, &mut ui, a, side, psel, search, pview);
        ui.finish();
        let used = ui.y - start_y;
        let (changed, click_left) = (ui.changed, ui.click);
        if click_left.is_some() {
            FOCUS.store(0, AOrd::Relaxed);
        }
        g.pop_clip();
        self.content_h = used + top + 20.0;
        self.click = None;
        let _ = click_left;
        let _ = self.gfx.end_overlay();
        if changed {
            a.apply_settings();
            unsafe {
                let _ = InvalidateRect(self.hwnd, None, false);
            }
        }
    }
}

impl Panel {
    /// CPU % and RAM of this process, refreshed about once a second.
    fn sample_sys(&mut self) -> (f32, f32) {
        use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        unsafe {
            let h = GetCurrentProcess();
            let (mut c, mut e, mut k, mut us) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let ft = |f: &FILETIME| (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 10_000.0;
            let _ = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut us);
            let ms = ft(&k) + ft(&us);
            let el = self.cpu_at.elapsed();
            if el >= Duration::from_millis(1000) {
                let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
                self.cpu_pct = (((ms - self.cpu_ms) / (el.as_secs_f64() * 1000.0 * cores)) * 100.0).max(0.0) as f32;
                self.cpu_ms = ms;
                self.cpu_at = Instant::now();
            }
            let mut pc = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            let _ = GetProcessMemoryInfo(h, &mut pc, pc.cb);
            (self.cpu_pct, pc.WorkingSetSize as f32 / 1_048_576.0)
        }
    }

    /// Right-hand column: live preview, quick actions, system info. Returns true when settings changed.
    fn draw_side(&mut self, a: &mut crate::app::App) -> bool {
        let (cpu, ram) = self.sample_sys();
        let mut changed = false;
        let x = W - 28.0 - SIDE_W;
        let w = SIDE_W;
        let g = &self.gfx;
        let head = |y: f32, icon: char, title: &str| {
            g.icon(icon, x + 28.0, y + 24.0, 15.0, ACCENT);
            g.text(title, x + 48.0, y + 11.0, w - 60.0, 26.0, 15.0, 700, TEXT, Align::Left);
        };

        // ---- preview
        let (py, ph) = (100.0, 214.0);
        glass_box(g, x, py, w, ph);
        head(py, '\u{E7B3}', "Preview");
        let inner = Rect { x: x + 14.0, y: py + 48.0, w: w - 28.0, h: ph - 62.0 };
        g.push_round_clip(inner.x, inner.y, inner.w, inner.h, 16.0);
        g.fill_rect(inner.x, inner.y, inner.w, inner.h, Color::rgb(18, 19, 24));
        g.vgradient(inner.x, inner.y + inner.h * 0.35, inner.w, inner.h * 0.65, ACCENT.a(0.0), ACCENT.a(0.30));
        let t = self.cpu_at.elapsed().as_secs_f32() * 0.0 + Instant::now().duration_since(self.last).as_secs_f32() * 0.0;
        let _ = t;
        let mut wave: Vec<(f32, f32)> = (0..=40).map(|i| {
            let u = i as f32 / 40.0;
            (inner.x + u * inner.w, inner.y + inner.h * 0.72 - (u * 5.2).sin() * 11.0 - (u * 2.1 + 0.6).sin() * 9.0)
        }).collect();
        let line = wave.clone();
        wave.push((inner.x + inner.w, inner.y + inner.h));
        wave.push((inner.x, inner.y + inner.h));
        g.poly(&wave, ACCENT.a(0.16), 0.0);
        g.polyline(&line, 1.6, ACCENT.a(0.85));
        // a miniature of the pill
        let pill = Rect { x: inner.x + 18.0, y: inner.y + 18.0, w: inner.w - 36.0, h: 46.0 };
        g.fill_round(pill.x, pill.y, pill.w, pill.h, 23.0, BLACK);
        g.stroke_round(pill.x + 0.5, pill.y + 0.5, pill.w - 1.0, pill.h - 1.0, 23.0, 1.0, Color::rgba(255, 255, 255, 0.14));
        g.fill_circle(pill.x + 25.0, pill.y + 23.0, 15.0, WHITE.a(0.08));
        g.icon('\u{E8D6}', pill.x + 25.0, pill.y + 23.0, 14.0, ACCENT);
        let m = a.media.snapshot();
        let (l1, l2) = if m.has_session && !m.title.is_empty() { ("Now Playing".to_string(), g.fit(&m.title, 10.0, 500, pill.w - 120.0)) } else { ("Nook".to_string(), "Nothing playing".to_string()) };
        g.text(&l1, pill.x + 50.0, pill.y + 8.0, pill.w - 110.0, 18.0, 12.0, 700, TEXT, Align::Left);
        g.text(&l2, pill.x + 50.0, pill.y + 26.0, pill.w - 110.0, 16.0, 10.0, 500, DIM, Align::Left);
        for (i, hh) in [8.0f32, 14.0, 10.0, 16.0, 7.0].iter().enumerate() {
            g.fill_round(pill.x + pill.w - 34.0 + i as f32 * 4.5, pill.y + 23.0 - hh / 2.0, 2.2, *hh, 1.1, ACCENT);
        }
        g.pop_layer();

        // ---- quick actions
        let (qy, qh) = (py + ph + 16.0, 190.0);
        glass_box(g, x, qy, w, qh);
        head(qy, '\u{E945}', "Quick Actions");
        let confirm = self.confirm_reset.map(|t| t.elapsed() < Duration::from_secs(4)).unwrap_or(false);
        let items = [('\u{E72C}', "Restart Nook".to_string(), false), ('\u{E8B7}', "Open data folder".to_string(), false), ('\u{E777}', if confirm { "Click again to reset".to_string() } else { "Reset settings".to_string() }, confirm)];
        let mut do_restart = false;
        let mut do_open = false;
        let mut do_reset = false;
        for (i, (ic, label, warn)) in items.iter().enumerate() {
            let r = Rect { x: x + 14.0, y: qy + 46.0 + i as f32 * 46.0, w: w - 28.0, h: 40.0 };
            let hot = r.contains(self.mouse.0, self.mouse.1);
            g.fill_round(r.x, r.y, r.w, r.h, 12.0, if hot { CARD_HI } else { WELL });
            g.stroke_round(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 1.0, 12.0, 1.0, if *warn { Color::rgba(255, 99, 88, 0.7) } else { Color::rgba(255, 255, 255, 0.08) });
            g.icon(*ic, r.x + 22.0, r.y + 20.0, 14.0, if *warn { Color::rgb(255, 99, 88) } else { TEXT });
            g.text(label, r.x + 44.0, r.y, r.w - 50.0, r.h, 13.0, 600, if *warn { Color::rgb(255, 130, 118) } else { TEXT }, Align::Left);
            if let Some(c) = self.click {
                if r.contains(c.0, c.1) {
                    self.click = None;
                    match i {
                        0 => do_restart = true,
                        1 => do_open = true,
                        _ => do_reset = true,
                    }
                }
            }
        }

        // ---- system
        let (sy, sh) = (qy + qh + 16.0, 168.0);
        glass_box(g, x, sy, w, sh);
        head(sy, '\u{E9D9}', "System");
        let rows = [("Status", "Running".to_string()), ("CPU", format!("{cpu:.1}%")), ("RAM", format!("{ram:.1} MB")), ("Version", format!("v{}", crate::update::VERSION))];
        for (i, (k, v)) in rows.iter().enumerate() {
            let ry = sy + 48.0 + i as f32 * 28.0;
            if i > 0 {
                g.fill_rect(x + 18.0, ry - 2.0, w - 36.0, 1.0, LINE);
            }
            g.text(k, x + 20.0, ry, 100.0, 26.0, 12.5, 600, TEXT, Align::Left);
            if i == 0 {
                let tw = g.measure(v, 11.5, 700) + 22.0;
                g.fill_round(x + w - 20.0 - tw, ry + 3.0, tw, 20.0, 10.0, Color::rgba(48, 209, 88, 0.18));
                g.text(v, x + w - 20.0 - tw, ry + 3.0, tw, 20.0, 11.5, 700, Color::rgb(80, 220, 110), Align::Center);
            } else {
                g.text(v, x + 120.0, ry, w - 140.0, 26.0, 12.5, 500, DIM, Align::Right);
            }
        }

        if do_open {
            sys::open_path(&config::dir());
        }
        if do_reset {
            if confirm {
                let keep = a.settings.tour_done;
                a.settings = Settings::default();
                a.settings.tour_done = keep;
                self.confirm_reset = None;
                changed = true;
            } else {
                self.confirm_reset = Some(Instant::now());
                self.moving.set(true);
            }
        }
        if confirm {
            self.moving.set(true); // keep repainting until the confirmation expires
        }
        if do_restart {
            use std::os::windows::process::CommandExt;
            if let Ok(exe) = std::env::current_exe() {
                if std::process::Command::new(exe).arg("--restarted").creation_flags(0x0800_0208).spawn().is_ok() {
                    a.remove_tray();
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
            }
        }
        changed
    }
}

fn pct_opts() -> [&'static str; 5] {
    ["Auto", "100%", "115%", "130%", "150%"]
}

fn page(tab: usize, ui: &mut Ui, a: &mut crate::app::App, side: &mut usize, psel: &mut usize, search: &mut String, pview: &mut usize) {
    let mut redo_tasks = false;
    let mut act_update = 0u8; // 1 = check now, 2 = install now
    {
        let st: &mut Settings = &mut a.settings;
        match tab {
            0 => {
                ui.section("Startup");
                ui.card(1);
                ui.toggle("Start with Windows", "Launch Nook quietly when you sign in", &mut st.autostart);
                ui.section("Updates");
                ui.card(2);
                ui.toggle("Update automatically", "New versions download in the background; Nook restarts itself when idle", &mut st.auto_update);
                let us = a.updater.snapshot();
                let (sub, btn) = match &us {
                    crate::update::UpdateState::Idle => ("Checks for new versions every 20 minutes".to_string(), "Check now"),
                    crate::update::UpdateState::Checking => ("Checking…".to_string(), "Check now"),
                    crate::update::UpdateState::UpToDate => ("You have the latest version".to_string(), "Check now"),
                    crate::update::UpdateState::Downloading(v) => (format!("Downloading v{v}…"), "Check now"),
                    crate::update::UpdateState::Ready { version, .. } => (format!("v{version} is ready"), "Restart & update"),
                    crate::update::UpdateState::Failed(e) => (format!("Last check failed: {e}"), "Try again"),
                };
                if matches!(us, crate::update::UpdateState::Checking | crate::update::UpdateState::Downloading(_)) {
                    ui.moving.set(true);
                }
                if ui.button(&format!("Nook {}", crate::update::VERSION), &sub, btn, false) {
                    act_update = if matches!(us, crate::update::UpdateState::Ready { .. }) { 2 } else { 1 };
                }
                ui.section("Notch");
                ui.card(3);
                let cur = match st.notch_scale { 100 => 1, 115 => 2, 130 => 3, 150 => 4, _ => 0 };
                if let Some(i) = ui.seg("Size", "Nook restarts the overlay to apply a new size", &pct_opts(), cur) {
                    st.notch_scale = [0, 100, 115, 130, 150][i];
                }
                ui.toggle("Minimal notch", "A smaller pill when nothing is playing", &mut st.minimal);
                ui.toggle("Mascot", "A little friend that lives in the notch", &mut st.mascot);
                ui.section("Sounds");
                ui.card(1);
                ui.toggle("Reminder sounds", "Chimes for Focus, To-do, Water and Prayer", &mut st.sounds);
                ui.section("Full screen");
                ui.card(1);
                ui.toggle("Hide in full screen", "Only while a game, video or slideshow fills the whole screen", &mut st.hide_fullscreen);
                ui.section("Performance");
                ui.card(3);
                let (cpu, ram) = *SYSINFO.lock().unwrap();
                ui.info("Status", "Running");
                ui.info("CPU", &format!("{cpu:.1}%"));
                ui.info("RAM", &format!("{ram:.1} MB"));
                let confirm = RESET_AT.lock().unwrap().map(|t| t.elapsed() < Duration::from_secs(4)).unwrap_or(false);
                if confirm {
                    ui.moving.set(true);
                }
                ui.card(3);
                if ui.button("Restart Nook", "Closes and starts Nook again", "Restart", false) {
                    act_update = 5;
                }
                if ui.button("Open data folder", "Settings, tasks and caches live here", "Open", false) {
                    sys::open_path(&config::dir());
                }
                if ui.button("Reset settings", if confirm { "Click again to confirm" } else { "Back to defaults (your data stays)" }, if confirm { "Confirm" } else { "Reset" }, true) {
                    act_update = 6;
                }
            }
            1 => {
                ui.section("Notch style");
                ui.card(1);
                let cur = if st.island { 2 } else if st.floating { 1 } else { 0 };
                if let Some(i) = ui.seg("Style", "Notch, floating pill, or an iOS-style Dynamic Island that splits and morphs", &["Notch", "Floating", "Island"], cur) {
                    st.floating = i >= 1;
                    st.island = i == 2;
                }
                ui.card(2);
                ui.toggle("Glass", "A translucent body instead of solid black", &mut st.glass);
                let t = format!("{}%", st.glass_tint);
                match ui.stepper("Glass tint", "How opaque the glass is", &t, 56.0) {
                    -1 => st.glass_tint = st.glass_tint.saturating_sub(5).max(10),
                    1 => st.glass_tint = (st.glass_tint + 5).min(100),
                    _ => {}
                }
                ui.section("Theme");
                ui.card(1);
                if let Some(i) = ui.seg("Glow colour", "Tints the lyrics view and the lyric bar", &["Artwork", "Violet", "Blue", "Pink", "Green", "Off"], st.theme as usize) {
                    st.theme = i as u32;
                }
                ui.section("Behaviour");
                ui.card(6);
                let cur = st.open_on_click as usize;
                if let Some(i) = ui.seg("Open the notch", "", &["Hover", "Click"], cur) {
                    st.open_on_click = i == 1;
                }
                let cur = st.dot_mode as usize;
                if let Some(i) = ui.seg("Closed island", "A full pill, or just a tiny dot that opens on hover / click", &["Pill", "Dot"], cur) {
                    st.dot_mode = i == 1;
                }
                let cur = (st.circle_mode as usize).min(7);
                if let Some(i) = ui.seg("Island circle", "What the circle beside the music pill shows", &["Auto", "Battery", "Volume", "Timer", "Prayer", "Water", "Light", "Off"], cur) {
                    st.circle_mode = i as u32;
                }
                ui.toggle("Idle scenes", "When nothing plays, the pill rotates date, next prayer, water, tasks and battery", &mut st.idle_scenes);
                ui.toggle("Page arrows", "Left and right arrows at the sides of the panel", &mut st.page_icons);
                let cur = st.fast_anim as usize;
                if let Some(i) = ui.seg("Animation speed", "", &["Normal", "Fast"], cur) {
                    st.fast_anim = i == 1;
                }
                ui.section("Status ring");
                ui.card(1);
                ui.toggle("Battery ring", "A ring next to the clock that shows charge", &mut st.status_ring);
            }
            2 => {
                ui.section("Notch screen");
                let n = monitor_count();
                let mut labels: Vec<String> = vec!["Main".into()];
                for i in 1..=n {
                    labels.push(format!("Display {i}"));
                }
                let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                ui.card(1);
                if let Some(i) = ui.seg("Display", if n > 1 { "Choose where the notch appears" } else { "Connect another display to choose" }, &refs, st.monitor as usize) {
                    st.monitor = i as u32;
                }
            }
            3 => {
                ui.section("Lyrics");
                ui.card(2);
                ui.toggle("Show lyrics", "Synced lyrics from LRCLIB and NetEase, with a plain-text fallback", &mut st.lyrics);
                ui.toggle("Lyrics in the notch", "Show the current line in the small bar while music plays", &mut st.lyric_bar);
                ui.note("Lyrics are looked up by track title and artist and cached only while the song plays.");
            }
            4 => {
                ui.section("Widgets page");
                ui.card(1);
                ui.toggle("Show Widgets page", "Two widgets of your choice, one swipe away", &mut st.widgets_page);
                ui.section("Choose widgets");
                ui.card(1);
                if let Some(i) = ui.seg("Editing", "", &["Left", "Right"], *side) {
                    *side = i;
                }
                // widget chips
                ui.card_h(96.0);
                let (cx, cy) = (ui.x + 16.0, ui.card_top + 14.0);
                let cur = if *side == 0 { st.widget_left } else { st.widget_right };
                let cw = (ui.w - 32.0 - 3.0 * 10.0) / 4.0;
                for (i, name) in crate::pages::WIDGET_NAMES.iter().enumerate() {
                    let r = Rect { x: cx + (i % 4) as f32 * (cw + 10.0), y: cy + (i / 4) as f32 * 40.0, w: cw, h: 32.0 };
                    if ui.clicked(r) {
                        if *side == 0 { st.widget_left = i as u32 } else { st.widget_right = i as u32 }
                        ui.changed = true;
                    }
                    let sel = cur == i as u32;
                    ui.g.fill_round(r.x, r.y, r.w, r.h, 12.0, if sel { ACCENT } else if ui.hot(r) { CARD_HI } else { WELL });
                    ui.g.text(name, r.x, r.y, r.w, r.h, 12.0, 600, if sel { Color::rgb(30, 20, 0) } else { TEXT }, Align::Center);
                }
                ui.section("Options");
                ui.card(3);
                let n = crate::pages::WORLD.len() as u32;
                let t = crate::pages::WORLD[st.clock_a as usize % n as usize].0;
                match ui.stepper("World clock, first city", "", t, 120.0) {
                    -1 => st.clock_a = (st.clock_a + n - 1) % n,
                    1 => st.clock_a = (st.clock_a + 1) % n,
                    _ => {}
                }
                let t = crate::pages::WORLD[st.clock_b as usize % n as usize].0;
                match ui.stepper("World clock, second city", "", t, 120.0) {
                    -1 => st.clock_b = (st.clock_b + n - 1) % n,
                    1 => st.clock_b = (st.clock_b + 1) % n,
                    _ => {}
                }
                let t = crate::pages::COUNTDOWNS[st.countdown as usize % 4];
                match ui.stepper("Countdown to", "Islamic dates are approximate", t, 120.0) {
                    -1 => st.countdown = (st.countdown + 3) % 4,
                    1 => st.countdown = (st.countdown + 1) % 4,
                    _ => {}
                }
            }
            5 => {
                ui.section("Reminders");
                ui.card(3);
                ui.toggle("When a task starts", "", &mut st.todo_start);
                ui.toggle("When time is up", "", &mut st.todo_end);
                let cur = match st.snooze { 5 => 0, 15 => 2, _ => 1 };
                if let Some(i) = ui.seg("Snooze for", "", &["5 min", "10 min", "15 min"], cur) {
                    st.snooze = [5, 10, 15][i];
                }
                ui.section("Tasks");
                let n = a.store.todos.len();
                ui.card(n.max(1) + 1);
                if n == 0 {
                    ui.next_row("No tasks yet", "");
                }
                let mut del = None;
                for (i, t) in a.store.todos.iter().enumerate() {
                    let sub = t.time.map(|m| format!("{:02}:{:02}", m / 60, m % 60)).unwrap_or_default();
                    let title = if t.done { format!("{}  (done)", t.title) } else { t.title.clone() };
                    if ui.button(&title, &sub, "Delete", true) {
                        del = Some(i);
                    }
                }
                if let Some(i) = del {
                    a.store.todos.remove(i);
                    redo_tasks = true;
                }
                if ui.button("Add a task", "", "Add…", false) {
                    open_task_dialog();
                }
            }
            6 => {
                ui.section("Options");
                ui.card(2);
                let cur = st.week_monday as usize;
                if let Some(i) = ui.seg("Week starts on", "", &["Sunday", "Monday"], cur) {
                    st.week_monday = i == 1;
                }
                let cur = st.cal_week_view as usize;
                if let Some(i) = ui.seg("Default view", "", &["Month", "Week"], cur) {
                    st.cal_week_view = i == 1;
                }
            }
            7 => {
                ui.section("Durations");
                ui.card(3);
                let t = format!("{} min", st.pomo_focus);
                match ui.stepper("Focus", "", &t, 64.0) {
                    -1 => st.pomo_focus = st.pomo_focus.saturating_sub(5).max(5),
                    1 => st.pomo_focus = (st.pomo_focus + 5).min(90),
                    _ => {}
                }
                let t = format!("{} min", st.pomo_short);
                match ui.stepper("Short break", "", &t, 64.0) {
                    -1 => st.pomo_short = st.pomo_short.saturating_sub(1).max(1),
                    1 => st.pomo_short = (st.pomo_short + 1).min(30),
                    _ => {}
                }
                let t = format!("{} min", st.pomo_long);
                match ui.stepper("Long break", "Every fourth break", &t, 64.0) {
                    -1 => st.pomo_long = st.pomo_long.saturating_sub(5).max(5),
                    1 => st.pomo_long = (st.pomo_long + 5).min(60),
                    _ => {}
                }
            }
            8 => {
                ui.section("Reminders");
                ui.card(1);
                ui.toggle("Water reminders", "Spread across your active hours", &mut st.water);
                ui.section("Plan");
                ui.card(4);
                let cur = match st.water_goal { 2000 => 1, 2500 => 2, 3000 => 3, _ => 0 };
                if let Some(i) = ui.seg("Daily goal", "", &["1.5 L", "2 L", "2.5 L", "3 L"], cur) {
                    st.water_goal = [1500, 2000, 2500, 3000][i];
                }
                let t = format!("{} ml", st.bottle);
                match ui.stepper("Bottle size", "How much one tap of Drink adds", &t, 70.0) {
                    -1 => st.bottle = st.bottle.saturating_sub(50).max(100),
                    1 => st.bottle = (st.bottle + 50).min(1500),
                    _ => {}
                }
                let t = format!("{:02}:{:02}", st.water_from / 60, st.water_from % 60);
                match ui.stepper("Active from", "", &t, 64.0) {
                    -1 => st.water_from = st.water_from.saturating_sub(30),
                    1 => st.water_from = (st.water_from + 30).min(st.water_to.saturating_sub(60)),
                    _ => {}
                }
                let t = format!("{:02}:{:02}", st.water_to / 60, st.water_to % 60);
                match ui.stepper("Active until", "", &t, 64.0) {
                    -1 => st.water_to = st.water_to.saturating_sub(30).max(st.water_from + 60),
                    1 => st.water_to = (st.water_to + 30).min(1410),
                    _ => {}
                }
                ui.section("Today");
                ui.card(1);
                if ui.button("Today's progress", &format!("{} ml so far", a.store.water_taken), "Reset", true) {
                    a.store.water_taken = 0;
                    a.store.water_hours = [0; 24];
                    a.store.save_water();
                }
            }
            9 => {
                ui.section("Prayer times");
                ui.card(1);
                ui.toggle("Show Prayer page", &format!("Method: {}", prayer::method_name_s(st)), &mut st.prayer);
                let cl = prayer::countries();
                let cur = prayer::current_country(st);
                let viewing = if *pview < cl.len() { *pview } else { cur.unwrap_or(0) };
                ui.section("Country");
                let items: Vec<(String, bool)> = cl.iter().enumerate().map(|(i, c)| (c.0.to_string(), i == viewing)).collect();
                if let Some(i) = ui.chips(&items) {
                    *pview = i;
                }
                let viewing = if *pview < cl.len() { *pview } else { cur.unwrap_or(0) };
                ui.section(&format!("Cities in {}", cl[viewing].0));
                let cities = prayer::cities_of(cl[viewing].2);
                let items: Vec<(String, bool)> = cities.iter().map(|c| (c.name.clone(), prayer::is_selected(st, &c.target))).collect();
                if let Some(i) = ui.chips(&items) {
                    prayer::apply_target(st, &cities[i].target);
                    ui.changed = true;
                }
                ui.note(&format!("Now showing {} · {}", prayer::place_label(st), prayer::method_name_s(st)));
                // type-to-search finder: built-in lists first, then any city in the world
                ui.section("Find a city anywhere");
                prayer::online_request(search);
                let online = prayer::online_results(search);
                let pending = prayer::online_pending(search);
                if pending {
                    ui.moving.set(true);
                }
                let mut items: Vec<(String, String, Option<(usize, usize)>, Option<prayer::Geo>)> = vec![];
                for (pi, ci) in prayer::search(search, 4) {
                    items.push((prayer::PROVINCES[pi].cities[ci].name.to_string(), format!("{} · {}", prayer::PROVINCES[pi].name, prayer::COUNTRIES[prayer::country_of(pi)]), Some((pi, ci)), None));
                }
                for gz in online.iter().take(6) {
                    let sub = [gz.admin.as_str(), gz.country.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
                    items.push((gz.name.clone(), sub, None, Some(gz.clone())));
                }
                let note_h = if items.is_empty() && search.chars().count() >= 3 { 30.0 } else { 0.0 };
                let hgt = 68.0 + if items.is_empty() { note_h } else { items.len() as f32 * 34.0 + 6.0 };
                ui.card_h(hgt);
                let (cx0, cy0, cw) = (ui.x, ui.card_top, ui.w);
                let g = ui.g;
                let fr = Rect { x: cx0 + 14.0, y: cy0 + 14.0, w: cw - 28.0, h: 40.0 };
                g.fill_round(fr.x, fr.y, fr.w, fr.h, 12.0, WELL);
                g.stroke_round(fr.x + 0.5, fr.y + 0.5, fr.w - 1.0, fr.h - 1.0, 12.0, 1.2, if search.is_empty() { Color::rgba(255, 255, 255, 0.10) } else { ACCENT });
                g.icon('\u{E721}', fr.x + 20.0, fr.y + 20.0, 13.0, DIM);
                if search.is_empty() {
                    g.text("Type any city in the world, for example Paris, Cairo, Toronto…", fr.x + 40.0, fr.y, fr.w - 50.0, fr.h, 13.0, 500, DIM, Align::Left);
                } else {
                    let shown = g.fit(search, 13.5, 500, fr.w - 70.0);
                    g.text(&shown, fr.x + 40.0, fr.y, fr.w - 60.0, fr.h, 13.5, 500, TEXT, Align::Left);
                    g.fill_rect(fr.x + 42.0 + g.measure(&shown, 13.5, 500), fr.y + 10.0, 1.5, 20.0, ACCENT);
                }
                if items.is_empty() && search.chars().count() >= 3 {
                    g.text(if pending { "Searching the world…" } else { "No matching city" }, cx0 + 20.0, cy0 + 62.0, cw - 40.0, 26.0, 12.5, 500, DIM, Align::Left);
                }
                for (i, (title, sub, local, geo)) in items.iter().enumerate() {
                    let r = Rect { x: cx0 + 8.0, y: cy0 + 62.0 + i as f32 * 34.0, w: cw - 16.0, h: 32.0 };
                    let sel = match (local, geo) {
                        (Some((pi, ci)), _) => !st.pray_custom && *pi == st.prayer_prov && *ci == st.prayer_city,
                        (_, Some(gz)) => st.pray_custom && st.pray_name == gz.label() && (st.pray_lat - gz.lat).abs() < 0.01,
                        _ => false,
                    };
                    if ui.hot(r) || sel {
                        g.fill_round(r.x, r.y, r.w, r.h, 10.0, if sel { Color::rgba(255, 159, 10, 0.16) } else { CARD_HI });
                    }
                    g.text(title, r.x + 14.0, r.y, 200.0, r.h, 13.5, 600, TEXT, Align::Left);
                    g.text(&g.fit(sub, 11.5, 500, r.w - 250.0), r.x + 190.0, r.y, r.w - 204.0, r.h, 11.5, 500, DIM, Align::Right);
                    if ui.clicked(r) {
                        match (local, geo) {
                            (Some((pi, ci)), _) => {
                                st.prayer_prov = *pi;
                                st.prayer_city = *ci;
                                st.pray_custom = false;
                            }
                            (_, Some(gz)) => {
                                st.pray_custom = true;
                                st.pray_name = gz.label();
                                st.pray_lat = gz.lat;
                                st.pray_lon = gz.lon;
                                st.pray_cc = gz.cc.clone();
                                st.pray_tz = String::new();
                                prayer::ensure(st);
                            }
                            _ => {}
                        }
                        search.clear();
                        ui.changed = true;
                    }
                }
                ui.section("Reminders");
                ui.card(4);
                ui.toggle("At prayer time", "", &mut st.pray_at);
                ui.toggle("10 minutes before", "", &mut st.pray_before);
                ui.toggle("Sound", "", &mut st.pray_sound);
                ui.toggle("Pause media at prayer time", "", &mut st.pray_pause);
                ui.section("Fine-tune");
                ui.card(2);
                match ui.stepper("Prayer", "", prayer::names_s(st)[*psel % 5], 100.0) {
                    -1 => *psel = (*psel + 4) % 5,
                    1 => *psel = (*psel + 1) % 5,
                    _ => {}
                }
                let adj = st.pray_adjust[*psel % 5];
                match ui.stepper("Shift", "Minutes added to the calculated time", &format!("{adj} min"), 80.0) {
                    -1 => st.pray_adjust[*psel % 5] = (adj - 1).max(-30),
                    1 => st.pray_adjust[*psel % 5] = (adj + 1).min(30),
                    _ => {}
                }
            }
            10 => {
                ui.section("Shelf");
                ui.card(1);
                ui.toggle("Start with an empty Shelf", "Files from your last session are cleared when Nook starts", &mut st.shelf_empty);
                ui.section("Clipboard history");
                ui.card(3);
                ui.toggle("Clipboard history", "Text you copy, ready to paste again in the Shelf", &mut st.clip_history);
                let t = format!("{} items", st.clip_max);
                match ui.stepper("Keep up to", "", &t, 70.0) {
                    -1 => st.clip_max = st.clip_max.saturating_sub(1).max(1),
                    1 => st.clip_max = (st.clip_max + 1).min(50),
                    _ => {}
                }
                ui.toggle("Remember after restart", "Saved only on this PC", &mut st.clip_persist);
            }
            12 => {
                ui.section("Assistant");
                ui.card(5);
                ui.toggle("Show Assistant page", "A voice and text AI page in the island's dock", &mut st.ai_enabled);
                ui.toggle("Speak answers aloud", "Uses installed Windows and cloud voices, no download needed", &mut st.ai_voice);
                ui.toggle("Natural AI voice", "Lifelike neural speech via free cloud or your API key; falls back to Windows voice", &mut st.ai_neural);
                let mut names = vec!["System default".to_string()];
                names.extend(crate::assistant::voices());

                let vi = names.iter().position(|n| *n == st.ai_voice_name).unwrap_or(0);
                let voice_sub = if names[vi].contains("Cloud") {
                    "Instant free neural voice, zero downloads"
                } else if names[vi].contains("Desktop") || names[vi].contains("Pablo") || names[vi].contains("Helena") || names[vi].contains("Laura") {
                    "Installed Windows speech engine"
                } else {
                    "Auto: Cloud voice when online, Windows offline"
                };

                match ui.stepper("Voice", voice_sub, &names[vi], 200.0) {
                    -1 => {
                        let next_i = (vi + names.len() - 1) % names.len();
                        st.ai_voice_name = if next_i == 0 { String::new() } else { names[next_i].clone() };
                    }
                    1 => {
                        let next_i = (vi + 1) % names.len();
                        st.ai_voice_name = if next_i == 0 { String::new() } else { names[next_i].clone() };
                    }
                    _ => {}
                }
                let rate = format!("{}%", st.ai_rate);
                match ui.stepper("Speaking speed", "", &rate, 80.0) {
                    -1 => st.ai_rate = st.ai_rate.saturating_sub(10).max(50),
                    1 => st.ai_rate = (st.ai_rate + 10).min(250),
                    _ => {}
                }
                if ui.button("Test voice", "Speaks a short sample", "Play", false) {
                    let cfg = crate::assistant::Cfg {
                        provider: st.ai_provider,
                        key: st.ai_key.clone(),
                        model: st.ai_model.clone(),
                        url: st.ai_url.clone(),
                        voice: st.ai_voice,
                        voice_name: st.ai_voice_name.clone(),
                        neural: st.ai_neural,
                        rate: st.ai_rate,
                        context: String::new(),
                    };
                    std::thread::spawn(move || {
                        unsafe {
                            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                        }
                        crate::assistant::speak_cfg("Hello, I'm Nook.", &cfg);
                    });
                }

                                ui.section("AI provider (free tiers & local)");
                let prov_items: Vec<(String, bool)> = crate::assistant::PROVIDERS.iter().enumerate().map(|(idx, name)| {
                    (name.to_string(), st.ai_provider as usize == idx)
                }).collect();
                if let Some(i) = ui.chips(&prov_items) {
                    if st.ai_provider != i as u32 {
                        st.ai_provider = i as u32;
                        st.ai_model.clear();
                        crate::assistant::force_refresh_models(st.ai_provider, &st.ai_key, &st.ai_url);
                    }
                }

                let (key_label, key_sub, key_hint, def_url, def_model) = match st.ai_provider {
                    0 => ("API key", "Free at console.groq.com (fast free tier)", "gsk_...", "https://api.groq.com/openai/v1", "llama-3.3-70b-versatile"),
                    1 => ("API key", "Free at aistudio.google.com (generous free tier)", "AIzaSy...", "https://generativelanguage.googleapis.com/v1beta", "gemini-2.0-flash"),
                    2 => ("API key", "Free at openrouter.ai/keys (access all free :free models)", "sk-or-v1-...", "https://openrouter.ai/api/v1", "meta-llama/llama-3.3-70b-instruct:free"),
                    3 => ("API key", "From platform.deepseek.com", "sk-...", "https://api.deepseek.com/v1", "deepseek-chat"),
                    4 => ("API key", "From console.mistral.ai", "...", "https://api.mistral.ai/v1", "mistral-small-latest"),
                    5 => ("API key", "From api.together.ai", "...", "https://api.together.xyz/v1", "meta-llama/Llama-3.3-70B-Instruct-Turbo"),
                    6 => ("API key (optional)", "Not needed for Ollama (runs locally on PC)", "(not needed)", "http://localhost:11434", "llama3.2"),
                    _ => ("API key (optional)", "Local LM Studio server (http://localhost:1234/v1)", "(not needed)", "http://localhost:1234/v1", "local-model"),
                };

                ui.section("Configuration");
                ui.card(3);
                ui.field(1, key_label, key_sub, &st.ai_key.clone(), true, key_hint);
                ui.field(2, "Model", "Pick from detected models below or type custom", &st.ai_model.clone(), false, def_model);
                ui.field(3, "Server URL", "API endpoint (leave empty for default)", &st.ai_url.clone(), false, def_url);
                ui.note("The key stays on this PC (%APPDATA%\\Nook\\settings.txt). Voice input uses Windows speech recognition.");

                let (models, is_fetching) = crate::assistant::get_cached_models(st.ai_provider, &st.ai_key, &st.ai_url);
                let detect_title = if is_fetching {
                    "Detected models (fetching from API…)"
                } else if st.ai_provider == 2 {
                    "Detected models (free :free models listed first)"
                } else {
                    "Detected models (click to select)"
                };
                ui.section(detect_title);
                ui.card(1);
                if ui.button("Auto-detect models", "Re-query provider endpoint with current key/url", "Detect Models", false) {
                    crate::assistant::force_refresh_models(st.ai_provider, &st.ai_key, &st.ai_url);
                }
                let model_chips: Vec<(String, bool)> = models.iter().take(18).map(|m| {
                    let sel = if st.ai_model.trim().is_empty() { m.id == def_model } else { m.id == st.ai_model.trim() };
                    (m.label.clone(), sel)
                }).collect();
                if !model_chips.is_empty() {
                    if let Some(i) = ui.chips(&model_chips) {
                        if let Some(picked) = models.get(i) {
                            st.ai_model = picked.id.clone();
                        }
                    }
                } else if !is_fetching {
                    ui.note("No models detected. Ensure your API key is set, then click Detect Models.");
                }

                ui.section("Things you can say");
                ui.card_h(130.0);
                let lines = ["“Open YouTube”  ·  “Search for pizza near me”", "“Volume up”  ·  “Mute”  ·  “Next song”  ·  “Pause”", "“What time is it?”  ·  “What day is it?”", "“When is the next prayer?”  ·  “Tell me a joke”"];
                for (i, l) in lines.iter().enumerate() {
                    ui.g.text(l, ui.x + 20.0, ui.card_top + 16.0 + i as f32 * 26.0, ui.w - 40.0, 22.0, 12.5, 500, DIM, Align::Left);
                }
            }
            11 => {
                ui.section("Indicators");
                ui.card(4);
                ui.toggle("Volume", "", &mut st.hud_volume);
                ui.toggle("Brightness", "Laptop screens only", &mut st.hud_brightness);
                ui.toggle("Caps Lock", "", &mut st.hud_caps);
                ui.toggle("Input language", "", &mut st.hud_lang);
                ui.section("Popups");
                ui.card(3);
                ui.toggle("Battery", "", &mut st.pop_battery);
                ui.toggle("Wi-Fi", "", &mut st.pop_wifi);
                ui.toggle("Bluetooth headset", "", &mut st.pop_bt);
            }
            _ => {
                ui.card_h(120.0);
                ui.g.fill_round(ui.x + 24.0, ui.card_top + 24.0, 72.0, 72.0, 20.0, ACCENT);
                ui.g.icon('\u{E7F4}', ui.x + 60.0, ui.card_top + 60.0, 26.0, Color::rgb(30, 20, 0));
                ui.g.text("Nook", ui.x + 116.0, ui.card_top + 22.0, 300.0, 34.0, 22.0, 700, TEXT, Align::Left);
                ui.g.text(&format!("Version {} · a living notch for Windows", crate::update::VERSION), ui.x + 116.0, ui.card_top + 54.0, 400.0, 22.0, 12.5, 500, DIM, Align::Left);
                ui.g.text("Written in Rust. Lyrics from LRCLIB and NetEase.", ui.x + 116.0, ui.card_top + 76.0, 400.0, 22.0, 12.0, 500, DIM, Align::Left);
                ui.section("Info");
                ui.card(4);
                if ui.button("Tutorial", "Walk through every part of Nook again", "Replay", false) {
                    act_update = 3;
                }
                if ui.button("What's new", "The changes in this version", "Show", false) {
                    act_update = 4;
                }
                if ui.button("Lyrics provider", "lrclib.net", "Open", false) {
                    sys::open_url("https://lrclib.net");
                }
                if ui.button("Your data", "Settings, tasks and water log live here", "Show folder", false) {
                    sys::open_path(&config::dir());
                }
            }
        }
    }
    if redo_tasks {
        a.store.save_todos();
    }
    match act_update {
        1 => a.updater.check_now(),
        2 => a.install_update(true),
        3 => a.start_tour(),
        4 => a.whats_new = true,
        5 => {
            use std::os::windows::process::CommandExt;
            if let Ok(exe) = std::env::current_exe() {
                if std::process::Command::new(exe).arg("--restarted").creation_flags(0x0800_0208).spawn().is_ok() {
                    a.remove_tray();
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
            }
        }
        6 => {
            let confirm = RESET_AT.lock().unwrap().map(|t| t.elapsed() < Duration::from_secs(4)).unwrap_or(false);
            if confirm {
                *RESET_AT.lock().unwrap() = None;
                let (done, seen) = (a.settings.tour_done, a.settings.last_seen_version.clone());
                a.settings = Settings::default();
                a.settings.tour_done = done;
                a.settings.last_seen_version = seen;
                a.apply_settings();
            } else {
                *RESET_AT.lock().unwrap() = Some(Instant::now());
            }
        }
        _ => {}
    }
}

// --------------------------------------------------------------- task dialog
pub struct TaskPanel {
    hwnd: HWND,
    gfx: Gfx,
    scale: f32,
    title: String,
    has_time: bool,
    minutes: u32,
    mouse: (f32, f32),
    click: Option<(f32, f32)>,
    ticks: u32,
    edit: Option<usize>,
    anim: RefCell<HashMap<u64, f32>>,
    moving: Cell<bool>,
    last: Instant,
}

const TW: f32 = 420.0;
const TH: f32 = 300.0;

pub fn open_task_dialog() {
    open_task_inner(None);
}

pub fn open_task_edit(i: usize) {
    open_task_inner(Some(i));
}

fn open_task_inner(edit: Option<usize>) {
    unsafe {
        if !TASKP.is_null() {
            let _ = SetForegroundWindow((*TASKP).hwnd);
            return;
        }
        let hinst = GetModuleHandleW(None).unwrap();
        let icon = LoadIconW(hinst, PCWSTR(1 as _)).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        let class = w!("NookTask");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(task_proc),
            hInstance: hinst.into(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let s = app().map(|a| a.scale).unwrap_or(1.0).clamp(1.0, 2.0);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let mut r = RECT { left: 0, top: 0, right: (TW * s) as i32, bottom: (TH * s) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, WS_EX_TOPMOST);
        let (ow, oh) = (r.right - r.left, r.bottom - r.top);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP, class, w!("Add Task"), style, (sw - ow) / 2, (sh - oh) / 3, ow, oh, None, None, hinst, None) else { return };
        let s = dpi_scale(hwnd);
        let mut r = RECT { left: 0, top: 0, right: (TW * s) as i32, bottom: (TH * s) as i32 };
        let _ = AdjustWindowRectEx(&mut r, style, false, WS_EX_TOPMOST);
        let _ = SetWindowPos(hwnd, None, 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER);
        dark_frame(hwnd);
        let mut cr = RECT::default();
        let _ = GetClientRect(hwnd, &mut cr);
        let Ok(gfx) = Gfx::new_overlay(hwnd, (cr.right - cr.left) as u32, (cr.bottom - cr.top) as u32, 96.0 * s) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let n = sys::now();
        TASKP = Box::into_raw(Box::new(TaskPanel { hwnd, gfx, scale: s, title: edit.and_then(|i| app().and_then(|a| a.store.todos.get(i).map(|t| t.title.clone()))).unwrap_or_default(), has_time: edit.and_then(|i| app().and_then(|a| a.store.todos.get(i).map(|t| t.time.is_some()))).unwrap_or(false), minutes: edit.and_then(|i| app().and_then(|a| a.store.todos.get(i).and_then(|t| t.time))).unwrap_or((n.minutes() + 30) / 30 * 30 % 1440), mouse: (-1.0, -1.0), click: None, ticks: 0, edit, anim: RefCell::new(HashMap::new()), moving: Cell::new(false), last: Instant::now() }));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);
        SetTimer(hwnd, 1, 500, None);
    }
}

unsafe extern "system" fn task_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if TASKP.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let p = &mut *TASKP;
    let pt = |lp: LPARAM| -> (f32, f32) {
        (((lp.0 & 0xFFFF) as i16) as f32 / p.scale, (((lp.0 >> 16) & 0xFFFF) as i16) as f32 / p.scale)
    };
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            p.paint();
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            p.ticks += 1;
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            p.mouse = pt(lp);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            p.click = Some(pt(lp));
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CHAR => {
            let c = wp.0 as u32;
            match c {
                8 => {
                    p.title.pop();
                }
                13 => p.save(),
                27 => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                22 => {
                    if let Some(t) = crate::app::read_clipboard_text() {
                        p.title.push_str(&t.replace(['\r', '\n'], " "));
                    }
                }
                c if c >= 32 => {
                    if let Some(ch) = char::from_u32(c) {
                        if p.title.chars().count() < 80 {
                            p.title.push(ch);
                        }
                    }
                }
                _ => {}
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, 1);
            drop(Box::from_raw(TASKP));
            TASKP = std::ptr::null_mut();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

impl TaskPanel {
    fn save(&mut self) {
        let t = self.title.trim().to_string();
        if t.is_empty() {
            return;
        }
        if let Some(a) = app() {
            let time = if self.has_time { Some(self.minutes) } else { None };
            match self.edit.and_then(|i| a.store.todos.get_mut(i)) {
                Some(td) => {
                    td.title = t;
                    td.time = time;
                    td.fired_start = false;
                }
                None => a.store.todos.push(Todo { title: t, time, done: false, fired_start: false, fired_end: false }),
            }
            a.store.save_todos();
            a.dirty.set(true);
        }
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn paint(&mut self) {
        if self.gfx.begin_overlay().is_err() {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let g = &self.gfx;
        glass_window(g, TW, TH);
        g.text(if self.edit.is_some() { "Edit task" } else { "New task" }, 28.0, 22.0, 300.0, 34.0, 20.0, 700, TEXT, Align::Left);
        g.text("What needs doing?", 28.0, 62.0, 300.0, 18.0, 11.5, 600, DIM, Align::Left);
        // text field
        let fr = Rect { x: 28.0, y: 84.0, w: TW - 56.0, h: 42.0 };
        g.fill_round(fr.x, fr.y, fr.w, fr.h, 12.0, CARD);
        g.stroke_round(fr.x + 0.5, fr.y + 0.5, fr.w - 1.0, fr.h - 1.0, 12.0, 1.5, ACCENT);
        let shown = g.fit(&self.title, 14.0, 500, fr.w - 32.0);
        if self.title.is_empty() {
            g.text("Task title", fr.x + 14.0, fr.y, fr.w, fr.h, 14.0, 500, DIM, Align::Left);
        } else {
            g.text(&shown, fr.x + 14.0, fr.y, fr.w - 28.0, fr.h, 14.0, 500, TEXT, Align::Left);
        }
        if self.ticks % 2 == 0 {
            let cx = fr.x + 14.0 + if self.title.is_empty() { 0.0 } else { g.measure(&shown, 14.0, 500) };
            g.fill_rect(cx + 1.0, fr.y + 11.0, 1.5, 20.0, ACCENT);
        }
        let no_titles = RefCell::new(vec![]);
        let mut ui = Ui { g, x: 28.0, w: TW - 56.0, y: 148.0, top: 0.0, bottom: TH, mouse: self.mouse, click: self.click, changed: false, card_top: 0.0, row: 0, anim: &self.anim, dt, moving: &self.moving, sel: usize::MAX, sec: 0, sup: false, sup_y: 0.0, titles: &no_titles };
        ui.card(2);
        ui.toggle("Remind me at a time", "", &mut self.has_time);
        let t = format!("{:02}:{:02}", self.minutes / 60, self.minutes % 60);
        match ui.stepper("Time", "", &t, 64.0) {
            -1 => self.minutes = (self.minutes + 1440 - 15) % 1440,
            1 => self.minutes = (self.minutes + 15) % 1440,
            _ => {}
        }
        // buttons
        let (cancel, save) = (Rect { x: TW - 28.0 - 190.0, y: TH - 56.0, w: 88.0, h: 36.0 }, Rect { x: TW - 28.0 - 92.0, y: TH - 56.0, w: 92.0, h: 36.0 });
        let mut do_save = false;
        let mut do_cancel = false;
        if ui.clicked(cancel) {
            do_cancel = true;
        }
        if ui.clicked(save) {
            do_save = true;
        }
        g.fill_round(cancel.x, cancel.y, cancel.w, cancel.h, 18.0, if cancel.contains(self.mouse.0, self.mouse.1) { CARD_HI } else { CARD });
        g.text("Cancel", cancel.x, cancel.y, cancel.w, cancel.h, 13.0, 600, TEXT, Align::Center);
        g.fill_round(save.x, save.y, save.w, save.h, 18.0, ACCENT);
        g.text("Save", save.x, save.y, save.w, save.h, 13.0, 700, Color::rgb(30, 20, 0), Align::Center);
        self.click = None;
        let _ = self.gfx.end_overlay();
        if do_cancel {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        } else if do_save {
            self.save();
        }
    }
}

#[allow(dead_code)]
fn unused() {
    let _ = VK_RETURN;
}
