//! System media session (GSMTC) worker. Polls on its own MTA thread and publishes a snapshot.
//!
//! Source selection follows what is actually playing (sticky, so two players never ping-pong),
//! artwork is re-read for a few seconds after a track change (browsers deliver it late), and
//! browser tabs that only offer the browser's own logo get real cover art from a public lookup.
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Foundation::Uri;
use windows::Media::Control::*;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

type Session = GlobalSystemMediaTransportControlsSession;

#[derive(Clone, Debug)]
pub struct MediaState {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    pub position: f64, // seconds at `stamp`
    pub duration: f64,
    pub stamp: Instant,
    pub art: Option<std::sync::Arc<Vec<u8>>>, // Arc: the UI snapshots the state every frame
    pub track_ver: u64, // bumps when the track changes
    pub art_ver: u64,   // bumps when new artwork bytes arrive
    pub sources: usize, // how many media sessions the system currently exposes
}

impl Default for MediaState {
    fn default() -> Self {
        Self {
            has_session: false,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            app: String::new(),
            playing: false,
            position: 0.0,
            duration: 0.0,
            stamp: Instant::now(),
            art: None,
            track_ver: 0,
            art_ver: 0,
            sources: 0,
        }
    }
}

impl MediaState {
    pub fn pos_now(&self) -> f64 {
        let p = if self.playing { self.position + self.stamp.elapsed().as_secs_f64() } else { self.position };
        if self.duration > 0.0 {
            p.min(self.duration)
        } else {
            p
        }
    }
}

pub enum MediaCmd {
    PlayPause,
    Next,
    Prev,
    Seek(f64),
    /// Cycle to the next media source (click on the artwork).
    NextSource,
}

#[derive(Clone)]
pub struct MediaHandle {
    pub state: Arc<Mutex<MediaState>>,
    tx: Sender<MediaCmd>,
}

impl MediaHandle {
    pub fn send(&self, c: MediaCmd) {
        let _ = self.tx.send(c);
    }
    pub fn snapshot(&self) -> MediaState {
        self.state.lock().unwrap().clone()
    }
}

pub fn spawn() -> MediaHandle {
    let state = Arc::new(Mutex::new(MediaState::default()));
    let (tx, rx) = channel();
    let st = state.clone();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        worker(st, rx);
    });
    MediaHandle { state, tx }
}

fn is_playing(s: &Session) -> bool {
    s.GetPlaybackInfo()
        .and_then(|pb| pb.PlaybackStatus())
        .map(|p| p == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false)
}

fn sid(s: &Session) -> String {
    s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default()
}

fn has_title(s: &Session) -> bool {
    s.TryGetMediaPropertiesAsync()
        .and_then(|o| o.get())
        .and_then(|p| p.Title())
        .map(|t| !t.is_empty())
        .unwrap_or(false)
}

fn is_browser(app: &str) -> bool {
    let a = app.to_lowercase();
    ["chrome", "msedge", "edge", "firefox", "opera", "brave", "vivaldi", "arc.exe", "zen"].iter().any(|k| a.contains(k))
}

struct Chooser {
    sticky: Option<String>, // last chosen source id
    pinned: Option<String>, // chosen by the user (artwork click)
}

impl Chooser {
    /// Prefer what is playing right now. A user pin wins until that source stops while another plays.
    fn pick(&mut self, mgr: &GlobalSystemMediaTransportControlsSessionManager) -> Option<(Session, usize)> {
        let list: Vec<Session> = mgr.GetSessions().ok()?.into_iter().collect();
        let n = list.len();
        if list.is_empty() {
            return mgr.GetCurrentSession().ok().map(|s| (s, 1));
        }
        let playing: Vec<&Session> = list.iter().filter(|s| is_playing(s)).collect();

        if let Some(p) = &self.pinned {
            if let Some(s) = list.iter().find(|s| &sid(s) == p) {
                if is_playing(s) || playing.is_empty() {
                    return Some((s.clone(), n));
                }
            }
            self.pinned = None;
        }
        // among playing sources: keep the sticky one, else the first that has a title, else the first
        if !playing.is_empty() {
            if let Some(st) = &self.sticky {
                if let Some(s) = playing.iter().find(|s| &sid(s) == st) {
                    // keep the current source for as long as it plays; switching on a momentary
                    // missing title is what made two players ping-pong
                    return Some(((*s).clone(), n));
                }
            }
            let chosen = playing.iter().find(|s| has_title(s)).unwrap_or(&playing[0]);
            self.sticky = Some(sid(chosen));
            return Some(((*chosen).clone(), n));
        }
        // nothing playing: system's current session, else the sticky one, else anything with a title
        if let Ok(cur) = mgr.GetCurrentSession() {
            return Some((cur, n));
        }
        if let Some(st) = &self.sticky {
            if let Some(s) = list.iter().find(|s| &sid(s) == st) {
                return Some((s.clone(), n));
            }
        }
        list.iter().find(|s| has_title(s)).or(list.first()).map(|s| (s.clone(), n))
    }

