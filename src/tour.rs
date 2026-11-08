//! First-run tutorial: dims the panel, frames one area at a time with a white rectangle and explains it
//! in a tooltip with Back / Next / Skip all. Runs once (`tour_done` in settings), replayable from
//! Settings -> About and the tray menu.
use std::time::Instant;

use crate::app::*;
use crate::gfx::{Align, Color};
use crate::spring::*;
use crate::sys;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TourKind {
    Welcome,
    Header,
    Dock,
    Page(PageKind),
    Lyrics,
    Settings,
    Done,
}

pub struct Tour {
    pub step: usize,
    pub steps: Vec<TourKind>,
    pub fade: Ease,
    pub text: Ease,
    pub rect: [Ease; 4],
    pub born: Instant,
}

fn copy(kind: TourKind) -> (&'static str, &'static str) {
    match kind {
        TourKind::Welcome => ("Welcome to Nook", "Nook lives at the top of your screen. Hover it to open it, move away to close it. This short tour shows what everything does."),
        TourKind::Header => ("Time, date and status", "The clock and date, plus a status ring that shows your battery and turns amber, then red, when it runs low. The little bars move while music plays."),
        TourKind::Dock => ("Your pages", "Every page has an icon here. Click one, scroll the mouse wheel over the panel, or use the arrows at the sides to switch pages."),
        TourKind::Page(PageKind::Media) => ("Music", "Whatever is playing, from any app or browser tab: artwork, seek bar, previous / play / next, and volume (scroll on the speaker). Click the artwork to switch between players."),
        TourKind::Lyrics => ("Synced lyrics", "Tap the speech bubble to show lyrics next to the player. They are found automatically for almost any song and follow the music line by line."),
        TourKind::Page(PageKind::Widgets) => ("Widgets", "Two glanceable cards: this week, next prayer, water, focus today, world clock, battery, moon or a countdown. Pick them in Settings."),
        TourKind::Page(PageKind::Calendar) => ("Calendar", "A month at a glance with today highlighted. Use the small arrows to move between months; your timed tasks show on the right."),
        TourKind::Page(PageKind::Focus) => ("Pomodoro", "Press Start for a focus round, then a break. The timeline shows rounds 1 to 4 and the long break, and a running timer also shows in the closed notch."),
        TourKind::Page(PageKind::Water) => ("Water reminder", "Tap Drink each time you drink. The chart shows every hour, and Settings sets your bottle, daily goal and reminder hours."),
        TourKind::Page(PageKind::Shelf) => ("Shelf", "Drop files here to keep them handy, then click to open them. Turn on clipboard history in Settings to get a Clipboard tab too."),
        TourKind::Page(PageKind::Todo) => ("To-do list", "Add tasks, optionally with a reminder time. Tick the circle to finish one, or select a task and press Edit to change it."),
        TourKind::Page(PageKind::Assistant) => ("Nook AI", "Ask anything by typing or tapping the mic: it answers out loud, opens sites and apps, controls volume and music, and knows your time, music and prayer. Add a free Groq or Gemini key (or use Ollama) in Settings, Assistant."),
        TourKind::Page(PageKind::Prayer) => ("Prayer times", "Prayer times for your city (Indonesia or Morocco) with a countdown to the next one and a sun arc. Tap the city name to pick another city from the list."),
        TourKind::Settings => ("Make it yours", "The gear opens Settings: pick Notch, Floating or Island style, tune lyrics, reminders and widgets, and turn auto-update on or off. Right-click the notch or its tray icon for Settings and Quit."),
        TourKind::Done => ("You're all set", "Hover the notch whenever you need it. Nook keeps itself up to date in the background. You can replay this tour anytime from Settings, About."),
    }
}

