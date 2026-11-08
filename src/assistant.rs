//! Voice + text assistant for the island.
//!
//! mic -> Windows speech recognition -> this engine -> (local Windows actions | AI provider) -> reply
//! -> Windows SAPI voice. Providers: Groq (OpenAI-compatible), Google Gemini, local Ollama.
//! The engine runs on worker threads and publishes a small state the UI reads every frame.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::*;
use windows::Foundation::Uri;
use windows::Storage::Streams::UnicodeEncoding;
use windows::Web::Http::{HttpClient, HttpStringContent, IHttpContent};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Idle,
    Listening,
    Thinking,
    Speaking,
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub user: bool,
    pub text: String,
}

pub struct State {
    pub msgs: Vec<Msg>,
    pub status: Status,
    pub err: String,
}

/// Everything the engine needs from the settings, copied at call time.
#[derive(Clone, Default)]
pub struct Cfg {
    pub provider: u32,
    pub key: String,
    pub model: String,
    pub url: String,
    pub voice: bool,
    pub voice_name: String,
    pub neural: bool,
    pub rate: u32,
    pub context: String, // time, now playing, next prayer… for the system prompt
}

pub const PROVIDERS: [&str; 8] = [
    "Groq",
    "Gemini",
    "OpenRouter",
    "DeepSeek",
    "Mistral",
    "Together",
    "Ollama",
    "LM Studio",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub is_free: bool,
}

static CACHED_PROVIDER: Mutex<Option<(u32, String, String, Vec<ModelChoice>)>> = Mutex::new(None);
static IS_FETCHING_MODELS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone)]
pub struct Assistant {
    pub st: Arc<Mutex<State>>,
}

/// A cheap copy for drawing.
pub struct View {
    pub status: Status,
    pub user: String,
    pub reply: String,
    pub err: String,
    pub count: usize,
}

impl Assistant {
    pub fn new() -> Self {
        Self { st: Arc::new(Mutex::new(State { msgs: vec![], status: Status::Idle, err: String::new() })) }
    }

    pub fn view(&self) -> View {
        let g = self.st.lock().unwrap();
        let last = |user: bool| g.msgs.iter().rev().find(|m| m.user == user).map(|m| m.text.clone()).unwrap_or_default();
        View { status: g.status, user: last(true), reply: last(false), err: g.err.clone(), count: g.msgs.len() }
    }

    pub fn status(&self) -> Status {
        self.st.lock().unwrap().status
    }

    pub fn clear(&self) {
        let mut g = self.st.lock().unwrap();
        g.msgs.clear();
        g.err.clear();
    }

    fn set(&self, s: Status) {
        self.st.lock().unwrap().status = s;
    }

    /// Typed message (or recognised speech).
    pub fn send(&self, text: String, cfg: Cfg) {
        let text = text.trim().to_string();
        if text.is_empty() || self.status() == Status::Thinking {
            return;
        }
        {
            let mut g = self.st.lock().unwrap();
            g.msgs.push(Msg { user: true, text: text.clone() });
            g.err.clear();
            g.status = Status::Thinking;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let reply = match local_action(&text) {
                Some(r) => r,
                None => match ask(&me, &cfg) {
                    Ok(r) => r,
                    Err(e) => {
                        me.st.lock().unwrap().err = e.clone();
                        format!("I couldn't answer: {e}")
                    }
                },
            };
            // Deliver the finished answer immediately so text is visible and complete
            {
                let mut g = me.st.lock().unwrap();
                g.msgs.push(Msg { user: false, text: reply.clone() });
                g.status = Status::Idle;
            }
            if cfg.voice {
                // Natural ~0.25s pause before stream dispatch (first audio packet arrives at ~0.5s)
                std::thread::sleep(Duration::from_millis(250));
                me.set(Status::Speaking);
                speak_cfg(&reply, &cfg);
                me.set(Status::Idle);
            }
        });
    }

    /// Mic: listen once, then handle what was said as a message.
    pub fn listen(&self, cfg: Cfg) {
        if self.status() != Status::Idle {
            return;
        }
        self.set(Status::Listening);
        let me = self.clone();
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            match recognise() {
                Ok(t) if !t.trim().is_empty() => {
                    me.set(Status::Idle);
                    me.send(t, cfg);
                }
                Ok(_) => {
                    me.st.lock().unwrap().err = "I didn't hear anything.".into();
                    me.set(Status::Idle);
                }
                Err(e) => {
                    me.st.lock().unwrap().err = e;
                    me.set(Status::Idle);
                }
            }
        });
    }
}

// ------------------------------------------------------------------ speech in / out
fn recognise() -> std::result::Result<String, String> {
    use windows::Media::SpeechRecognition::{SpeechRecognitionResultStatus, SpeechRecognizer};
    let explain = |e: Error| -> String {
        match e.code().0 as u32 {
            0x80045509 => "Turn on Online speech recognition in Windows Settings > Privacy > Speech.".to_string(),
            0x80070005 => "Microphone access is blocked. Allow it in Windows Settings > Privacy > Microphone.".to_string(),
            _ => format!("Speech recognition failed ({}).", e.message().to_string().trim()),
        }
    };
    let rec = SpeechRecognizer::new().map_err(explain)?;
    let comp = rec.CompileConstraintsAsync().map_err(explain)?.get().map_err(explain)?;
    if comp.Status().map_err(explain)? != SpeechRecognitionResultStatus::Success {
        return Err("Speech recognition is not available on this PC.".into());
    }
    let res = rec.RecognizeAsync().map_err(explain)?.get().map_err(explain)?;
    Ok(res.Text().map(|t| t.to_string()).unwrap_or_default())
}

