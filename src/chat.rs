//! The Assistant page of the island: transcript, text field, mic and send buttons, plus the keyboard
//! handling that lets you type into the notch (the window becomes focusable only while the field is active).
use crate::app::*;
use crate::assistant::{Cfg, Status};
use crate::gfx::{Align, Color};
use crate::sys;

pub const PURPLE: Color = Color::rgb(167, 139, 250);

impl App {
    /// Settings + a short context line (time, music, next prayer) for the AI's system prompt.
    pub fn ai_cfg(&self) -> Cfg {
        let s = &self.settings;
        let n = sys::now();
        let m = self.media.snapshot();
        let (t, _, _) = crate::prayer::today(s);
        let (i, at, _) = crate::prayer::next(&t, crate::prayer::minutes_now(s));
        let open = self.store.todos.iter().filter(|t| !t.done).count();
        let context = format!(
            "It is {:02}:{:02} on {} {} {} {}. {} Next prayer: {} at {} ({}). Open tasks: {}.",
            n.hour,
            n.min,
            sys::DOW[n.dow as usize],
            n.day,
            sys::MON_FULL[(n.month - 1) as usize],
            n.year,
            if m.has_session && !m.title.is_empty() { format!("Now playing: {} by {}.", m.title, m.artist) } else { "Nothing is playing.".to_string() },
            crate::prayer::names_s(s)[i],
            crate::prayer::fmt(at),
            crate::prayer::place_label(s),
            open
        );
        Cfg {
            provider: s.ai_provider,
            key: s.ai_key.clone(),
            model: s.ai_model.clone(),
            url: s.ai_url.clone(),
            voice: s.ai_voice,
            voice_name: s.ai_voice_name.clone(),
            neural: s.ai_neural,
            rate: s.ai_rate,
            context,
        }
    }

    pub fn ai_busy(&self) -> bool {
        self.assistant.status() != Status::Idle
    }

    /// Four-pointed sparkle (the assistant's mark).
    pub fn sparkle(&self, cx: f32, cy: f32, r: f32, c: Color) {
        let k = r * 0.28;
        self.gfx.poly(&[(cx, cy - r), (cx + k, cy - k), (cx + r, cy), (cx + k, cy + k), (cx, cy + r), (cx - k, cy + k), (cx - r, cy), (cx - k, cy - k)], c, 0.8);
    }

