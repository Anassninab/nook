//! Synced lyrics from several public sources, tried in order until one has the song:
//! LRCLIB (exact `get`, then `search`), NetEase Cloud Music (huge catalogue incl. Asian / Arabic /
//! Indonesian songs) and lyrics.ovh (plain text, last resort). Video-site titles such as
//! "Artist - Song (Official Video) ft. X" are broken into every plausible (artist, title) pair first.
use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use windows::core::*;
use windows::Foundation::Uri;
use windows::Web::Http::HttpClient;

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsStatus {
    Idle,
    Loading,
    Ready,
    Missing,
    Offline,
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub status: LyricsStatus,
    pub key: String,
    pub synced: bool,
    pub lines: Vec<(f64, String)>,
    pub tries: u32,
    pub at: std::time::Instant,
}

#[derive(Clone)]
pub struct LyricsHandle {
    pub state: Arc<Mutex<Lyrics>>,
}

type Found = (bool, Vec<(f64, String)>);

fn cache() -> &'static Mutex<HashMap<String, Found>> {
    static C: OnceLock<Mutex<HashMap<String, Found>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

impl LyricsHandle {
    pub fn new() -> Self {
        Self { state: Arc::new(Mutex::new(Lyrics { status: LyricsStatus::Idle, key: String::new(), synced: false, lines: vec![], tries: 0, at: std::time::Instant::now() })) }
    }

    /// (status, synced, previous, current, next, active_index) at `pos` without cloning the whole song.
    pub fn around(&self, pos: f64, dur: f64) -> (LyricsStatus, bool, String, String, String, Option<usize>) {
        let (st, synced, v, idx) = self.window(pos, dur);
        (st, synced, v[1].clone(), v[2].clone(), v[3].clone(), idx)
    }

    /// Five lines centred on the active one: [prev2, prev, current, next, next2], plus active line index.
    pub fn window(&self, pos: f64, dur: f64) -> (LyricsStatus, bool, [String; 5], Option<usize>) {
        let g = self.state.lock().unwrap();
        let empty: [String; 5] = Default::default();
        if g.status != LyricsStatus::Ready || g.lines.is_empty() {
            return (g.status.clone(), g.synced, empty, None);
        }
        let get = |i: isize| -> String {
            if i < 0 {
                String::new()
            } else {
                g.lines.get(i as usize).map(|l| l.1.clone()).unwrap_or_default()
            }
        };
        let idx: Option<isize> = if !g.synced {
            // plain lyrics: estimate the line from playback progress
            let n = g.lines.len();
            Some((if dur > 0.0 { ((pos / dur) * n as f64) as usize } else { 0 }.min(n - 1)) as isize)
        } else {
            current_index(&g.lines, pos).map(|i| i as isize)
        };
        let line_idx = idx.and_then(|i| if i >= 0 { Some(i as usize) } else { None });
        match idx {
            Some(i) => (LyricsStatus::Ready, g.synced, [get(i - 2), get(i - 1), get(i), get(i + 1), get(i + 2)], line_idx),
            None => (LyricsStatus::Ready, g.synced, [String::new(), String::new(), String::new(), get(0), get(1)], None),
        }
    }

    /// A lookup that found nothing, or failed for lack of network, is tried again after a while (a few times):
    /// the first attempt often loses to a slow network or to a track whose metadata was still half-filled.
    pub fn retry_stale(&self, title: &str, artist: &str, duration: f64) {
        let tries = {
            let g = self.state.lock().unwrap();
            let wait = if g.status == LyricsStatus::Offline { 10 } else { 25 };
            let stale = matches!(g.status, LyricsStatus::Missing | LyricsStatus::Offline) && g.at.elapsed() > Duration::from_secs(wait) && g.tries < 4;
            if !stale {
                return;
            }
            g.tries
        };
        self.state.lock().unwrap().key.clear();
        self.request_n(title, artist, duration, tries + 1);
    }

    pub fn request(&self, title: &str, artist: &str, duration: f64) {
        self.request_n(title, artist, duration, 0);
    }