/// Installed Windows voices (modern speech engine, includes "Natural" voices when installed).
pub fn voices() -> Vec<String> {
    static V: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        use windows::Media::SpeechSynthesis::SpeechSynthesizer;
        let mut out = vec!["Cloud: Natural (Free)".to_string()];
        if let Ok(list) = SpeechSynthesizer::AllVoices() {
            for v in list {
                if let Ok(n) = v.DisplayName() {
                    out.push(n.to_string());
                }
            }
        }
        if !out.iter().any(|v| v.contains("Zira")) {
            out.push("Microsoft Zira Desktop".to_string());
        }
        out
    })
    .clone()
}

static STOPPER: Mutex<Option<std::sync::mpsc::Sender<()>>> = Mutex::new(None);

/// Cut the voice off.
pub fn stop_speaking() {
    if let Some(tx) = STOPPER.lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

/// Speak with the chosen Windows voice at `rate` percent; synthesis and playback happen on the caller's
/// worker thread (never the UI thread), the engine is created per answer and released afterwards.
/// Falls back to the classic SAPI voice if the modern engine is unavailable.
pub fn speak(text: &str, voice: &str, rate: u32) {
    let clean: String = text.chars().filter(|c| !matches!(c, '*' | '`' | '#' | '_')).collect();
    if voice.contains("Desktop") || (voice.is_empty() && is_english(&clean)) {
        if speak_sapi_voice(&clean, voice, rate).is_ok() {
            return;
        }
    }
    if speak_modern(&clean, voice, rate).is_err() {
        let _ = speak_sapi_voice(&clean, voice, rate);
    }
}

fn is_english(text: &str) -> bool {
    let ascii_count = text.chars().filter(|c| c.is_ascii()).count();
    let total = text.chars().count().max(1);
    (ascii_count as f32 / total as f32) > 0.85
}

fn speak_sapi_voice(text: &str, voice_name: &str, rate: u32) -> Result<()> {
    use windows::Win32::Media::Speech::{ISpVoice, SpVoice, ISpObjectTokenCategory, SpObjectTokenCategory};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    unsafe {
        let v: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
        let r = ((rate as i32 - 100) / 10).clamp(-10, 10);
        let _ = v.SetRate(r);

        if let Ok(cat) = CoCreateInstance::<_, ISpObjectTokenCategory>(&SpObjectTokenCategory, None, CLSCTX_ALL) {
            let cat_id = windows::core::w!("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices");
            if cat.SetId(cat_id, false).is_ok() {
                if let Ok(enums) = cat.EnumTokens(None, None) {
                    let mut count = 0u32;
                    if enums.GetCount(&mut count).is_ok() {
                        let target_key = if voice_name.contains("Zira") || (voice_name.is_empty() && is_english(text)) {
                            "ZIRA"
                        } else if voice_name.contains("Helena") {
                            "HELENA"
                        } else {
                            ""
                        };
                        if !target_key.is_empty() {
                            for i in 0..count {
                                if let Ok(tok) = enums.Item(i) {
                                    if let Ok(id_ptr) = tok.GetId() {
                                        let id_str = id_ptr.to_string().unwrap_or_default().to_uppercase();
                                        windows::Win32::System::Com::CoTaskMemFree(Some(id_ptr.0 as *const _));
                                        if id_str.contains(target_key) {
                                            let _ = v.SetVoice(&tok);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let _ = v.Speak(windows::core::PCWSTR(wide.as_ptr()), 0, None);
        Ok(())
    }
}

fn speak_modern(text: &str, voice: &str, rate: u32) -> Result<()> {
    use windows::Media::Core::MediaSource;
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    let synth = SpeechSynthesizer::new()?;
    if !voice.is_empty() {
        for v in SpeechSynthesizer::AllVoices()? {
            if v.DisplayName()? == voice {
                synth.SetVoice(&v)?;
                break;
            }
        }
    }
    let _ = synth.Options().and_then(|o| o.SetSpeakingRate((rate.clamp(50, 250) as f64) / 100.0));
    let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.get()?;
    let src = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
    play_source(&src)
}

/// Play an audio source on a fresh player until it ends or `stop_speaking` is called.
fn play_source(src: &windows::Media::Core::MediaSource) -> Result<()> {
    use windows::core::IInspectable;
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Playback::MediaPlayer;
    let player = MediaPlayer::new()?;
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *STOPPER.lock().unwrap() = Some(tx.clone());
    let done = tx.clone();
    player.MediaEnded(&TypedEventHandler::<MediaPlayer, IInspectable>::new(move |_, _| {
        let _ = done.send(());
        Ok(())
    }))?;
    player.SetSource(src)?;
    player.Play()?;
    let _ = rx.recv_timeout(Duration::from_secs(240));
    let _ = player.Pause();
    let _ = player.Close();
    *STOPPER.lock().unwrap() = None;
    Ok(())
}

pub fn play_audio_stream(bytes: &[u8], content_type: &str) -> Result<()> {
    use windows::Media::Core::MediaSource;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    let stream = InMemoryRandomAccessStream::new()?;
    let w = DataWriter::CreateDataWriter(&stream)?;
    w.WriteBytes(bytes)?;
    w.StoreAsync()?.get()?;
    w.DetachStream()?;
    stream.Seek(0)?;
    let src = MediaSource::CreateFromStream(&stream, &HSTRING::from(content_type))?;
    play_source(&src)
}

fn wav_from_pcm16(pcm: &[u8], rate: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(pcm.len() + 44);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    v.extend_from_slice(pcm);
    v
}

fn b64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        if let Some(v) = val(c) {
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
    }
    out
}

fn neural_wav(text: &str, cfg: &Cfg) -> std::result::Result<Vec<u8>, String> {
    let text: String = text.chars().take(1200).collect();
    match cfg.provider {
        1 => {
            let body = serde_json::json!({
                "contents": [{"parts": [{"text": text}]}],
                "generationConfig": {"responseModalities": ["AUDIO"], "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {"voiceName": "Kore"}}}}
            });
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent?key={}", cfg.key.trim());
            let v = post_json(&url, &[], &body.to_string())?;
            let data = v["candidates"][0]["content"]["parts"][0]["inlineData"]["data"].as_str().ok_or_else(|| api_error(&v))?;
            let pcm = b64_decode(data);
            if pcm.len() < 2000 {
                return Err("empty audio".into());
            }
            Ok(wav_from_pcm16(&pcm, 24000))
        }
        0 => {
            let body = serde_json::json!({"model": "playai-tts", "voice": "Fritz-PlayAI", "input": text, "response_format": "wav"});
            post_bytes("https://api.groq.com/openai/v1/audio/speech", &[("Authorization", format!("Bearer {}", cfg.key.trim()))], &body.to_string())
        }
        _ => Err("this provider has no voice endpoint".into()),
    }
}

/// Natural AI voice from provider API (Gemini / Groq) or immediate local Windows speech.
fn post_bytes(url: &str, headers: &[(&str, String)], body: &str) -> std::result::Result<Vec<u8>, String> {
    let (url, body) = (url.to_string(), body.to_string());
    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<(bool, Vec<u8>)> {
            use windows::Storage::Streams::{DataReader, UnicodeEncoding};
            use windows::Web::Http::{HttpClient, HttpStringContent, IHttpContent};
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            for (k, v) in &headers {
                hd.Append(&HSTRING::from(k.as_str()), &HSTRING::from(v.as_str()))?;
            }
            let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(&HSTRING::from(body), UnicodeEncoding::Utf8, &HSTRING::from("application/json"))?;
            let ic: IHttpContent = content.cast()?;
            let resp = client.PostAsync(&windows::Foundation::Uri::CreateUri(&HSTRING::from(url))?, &ic)?.get()?;
            let ok = resp.IsSuccessStatusCode()?;
            let buf = resp.Content()?.ReadAsBufferAsync()?.get()?;
            let reader = DataReader::FromBuffer(&buf)?;
            let mut out = vec![0u8; buf.Length()? as usize];
            reader.ReadBytes(&mut out)?;
            Ok((ok, out))
        })();
        let _ = tx.send(r.map_err(|e| format!("network error ({})", e.message())));
    });
    match rx.recv_timeout(Duration::from_secs(45)) {
        Ok(Ok((true, b))) if b.len() > 1000 => Ok(b),
        Ok(Ok((_, b))) => Err(serde_json::from_slice::<serde_json::Value>(&b).map(|v| api_error(&v)).unwrap_or_else(|_| "bad audio response".into())),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("timed out".into()),
    }
}

fn split_first_sentence(text: &str) -> (String, String) {
    let text = text.trim();
    let mut end_idx = None;
    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?') && i >= 12 {
            end_idx = Some(i + c.len_utf8());
            break;
        }
        if c == '\n' && i >= 12 {
            end_idx = Some(i);
            break;
        }
    }
    if let Some(idx) = end_idx {
        let first = text[..idx].trim().to_string();
        let rest = text[idx..].trim().to_string();
        (first, rest)
    } else if text.len() > 140 {
        let break_point = text[..140].rfind(' ').unwrap_or(140);
        (text[..break_point].trim().to_string(), text[break_point..].trim().to_string())
    } else {
        (text.to_string(), String::new())
    }
}

fn fetch_cloud_chunk(text: &str) -> std::result::Result<Vec<u8>, String> {
    let enc: String = text.chars().take(220).map(|c| match c {
        ' ' => "%20".to_string(),
        'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        _ => format!("%{:02X}", c as u32),
    }).collect();
    let url = format!("https://translate.google.com/translate_tts?ie=UTF-8&tl=en&client=tw-ob&q={enc}");
    get_raw_bytes(&url)
}

fn get_raw_bytes(url: &str) -> std::result::Result<Vec<u8>, String> {
    let url = url.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<(bool, Vec<u8>)> {
            use windows::Foundation::Uri;
            use windows::Storage::Streams::DataReader;
            use windows::Web::Http::HttpClient;
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            let _ = hd.UserAgent()?.TryParseAdd(&HSTRING::from("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
            let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.get()?;
            let ok = resp.IsSuccessStatusCode()?;
            let buf = resp.Content()?.ReadAsBufferAsync()?.get()?;
            let reader = DataReader::FromBuffer(&buf)?;
            let mut out = vec![0u8; buf.Length()? as usize];
            reader.ReadBytes(&mut out)?;
            Ok((ok, out))
        })();
        let _ = tx.send(r.map_err(|e| format!("network error ({})", e.message())));
    });
    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok((true, b))) if !b.is_empty() => Ok(b),
        Ok(Ok((_, _))) => Err("http error".into()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("timed out".into()),
    }
}

pub fn speak_cloud_streaming(text: &str) -> Result<()> {
    let (first, rest) = split_first_sentence(text);
    if first.is_empty() {
        return Ok(());
    }
    let first_bytes = fetch_cloud_chunk(&first).map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e))?;
    let rest_thread = if !rest.is_empty() {
        Some(std::thread::spawn(move || {
            fetch_cloud_chunk(&rest)
        }))
    } else {
        None
    };

    play_audio_stream(&first_bytes, "audio/mpeg")?;

    if let Some(h) = rest_thread {
        if let Ok(Ok(rest_bytes)) = h.join() {
            let _ = play_audio_stream(&rest_bytes, "audio/mpeg");
        }
    }
    Ok(())
}

pub fn speak_cfg(text: &str, cfg: &Cfg) {
    let clean: String = text.chars().filter(|c| !matches!(c, '*' | '`' | '#' | '_')).collect();
    if cfg.neural {
        if (cfg.provider == 0 || cfg.provider == 1) && !cfg.key.trim().is_empty() {
            if let Ok(wav) = neural_wav(&clean, cfg) {
                if play_audio_stream(&wav, "audio/wav").is_ok() {
                    return;
                }
            }
        }
        if cfg.voice_name.contains("Cloud") || cfg.voice_name.is_empty() {
            if speak_cloud_streaming(&clean).is_ok() {
                return;
            }
        }
    } else if cfg.voice_name.contains("Cloud") {
        if speak_cloud_streaming(&clean).is_ok() {
            return;
        }
    }
    speak(text, &cfg.voice_name, cfg.rate);
}

// ------------------------------------------------------------------ Windows actions (no AI needed)
fn local_action(raw: &str) -> Option<String> {
    use crate::sys;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let t = raw.to_lowercase();
    let t = t.trim().trim_end_matches(|c: char| ".!?".contains(c)).to_string();
    // commands are short ("mute", "next song"); long sentences go to the AI so "commute" never mutes anything
    let short = t.split_whitespace().count() <= 5;
    let has = |ks: &[&str]| short && ks.iter().any(|k| t.split_whitespace().collect::<Vec<_>>().join(" ").contains(k) && (k.contains(' ') || t.split_whitespace().any(|w| w == *k)));

    if t == "what time is it" || t == "time" || t == "what's the time" || t == "what is the time" {
        let n = sys::now();
        return Some(format!("It's {:02}:{:02}.", n.hour, n.min));
    }
    if t == "what day is it" || t == "date" || t == "what's the date" || t == "what is today" {
        let n = sys::now();
        return Some(format!("Today is {} {} {} {}.", sys::DOW[n.dow as usize], n.day, sys::MON_FULL[(n.month - 1) as usize], n.year));
    }
    if has(&["volume up", "louder", "turn it up"]) {
        sys::key_tap(VK_VOLUME_UP);
        sys::key_tap(VK_VOLUME_UP);
        sys::key_tap(VK_VOLUME_UP);
        return Some("Volume up.".into());
    }
    if has(&["volume down", "quieter", "turn it down"]) {
        sys::key_tap(VK_VOLUME_DOWN);
        sys::key_tap(VK_VOLUME_DOWN);
        sys::key_tap(VK_VOLUME_DOWN);
        return Some("Volume down.".into());
    }
    if has(&["unmute", "mute"]) {
        sys::key_tap(VK_VOLUME_MUTE);
        return Some("Toggled mute.".into());
    }
    if let Some(i) = t.find("volume to ") {
        let n: String = t[i + 10..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = n.parse::<f32>() {
            sys::set_volume((v / 100.0).clamp(0.0, 1.0));
            return Some(format!("Volume set to {}%.", v.clamp(0.0, 100.0) as u32));
        }
    }
    if has(&["next song", "next track", "skip"]) {
        sys::key_tap(VK_MEDIA_NEXT_TRACK);
        return Some("Next track.".into());
    }
    if has(&["previous song", "previous track", "last song", "go back a song"]) {
        sys::key_tap(VK_MEDIA_PREV_TRACK);
        return Some("Previous track.".into());
    }
    if t == "pause" || t == "play" || t == "resume" || has(&["pause the music", "pause music", "play music", "resume music", "stop the music"]) {
        sys::key_tap(VK_MEDIA_PLAY_PAUSE);
        return Some("Done.".into());
    }
    // search / open (only websites and a short list of built-in apps)
    for p in ["search for ", "google ", "search "] {
        if let Some(q) = t.strip_prefix(p) {
            sys::open_url(&format!("https://www.google.com/search?q={}", crate::lyrics::enc(q)));
            return Some(format!("Searching for {q}."));
        }
    }
    if let Some(what) = t.strip_prefix("open ").or_else(|| t.strip_prefix("launch ")).or_else(|| t.strip_prefix("start ")) {
        let what = what.trim().trim_start_matches("the ").trim_end_matches(" app");
        let site = |u: &str| {
            sys::open_url(u);
            Some(format!("Opening {what}."))
        };
        return match what {
            "youtube" => site("https://www.youtube.com"),
            "google" => site("https://www.google.com"),
            "gmail" | "mail" | "email" => site("https://mail.google.com"),
            "github" => site("https://github.com"),
            "whatsapp" => site("https://web.whatsapp.com"),
            "spotify" => site("https://open.spotify.com"),
            "netflix" => site("https://www.netflix.com"),
            "maps" | "google maps" => site("https://maps.google.com"),
            "settings" | "windows settings" => site("ms-settings:"),
            "notepad" => launch("notepad.exe", what),
            "calculator" | "calc" => launch("calc.exe", what),
            "paint" => launch("mspaint.exe", what),
            "explorer" | "files" | "file explorer" => launch("explorer.exe", what),
            "task manager" => launch("taskmgr.exe", what),
            "command prompt" | "cmd" | "terminal" => launch("cmd.exe", what),
            _ => None, // anything else goes to the AI instead of being executed
        };
    }
    None
}

fn launch(exe: &str, what: &str) -> Option<String> {
    crate::sys::open_url(exe);
    Some(format!("Opening {what}."))
}

// ------------------------------------------------------------------ AI providers
fn system_prompt(ctx: &str) -> String {
    format!("You are Nook, a friendly voice assistant that lives in a Dynamic Island on the user's Windows PC. Reply in plain text only (no markdown, no lists unless asked), in one to three short sentences unless the user asks for more. Context: {ctx}")
}

fn ask(me: &Assistant, cfg: &Cfg) -> std::result::Result<String, String> {
    let history: Vec<Msg> = {
        let g = me.st.lock().unwrap();
        let n = g.msgs.len();
        g.msgs[n.saturating_sub(12)..].to_vec()
    };
    let sys_prompt = system_prompt(&cfg.context);
    let openai_msgs = || -> Vec<serde_json::Value> {
        let mut v = vec![serde_json::json!({"role": "system", "content": sys_prompt})];
        for m in &history {
            v.push(serde_json::json!({"role": if m.user { "user" } else { "assistant" }, "content": m.text}));
        }
        v
    };
    let model = |d: &str| if cfg.model.trim().is_empty() { d.to_string() } else { cfg.model.trim().to_string() };
    let text = match cfg.provider {
        1 => {
            // Google Gemini
            if cfg.key.trim().is_empty() {
                return Err("Add your Gemini API key in Settings > Assistant.".into());
            }
            let contents: Vec<_> = history.iter().map(|m| serde_json::json!({"role": if m.user { "user" } else { "model" }, "parts": [{"text": m.text}]})).collect();
            let body = serde_json::json!({"systemInstruction": {"parts": [{"text": sys_prompt}]}, "contents": contents});
            let base = if cfg.url.trim().is_empty() { "https://generativelanguage.googleapis.com/v1beta" } else { cfg.url.trim().trim_end_matches('/') };
            let url = format!("{base}/models/{}:generateContent?key={}", model("gemini-2.0-flash"), cfg.key.trim());
            let v = post_json(&url, &[], &body.to_string())?;
            v["candidates"][0]["content"]["parts"][0]["text"].as_str().map(|s| s.to_string()).ok_or_else(|| api_error(&v))?
        }
        6 => {
            // Ollama
            let base = if cfg.url.trim().is_empty() { "http://localhost:11434".to_string() } else { cfg.url.trim().trim_end_matches('/').to_string() };
            let body = serde_json::json!({"model": model("llama3.2"), "messages": openai_msgs(), "stream": false});
            let v = post_json(&format!("{base}/api/chat"), &[], &body.to_string())?;
            v["message"]["content"].as_str().map(|s| s.to_string()).ok_or_else(|| api_error(&v))?
        }
        _ => {
            // OpenAI-compatible providers: Groq (0), OpenRouter (2), DeepSeek (3), Mistral (4), Together (5), LM Studio (7)
            let (def_url, def_model, need_key, prov_name) = match cfg.provider {
                0 => ("https://api.groq.com/openai/v1", "llama-3.3-70b-versatile", true, "Groq"),
                2 => ("https://openrouter.ai/api/v1", "meta-llama/llama-3.3-70b-instruct:free", true, "OpenRouter"),
                3 => ("https://api.deepseek.com/v1", "deepseek-chat", true, "DeepSeek"),
                4 => ("https://api.mistral.ai/v1", "mistral-small-latest", true, "Mistral"),
                5 => ("https://api.together.xyz/v1", "meta-llama/Llama-3.3-70B-Instruct-Turbo", true, "Together"),
                _ => ("http://localhost:1234/v1", "local-model", false, "LM Studio"),
            };
            if need_key && cfg.key.trim().is_empty() {
                return Err(format!("Add your {prov_name} API key in Settings > Assistant."));
            }
            let base = if cfg.url.trim().is_empty() { def_url.to_string() } else { cfg.url.trim().trim_end_matches('/').to_string() };
            let auth = if cfg.key.trim().is_empty() { "Bearer none".to_string() } else { format!("Bearer {}", cfg.key.trim()) };
            let mut headers = vec![("Authorization", auth)];
            if cfg.provider == 2 {
                headers.push(("HTTP-Referer", "https://github.com/Anassninab/nook".to_string()));
                headers.push(("X-Title", "Nook".to_string()));
            }
            let body = serde_json::json!({
                "model": model(def_model),
                "messages": openai_msgs(),
                "temperature": 0.6,
                "max_tokens": 450
            });
            let v = post_json(&format!("{base}/chat/completions"), &headers, &body.to_string())?;
            v["choices"][0]["message"]["content"].as_str().map(|s| s.to_string()).ok_or_else(|| api_error(&v))?
        }
    };
    Ok(tidy(&text))
}

fn api_error(v: &serde_json::Value) -> String {
    v["error"]["message"].as_str().or(v["error"].as_str()).map(|s| s.chars().take(160).collect()).unwrap_or_else(|| "The AI returned no answer.".into())
}

/// Plain text for a small display and for the voice.
fn tidy(s: &str) -> String {
    s.replace("**", "").replace('`', "").replace("\n\n", "\n").trim().to_string()
}

pub fn detect_defaults(provider: u32) -> Vec<ModelChoice> {
    match provider {
        0 => vec![
            ModelChoice { id: "llama-3.3-70b-versatile".into(), label: "llama-3.3-70b-versatile [Free Tier]".into(), is_free: true },
            ModelChoice { id: "llama-3.1-8b-instant".into(), label: "llama-3.1-8b-instant [Free Tier]".into(), is_free: true },
            ModelChoice { id: "deepseek-r1-distill-llama-70b".into(), label: "deepseek-r1-distill-llama-70b [Free Tier]".into(), is_free: true },
            ModelChoice { id: "mixtral-8x7b-32768".into(), label: "mixtral-8x7b-32768 [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemma2-9b-it".into(), label: "gemma2-9b-it [Free Tier]".into(), is_free: true },
        ],
        1 => vec![
            ModelChoice { id: "gemini-2.0-flash".into(), label: "gemini-2.0-flash [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemini-2.5-flash".into(), label: "gemini-2.5-flash [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemini-1.5-flash".into(), label: "gemini-1.5-flash [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemini-1.5-flash-8b".into(), label: "gemini-1.5-flash-8b [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemini-2.0-flash-thinking-exp".into(), label: "gemini-2.0-flash-thinking-exp [Free Tier]".into(), is_free: true },
            ModelChoice { id: "gemini-1.5-pro".into(), label: "gemini-1.5-pro [Free Tier]".into(), is_free: true },
        ],
        2 => vec![
            ModelChoice { id: "meta-llama/llama-3.3-70b-instruct:free".into(), label: "meta-llama/llama-3.3-70b-instruct:free [FREE]".into(), is_free: true },
            ModelChoice { id: "deepseek/deepseek-r1:free".into(), label: "deepseek/deepseek-r1:free [FREE]".into(), is_free: true },
            ModelChoice { id: "deepseek/deepseek-chat:free".into(), label: "deepseek/deepseek-chat:free [FREE]".into(), is_free: true },
            ModelChoice { id: "google/gemini-2.0-flash-exp:free".into(), label: "google/gemini-2.0-flash-exp:free [FREE]".into(), is_free: true },
            ModelChoice { id: "qwen/qwen-2.5-coder-32b-instruct:free".into(), label: "qwen/qwen-2.5-coder-32b-instruct:free [FREE]".into(), is_free: true },
            ModelChoice { id: "mistralai/mistral-7b-instruct:free".into(), label: "mistralai/mistral-7b-instruct:free [FREE]".into(), is_free: true },
            ModelChoice { id: "nvidia/nemotron-3.5-super-vl:free".into(), label: "nvidia/nemotron-3.5-super-vl:free [FREE]".into(), is_free: true },
            ModelChoice { id: "liquid/lfm-3-1.7b:free".into(), label: "liquid/lfm-3-1.7b:free [FREE]".into(), is_free: true },
        ],
        3 => vec![
            ModelChoice { id: "deepseek-chat".into(), label: "deepseek-chat (V3)".into(), is_free: false },
            ModelChoice { id: "deepseek-reasoner".into(), label: "deepseek-reasoner (R1)".into(), is_free: false },
        ],
        4 => vec![
            ModelChoice { id: "mistral-small-latest".into(), label: "mistral-small-latest".into(), is_free: false },
            ModelChoice { id: "codestral-latest".into(), label: "codestral-latest".into(), is_free: false },
            ModelChoice { id: "open-mistral-7b".into(), label: "open-mistral-7b [Free Tier]".into(), is_free: true },
        ],
        5 => vec![
            ModelChoice { id: "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(), label: "Llama-3.3-70B-Turbo".into(), is_free: false },
            ModelChoice { id: "mistralai/Mixtral-8x7B-Instruct-v0.1".into(), label: "Mixtral-8x7B".into(), is_free: false },
        ],
        6 => vec![
            ModelChoice { id: "llama3.2".into(), label: "llama3.2 [Local Free]".into(), is_free: true },
            ModelChoice { id: "llama3.1".into(), label: "llama3.1 [Local Free]".into(), is_free: true },
            ModelChoice { id: "deepseek-r1".into(), label: "deepseek-r1 [Local Free]".into(), is_free: true },
            ModelChoice { id: "mistral".into(), label: "mistral [Local Free]".into(), is_free: true },
            ModelChoice { id: "qwen2.5".into(), label: "qwen2.5 [Local Free]".into(), is_free: true },
        ],
        _ => vec![
            ModelChoice { id: "local-model".into(), label: "local-model [Local Free]".into(), is_free: true },
        ],
    }
}

pub fn detect_models(provider: u32, key: &str, url: &str) -> Vec<ModelChoice> {
    let key = key.trim();
    let url = url.trim();
    match provider {
        0 => {
            if !key.is_empty() {
                let base = if url.is_empty() { "https://api.groq.com/openai/v1" } else { url.trim_end_matches('/') };
                let auth = format!("Bearer {key}");
                if let Ok(v) = get_json(&format!("{base}/models"), &[("Authorization", auth)]) {
                    if let Some(arr) = v["data"].as_array() {
                        let mut models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                            let id = m["id"].as_str()?.to_string();
                            let active = m["active"].as_bool().unwrap_or(true);
                            if !active || id.contains("whisper") {
                                return None;
                            }
                            Some(ModelChoice {
                                id: id.clone(),
                                label: format!("{id} [Free Tier]"),
                                is_free: true,
                            })
                        }).collect();
                        if !models.is_empty() {
                            models.sort_by_key(|m| {
                                if m.id.contains("llama-3.3-70b") { 0 }
                                else if m.id.contains("llama-3.1-8b") { 1 }
                                else if m.id.contains("deepseek-r1") { 2 }
                                else if m.id.contains("mixtral") { 3 }
                                else if m.id.contains("gemma") { 4 }
                                else { 10 }
                            });
                            return models;
                        }
                    }
                }
            }
            detect_defaults(0)
        }
        1 => {
            if !key.is_empty() {
                let base = if url.is_empty() { "https://generativelanguage.googleapis.com/v1beta" } else { url.trim_end_matches('/') };
                if let Ok(v) = get_json(&format!("{base}/models?key={key}"), &[]) {
                    if let Some(arr) = v["models"].as_array() {
                        let mut models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                            let raw_name = m["name"].as_str()?;
                            let id = raw_name.trim_start_matches("models/").to_string();
                            let methods = m["supportedGenerationMethods"].as_array()?;
                            let can_generate = methods.iter().any(|method| method.as_str() == Some("generateContent"));
                            if !can_generate || id.contains("embedding") || id.contains("aqa") || id.contains("imagen") {
                                return None;
                            }
                            Some(ModelChoice {
                                id: id.clone(),
                                label: format!("{id} [Free Tier]"),
                                is_free: true,
                            })
                        }).collect();
                        if !models.is_empty() {
                            models.sort_by_key(|m| {
                                if m.id == "gemini-2.0-flash" { 0 }
                                else if m.id == "gemini-2.5-flash" { 1 }
                                else if m.id == "gemini-1.5-flash" { 2 }
                                else if m.id == "gemini-1.5-flash-8b" { 3 }
                                else if m.id.contains("flash") { 4 }
                                else if m.id.contains("pro") { 5 }
                                else { 10 }
                            });
                            return models;
                        }
                    }
                }
            }
            detect_defaults(1)
        }
        2 => {
            let base = if url.is_empty() { "https://openrouter.ai/api/v1" } else { url.trim_end_matches('/') };
            let mut headers = vec![];
            if !key.is_empty() {
                headers.push(("Authorization", format!("Bearer {key}")));
            }
            if let Ok(v) = get_json(&format!("{base}/models"), &headers) {
                if let Some(arr) = v["data"].as_array() {
                    let mut free_models: Vec<ModelChoice> = Vec::new();
                    let mut paid_models: Vec<ModelChoice> = Vec::new();
                    for m in arr {
                        let Some(id) = m["id"].as_str() else { continue };
                        let is_free = id.ends_with(":free") || (m["pricing"]["prompt"].as_str() == Some("0") && m["pricing"]["completion"].as_str() == Some("0"));
                        let item = ModelChoice {
                            id: id.to_string(),
                            label: if is_free { format!("{id} [FREE]") } else { id.to_string() },
                            is_free,
                        };
                        if is_free {
                            free_models.push(item);
                        } else {
                            paid_models.push(item);
                        }
                    }
                    if !free_models.is_empty() {
                        free_models.sort_by_key(|m| {
                            if m.id.contains("llama-3.3-70b") { 0 }
                            else if m.id.contains("deepseek-r1") { 1 }
                            else if m.id.contains("deepseek-chat") { 2 }
                            else if m.id.contains("gemini-2.0-flash") { 3 }
                            else if m.id.contains("qwen-2.5-coder") { 4 }
                            else if m.id.contains("mistral") { 5 }
                            else { 10 }
                        });
                        let mut combined = free_models;
                        paid_models.truncate(10);
                        combined.extend(paid_models);
                        return combined;
                    }
                }
            }
            detect_defaults(2)
        }
        3 => {
            if !key.is_empty() {
                let base = if url.is_empty() { "https://api.deepseek.com/v1" } else { url.trim_end_matches('/') };
                if let Ok(v) = get_json(&format!("{base}/models"), &[("Authorization", format!("Bearer {key}"))]) {
                    if let Some(arr) = v["data"].as_array() {
                        let models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                            let id = m["id"].as_str()?.to_string();
                            Some(ModelChoice { id: id.clone(), label: id, is_free: false })
                        }).collect();
                        if !models.is_empty() {
                            return models;
                        }
                    }
                }
            }
            detect_defaults(3)
        }
        4 => {
            if !key.is_empty() {
                let base = if url.is_empty() { "https://api.mistral.ai/v1" } else { url.trim_end_matches('/') };
                if let Ok(v) = get_json(&format!("{base}/models"), &[("Authorization", format!("Bearer {key}"))]) {
                    if let Some(arr) = v["data"].as_array() {
                        let models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                            let id = m["id"].as_str()?.to_string();
                            let is_free = id.contains("open-mistral");
                            Some(ModelChoice {
                                id: id.clone(),
                                label: if is_free { format!("{id} [Free Tier]") } else { id },
                                is_free,
                            })
                        }).collect();
                        if !models.is_empty() {
                            return models;
                        }
                    }
                }
            }
            detect_defaults(4)
        }
        5 => {
            if !key.is_empty() {
                let base = if url.is_empty() { "https://api.together.xyz/v1" } else { url.trim_end_matches('/') };
                if let Ok(v) = get_json(&format!("{base}/models"), &[("Authorization", format!("Bearer {key}"))]) {
                    if let Some(arr) = v.as_array() {
                        let models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                            let id = m["id"].as_str()?.to_string();
                            Some(ModelChoice { id: id.clone(), label: id, is_free: false })
                        }).take(12).collect();
                        if !models.is_empty() {
                            return models;
                        }
                    }
                }
            }
            detect_defaults(5)
        }
        6 => {
            let base = if url.is_empty() { "http://localhost:11434" } else { url.trim_end_matches('/') };
            if let Ok(v) = get_json(&format!("{base}/api/tags"), &[]) {
                if let Some(arr) = v["models"].as_array() {
                    let models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                        let name = m["name"].as_str()?.to_string();
                        Some(ModelChoice {
                            id: name.clone(),
                            label: format!("{name} [Local Free]"),
                            is_free: true,
                        })
                    }).collect();
                    if !models.is_empty() {
                        return models;
                    }
                }
            }
            detect_defaults(6)
        }
        _ => {
            let base = if url.is_empty() { "http://localhost:1234/v1" } else { url.trim_end_matches('/') };
            if let Ok(v) = get_json(&format!("{base}/models"), &[]) {
                if let Some(arr) = v["data"].as_array() {
                    let models: Vec<ModelChoice> = arr.iter().filter_map(|m| {
                        let id = m["id"].as_str()?.to_string();
                        Some(ModelChoice {
                            id: id.clone(),
                            label: format!("{id} [Local Free]"),
                            is_free: true,
                        })
                    }).collect();
                    if !models.is_empty() {
                        return models;
                    }
                }
            }
            detect_defaults(7)
        }
    }
}