    pub fn page_assistant(&self, ox: f32, yo: f32, a: f32) {
        let g = &self.gfx;
        let v = self.assistant.view();
        let t = self.last.duration_since(self.start).as_secs_f32();

        // Exact match with bottom dock pill dimensions
        let kinds = self.pages();
        let slot = 33.0;
        let n = kinds.len() as f32;
        let total = n * slot + 10.0 + slot;
        let pill_w = total + 14.0;
        let cx = ox + 420.0 / 2.0;
        let pill_x = cx - pill_w / 2.0;
        let pill_h = 26.0;
        let pill_y = yo + 88.0;
        let pill = Rect { x: pill_x, y: pill_y, w: pill_w, h: pill_h };
        let r_pill = 13.0;

        if v.count == 0 && v.err.is_empty() && v.status == Status::Idle {
            // Idle prompt aligned with the pill
            self.sparkle(pill.x + 14.0, yo + 53.0, 6.5, PURPLE.a(a));
            g.text("What would you like to do?", pill.x + 28.0, yo + 42.0, pill.w - 32.0, 22.0, 13.5, 500, WHITE.a(0.92 * a), Align::Left);
            let clock = self.clock_text();
            g.text(&clock, pill.x + pill.w - 48.0, yo + 4.0, 48.0, 20.0, 13.0, 600, WHITE.a(0.55 * a), Align::Right);
        } else {
            // Active transcript & status header with clean pill badge
            self.sparkle(pill.x + 8.0, yo + 14.0, 5.5, PURPLE.a(a));
            g.text("Nook AI", pill.x + 18.0, yo + 4.0, 56.0, 20.0, 13.0, 700, WHITE.a(a), Align::Left);

            let (status, scol) = match v.status {
                Status::Listening => ("Listening…", Color::rgb(255, 99, 130)),
                Status::Thinking => ("Thinking…", PURPLE),
                Status::Speaking => ("Speaking…", Color::rgb(48, 209, 88)),
                Status::Idle => ("", WHITE),
            };
            if v.status != Status::Idle {
                let bw = g.measure(status, 10.5, 600) + 24.0;
                let bx = pill.x + 78.0;
                let by = yo + 4.0;
                let bh = 20.0;
                let br = 10.0;
                g.fill_round(bx, by, bw, bh, br, scol.a(0.14 * a));
                g.stroke_round(bx + 0.5, by + 0.5, bw - 1.0, bh - 1.0, br, 1.0, scol.a(0.38 * a));
                let pulse = 0.5 + 0.5 * (t * 6.0).sin().abs();
                g.fill_circle(bx + 9.0, by + 10.0, 2.8, scol.a(a * pulse));
                g.text(status, bx + 16.0, by, bw - 18.0, bh, 10.5, 600, scol.a(a), Align::Left);
            }
            let clock = self.clock_text();
            g.text(&clock, pill.x + pill.w - 48.0, yo + 4.0, 48.0, 20.0, 13.0, 600, WHITE.a(0.55 * a), Align::Right);
            if v.count > 0 {
                let r = Rect { x: pill.x + pill.w - 72.0, y: yo + 5.0, w: 18.0, h: 18.0 };
                g.icon('\u{E711}', r.x + 9.0, r.y + 9.0, 7.5, WHITE.a((if self.hovered(r) { 0.95 } else { 0.40 }) * a));
                self.hit(r, Act::ChatClear);
            }

            // Transcript area cleanly fitted above the input pill
            let area = Rect { x: pill.x + 2.0, y: yo + 28.0, w: pill.w - 4.0, h: 54.0 };
            g.push_clip(area.x, area.y, area.w, area.h);
            if !v.user.is_empty() {
                g.text(&g.fit(&format!("You: {}", v.user), 10.0, 500, area.w - 20.0), area.x, area.y, area.w - 20.0, 12.0, 10.0, 500, WHITE.a(0.40 * a), Align::Left);
            }
            let (text, col) = if !v.err.is_empty() {
                (v.err.clone(), Color::rgb(255, 120, 108))
            } else {
                match v.status {
                    Status::Listening => ("Listening… speak now.".to_string(), WHITE.a(0.7)),
                    Status::Thinking => ("Thinking…".to_string(), WHITE.a(0.7)),
                    _ => (v.reply.clone(), WHITE),
                }
            };
            let reply_y = area.y + if v.user.is_empty() { 0.0 } else { 14.0 };
            g.text_wrap(&text, area.x, reply_y - self.chat_scroll.get(), area.w - 4.0, 400.0, 11.0, 500, col.a(0.95 * a), Align::Left);
            g.pop_clip();
        }

        // Unified input pill bar (identical size to dock pill)
        let pill_bg = Color::rgba(255, 255, 255, if self.chat_focus { 0.08 } else { 0.05 } * a);
        g.fill_round(pill.x, pill.y, pill.w, pill.h, r_pill, pill_bg);

        let pill_stroke = if self.chat_focus { PURPLE.a(0.85 * a) } else { Color::rgba(255, 255, 255, 0.10 * a) };
        g.stroke_round(pill.x, pill.y, pill.w, pill.h, r_pill, if self.chat_focus { 1.2 } else { 1.0 }, pill_stroke);

        // Send button (far right)
        let scx = pill.x + pill.w - 14.0;
        let scy = pill.y + 13.0;
        let s_rect = Rect { x: scx - 10.0, y: scy - 10.0, w: 20.0, h: 20.0 };
        let s_hot = self.hovered(s_rect);
        let on = !self.chat_input.trim().is_empty();

        let s_bg = if on { PURPLE.a((if s_hot { 1.0 } else { 0.85 }) * a) } else { Color::rgba(255, 255, 255, (if s_hot { 0.14 } else { 0.07 }) * a) };
        g.fill_circle(scx, scy, 10.0, s_bg);
        if on {
            g.stroke_circle(scx, scy, 10.0, 1.0, PURPLE.a(a));
        } else {
            g.stroke_circle(scx, scy, 10.0, 1.0, Color::rgba(255, 255, 255, (if s_hot { 0.22 } else { 0.12 }) * a));
        }
        let plane_col = if on { WHITE.a(a) } else { WHITE.a((if s_hot { 0.95 } else { 0.72 }) * a) };
        g.paper_plane(scx, scy, 5.0, plane_col);
        self.hit(s_rect, Act::ChatSend);

        // Mic button (left of send)
        let mcx = pill.x + pill.w - 38.0;
        let mcy = pill.y + 13.0;
        let m_rect = Rect { x: mcx - 10.0, y: mcy - 10.0, w: 20.0, h: 20.0 };
        let m_hot = self.hovered(m_rect);
        let listening = v.status == Status::Listening;

        let m_bg = if listening { Color::rgb(255, 99, 130).a(0.85 * a) } else { Color::rgba(255, 255, 255, (if m_hot { 0.14 } else { 0.07 }) * a) };
        g.fill_circle(mcx, mcy, 10.0, m_bg);
        if listening {
            g.stroke_circle(mcx, mcy, 10.0 + 2.5 * (t * 5.0).sin().abs(), 1.0, Color::rgb(255, 99, 130).a(0.6 * a));
        } else {
            g.stroke_circle(mcx, mcy, 10.0, 1.0, Color::rgba(255, 255, 255, (if m_hot { 0.22 } else { 0.12 }) * a));
        }
        g.icon('\u{E720}', mcx, mcy, 9.5, WHITE.a((if m_hot { 1.0 } else { 0.85 }) * a));
        self.hit(m_rect, Act::ChatMic);

        // Vertical divider line
        let div_x = pill.x + pill.w - 53.0;
        g.line(div_x, pill.y + 6.0, div_x, pill.y + pill.h - 6.0, 1.0, Color::rgba(255, 255, 255, 0.18 * a));

        // Text area
        let tf_rect = Rect { x: pill.x, y: pill.y, w: div_x - pill.x, h: pill.h };
        let tx = pill.x + 13.0;
        let tw = div_x - pill.x - 18.0;
        if self.chat_input.is_empty() {
            let placeholder = if self.chat_focus { "Type, then press Enter…" } else { "Ask anything…" };
            g.text_italic(placeholder, tx, pill.y, tw, pill.h, 11.5, 400, WHITE.a(0.40 * a), Align::Left);
        } else {
            let mut s: Vec<char> = self.chat_input.chars().collect();
            while s.len() > 1 && g.measure(&s.iter().collect::<String>(), 11.5, 500) > tw - 14.0 {
                s.remove(0);
            }
            let shown: String = s.iter().collect();
            g.text(&shown, tx, pill.y, tw, pill.h, 11.5, 500, WHITE.a(a), Align::Left);
            if self.chat_focus && (t % 1.0) < 0.55 {
                let caret_x = tx + 1.0 + g.measure(&shown, 11.5, 500);
                g.fill_rect(caret_x, pill.y + 6.0, 1.3, 14.0, PURPLE.a(a));
            }
        }
        self.hit(tf_rect, Act::ChatFocus);
    }