    fn request_n(&self, title: &str, artist: &str, duration: f64, tries: u32) {
        let key = format!("{title}\u{1}{artist}");
        {
            let mut g = self.state.lock().unwrap();
            if g.key == key {
                return;
            }
            *g = Lyrics { status: LyricsStatus::Loading, key: key.clone(), synced: false, lines: vec![], tries, at: std::time::Instant::now() };
        }
        if title.is_empty() {
            self.state.lock().unwrap().status = LyricsStatus::Idle;
            return;
        }
        if let Some((synced, lines)) = cache().lock().unwrap().get(&key).cloned() {
            let mut g = self.state.lock().unwrap();
            g.status = LyricsStatus::Ready;
            g.synced = synced;
            g.lines = lines;
            return;
        }
        let st = self.state.clone();
        let (title, artist) = (title.to_string(), artist.to_string());
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let res = fetch(&title, &artist, duration);
            crate::sys::log(&format!(
                "lyrics '{title}' / '{artist}' -> {}",
                match &res {
                    Ok(Some((s, l))) => format!("ok synced={s} lines={}", l.len()),
                    Ok(None) => "none".into(),
                    Err(e) => format!("err {e:?}"),
                }
            ));
            if let Ok(Some(f)) = &res {
                cache().lock().unwrap().insert(key.clone(), f.clone());
            }
            let mut g = st.lock().unwrap();
            if g.key != key {
                return; // track changed while we were fetching
            }
            match res {
                Ok(Some((synced, lines))) => {
                    g.status = LyricsStatus::Ready;
                    g.synced = synced;
                    g.lines = lines;
                }
                Ok(None) => g.status = LyricsStatus::Missing,
                Err(_) => g.status = LyricsStatus::Offline,
            }
            g.at = std::time::Instant::now();
        });
    }
}

pub(crate) fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

const NOISE: [&str; 24] = [
    "official", "lyric", "video", "audio", "visualizer", "remaster", "hd", "4k", "mv", "m/v", "music video", "with lyrics", "full song",
    "full video", "hq", "explicit", "clean", "live", "version", "performance", "karaoke", "color coded", "sub", "topic",
];

fn is_noise_group(inner: &str) -> bool {
    let l = inner.to_lowercase();
    NOISE.iter().any(|k| l.split(|c: char| !c.is_alphanumeric() && c != '/').any(|w| w == *k || (w.len() == k.len() + 1 && w.starts_with(*k) && w.ends_with('s'))) || (k.contains(' ') && l.contains(k)))
}

/// Drop "(Official Video)", "[Lyrics]", "【MV】" style groups and "feat." tails.
pub(crate) fn clean_title(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut group = String::new();
    let mut open = ' ';
    for c in t.chars() {
        match c {
            '(' | '[' | '【' | '（' | '「' if depth == 0 => {
                depth = 1;
                open = c;
                group.clear();
            }
            ')' | ']' | '】' | '）' | '」' if depth > 0 => {
                depth = 0;
                // keep meaningful groups like "(Remix)" / "(Acoustic)"; noise and features go
                let l = group.to_lowercase();
                let feat = l.starts_with("feat") || l.starts_with("ft") || l.starts_with("with ");
                if !is_noise_group(&group) && !feat {
                    out.push(open);
                    out.push_str(&group);
                    out.push(c);
                }
            }
            _ if depth > 0 => group.push(c),
            _ => out.push(c),
        }
    }
    if depth > 0 {
        // unbalanced bracket: ignore the tail
    }
    let mut s = out;
    for sep in [" ft.", " feat.", " ft ", " feat ", " featuring ", " Ft.", " Feat."] {
        if let Some(i) = s.find(sep) {
            s.truncate(i);
        }
    }
    if let Some(i) = s.find('|') {
        s.truncate(i);
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| c == '-' || c == ' ').to_string()
}

fn clean_artist(a: &str) -> String {
    let mut s = a.trim().to_string();
    for suf in ["VEVO", "Vevo", " - Topic", " Official", " Music", " Records", " Lyrics", "Official"] {
        if let Some(x) = s.strip_suffix(suf) {
            s = x.trim().to_string();
        }
    }
    s
}