impl App {
    fn tour_kinds(&self) -> Vec<TourKind> {
        let mut v = vec![TourKind::Welcome, TourKind::Header, TourKind::Dock];
        for k in self.pages() {
            v.push(TourKind::Page(k));
            if k == PageKind::Media && self.settings.lyrics {
                v.push(TourKind::Lyrics);
            }
        }
        v.push(TourKind::Settings);
        v.push(TourKind::Done);
        v
    }

    /// Where the highlight goes for a step, in canvas coordinates, plus its corner radius.
    /// Always measured from the panel as it is drawn right now (position, width, island offset), so the
    /// frame can never drift away from the real elements.
    fn tour_rect(&self, kind: TourKind) -> (Rect, f32) {
        let cx = WIN_W / 2.0;
        let (w, h, yo) = (self.w.value.max(60.0), self.h.value, self.y_off.value);
        let l = cx - w / 2.0;
        let panel_r = if self.settings.island { 34.0 } else { 24.0 };
        let n = self.pages().len() as f32;
        let slot = 33.0;
        let total = n * slot + 10.0 + slot;
        let dock_cy = yo + EXP_H - 17.0;
        let x0 = cx - total / 2.0;
        match kind {
            TourKind::Welcome | TourKind::Done => (Rect { x: l, y: yo, w, h }, panel_r),
            TourKind::Header => {
                // ring | date | clock | bars, exactly as `draw_expanded` lays them out
                let n = sys::now();
                let date = format!("{} {} {} {}", sys::DOW[n.dow as usize], n.day, sys::MON[(n.month - 1) as usize], n.year);
                let dw = self.gfx.measure(&date, 12.5, 500);
                let date_right = l + w - 68.0 - 52.0 - 8.0;
                let left = if self.settings.status_ring { date_right - dw - 14.0 - 8.0 } else { date_right - dw };
                let right = l + w - 38.0;
                (Rect { x: left - 8.0, y: yo + 18.0 - 14.0, w: right - left + 16.0, h: 28.0 }, 14.0)
            }
            TourKind::Dock => (Rect { x: x0 - 9.0, y: dock_cy - 15.0, w: total + 18.0, h: 30.0 }, 15.0),
            TourKind::Page(_) => (Rect { x: l + 14.0, y: yo + 32.0, w: w - 28.0, h: 92.0 }, 16.0),
            TourKind::Lyrics => (Rect { x: l + PAGE_PAD + 110.0 - 17.0, y: yo + 108.0 - 16.0, w: 34.0, h: 32.0 }, 16.0),
            TourKind::Settings => {
                let gx = x0 + n * slot + 10.0 + slot / 2.0;
                (Rect { x: gx - 17.0, y: dock_cy - 15.0, w: 34.0, h: 30.0 }, 15.0)
            }
        }
    }

    pub fn start_tour(&mut self) {
        let steps = self.tour_kinds();
        let (r, _) = self.tour_rect(steps[0]);
        self.expanded = true;
        self.toast = None;
        self.toast_q.clear();
        self.hud = None;
        self.page = 0;
        self.lyrics_view = false;
        self.water_cfg = false;
        self.tour = Some(Tour {
            step: 0,
            steps,
            fade: Ease::new(0.0, 7.0),
            text: Ease::new(0.0, 9.0),
            rect: [Ease::new(r.x, 12.0), Ease::new(r.y, 12.0), Ease::new(r.w, 12.0), Ease::new(r.h, 12.0)],
            born: Instant::now(),
        });
        if let Some(t) = &mut self.tour {
            t.fade.target = 1.0;
            t.text.target = 1.0;
        }
        self.dirty.set(true);
    }

    pub fn tour_enter(&mut self) {
        let Some(t) = &self.tour else { return };
        let kind = t.steps[t.step];
        let (r, _) = self.tour_rect(kind);
        let kinds = self.pages();
        match kind {
            TourKind::Page(k) => {
                if let Some(i) = kinds.iter().position(|x| *x == k) {
                    self.page = i;
                }
            }
            TourKind::Lyrics => self.page = 0,
            TourKind::Welcome | TourKind::Header | TourKind::Dock => self.page = 0,
            _ => {}
        }
        if let Some(t) = &mut self.tour {
            t.rect[0].target = r.x;
            t.rect[1].target = r.y;
            t.rect[2].target = r.w;
            t.rect[3].target = r.h;
            t.text.value = 0.0;
            t.text.target = 1.0;
        }
        self.dirty.set(true);
    }

