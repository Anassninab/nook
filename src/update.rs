//! Self-update from the public GitHub Releases of the Nook repository.
//!
//! How it works
//! * A worker thread reads `latest.json` ({"version","sha256","size"}) from
//!   `https://github.com/<repo>/releases/latest/download/` (plain file downloads: no API rate limit).
//! * When the version is newer it downloads `nook.exe` next to it, and only accepts the file when the
//!   size, the "MZ" executable header and the SHA-256 from `latest.json` all match.
//! * The app then swaps itself while running (Windows lets a running exe be *renamed*, not overwritten):
//!   `nook.exe` -> `nook.old.exe`, new file -> `nook.exe`, start it with `--updated <version>`, quit.
//!   The new process waits for the old one's single-instance lock, shows an "updated" toast and
//!   removes `nook.old.exe`.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::*;
use windows::Foundation::Uri;
use windows::Storage::Streams::DataReader;
use windows::Web::Http::HttpClient;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const BASE: &str = "https://github.com/Anassninab/nook/releases/latest/download/";

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Downloading(String),
    Ready { version: String, path: PathBuf },
    Failed(String),
}

enum Cmd {
    CheckNow,
}

#[derive(Clone)]
pub struct UpdateHandle {
    state: Arc<Mutex<UpdateState>>,
    auto: Arc<AtomicBool>,
    tx: Sender<Cmd>,
}

impl UpdateHandle {
    pub fn snapshot(&self) -> UpdateState {
        self.state.lock().unwrap().clone()
    }
    pub fn check_now(&self) {
        let _ = self.tx.send(Cmd::CheckNow);
    }
    pub fn fail(&self, why: String) {
        *self.state.lock().unwrap() = UpdateState::Failed(why);
    }
    pub fn set_auto(&self, on: bool) {
        self.auto.store(on, Ordering::Relaxed);
    }
}

fn base() -> String {
    // test hook: point the updater at a local server
    std::env::var("NOOK_UPDATE_BASE").unwrap_or_else(|_| BASE.to_string())
}

fn secs(var: &str, default: u64) -> Duration {
    Duration::from_secs(std::env::var(var).ok().and_then(|v| v.parse().ok()).unwrap_or(default))
}

pub fn dir() -> PathBuf {
    let p = crate::config::local_dir().join("update");
    let _ = fs::create_dir_all(&p);
    p
}

pub fn spawn(auto: bool) -> UpdateHandle {
    let state = Arc::new(Mutex::new(UpdateState::Idle));
    let auto = Arc::new(AtomicBool::new(auto));
    let (tx, rx) = channel();
    let (st, au) = (state.clone(), auto.clone());
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let every = secs("NOOK_UPDATE_EVERY_SECS", 20 * 60);
        let mut wait = secs("NOOK_UPDATE_FIRST_SECS", 15);
        let mut cleaned = false;
        loop {
            let manual = match rx.recv_timeout(wait) {
                Ok(Cmd::CheckNow) => true,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            wait = every;
            if !cleaned {
                cleaned = cleanup_old();
            }
            if !manual && !au.load(Ordering::Relaxed) {
                continue;
            }
            if matches!(*st.lock().unwrap(), UpdateState::Ready { .. }) {
                continue; // already downloaded, waiting for the app to install it
            }
            *st.lock().unwrap() = UpdateState::Checking;
            let res = check(&st);
            *st.lock().unwrap() = match res {
                Ok(Some((version, path))) => UpdateState::Ready { version, path },
                Ok(None) => UpdateState::UpToDate,
                Err(e) => {
                    crate::sys::log(&format!("update check failed: {e}"));
                    UpdateState::Failed(e)
                }
            };
        }
    });
    UpdateHandle { state, auto, tx }
}