/// Every plausible (artist, title) reading of what the player reports.
pub(crate) fn candidates(title: &str, artist: &str) -> Vec<(String, String)> {
    let t = clean_title(title);
    let a = clean_artist(artist);
    let mut v: Vec<(String, String)> = vec![];
    let mut push = |x: &str, y: &str| {
        let (x, y) = (x.trim().to_string(), y.trim().to_string());
        if !y.is_empty() && !v.iter().any(|(p, q)| *p == x && *q == y) {
            v.push((x, y));
        }
    };
    // "Artist - Song" (and the swapped reading) inside the title: common for video uploads
    for sep in [" - ", " – ", " — ", " ~ ", " : "] {
        if let Some((l, r)) = t.split_once(sep) {
            push(l, r);
            push(r, l);
            break;
        }
    }
    push(&a, &t);
    push("", &t);
    // the uploader channel is often "Artist Lyrics" / "Artist - Topic" even when not exact
    if t.contains('-') {
        for part in t.split('-') {
            push(&a, part);
        }
    }
    v
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

pub(crate) fn http_get(url: String, referer: Option<&'static str>) -> Result<Option<String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<Option<String>> {
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            let _ = hd.UserAgent()?.TryParseAdd(h!("Mozilla/5.0 Nook/0.2"));
            if let Some(r) = referer {
                let _ = hd.Append(h!("Referer"), &HSTRING::from(r));
            }
            let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.get()?;
            if !resp.IsSuccessStatusCode()? {
                return Ok(None);
            }
            Ok(Some(resp.Content()?.ReadAsStringAsync()?.get()?.to_string()))
        })();
        let _ = tx.send(r);
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(r) => r,
        Err(_) => Err(Error::from(HRESULT(0x800705B4u32 as i32))), // timed out
    }
}

fn json(body: Option<String>) -> Option<serde_json::Value> {
    body.and_then(|b| serde_json::from_str(&b).ok())
}

fn synced_or_plain(it: &serde_json::Value) -> Option<Found> {
    if let Some(s) = it["syncedLyrics"].as_str().filter(|s| !s.is_empty()) {
        let lines = parse_lrc(s);
        if !lines.is_empty() {
            return Some((true, lines));
        }
    }
    if let Some(p) = it["plainLyrics"].as_str().filter(|s| !s.is_empty()) {
        return Some((false, p.lines().map(|l| (0.0, l.to_string())).collect()));
    }
    None
}

fn lrclib(artist: &str, title: &str, duration: f64, net_ok: &mut bool) -> Option<Found> {
    // exact lookup (best when the duration is known)
    if !artist.is_empty() {
        let mut url = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", enc(artist), enc(title));
        if duration > 0.0 {
            url.push_str(&format!("&duration={}", duration.round() as i64));
        }
        match http_get(url, None) {
            Ok(b) => {
                *net_ok = true;
                if let Some(f) = json(b).as_ref().and_then(synced_or_plain) {
                    return Some(f);
                }
            }
            Err(_) => {}
        }
    }
    let url = if artist.is_empty() {
        format!("https://lrclib.net/api/search?q={}", enc(title))
    } else {
        format!("https://lrclib.net/api/search?track_name={}&artist_name={}", enc(title), enc(artist))
    };
    let Ok(b) = http_get(url, None) else { return None };
    *net_ok = true;
    let v = json(b)?;
    let arr = v.as_array()?;
    let want = norm(title);
    let mut best: Option<(f64, &serde_json::Value)> = None;
    for it in arr {
        let has = it["syncedLyrics"].as_str().map(|s| !s.is_empty()).unwrap_or(false) || it["plainLyrics"].as_str().map(|s| !s.is_empty()).unwrap_or(false);
        if !has {
            continue;
        }
        let name = norm(it["trackName"].as_str().unwrap_or(""));
        if !name.is_empty() && !want.is_empty() && !(name.contains(&want) || want.contains(&name)) {
            continue; // a different song that merely shares words
        }
        let d = it["duration"].as_f64().unwrap_or(0.0);
        let mut score = if duration > 0.0 && d > 0.0 { (d - duration).abs() } else { 5.0 };
        if duration > 0.0 && d > 0.0 && (d - duration).abs() > 12.0 {
            continue; // wrong cut / different recording
        }
        if it["syncedLyrics"].as_str().map(|s| !s.is_empty()).unwrap_or(false) {
            score -= 2.0;
        }
        if best.map(|(s, _)| score < s).unwrap_or(true) {
            best = Some((score, it));
        }
    }
    best.and_then(|(_, it)| synced_or_plain(it))
}