    pub fn tour_next(&mut self) {
        let Some(t) = &mut self.tour else { return };
        if t.step + 1 < t.steps.len() {
            t.step += 1;
            self.tour_enter();
        } else {
            self.tour_finish();
        }
    }

    pub fn tour_back(&mut self) {
        let Some(t) = &mut self.tour else { return };
        if t.step > 0 {
            t.step -= 1;
            self.tour_enter();
        }
    }

    pub fn tour_finish(&mut self) {
        self.tour = None;
        self.page = 0;
        self.settings.tour_done = true;
        self.settings.save();
        self.dirty.set(true);
    }

    /// Per-frame animation + first-run trigger. Returns true while the tour animates.
    pub fn tour_tick(&mut self, dt: f32) -> bool {
        // "What's new" after an update (first-time users just get the tour)
        if !self.whats_new && self.settings.last_seen_version != crate::update::VERSION {
            if !self.settings.tour_done {
                self.settings.last_seen_version = crate::update::VERSION.to_string();
                self.settings.save();
            } else if self.tour.is_none() && !self.hidden && self.toast.is_none() && self.hud.is_none() && !self.expanded && !crate::settings_ui::is_open() && self.start.elapsed().as_secs_f32() > 3.0 {
                self.whats_new = true;
            }
        }
        if self.whats_new && self.expanded {
            self.whats_new_close(); // the user opened the notch: the card has done its job
        }
        if self.tour.is_none() {
            if !self.settings.tour_done
                && !self.hidden
                && self.toast.is_none()
                && self.hud.is_none()
                && self.hover_since.is_none()
                && !crate::settings_ui::is_open()
                && self.start.elapsed().as_secs_f32() > 2.5
            {
                self.start_tour();
            }
            return false;
        }
        let kind = {
            let t = self.tour.as_ref().unwrap();
            t.steps[t.step]
        };
        let (target, _) = self.tour_rect(kind);
        let t = self.tour.as_mut().unwrap();
        t.rect[0].target = target.x;
        t.rect[1].target = target.y;
        t.rect[2].target = target.w;
        t.rect[3].target = target.h;
        t.fade.step(dt);
        t.text.step(dt);
        for e in t.rect.iter_mut() {
            e.step(dt);
        }
        true
    }

    /// The tooltip rectangle (below the panel), used for drawing and for hit-testing the window.
    pub fn tour_box(&self) -> Rect {
        let cx = WIN_W / 2.0;
        Rect { x: cx - 215.0, y: self.y_off.value + EXP_H + 12.0, w: 430.0, h: 124.0 }
    }