pub fn get_cached_models(provider: u32, key: &str, url: &str) -> (Vec<ModelChoice>, bool) {
    let fetching = IS_FETCHING_MODELS.load(std::sync::atomic::Ordering::Relaxed);
    {
        let cache = CACHED_PROVIDER.lock().unwrap();
        if let Some((p, ref k, ref u, ref models)) = *cache {
            if p == provider && k == key && u == url {
                return (models.clone(), fetching);
            }
        }
    }
    if !fetching {
        force_refresh_models(provider, key, url);
    }
    (detect_defaults(provider), true)
}

pub fn force_refresh_models(provider: u32, key: &str, url: &str) {
    IS_FETCHING_MODELS.store(true, std::sync::atomic::Ordering::Relaxed);
    let key = key.to_string();
    let url = url.to_string();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let models = detect_models(provider, &key, &url);
        let mut c = CACHED_PROVIDER.lock().unwrap();
        *c = Some((provider, key, url, models));
        IS_FETCHING_MODELS.store(false, std::sync::atomic::Ordering::Relaxed);
    });
}

fn get_json(url: &str, headers: &[(&str, String)]) -> std::result::Result<serde_json::Value, String> {
    let url = url.to_string();
    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<(bool, String)> {
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            let _ = hd.UserAgent()?.TryParseAdd(&HSTRING::from(format!("Nook/{}", crate::update::VERSION)));
            for (k, v) in &headers {
                hd.Append(&HSTRING::from(k.as_str()), &HSTRING::from(v.as_str()))?;
            }
            let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.get()?;
            let ok = resp.IsSuccessStatusCode()?;
            Ok((ok, resp.Content()?.ReadAsStringAsync()?.get()?.to_string()))
        })();
        let _ = tx.send(r.map_err(|e| format!("network error ({})", e.message().to_string().trim())));
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(Ok((_ok, text))) => serde_json::from_str(&text).map_err(|_| "The server sent unexpected JSON.".to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the request timed out".into()),
    }
}