fn netease(artist: &str, title: &str, duration: f64, net_ok: &mut bool) -> Option<Found> {
    let q = if artist.is_empty() { title.to_string() } else { format!("{artist} {title}") };
    let url = format!("https://music.163.com/api/search/get/web?csrf_token=&hlpretag=&hltag=&s={}&type=1&offset=0&total=true&limit=8", enc(&q));
    let Ok(b) = http_get(url, Some("https://music.163.com")) else { return None };
    *net_ok = true;
    let v = json(b)?;
    let songs = v["result"]["songs"].as_array()?;
    let (want_t, want_a) = (norm(title), norm(artist));
    let mut best: Option<(f64, u64)> = None;
    for s in songs {
        let name = norm(s["name"].as_str().unwrap_or(""));
        if name.is_empty() || !(name.contains(&want_t) || want_t.contains(&name)) {
            continue;
        }
        let artists: String = s["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).map(norm).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        if !want_a.is_empty() && !artists.is_empty() && !(artists.contains(&want_a) || want_a.contains(&artists) || artists.split(' ').any(|p| !p.is_empty() && want_a.contains(p))) {
            continue;
        }
        let d = s["duration"].as_f64().or(s["dt"].as_f64()).unwrap_or(0.0) / 1000.0;
        if duration > 0.0 && d > 0.0 && (d - duration).abs() > 12.0 {
            continue;
        }
        let score = if duration > 0.0 && d > 0.0 { (d - duration).abs() } else { 5.0 } - if name == want_t { 3.0 } else { 0.0 };
        if let Some(id) = s["id"].as_u64() {
            if best.map(|(sc, _)| score < sc).unwrap_or(true) {
                best = Some((score, id));
            }
        }
    }
    let (_, id) = best?;
    let url = format!("https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=-1");
    let Ok(b) = http_get(url, Some("https://music.163.com")) else { return None };
    let v = json(b)?;
    let lrc = v["lrc"]["lyric"].as_str()?;
    let lines = parse_lrc(lrc);
    if lines.len() >= 3 {
        Some((true, lines))
    } else {
        None
    }
}

fn lyrics_ovh(artist: &str, title: &str, net_ok: &mut bool) -> Option<Found> {
    if artist.is_empty() {
        return None;
    }
    let Ok(b) = http_get(format!("https://api.lyrics.ovh/v1/{}/{}", enc(artist), enc(title)), None) else { return None };
    *net_ok = true;
    let v = json(b)?;
    let t = v["lyrics"].as_str()?;
    let lines: Vec<(f64, String)> = t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.to_lowercase().starts_with("paroles de la chanson")).map(|l| (0.0, l)).collect();
    if lines.len() >= 4 {
        Some((false, lines))
    } else {
        None
    }
}

/// Providers in order of quality; synced lyrics from any provider beat plain lyrics from an earlier one.
fn fetch(title: &str, artist: &str, duration: f64) -> Result<Option<Found>> {
    let cands = candidates(title, artist);
    let mut net_ok = false;
    let mut plain: Option<Found> = None;
    let keep = |f: Found, plain: &mut Option<Found>| -> Option<Found> {
        if f.0 {
            Some(f)
        } else {
            if plain.is_none() {
                *plain = Some(f);
            }
            None
        }
    };
    let t0 = std::time::Instant::now();
    let budget = std::time::Duration::from_secs(22); // never leave the UI on "Finding lyrics" for minutes
    for (a, t) in cands.iter().take(5) {
        if t0.elapsed() > budget {
            break;
        }
        if let Some(f) = lrclib(a, t, duration, &mut net_ok).and_then(|f| keep(f, &mut plain)) {
            return Ok(Some(f));
        }
    }
    for (a, t) in cands.iter().take(4) {
        if t0.elapsed() > budget {
            break;
        }
        if let Some(f) = netease(a, t, duration, &mut net_ok).and_then(|f| keep(f, &mut plain)) {
            return Ok(Some(f));
        }
    }
    if plain.is_some() {
        return Ok(plain);
    }
    for (a, t) in cands.iter().filter(|(a, _)| !a.is_empty()).take(2) {
        if let Some(f) = lyrics_ovh(a, t, &mut net_ok) {
            return Ok(Some(f));
        }
    }
    if net_ok {
        Ok(None)
    } else {
        Err(Error::from(HRESULT(0x80072EE7u32 as i32))) // no network at all
    }
}