    fn cycle(&mut self, mgr: &GlobalSystemMediaTransportControlsSessionManager) {
        let Ok(list) = mgr.GetSessions() else { return };
        let ids: Vec<String> = list.into_iter().map(|s| sid(&s)).collect();
        if ids.len() < 2 {
            return;
        }
        let cur = self.pinned.clone().or_else(|| self.sticky.clone()).unwrap_or_default();
        let i = ids.iter().position(|x| *x == cur).map(|i| i + 1).unwrap_or(0) % ids.len();
        self.pinned = Some(ids[i].clone());
        self.sticky = Some(ids[i].clone());
    }
}

struct ArtFetch {
    key: String,
    bytes: Vec<u8>,
}

fn worker(state: Arc<Mutex<MediaState>>, rx: Receiver<MediaCmd>) {
    let mgr = loop {
        match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|o| o.get()) {
            Ok(m) => break m,
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    let mut chooser = Chooser { sticky: None, pinned: None };
    let mut last_key = String::new();
    let mut pending_key = String::new();
    let mut was_playing = false;
    let mut art_hash: u64 = 0;
    let mut art_tries: u32 = 0;
    let mut art_next = Instant::now();
    let mut generic_seen: HashMap<u64, HashSet<String>> = HashMap::new();
    let lookup: Arc<Mutex<Option<ArtFetch>>> = Arc::new(Mutex::new(None));
    let mut lookup_for = String::new();

    loop {
        // commands
        while let Ok(cmd) = rx.try_recv() {
            if let MediaCmd::NextSource = cmd {
                chooser.cycle(&mgr);
                last_key.clear(); // force a refresh of title + art for the new source
                continue;
            }
            if let Some((s, _)) = chooser.pick(&mgr) {
                match cmd {
                    MediaCmd::PlayPause => {
                        let _ = s.TryTogglePlayPauseAsync().and_then(|o| o.get());
                    }
                    MediaCmd::Next => {
                        let _ = s.TrySkipNextAsync().and_then(|o| o.get());
                    }
                    MediaCmd::Prev => {
                        let _ = s.TrySkipPreviousAsync().and_then(|o| o.get());
                    }
                    MediaCmd::Seek(sec) => {
                        let _ = s.TryChangePlaybackPositionAsync((sec * 1.0e7) as i64).and_then(|o| o.get());
                        // optimistic update
                        let mut g = state.lock().unwrap();
                        g.position = sec;
                        g.stamp = Instant::now();
                    }
                    MediaCmd::NextSource => {}
                }
            }
        }

        match chooser.pick(&mgr) {
            Some((s, count)) => {
                // every part is optional: a missing timeline must not hide the track
                let props = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()).ok();
                let (title, artist, album) = match &props {
                    Some(p) => (
                        p.Title().map(|h| h.to_string()).unwrap_or_default(),
                        p.Artist().map(|h| h.to_string()).unwrap_or_default(),
                        p.AlbumTitle().map(|h| h.to_string()).unwrap_or_default(),
                    ),
                    None => Default::default(),
                };
                let app = sid(&s);
                let playing = is_playing(&s);
                let (mut pos, mut start, mut end) = (0.0, 0.0, 0.0);
                let mut last_updated_time = 0i64;
                if let Ok(tl) = s.GetTimelineProperties() {
                    pos = tl.Position().map(|t| t.Duration as f64 / 1.0e7).unwrap_or(0.0);
                    start = tl.StartTime().map(|t| t.Duration as f64 / 1.0e7).unwrap_or(0.0);
                    end = tl.EndTime().map(|t| t.Duration as f64 / 1.0e7).unwrap_or(0.0);
                    last_updated_time = tl.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
                }
                let duration = (end - start).max(0.0);

                let key = format!("{app}|{title}|{artist}|{album}");
                // a new track must be seen on two polls in a row (browsers briefly report half-filled metadata)
                if key != last_key && !last_key.is_empty() && pending_key != key {
                    pending_key = key.clone();
                    std::thread::sleep(Duration::from_millis(350));
                    continue;
                }
                pending_key.clear();
                let changed = key != last_key;
                let mut new_art: Option<Option<Vec<u8>>> = None;
                if changed {
                    last_key = key.clone();
                    art_tries = 0;
                    art_hash = 0;
                    art_next = Instant::now();
                    *lookup.lock().unwrap() = None;
                    lookup_for.clear();
                }
                // Browsers often deliver the cover a moment after the title: keep re-reading for a while.
                if let Some(p) = &props {
                    let due = Instant::now() >= art_next && (art_tries < 8 || (art_tries < 40 && art_hash == 0));
                    if due {
                        art_tries += 1;
                        art_next = Instant::now() + Duration::from_millis(if art_tries < 4 { 700 } else { 2500 });
                        let bytes = read_thumbnail(p);
                        let h = bytes.as_ref().map(|b| fnv(b)).unwrap_or(0);
                        if h != art_hash || (changed && art_tries == 1) {
                            art_hash = h;
                            let mut keep = bytes;
                            if let Some(b) = &keep {
                                if is_browser(&app) && looks_generic(b, &mut generic_seen, h, &title) {
                                    // the browser's own logo: ask for real artwork instead
                                    if lookup_for != key {
                                        lookup_for = key.clone();
                                        spawn_lookup(lookup.clone(), key.clone(), title.clone(), artist.clone());
                                    }
                                    if generic_seen.get(&h).map(|s| s.len() >= 2).unwrap_or(false) {
                                        keep = None; // confirmed logo: show a placeholder, not the logo
                                    }
                                }
                            } else if is_browser(&app) && lookup_for != key && !title.is_empty() {
                                lookup_for = key.clone();
                                spawn_lookup(lookup.clone(), key.clone(), title.clone(), artist.clone());
                            }
                            new_art = Some(keep);
                        }
                    }
                }
                // real artwork arrived from the lookup
                if let Some(f) = lookup.lock().unwrap().take() {
                    if f.key == last_key {
                        new_art = Some(Some(f.bytes));
                    }
                }

                let mut g = state.lock().unwrap();
                g.has_session = true;
                g.title = title;
                g.artist = artist;
                g.album = album;
                g.app = app;
                g.playing = playing;
                // Maintain a jitter-free monotonic clock: compensate GSMTC snapshot latency
                // using LastUpdatedTime, and re-anchor on track change, play/pause flip,
                // or when a real seek causes reported position to diverge from our smooth clock.
                let mut reported = (pos - start).max(0.0);
                if playing && last_updated_time > 0 {
                    let unix_dur = std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    let now_ticks = (unix_dur.as_secs() * 10_000_000 + unix_dur.subsec_nanos() as u64 / 100) + 116_444_736_000_000_000;
                    if (now_ticks as i64) >= last_updated_time {
                        let lag = ((now_ticks as i64 - last_updated_time) as f64 / 1.0e7).clamp(0.0, 10.0);
                        reported = (reported + lag).min(duration);
                    }
                }
                let cur_est = g.pos_now();
                if changed || !playing || was_playing != playing || (reported - cur_est).abs() > 0.4 {
                    g.position = reported;
                    g.stamp = Instant::now();
                }
                was_playing = playing;
                g.duration = duration;
                g.sources = count;
                if changed {
                    g.track_ver += 1;
                    if new_art.is_none() {
                        new_art = Some(None);
                    }
                }
                if let Some(a) = new_art {
                    g.art = a.map(std::sync::Arc::new);
                    g.art_ver += 1;
                }
            }
            None => {
                let mut g = state.lock().unwrap();
                if g.has_session {
                    *g = MediaState { track_ver: g.track_ver + 1, art_ver: g.art_ver + 1, ..Default::default() };
                    last_key.clear();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn fnv(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let dec = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        Some((w, h))
    }
}

/// A browser tab without cover art gets the browser's logo: same bytes for every track, and a
/// small square. Real video thumbnails are wide; real music covers are larger.
fn looks_generic(bytes: &[u8], seen: &mut HashMap<u64, HashSet<String>>, h: u64, title: &str) -> bool {
    let set = seen.entry(h).or_default();
    set.insert(title.to_string());
    if set.len() >= 2 {
        return true;
    }
    match image_size(bytes) {
        Some((w, hh)) => w == hh && w <= 300,
        None => false,
    }
}

fn spawn_lookup(slot: Arc<Mutex<Option<ArtFetch>>>, key: String, title: String, artist: String) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if let Ok(Some(bytes)) = fetch_cover(&title, &artist) {
            *slot.lock().unwrap() = Some(ArtFetch { key, bytes });
        }
    });
}

/// Cover art lookup (iTunes Search API, no key). Only accepted when the track name matches.
fn fetch_cover(title: &str, artist: &str) -> Result<Option<Vec<u8>>> {
    let mut t = crate::lyrics::clean_title(title);
    if let Some(i) = t.find('|') {
        t.truncate(i);
    }
    for sep in [" ft.", " feat.", " ft ", " feat "] {
        if let Some(i) = t.to_lowercase().find(sep) {
            t.truncate(i);
        }
    }
    let t = t.trim().to_string();
    let a = artist.trim().trim_end_matches("VEVO").trim_end_matches(" - Topic").trim().to_string();
    let (term, want) = match t.split_once(" - ") {
        Some((l, r)) => (format!("{} {}", l.trim(), r.trim()), r.trim().to_lowercase()),
        None => (format!("{a} {t}"), t.to_lowercase()),
    };
    if want.is_empty() {
        return Ok(None);
    }
    let client = HttpClient::new()?;
    let _ = client.DefaultRequestHeaders()?.UserAgent()?.TryParseAdd(h!("Nook/0.1"));
    let url = format!("https://itunes.apple.com/search?media=music&entity=song&limit=5&term={}", crate::lyrics::enc(&term));
    let body = client.GetStringAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.get()?;
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body.to_string()) else { return Ok(None) };
    let Some(arr) = v["results"].as_array() else { return Ok(None) };
    for it in arr {
        let name = it["trackName"].as_str().unwrap_or("").to_lowercase();
        if name.is_empty() || !(want.contains(&name) || name.contains(&want)) {
            continue;
        }
        let Some(art) = it["artworkUrl100"].as_str() else { continue };
        let big = art.replace("100x100", "600x600");
        let buf = client.GetBufferAsync(&Uri::CreateUri(&HSTRING::from(big))?)?.get()?;
        let reader = DataReader::FromBuffer(&buf)?;
        let mut out = vec![0u8; buf.Length()? as usize];
        reader.ReadBytes(&mut out)?;
        if !out.is_empty() {
            return Ok(Some(out));
        }
    }
    Ok(None)
}

fn read_thumbnail(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Vec<u8>> {
    let r = props.Thumbnail().ok()?;
    let stream = r.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 || size > 20_000_000 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_lookup_for_browser_titles() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        for (t, a) in [("Drake - One Dance (Lyrics) ft. Wizkid & Kyla", "Billion Stars"), ("Libianca - People ft. Ayra Starr, Omah Lay", "LatinHype")] {
            let r = fetch_cover(t, a).expect("http");
            let n = r.as_ref().map(|b| b.len()).unwrap_or(0);
            println!("{t} -> {n} bytes");
            assert!(n > 1000, "no cover for {t}");
            assert!(image_size(r.as_ref().unwrap()).is_some());
        }
    }
}