fn post_json(url: &str, headers: &[(&str, String)], body: &str) -> std::result::Result<serde_json::Value, String> {
    let (url, body) = (url.to_string(), body.to_string());
    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<(bool, String)> {
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            let _ = hd.UserAgent()?.TryParseAdd(&HSTRING::from(format!("Nook/{}", crate::update::VERSION)));
            for (k, v) in &headers {
                hd.Append(&HSTRING::from(k.as_str()), &HSTRING::from(v.as_str()))?;
            }
            let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(&HSTRING::from(body), UnicodeEncoding::Utf8, &HSTRING::from("application/json"))?;
            let ic: IHttpContent = content.cast()?;
            let resp = client.PostAsync(&Uri::CreateUri(&HSTRING::from(url))?, &ic)?.get()?;
            let ok = resp.IsSuccessStatusCode()?;
            Ok((ok, resp.Content()?.ReadAsStringAsync()?.get()?.to_string()))
        })();
        let _ = tx.send(r.map_err(|e| format!("network error ({})", e.message().to_string().trim())));
    });
    match rx.recv_timeout(Duration::from_secs(45)) {
        Ok(Ok((_ok, text))) => serde_json::from_str(&text).map_err(|_| "The server sent something unexpected.".to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the request timed out".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_commands_and_passthrough() {
        assert!(local_action("What time is it?").unwrap().starts_with("It's "));
        assert!(local_action("what day is it").unwrap().starts_with("Today is"));
        // anything that is not a known action goes to the AI, in particular arbitrary programs are never launched
        assert!(local_action("open powershell and delete everything").is_none());
        assert!(local_action("tell me a joke").is_none());
        assert_eq!(tidy("**Hi** `there`"), "Hi there");
    }

    #[test]
    fn provider_round_trip_with_a_bad_key() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let a = Assistant::new();
        a.st.lock().unwrap().msgs.push(Msg { user: true, text: "hello".into() });
        let cfg = Cfg { provider: 0, key: "gsk_not_a_real_key".into(), ..Default::default() };
        let r = ask(&a, &cfg);
        println!("GROQ -> {r:?}");
        // POST, headers and TLS all worked if the server answered with its own error about the key
        assert!(matches!(&r, Err(e) if e.to_lowercase().contains("key") || e.to_lowercase().contains("auth")), "{r:?}");
        let r = ask(&a, &Cfg { provider: 1, key: "bad".into(), ..Default::default() });
        println!("GEMINI -> {r:?}");
        assert!(r.is_err());
        let r = ask(&a, &Cfg { provider: 0, ..Default::default() });
        assert!(matches!(&r, Err(e) if e.contains("Settings")));
    }

    #[test]
    fn windows_voices_synthesise() {
        use windows::Media::SpeechSynthesis::SpeechSynthesizer;
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let v = voices();
        println!("VOICES {v:?}");
        assert!(!v.is_empty(), "no Windows voices");
        let synth = SpeechSynthesizer::new().unwrap();
        let _ = synth.Options().and_then(|o| o.SetSpeakingRate(1.2));
        let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from("Hello from Nook")).unwrap().get().unwrap();
        println!("AUDIO bytes {} type {}", stream.Size().unwrap(), stream.ContentType().unwrap());
        assert!(stream.Size().unwrap() > 5000);
    }
}