    pub fn chat_act(&mut self, act: Act) {
        match act {
            Act::ChatFocus => self.chat_set_focus(true),
            Act::ChatMic => {
                if self.assistant.status() == Status::Speaking {
                    crate::assistant::stop_speaking(); // the mic button is a stop button while it talks
                    return;
                }
                self.chat_set_focus(false);
                let cfg = self.ai_cfg();
                self.assistant.listen(cfg);
            }
            Act::ChatSend => self.chat_send(),
            Act::ChatClear => {
                self.assistant.clear();
                self.chat_scroll.set(0.0);
            }
            _ => {}
        }
    }

    fn chat_send(&mut self) {
        let text = std::mem::take(&mut self.chat_input);
        if text.trim().is_empty() {
            return;
        }
        self.chat_scroll.set(0.0);
        let cfg = self.ai_cfg();
        self.assistant.send(text, cfg);
    }

    /// Typed characters while the field is active (Enter sends, Esc leaves, Ctrl+V pastes).
    pub fn chat_char(&mut self, ch: u32) {
        match ch {
            8 => {
                self.chat_input.pop();
            }
            13 => self.chat_send(),
            27 => self.chat_set_focus(false),
            22 => {
                if let Some(t) = read_clipboard_text() {
                    let t: String = t.chars().filter(|c| !c.is_control()).take(300).collect();
                    self.chat_input.push_str(&t);
                }
            }
            c if c >= 32 && c != 127 => {
                if let Some(c) = char::from_u32(c) {
                    if self.chat_input.chars().count() < 300 {
                        self.chat_input.push(c);
                    }
                }
            }
            _ => {}
        }
        self.dirty.set(true);
    }

    /// The overlay never takes focus, except while the text field is in use.
    pub fn chat_set_focus(&mut self, on: bool) {
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        use windows::Win32::UI::WindowsAndMessaging::*;
        if self.chat_focus == on {
            return;
        }
        self.chat_focus = on;
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let no = WS_EX_NOACTIVATE.0 as isize;
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, if on { ex & !no } else { ex | no });
            if on {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.hwnd);
            }
        }
        self.dirty.set(true);
    }
}