    pub fn draw_tour(&self, geo: Option<&windows::Win32::Graphics::Direct2D::ID2D1PathGeometry>, l: f32, yo: f32, w: f32, h: f32) {
        let Some(t) = &self.tour else { return };
        let g = &self.gfx;
        let kind = t.steps[t.step];
        let fade = t.fade.value.clamp(0.0, 1.0);
        let (_, rad) = self.tour_rect(kind);
        let r = Rect { x: t.rect[0].value, y: t.rect[1].value, w: t.rect[2].value, h: t.rect[3].value };

        let pulse = 0.5 + 0.5 * (self.last.duration_since(t.born).as_secs_f32() * 3.2).sin();
        if matches!(kind, TourKind::Welcome | TourKind::Done) {
            // the whole panel: outline its actual silhouette
            if let Some(geo) = geo {
                g.stroke_geo(geo, 7.0, WHITE.a((0.06 + 0.12 * pulse) * fade));
                g.stroke_geo(geo, 2.0, WHITE.a(0.96 * fade));
            }
        } else {
            // dim the panel everywhere except the highlighted area
            match geo {
                Some(geo) => g.push_geo_clip(geo),
                None => g.push_round_clip(l, yo, w, h, 24.0),
            }
            g.dim_with_hole(l - 6.0, yo - 6.0, w + 12.0, h + 12.0, (r.x, r.y, r.w, r.h), rad, Color::rgba(0, 0, 0, 0.66 * fade));
            g.pop_layer();
            // white frame with a gentle pulse
            g.stroke_round(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0, rad + 3.0, 3.0, WHITE.a((0.08 + 0.14 * pulse) * fade));
            g.stroke_round(r.x - 0.5, r.y - 0.5, r.w + 1.0, r.h + 1.0, rad + 0.5, 2.0, WHITE.a(0.96 * fade));
        }

        // tooltip
        let (title, body) = copy(kind);
        let b = self.tour_box();
        let ta = t.text.value.clamp(0.0, 1.0) * fade;
        let slide = (1.0 - t.text.value.clamp(0.0, 1.0)) * 6.0;
        let by = b.y + slide;
        let body_h = g.measure_wrap(body, 12.0, 500, b.w - 36.0);
        let bh = (16.0 + 20.0 + 6.0 + body_h + 14.0 + 28.0 + 14.0).max(100.0);
        let bg = Color::rgba(22, 22, 28, 0.985 * fade);
        // arrow toward the highlighted area
        let ax = (r.x + r.w / 2.0).clamp(b.x + 28.0, b.x + b.w - 28.0);
        g.poly(&[(ax - 8.0, by + 0.5), (ax + 8.0, by + 0.5), (ax, by - 8.0)], bg, 0.6);
        g.fill_round(b.x, by, b.w, bh, 18.0, bg);
        g.stroke_round(b.x + 0.5, by + 0.5, b.w - 1.0, bh - 1.0, 18.0, 1.0, WHITE.a(0.14 * fade));
        g.text(title, b.x + 18.0, by + 12.0, b.w - 110.0, 22.0, 14.5, 700, WHITE.a(ta), Align::Left);
        g.text(&format!("{} / {}", t.step + 1, t.steps.len()), b.x + b.w - 78.0, by + 12.0, 60.0, 22.0, 11.0, 600, WHITE.a(0.45 * ta), Align::Right);
        g.text_wrap(body, b.x + 18.0, by + 40.0, b.w - 36.0, body_h + 4.0, 12.0, 500, WHITE.a(0.72 * ta), Align::Left);

        // buttons
        let row = by + bh - 14.0 - 28.0;
        let last = t.step + 1 == t.steps.len();
        let skip = Rect { x: b.x + 14.0, y: row, w: 74.0, h: 28.0 };
        g.text("Skip all", skip.x, skip.y, skip.w, skip.h, 12.0, 600, WHITE.a((if self.hovered(skip) { 0.95 } else { 0.5 }) * fade), Align::Center);
        self.hit(skip, Act::TourSkip);
        let next = Rect { x: b.x + b.w - 14.0 - 86.0, y: row, w: 86.0, h: 28.0 };
        g.fill_round(next.x, next.y, next.w, next.h, 14.0, WHITE.a((if self.hovered(next) { 1.0 } else { 0.92 }) * fade));
        g.text(if last { "Done" } else { "Next" }, next.x, next.y, next.w, next.h, 12.5, 700, BLACK.a(fade), Align::Center);
        self.hit(next, Act::TourNext);
        if t.step > 0 {
            let back = Rect { x: next.x - 8.0 - 70.0, y: row, w: 70.0, h: 28.0 };
            g.fill_round(back.x, back.y, back.w, back.h, 14.0, WHITE.a((if self.hovered(back) { 0.22 } else { 0.12 }) * fade));
            g.text("Back", back.x, back.y, back.w, back.h, 12.5, 600, WHITE.a(fade), Align::Center);
            self.hit(back, Act::TourBack);
        }
    }
}

