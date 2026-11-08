//! Persistent user data: to-dos, water log, shelf files, clipboard history, focus stats.
use crate::config;
use crate::sys;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Todo {
    pub title: String,
    pub time: Option<u32>, // minute of day
    pub done: bool,
    pub fired_start: bool,
    pub fired_end: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Idle,
    Focus,
    Short,
    Long,
}

pub struct Store {
    pub todos: Vec<Todo>,
    pub water_day: u32,
    pub water_taken: u32,
    pub water_hours: [u32; 24],
    pub water_last_fire: u32, // minute of day
    pub shelf: Vec<PathBuf>,
    pub clips: Vec<String>,
    pub focus_day: u32,
    pub focus_minutes: u32,
    pub focus_sessions: u32,
    // pomodoro runtime
    pub phase: Phase,
    pub running: bool,
    pub remaining: f64,
    pub phase_total: f64,
    pub cycle: u32,
}

impl Store {
    pub fn load(shelf_empty: bool, clip_persist: bool) -> Store {
        let today = sys::now().ymd();
        let mut s = Store {
            todos: vec![],
            water_day: today,
            water_taken: 0,
            water_hours: [0; 24],
            water_last_fire: 0,
            shelf: vec![],
            clips: vec![],
            focus_day: today,
            focus_minutes: 0,
            focus_sessions: 0,
            phase: Phase::Idle,
            running: false,
            remaining: 0.0,
            phase_total: 0.0,
            cycle: 0,
        };
        if let Ok(t) = fs::read_to_string(config::dir().join("tasks.txt")) {
            for l in t.lines() {
                let p: Vec<&str> = l.splitn(3, '|').collect();
                if p.len() == 3 {
                    s.todos.push(Todo {
                        done: p[0] == "1",
                        time: p[1].parse().ok(),
                        title: p[2].to_string(),
                        fired_start: false,
                        fired_end: false,
                    });
                }
            }
        }
        let w = config::read_kv("water.txt");
        if w.get("day").and_then(|v| v.parse::<u32>().ok()) == Some(today) {
            s.water_taken = w.get("taken").and_then(|v| v.parse().ok()).unwrap_or(0);
            if let Some(h) = w.get("hours") {
                for (i, p) in h.split(',').take(24).enumerate() {
                    s.water_hours[i] = p.parse().unwrap_or(0);
                }
            }
        }
        let f = config::read_kv("focus.txt");
        if f.get("day").and_then(|v| v.parse::<u32>().ok()) == Some(today) {
            s.focus_minutes = f.get("minutes").and_then(|v| v.parse().ok()).unwrap_or(0);
            s.focus_sessions = f.get("sessions").and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        if !shelf_empty {
            if let Ok(t) = fs::read_to_string(config::local_dir().join("shelf.txt")) {
                for l in t.lines() {
                    let p = PathBuf::from(l);
                    if p.exists() {
                        s.shelf.push(p);
                    }
                }
            }
        }
        if clip_persist {
            if let Ok(t) = fs::read_to_string(config::dir().join("clipboard.txt")) {
                for l in t.lines() {
                    s.clips.push(l.replace("\\n", "\n").replace("\\\\", "\\"));
                }
            }
        }
        s
    }

    pub fn roll_day(&mut self) {
        let today = sys::now().ymd();
        if today != self.water_day {
            self.water_day = today;
            self.water_taken = 0;
            self.water_hours = [0; 24];
            self.save_water();
        }
        if today != self.focus_day {
            self.focus_day = today;
            self.focus_minutes = 0;
            self.focus_sessions = 0;
            self.save_focus();
        }
        for t in &mut self.todos {
            if t.done {
                continue;
            }
        }
    }

    pub fn save_todos(&self) {
        let mut s = String::new();
        for t in &self.todos {
            s.push_str(&format!("{}|{}|{}\n", t.done as u8, t.time.map(|x| x as i64).unwrap_or(-1), t.title.replace('\n', " ")));
        }
        let _ = fs::write(config::dir().join("tasks.txt"), s);
    }
    pub fn save_water(&self) {
        let mut m = std::collections::BTreeMap::new();
        m.insert("day".to_string(), self.water_day.to_string());
        m.insert("taken".to_string(), self.water_taken.to_string());
        m.insert("hours".to_string(), self.water_hours.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","));
        config::write_kv("water.txt", &m);
    }
    pub fn save_focus(&self) {
        let mut m = std::collections::BTreeMap::new();
        m.insert("day".to_string(), self.focus_day.to_string());
        m.insert("minutes".to_string(), self.focus_minutes.to_string());
        m.insert("sessions".to_string(), self.focus_sessions.to_string());
        config::write_kv("focus.txt", &m);
    }
    pub fn save_shelf(&self) {
        let s: Vec<String> = self.shelf.iter().map(|p| p.to_string_lossy().to_string()).collect();
        let _ = fs::write(config::local_dir().join("shelf.txt"), s.join("\n"));
    }
    pub fn save_clips(&self, persist: bool) {
        let path = config::dir().join("clipboard.txt");
        if !persist {
            let _ = fs::remove_file(path);
            return;
        }
        let s: Vec<String> = self.clips.iter().map(|c| c.replace('\\', "\\\\").replace('\n', "\\n")).collect();
        let _ = fs::write(path, s.join("\n"));
    }

    pub fn add_clip(&mut self, text: String, max: usize) {
        if text.trim().is_empty() {
            return;
        }
        self.clips.retain(|c| c != &text);
        self.clips.insert(0, text);
        self.clips.truncate(max.max(1));
    }

    pub fn drink(&mut self, ml: u32) {
        let h = sys::now().hour as usize;
        self.water_taken += ml;
        self.water_hours[h.min(23)] += ml;
        self.save_water();
    }
}