/// Download and verify the newest build, if there is one.
fn check(state: &Arc<Mutex<UpdateState>>) -> std::result::Result<Option<(String, PathBuf)>, String> {
    let base = base();
    let meta = http_bytes(&format!("{base}latest.json"), 30)?;
    let v: serde_json::Value = serde_json::from_slice(&meta).map_err(|_| "latest.json is not valid".to_string())?;
    let version = v["version"].as_str().ok_or("latest.json has no version")?.trim_start_matches('v').to_string();
    if !is_newer(&version, VERSION) {
        return Ok(None);
    }
    let want_sha = v["sha256"].as_str().ok_or("latest.json has no sha256")?.to_lowercase();
    let want_size = v["size"].as_u64().ok_or("latest.json has no size")? as usize;
    if want_sha.len() != 64 || !want_sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("latest.json sha256 is malformed".into());
    }
    *state.lock().unwrap() = UpdateState::Downloading(version.clone());
    let exe = http_bytes(&format!("{base}nook.exe"), 240)?;
    if exe.len() != want_size {
        return Err(format!("download is {} bytes, expected {want_size}", exe.len()));
    }
    if exe.len() < 100_000 || &exe[..2] != b"MZ" {
        return Err("download is not a Windows program".into());
    }
    if sha256_hex(&exe) != want_sha {
        return Err("download failed the SHA-256 check".into());
    }
    let path = dir().join("nook-new.exe");
    fs::write(&path, &exe).map_err(|e| format!("cannot save update: {e}"))?;
    Ok(Some((version, path)))
}

fn parse_ver(s: &str) -> (u32, u32, u32) {
    let mut it = s.trim_start_matches('v').split(|c: char| c == '.' || c == '-').map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    parse_ver(remote) > parse_ver(local)
}

/// Swap the running exe for the downloaded one and start it. The caller must quit right after.
pub fn apply(new: &Path, version: &str) -> std::result::Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let old = exe.with_file_name("nook.old.exe");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("cannot replace the app ({e}); is it in a read-only folder?"))?;
    if let Err(e) = fs::copy(new, &exe) {
        let _ = fs::remove_file(&exe);
        let _ = fs::rename(&old, &exe);
        return Err(format!("cannot write the new version: {e}"));
    }
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
    let spawned = std::process::Command::new(&exe).arg("--updated").arg(version).creation_flags(0x0800_0208).spawn();
    if let Err(e) = spawned {
        let _ = fs::remove_file(&exe);
        let _ = fs::rename(&old, &exe);
        return Err(format!("cannot start the new version: {e}"));
    }
    let _ = fs::remove_file(new);
    Ok(())
}

/// Remove the previous version left behind by an update. True once nothing is left to remove.
pub fn cleanup_old() -> bool {
    let Ok(exe) = std::env::current_exe() else { return true };
    let old = exe.with_file_name("nook.old.exe");
    !old.exists() || fs::remove_file(&old).is_ok()
}

/// GET with a hard timeout (the WinRT client has none of its own).
fn http_bytes(url: &str, timeout_s: u64) -> std::result::Result<Vec<u8>, String> {
    let url = url.to_string();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let r = (|| -> Result<Vec<u8>> {
            let client = HttpClient::new()?;
            let hd = client.DefaultRequestHeaders()?;
            let _ = hd.UserAgent()?.TryParseAdd(&HSTRING::from(format!("Nook/{VERSION}")));
            let _ = hd.Append(h!("Cache-Control"), h!("no-cache"));
            let resp = client.GetAsync(&Uri::CreateUri(&HSTRING::from(url))?)?.get()?;
            if !resp.IsSuccessStatusCode()? {
                return Err(Error::new(windows::Win32::Foundation::E_FAIL, "http status"));
            }
            let buf = resp.Content()?.ReadAsBufferAsync()?.get()?;
            let reader = DataReader::FromBuffer(&buf)?;
            let mut out = vec![0u8; buf.Length()? as usize];
            reader.ReadBytes(&mut out)?;
            Ok(out)
        })();
        let _ = tx.send(r.map_err(|e| e.message().to_string()));
    });
    match rx.recv_timeout(Duration::from_secs(timeout_s)) {
        Ok(Ok(b)) => Ok(b),
        Ok(Err(e)) => Err(if e.is_empty() { "network error".into() } else { e }),
        Err(_) => Err("timed out".into()),
    }
}

// ---------------------------------------------------------------- SHA-256
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = vec![b'a'; 1_000_000];
        assert_eq!(sha256_hex(&long), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn version_order() {
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
    }
}