// ------------------------------------------------------------------ what's new
/// Bullets of this version's notes, shipped inside the exe (`assets/whatsnew.txt`).
pub fn whats_new_lines() -> Vec<String> {
    include_str!("../assets/whatsnew.txt")
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("- ") || l.starts_with("* "))
        .map(|l| l[2..].replace("**", ""))
        .take(8)
        .collect()
}

impl App {
    pub fn whats_new_close(&mut self) {
        self.whats_new = false;
        self.settings.last_seen_version = crate::update::VERSION.to_string();
        self.settings.save();
        self.dirty.set(true);
    }

    pub fn whats_new_box(&self) -> Rect {
        let lines = whats_new_lines();
        let g = &self.gfx;
        let body: f32 = lines.iter().map(|l| g.measure_wrap(l, 11.5, 500, 340.0) + 7.0).sum();
        let cx = WIN_W / 2.0;
        Rect { x: cx - 190.0, y: self.y_off.target + 36.0 + 14.0, w: 380.0, h: (20.0 + 22.0 + 10.0 + body + 10.0 + 30.0 + 16.0).max(120.0) }
    }

    pub fn draw_whats_new(&self) {
        let g = &self.gfx;
        let b = self.whats_new_box();
        let lines = whats_new_lines();
        let age = (self.last.duration_since(self.start).as_secs_f32() - 3.0).max(0.0);
        let fade = (age * 3.0).clamp(0.0, 1.0);
        let by = b.y + (1.0 - fade) * 8.0;
        let bg = Color::rgba(18, 18, 22, 0.985 * fade);
        g.poly(&[(b.x + b.w / 2.0 - 8.0, by + 0.5), (b.x + b.w / 2.0 + 8.0, by + 0.5), (b.x + b.w / 2.0, by - 8.0)], bg, 0.6);
        g.fill_round(b.x, by, b.w, b.h, 18.0, bg);
        g.stroke_round(b.x + 0.5, by + 0.5, b.w - 1.0, b.h - 1.0, 18.0, 1.0, WHITE.a(0.14 * fade));
        g.fill_round(b.x + 16.0, by + 15.0, 26.0, 26.0, 8.0, AMBER.a(0.18 * fade));
        g.icon('\u{E946}', b.x + 29.0, by + 28.0, 13.0, AMBER.a(fade));
        g.text(&format!("What's new in v{}", crate::update::VERSION), b.x + 52.0, by + 14.0, b.w - 90.0, 28.0, 14.5, 700, WHITE.a(fade), Align::Left);
        let mut y = by + 52.0;
        for l in &lines {
            let h = g.measure_wrap(l, 11.5, 500, 340.0);
            g.fill_circle(b.x + 24.0, y + 7.0, 2.5, AMBER.a(fade));
            g.text_wrap(l, b.x + 36.0, y, 340.0, h + 4.0, 11.5, 500, WHITE.a(0.78 * fade), Align::Left);
            y += h + 7.0;
        }
        let btn = Rect { x: b.x + b.w - 16.0 - 92.0, y: by + b.h - 16.0 - 30.0, w: 92.0, h: 30.0 };
        g.fill_round(btn.x, btn.y, btn.w, btn.h, 15.0, WHITE.a((if self.hovered(btn) { 1.0 } else { 0.92 }) * fade));
        g.text("Got it", btn.x, btn.y, btn.w, btn.h, 12.5, 700, BLACK.a(fade), Align::Center);
        self.hit(btn, Act::WhatsNewClose);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn whats_new_notes_are_bullets() {
        let l = super::whats_new_lines();
        assert!(!l.is_empty() && l.len() <= 8);
        assert!(l.iter().all(|x| !x.contains("**") && !x.starts_with('-')));
    }
}