fn is_credit(line: &str) -> bool {
    let l = line.trim();
    ["作词", "作曲", "编曲", "制作", "Lyricist", "Composer", "Lyrics by", "Music by", "Arranger", "Producer", "作詞", "作曲 "].iter().any(|k| l.starts_with(k)) && l.contains([':', '：'])
}

pub fn parse_lrc(s: &str) -> Vec<(f64, String)> {
    let mut out = vec![];
    for line in s.lines() {
        let mut rest = line.trim();
        let mut stamps = vec![];
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some((m, sec)) = tag.split_once(':') {
                if let (Ok(m), Ok(sec)) = (m.parse::<f64>(), sec.parse::<f64>()) {
                    stamps.push(m * 60.0 + sec);
                }
            }
            rest = rest[end + 1..].trim_start();
        }
        // enhanced LRC carries <mm:ss.xx> word stamps inside the line
        let mut text = String::new();
        let mut skip = false;
        for c in rest.chars() {
            match c {
                '<' => skip = true,
                '>' if skip => skip = false,
                _ if !skip => text.push(c),
                _ => {}
            }
        }
        let text = text.trim().to_string();
        if is_credit(&text) {
            continue;
        }
        for t in stamps {
            out.push((t, text.clone()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out
}

/// Index of the line active at `pos` seconds.
pub fn current_index(lines: &[(f64, String)], pos: f64) -> Option<usize> {
    if lines.is_empty() || pos < lines[0].0 - 0.25 {
        return None;
    }
    // binary search: last line whose stamp <= pos (+ 0.20s lead so line appears right on the vocal attack)
    let p = pos + 0.15;
    let i = lines.partition_point(|l| l.0 <= p);
    Some(i.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_parsing() {
        let c = candidates("Drake - One Dance (Lyrics) ft. Wizkid & Kyla", "Billion Stars");
        assert_eq!(c[0], ("Drake".to_string(), "One Dance".to_string()));
        let c = candidates("周杰伦 Jay Chou【晴天 Sunny Day】Official MV", "JVR Music");
        println!("{c:?}");
        assert!(!c.is_empty());
    }

    #[test]
    fn live_lookup() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let cases = [
            ("Drake - One Dance (Lyrics) ft. Wizkid & Kyla", "Billion Stars", 174.0),
            ("Libianca - People ft. Ayra Starr, Omah Lay", "LatinHype", 0.0),
            ("Tulus - Hati-Hati di Jalan (Official Music Video)", "Tulus", 0.0),
            ("Billie Eilish - bad guy (Official Audio)", "Billie Eilish", 0.0),
            ("周杰伦 - 晴天", "Jay Chou", 0.0),
            ("Kun Anta", "Humood AlKhudher", 0.0),
            ("Rich Brian - Dat $tick", "88rising", 0.0),
        ];
        let mut hits = 0;
        for (t, a, d) in cases {
            let r = fetch(t, a, d);
            let s = match &r {
                Ok(Some((sy, l))) => {
                    hits += 1;
                    format!("OK synced={sy} lines={} first='{}'", l.len(), l.iter().find(|x| !x.1.is_empty()).map(|x| x.1.clone()).unwrap_or_default())
                }
                Ok(None) => "none".into(),
                Err(e) => format!("err {e:?}"),
            };
            println!("{t} -> {s}");
        }
        assert!(hits >= 5, "only {hits} of {} found", cases.len());
    }
}
