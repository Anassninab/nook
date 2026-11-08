//! Prayer times for Indonesia (Kemenag RI: Fajr 20°, Isha 18°, +2 min ihtiyat) and Morocco (Habous / MWL-style:
//! Fajr 19°, Isha 17°, Dhuhr +5 min, Maghrib +5 min). Asr uses the standard shadow factor 1 for both.
use crate::config::Settings;
use crate::sys;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

pub struct City {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub tz: i32, // hours from UTC
}

pub struct Province {
    /// 0 = Indonesia, 1 = Morocco
    pub country: u8,
    pub name: &'static str,
    pub cities: &'static [City],
}

macro_rules! c {
    ($n:expr, $la:expr, $lo:expr, $tz:expr) => {
        City { name: $n, lat: $la, lon: $lo, tz: $tz }
    };
}

pub static PROVINCES: &[Province] = &[
    Province { country: 0, name: "DKI Jakarta", cities: &[c!("Jakarta", -6.2088, 106.8456, 7)] },
    Province { country: 0, name: "Jawa Barat", cities: &[
        c!("Bandung", -6.9175, 107.6191, 7), c!("Bekasi", -6.2383, 106.9756, 7), c!("Bogor", -6.5950, 106.8166, 7),
        c!("Depok", -6.4025, 106.7942, 7), c!("Cirebon", -6.7320, 108.5523, 7)] },
    Province { country: 0, name: "Jawa Tengah", cities: &[
        c!("Semarang", -6.9667, 110.4167, 7), c!("Surakarta", -7.5755, 110.8243, 7), c!("Magelang", -7.4797, 110.2177, 7),
        c!("Tegal", -6.8694, 109.1402, 7)] },
    Province { country: 0, name: "DI Yogyakarta", cities: &[c!("Yogyakarta", -7.7956, 110.3695, 7), c!("Sleman", -7.7160, 110.3550, 7), c!("Bantul", -7.8880, 110.3290, 7)] },
    Province { country: 0, name: "Jawa Timur", cities: &[
        c!("Surabaya", -7.2575, 112.7521, 7), c!("Malang", -7.9666, 112.6326, 7), c!("Kediri", -7.8480, 112.0178, 7),
        c!("Banyuwangi", -8.2192, 114.3691, 7)] },
    Province { country: 0, name: "Banten", cities: &[
        c!("Serang", -6.1200, 106.1503, 7), c!("Tangerang", -6.1783, 106.6319, 7), c!("Tangerang Selatan", -6.2886, 106.7179, 7),
        c!("Cilegon", -6.0025, 106.0112, 7)] },
    Province { country: 0, name: "Aceh", cities: &[c!("Banda Aceh", 5.5483, 95.3238, 7), c!("Lhokseumawe", 5.1801, 97.1507, 7)] },
    Province { country: 0, name: "Sumatera Utara", cities: &[c!("Medan", 3.5952, 98.6722, 7), c!("Pematangsiantar", 2.9595, 99.0687, 7)] },
    Province { country: 0, name: "Sumatera Barat", cities: &[c!("Padang", -0.9471, 100.4172, 7), c!("Bukittinggi", -0.3055, 100.3692, 7)] },
    Province { country: 0, name: "Riau", cities: &[c!("Pekanbaru", 0.5071, 101.4478, 7), c!("Dumai", 1.6665, 101.4474, 7)] },
    Province { country: 0, name: "Kepulauan Riau", cities: &[c!("Batam", 1.0456, 104.0305, 7), c!("Tanjung Pinang", 0.9186, 104.4558, 7)] },
    Province { country: 0, name: "Jambi", cities: &[c!("Jambi", -1.6101, 103.6131, 7)] },
    Province { country: 0, name: "Sumatera Selatan", cities: &[c!("Palembang", -2.9761, 104.7754, 7)] },
    Province { country: 0, name: "Bengkulu", cities: &[c!("Bengkulu", -3.7928, 102.2608, 7)] },
    Province { country: 0, name: "Lampung", cities: &[c!("Bandar Lampung", -5.3971, 105.2668, 7)] },
    Province { country: 0, name: "Kep. Bangka Belitung", cities: &[c!("Pangkal Pinang", -2.1291, 106.1138, 7)] },
    Province { country: 0, name: "Kalimantan Barat", cities: &[c!("Pontianak", -0.0263, 109.3425, 7)] },
    Province { country: 0, name: "Kalimantan Tengah", cities: &[c!("Palangka Raya", -2.2161, 113.9135, 7)] },
    Province { country: 0, name: "Kalimantan Selatan", cities: &[c!("Banjarmasin", -3.3194, 114.5908, 8)] },
    Province { country: 0, name: "Kalimantan Timur", cities: &[c!("Samarinda", -0.5022, 117.1536, 8), c!("Balikpapan", -1.2379, 116.8529, 8)] },
    Province { country: 0, name: "Kalimantan Utara", cities: &[c!("Tanjung Selor", 2.8379, 117.3667, 8)] },
    Province { country: 0, name: "Bali", cities: &[c!("Denpasar", -8.6705, 115.2126, 8)] },
    Province { country: 0, name: "Nusa Tenggara Barat", cities: &[c!("Mataram", -8.5833, 116.1167, 8)] },
    Province { country: 0, name: "Nusa Tenggara Timur", cities: &[c!("Kupang", -10.1772, 123.6070, 8)] },
    Province { country: 0, name: "Sulawesi Utara", cities: &[c!("Manado", 1.4748, 124.8421, 8)] },
    Province { country: 0, name: "Gorontalo", cities: &[c!("Gorontalo", 0.5435, 123.0568, 8)] },
    Province { country: 0, name: "Sulawesi Tengah", cities: &[c!("Palu", -0.9003, 119.8779, 8)] },
    Province { country: 0, name: "Sulawesi Barat", cities: &[c!("Mamuju", -2.6740, 118.8863, 8)] },
    Province { country: 0, name: "Sulawesi Selatan", cities: &[c!("Makassar", -5.1477, 119.4327, 8)] },
    Province { country: 0, name: "Sulawesi Tenggara", cities: &[c!("Kendari", -3.9985, 122.5130, 8)] },
    Province { country: 0, name: "Maluku", cities: &[c!("Ambon", -3.6954, 128.1814, 9)] },
    Province { country: 0, name: "Maluku Utara", cities: &[c!("Ternate", 0.7833, 127.3667, 9)] },
    Province { country: 0, name: "Papua", cities: &[c!("Jayapura", -2.5337, 140.7181, 9)] },
    Province { country: 0, name: "Papua Barat", cities: &[c!("Manokwari", -0.8615, 134.0620, 9)] },
    // ---------------------------------------------------------------- Morocco (tz 99 = follow the PC's own UTC offset,
    // which already knows about the Ramadan clock change)
    Province { country: 1, name: "Casablanca-Settat", cities: &[
        c!("Casablanca", 33.5731, -7.5898, 99), c!("Mohammedia", 33.6866, -7.3830, 99), c!("El Jadida", 33.2316, -8.5007, 99),
        c!("Settat", 33.0010, -7.6166, 99), c!("Berrechid", 33.2650, -7.5870, 99)] },
    Province { country: 1, name: "Rabat-Salé-Kénitra", cities: &[
        c!("Rabat", 34.0209, -6.8416, 99), c!("Salé", 34.0531, -6.7985, 99), c!("Kénitra", 34.2610, -6.5802, 99),
        c!("Témara", 33.9278, -6.9067, 99), c!("Khémisset", 33.8240, -6.0660, 99)] },
    Province { country: 1, name: "Marrakech-Safi", cities: &[
        c!("Marrakech", 31.6295, -7.9811, 99), c!("Safi", 32.2994, -9.2372, 99), c!("Essaouira", 31.5085, -9.7595, 99),
        c!("El Kelâa des Sraghna", 32.0520, -7.4090, 99)] },
    Province { country: 1, name: "Fès-Meknès", cities: &[
        c!("Fès", 34.0181, -5.0078, 99), c!("Meknès", 33.8935, -5.5473, 99), c!("Ifrane", 33.5228, -5.1106, 99),
        c!("Taza", 34.2100, -4.0100, 99), c!("Sefrou", 33.8300, -4.8300, 99)] },
    Province { country: 1, name: "Tanger-Tétouan-Al Hoceïma", cities: &[
        c!("Tanger", 35.7595, -5.8340, 99), c!("Tétouan", 35.5785, -5.3684, 99), c!("Al Hoceïma", 35.2517, -3.9372, 99),
        c!("Larache", 35.1932, -6.1557, 99), c!("Chefchaouen", 35.1688, -5.2636, 99)] },
    Province { country: 1, name: "Souss-Massa", cities: &[
        c!("Agadir", 30.4278, -9.5981, 99), c!("Inezgane", 30.3560, -9.5370, 99), c!("Taroudant", 30.4703, -8.8770, 99),
        c!("Tiznit", 29.6974, -9.7316, 99)] },
    Province { country: 1, name: "L'Oriental", cities: &[
        c!("Oujda", 34.6814, -1.9086, 99), c!("Nador", 35.1681, -2.9335, 99), c!("Berkane", 34.9200, -2.3200, 99),
        c!("Figuig", 32.1090, -1.2290, 99)] },
    Province { country: 1, name: "Béni Mellal-Khénifra", cities: &[
        c!("Béni Mellal", 32.3373, -6.3498, 99), c!("Khouribga", 32.8811, -6.9063, 99), c!("Khénifra", 32.9340, -5.6680, 99),
        c!("Azilal", 31.9600, -6.5700, 99)] },
    Province { country: 1, name: "Drâa-Tafilalet", cities: &[
        c!("Errachidia", 31.9314, -4.4244, 99), c!("Ouarzazate", 30.9189, -6.8934, 99), c!("Zagora", 30.3324, -5.8384, 99),
        c!("Midelt", 32.6800, -4.7400, 99)] },
    Province { country: 1, name: "Guelmim-Oued Noun", cities: &[
        c!("Guelmim", 28.9870, -10.0574, 99), c!("Tan-Tan", 28.4380, -11.1030, 99), c!("Sidi Ifni", 29.3797, -10.1730, 99)] },
    Province { country: 1, name: "Laâyoune-Sakia El Hamra", cities: &[
        c!("Laâyoune", 27.1253, -13.1625, 99), c!("Smara", 26.7384, -11.6719, 99)] },
    Province { country: 1, name: "Dakhla-Oued Ed-Dahab", cities: &[c!("Dakhla", 23.6848, -15.9580, 99)] },
];

pub const COUNTRIES: [&str; 2] = ["Indonesia", "Morocco"];

/// How a country calculates its prayer times.
pub struct Method {
    pub name: &'static str,
    pub fajr: f64,
    pub isha: f64,
    /// minutes added to every prayer (Kemenag "ihtiyat")
    pub ihtiyat: f64,
    /// extra minutes per prayer: Subuh, Dzuhur, Ashar, Maghrib, Isya
    pub offs: [f64; 5],
    pub names: [&'static str; 5],
}

pub const KEMENAG: Method = Method { name: "Kemenag RI", fajr: 20.0, isha: 18.0, ihtiyat: 2.0, offs: [0.0; 5], names: ["Subuh", "Dzuhur", "Ashar", "Maghrib", "Isya"] };
pub const MOROCCO: Method = Method { name: "Habous, Morocco", fajr: 19.0, isha: 17.0, ihtiyat: 0.0, offs: [0.0, 5.0, 0.0, 5.0, 0.0], names: ["Fajr", "Dhuhr", "Asr", "Maghrib", "Isha"] };

pub fn method_of(prov: usize) -> &'static Method {
    if PROVINCES[prov.min(PROVINCES.len() - 1)].country == 1 { &MOROCCO } else { &KEMENAG }
}
pub fn names(prov: usize) -> [&'static str; 5] {
    method_of(prov).names
}
/// "Waktu Subuh" / "Fajr time"
pub fn time_title(prov: usize, name: &str) -> String {
    if method_of(prov).names[0] == "Subuh" { format!("Waktu {name}") } else { format!("{name} time") }
}
pub fn country_of(prov: usize) -> usize {
    PROVINCES[prov.min(PROVINCES.len() - 1)].country as usize
}

/// Every (province, city) of a country, sorted by city name, for the pick lists.
pub fn all_cities(country: usize) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = vec![];
    for (pi, p) in PROVINCES.iter().enumerate() {
        if p.country as usize == country {
            for ci in 0..p.cities.len() {
                v.push((pi, ci));
            }
        }
    }
    v.sort_by_key(|(p, c)| PROVINCES[*p].cities[*c].name.to_lowercase());
    v
}

/// Search every city (name or region) for the Settings finder.
pub fn search(q: &str, limit: usize) -> Vec<(usize, usize)> {
    let q = q.trim().to_lowercase();
    let mut out: Vec<(u8, usize, usize)> = vec![];
    if q.is_empty() {
        return vec![];
    }
    for (pi, p) in PROVINCES.iter().enumerate() {
        for (ci, c) in p.cities.iter().enumerate() {
            let (n, r) = (c.name.to_lowercase(), p.name.to_lowercase());
            let score = if n.starts_with(&q) { 0 } else if n.contains(&q) { 1 } else if r.contains(&q) || COUNTRIES[p.country as usize].to_lowercase().starts_with(&q) { 2 } else { continue };
            out.push((score, pi, ci));
        }
    }
    out.sort();
    out.into_iter().take(limit).map(|(_, p, c)| (p, c)).collect()
}


fn rad(d: f64) -> f64 { d.to_radians() }
fn deg(r: f64) -> f64 { r.to_degrees() }
fn fix(a: f64, b: f64) -> f64 { let r = a - b * (a / b).floor(); if r < 0.0 { r + b } else { r } }

fn julian(y: i32, m: u32, d: u32) -> f64 {
    let (mut y, mut m) = (y as f64, m as f64);
    if m <= 2.0 { y -= 1.0; m += 12.0; }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + d as f64 + b - 1524.5
}

fn sun(jd: f64) -> (f64, f64) {
    let d = jd - 2451545.0;
    let g = fix(357.529 + 0.98560028 * d, 360.0);
    let q = fix(280.459 + 0.98564736 * d, 360.0);
    let l = fix(q + 1.915 * rad(g).sin() + 0.020 * rad(2.0 * g).sin(), 360.0);
    let e = 23.439 - 0.00000036 * d;
    let ra = fix(deg((rad(e).cos() * rad(l).sin()).atan2(rad(l).cos())) / 15.0, 24.0);
    let eqt = q / 15.0 - ra;
    let decl = deg((rad(e).sin() * rad(l).sin()).asin());
    (decl, eqt)
}

/// Returns minutes-from-midnight for [Subuh, Dzuhur, Ashar, Maghrib, Isya] plus sunrise (Terbit).
pub fn times(city: &City, method: &Method, y: i32, m: u32, d: u32, adjust: &[i32; 5]) -> ([f64; 5], f64) {
    // tz 99: use the PC's own UTC offset (Morocco: +1, or +0 during Ramadan)
    let tz = if city.tz == 99 { sys::utc_offset_minutes() as f64 / 60.0 } else { city.tz as f64 };
    times_tz(city, method, tz, y, m, d, adjust)
}

pub fn times_tz(city: &City, method: &Method, tz: f64, y: i32, m: u32, d: u32, adjust: &[i32; 5]) -> ([f64; 5], f64) {
    let jd = julian(y, m, d) - city.lon / (15.0 * 24.0);
    let (_, eqt0) = sun(jd + 0.5);
    let noon_of = |t: f64| {
        let (decl, eqt) = sun(jd + t);
        (decl, 12.0 - eqt)
    };
    let angle_time = |angle: f64, t: f64, ccw: bool| {
        let (decl, _) = noon_of(t);
        let (_, noon) = noon_of(t);
        let x = (-rad(angle).sin() - rad(decl).sin() * rad(city.lat).sin()) / (rad(decl).cos() * rad(city.lat).cos());
        let h = deg(x.clamp(-1.0, 1.0).acos()) / 15.0;
        if ccw { noon - h } else { noon + h }
    };
    let _ = eqt0;
    let (_, noon_base) = noon_of(0.5);
    let adj = tz - city.lon / 15.0;
    let fajr = angle_time(method.fajr, 5.0 / 24.0, true) + adj;
    let rise = angle_time(0.833, 6.0 / 24.0, true) + adj;
    let dhuhr = noon_base + adj;
    let (decl, _) = noon_of(13.0 / 24.0);
    let asr_angle = -deg((1.0 / (1.0 + (rad(city.lat - decl)).tan().abs())).atan());
    let asr = angle_time(asr_angle, 13.0 / 24.0, false) + adj;
    let maghrib = angle_time(0.833, 18.0 / 24.0, false) + adj;
    let isha = angle_time(method.isha, 18.0 / 24.0, false) + adj;
    let ih = method.ihtiyat;
    let mut out = [fajr + ih / 60.0, dhuhr + ih / 60.0, asr + ih / 60.0, maghrib + ih / 60.0, isha + ih / 60.0];
    let mut res = [0.0; 5];
    for i in 0..5 {
        res[i] = (out[i] * 60.0 + method.offs[i]).round() + adjust[i] as f64;
        out[i] = res[i];
    }
    (res, (rise * 60.0 - ih).round())
}

pub fn fmt(min: f64) -> String {
    let m = min.rem_euclid(1440.0) as u32;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// (index, minutes, tomorrow) of the next prayer after `now_min`.
pub fn next(t: &[f64; 5], now_min: f64) -> (usize, f64, bool) {
    for i in 0..5 {
        if t[i] > now_min {
            return (i, t[i], false);
        }
    }
    (0, t[0], true)
}

pub const MWL: Method = Method { name: "Muslim World League", fajr: 18.0, isha: 17.0, ihtiyat: 0.0, offs: [0.0; 5], names: ["Fajr", "Dhuhr", "Asr", "Maghrib", "Isha"] };

/// The official calculation method (Aladhan id, label) for a country, ISO 3166 code.
pub fn aladhan_method(cc: &str) -> (u32, &'static str) {
    match cc {
        "SA" => (4, "Umm Al-Qura"),
        "EG" => (5, "Egyptian General Authority"),
        "PK" | "IN" | "BD" | "AF" => (1, "Univ. of Islamic Sciences, Karachi"),
        "US" | "CA" => (2, "ISNA"),
        "ID" => (20, "Kemenag RI"),
        "MA" => (21, "Habous, Morocco"),
        "SG" => (11, "MUIS Singapore"),
        "MY" | "BN" => (17, "JAKIM Malaysia"),
        "TR" => (13, "Diyanet, Turkey"),
        "AE" => (16, "Dubai"),
        "KW" => (9, "Kuwait"),
        "QA" => (10, "Qatar"),
        "BH" | "OM" | "YE" => (8, "Gulf Region"),
        "IR" => (7, "Tehran"),
        "FR" => (12, "UOIF France"),
        "RU" => (14, "Russia"),
        "TN" => (18, "Tunisia"),
        "DZ" => (19, "Algeria"),
        "PT" => (22, "Portugal"),
        "JO" => (23, "Jordan"),
        _ => (3, "Muslim World League"),
    }
}

// ------------------------------------------------------------------ any city in the world
#[derive(Clone, Debug)]
pub struct Geo {
    pub name: String,
    pub admin: String,
    pub country: String,
    pub cc: String,
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
}

impl Geo {
    pub fn label(&self) -> String {
        if self.cc.is_empty() { self.name.clone() } else { format!("{}, {}", self.name, self.cc) }
    }
}

static ONLINE: Mutex<(String, Vec<Geo>)> = Mutex::new((String::new(), Vec::new()));
static LATEST_Q: Mutex<String> = Mutex::new(String::new());

/// Ask the geocoder for cities matching `q` (debounced, runs on a thread); read with `online_results`.
pub fn online_request(q: &str) {
    let q = q.trim().to_string();
    if q.chars().count() < 3 || ONLINE.lock().unwrap().0 == q || *LATEST_Q.lock().unwrap() == q {
        return;
    }
    *LATEST_Q.lock().unwrap() = q.clone();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        std::thread::sleep(std::time::Duration::from_millis(450));
        if *LATEST_Q.lock().unwrap() != q {
            return; // typed on since
        }
        let url = format!("https://nominatim.openstreetmap.org/search?q={}&format=json&limit=8&addressdetails=1&accept-language=en", crate::lyrics::enc(&q));
        let mut out: Vec<Geo> = vec![];
        if let Ok(Some(body)) = crate::lyrics::http_get(url, None) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                for r in v.as_array().into_iter().flatten() {
                    let ad = &r["address"];
                    let name = r["name"].as_str().filter(|n| !n.is_empty()).or_else(|| ad["city"].as_str()).unwrap_or("").to_string();
                    let admin = ad["state"].as_str().or(ad["province"].as_str()).or(ad["region"].as_str()).unwrap_or("").to_string();
                    let g = Geo {
                        name,
                        admin,
                        country: ad["country"].as_str().unwrap_or("").to_string(),
                        cc: ad["country_code"].as_str().unwrap_or("").to_uppercase(),
                        lat: r["lat"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0),
                        lon: r["lon"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0.0),
                        tz: String::new(),
                    };
                    if !g.name.is_empty() && !out.iter().any(|o| o.name == g.name && o.admin == g.admin && o.cc == g.cc) {
                        out.push(g);
                    }
                }
            }
        }
        *ONLINE.lock().unwrap() = (q, out);
    });
}

pub fn online_results(q: &str) -> Vec<Geo> {
    let g = ONLINE.lock().unwrap();
    if g.0 == q.trim() { g.1.clone() } else { vec![] }
}

pub fn online_pending(q: &str) -> bool {
    let q = q.trim();
    q.chars().count() >= 3 && ONLINE.lock().unwrap().0 != q
}

/// Month of exact times fetched from the official-method calendar, cached on disk.
struct MonthCache {
    key: String,
    off_min: i32,
    days: Vec<[f64; 6]>, // fajr, sunrise, dhuhr, asr, maghrib, isha (minutes after local midnight)
}

static CACHE: Mutex<Option<MonthCache>> = Mutex::new(None);
static FETCHING: AtomicBool = AtomicBool::new(false);
static PLACE_OFF: AtomicI32 = AtomicI32::new(i32::MIN);

fn cache_path() -> std::path::PathBuf {
    crate::config::local_dir().join("prayer_cache.json")
}

/// The wall clock of the chosen place: (year, month, day, minutes since its midnight).
pub fn place_now(s: &Settings) -> (i32, u32, u32, f64) {
    let n = sys::now();
    let local = n.minutes() as f64 + n.sec as f64 / 60.0;
    let delta = if s.pray_custom {
        let off = PLACE_OFF.load(Ordering::Relaxed);
        if off == i32::MIN { 0 } else { off - sys::utc_offset_minutes() }
    } else {
        0
    };
    let total = sys::days_from_civil(n.year, n.month, n.day) as f64 * 1440.0 + local + delta as f64;
    let day = (total / 1440.0).floor();
    let (y, m, d) = sys::civil_from_days(day as i64);
    (y, m, d, total - day * 1440.0)
}

pub fn minutes_now(s: &Settings) -> f64 {
    place_now(s).3
}

fn leaked(name: &str, lat: f64, lon: f64) -> &'static City {
    static M: OnceLock<Mutex<HashMap<String, &'static City>>> = OnceLock::new();
    let mut m = M.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    let key = format!("{name}|{lat}|{lon}");
    m.entry(key).or_insert_with(|| Box::leak(Box::new(City { name: Box::leak(name.to_string().into_boxed_str()), lat, lon, tz: 99 })))
}

/// Make sure this month's exact times for the chosen custom place are cached (disk, then network).
pub fn ensure(s: &Settings) {
    if !s.pray_custom || s.pray_name.is_empty() {
        return;
    }
    let (y, m, _, _) = place_now(s);
    let (method, _) = aladhan_method(&s.pray_cc);
    let key = format!("{:.3},{:.3},{y}-{m},{method}", s.pray_lat, s.pray_lon);
    if CACHE.lock().unwrap().as_ref().map(|c| c.key == key).unwrap_or(false) {
        return;
    }
    // disk
    if let Ok(t) = std::fs::read_to_string(cache_path()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
            if v["key"].as_str() == Some(&key) {
                let days: Vec<[f64; 6]> = v["days"].as_array().map(|a| a.iter().map(|d| {
                    let mut r = [0.0; 6];
                    for (i, x) in d.as_array().into_iter().flatten().take(6).enumerate() {
                        r[i] = x.as_f64().unwrap_or(0.0);
                    }
                    r
                }).collect()).unwrap_or_default();
                let off = v["off"].as_i64().unwrap_or(0) as i32;
                if !days.is_empty() {
                    PLACE_OFF.store(off, Ordering::Relaxed);
                    *CACHE.lock().unwrap() = Some(MonthCache { key, off_min: off, days });
                    return;
                }
            }
        }
    }
    if FETCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (lat, lon) = (s.pray_lat, s.pray_lon);
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let res = (|| -> Option<MonthCache> {
            let url = format!("https://api.aladhan.com/v1/calendar/{y}/{m}?latitude={lat}&longitude={lon}&method={method}");
            let body = crate::lyrics::http_get(url, None).ok()??;
            let v: serde_json::Value = serde_json::from_str(&body).ok()?;
            let hm = |t: &serde_json::Value| -> f64 {
                let s = t.as_str().unwrap_or("00:00");
                let (h, mi) = s.get(..5).unwrap_or("00:00").split_once(':').unwrap_or(("0", "0"));
                h.parse::<f64>().unwrap_or(0.0) * 60.0 + mi.parse::<f64>().unwrap_or(0.0)
            };
            let mut days = vec![];
            for d in v["data"].as_array()? {
                let t = &d["timings"];
                days.push([hm(&t["Fajr"]), hm(&t["Sunrise"]), hm(&t["Dhuhr"]), hm(&t["Asr"]), hm(&t["Maghrib"]), hm(&t["Isha"])]);
            }
            // current UTC offset of the place (DST included): its wall clock now vs UTC now
            let tz = v["data"][0]["meta"]["timezone"].as_str().unwrap_or("").to_string();
            let pc = sys::utc_offset_minutes();
            let off = (|| -> Option<i32> {
                let b = crate::lyrics::http_get(format!("https://timeapi.io/api/Time/current/zone?timeZone={}", crate::lyrics::enc(&tz)), None).ok()??;
                let j: serde_json::Value = serde_json::from_str(&b).ok()?;
                let place_min = j["hour"].as_i64()? * 60 + j["minute"].as_i64()?;
                let n = sys::now();
                let utc_min = (n.minutes() as i64 - pc as i64).rem_euclid(1440);
                let mut diff = (place_min - utc_min).rem_euclid(1440);
                if diff > 720 {
                    diff -= 1440;
                }
                Some(((diff as f64 / 15.0).round() * 15.0) as i32)
            })()
            .unwrap_or(pc);
            Some(MonthCache { key: key.clone(), off_min: off, days })
        })();
        if let Some(c) = res {
            let j = serde_json::json!({ "key": c.key, "off": c.off_min, "days": c.days });
            let _ = std::fs::write(cache_path(), j.to_string());
            PLACE_OFF.store(c.off_min, Ordering::Relaxed);
            *CACHE.lock().unwrap() = Some(c);
        }
        FETCHING.store(false, Ordering::SeqCst);
    });
}

pub fn today(s: &Settings) -> ([f64; 5], f64, &'static City) {
    let adjust = &s.pray_adjust;
    if s.pray_custom && !s.pray_name.is_empty() {
        let city = leaked(&s.pray_name, s.pray_lat, s.pray_lon);
        let (y, m, d, _) = place_now(s);
        let (method, _) = aladhan_method(&s.pray_cc);
        let key = format!("{:.3},{:.3},{y}-{m},{method}", s.pray_lat, s.pray_lon);
        if let Some(c) = CACHE.lock().unwrap().as_ref() {
            if c.key == key {
                if let Some(day) = c.days.get(d as usize - 1) {
                    let t = [day[0] + adjust[0] as f64, day[2] + adjust[1] as f64, day[3] + adjust[2] as f64, day[4] + adjust[3] as f64, day[5] + adjust[4] as f64];
                    return (t, day[1], city);
                }
            }
        }
        // until the exact table arrives: local calculation at the place's clock
        let off = PLACE_OFF.load(Ordering::Relaxed);
        let tz = if off == i32::MIN { sys::utc_offset_minutes() as f64 / 60.0 } else { off as f64 / 60.0 };
        let meth = if s.pray_cc == "ID" { &KEMENAG } else if s.pray_cc == "MA" { &MOROCCO } else { &MWL };
        let (t, rise) = times_tz(city, meth, tz, y, m, d, adjust);
        return (t, rise, city);
    }
    let p = &PROVINCES[s.prayer_prov.min(PROVINCES.len() - 1)];
    let c = &p.cities[s.prayer_city.min(p.cities.len() - 1)];
    let n = sys::now();
    let (t, rise) = times(c, method_of(s.prayer_prov), n.year, n.month, n.day, adjust);
    (t, rise, c)
}

pub fn names_s(s: &Settings) -> [&'static str; 5] {
    if s.pray_custom {
        if s.pray_cc == "ID" { KEMENAG.names } else { MWL.names }
    } else {
        names(s.prayer_prov)
    }
}

pub fn time_title_s(s: &Settings, name: &str) -> String {
    if names_s(s)[0] == "Subuh" { format!("Waktu {name}") } else { format!("{name} time") }
}

pub fn method_name_s(s: &Settings) -> String {
    if s.pray_custom { aladhan_method(&s.pray_cc).1.to_string() } else { method_of(s.prayer_prov).name.to_string() }
}

pub fn place_label(s: &Settings) -> String {
    if s.pray_custom && !s.pray_name.is_empty() {
        s.pray_name.clone()
    } else {
        let p = &PROVINCES[s.prayer_prov.min(PROVINCES.len() - 1)];
        p.cities[s.prayer_city.min(p.cities.len() - 1)].name.to_string()
    }
}


// ------------------------------------------------------------------ countries (besides the detailed Indonesia / Morocco lists)
pub struct World {
    pub name: &'static str,
    pub cc: &'static str,
    pub cities: &'static [(&'static str, f64, f64)],
}

macro_rules! w {
    ($n:expr, $cc:expr, [$(($c:expr, $la:expr, $lo:expr)),* $(,)?]) => {
        World { name: $n, cc: $cc, cities: &[$(($c, $la, $lo)),*] }
    };
}

pub static WORLD: &[World] = &[
    w!("Saudi Arabia", "SA", [("Mecca", 21.3891, 39.8579), ("Medina", 24.5247, 39.5692), ("Riyadh", 24.7136, 46.6753), ("Jeddah", 21.4858, 39.1925), ("Dammam", 26.4207, 50.0888)]),
    w!("Egypt", "EG", [("Cairo", 30.0444, 31.2357), ("Alexandria", 31.2001, 29.9187), ("Giza", 30.0131, 31.2089), ("Luxor", 25.6872, 32.6396), ("Aswan", 24.0889, 32.8998)]),
    w!("Turkey", "TR", [("Istanbul", 41.0082, 28.9784), ("Ankara", 39.9334, 32.8597), ("Izmir", 38.4237, 27.1428), ("Bursa", 40.1885, 29.0610), ("Konya", 37.8746, 32.4932)]),
    w!("United Arab Emirates", "AE", [("Dubai", 25.2048, 55.2708), ("Abu Dhabi", 24.4539, 54.3773), ("Sharjah", 25.3463, 55.4209)]),
    w!("Qatar", "QA", [("Doha", 25.2854, 51.5310)]),
    w!("Kuwait", "KW", [("Kuwait City", 29.3759, 47.9774)]),
    w!("Bahrain", "BH", [("Manama", 26.2285, 50.5860)]),
    w!("Oman", "OM", [("Muscat", 23.5880, 58.3829)]),
    w!("Jordan", "JO", [("Amman", 31.9454, 35.9284)]),
    w!("Lebanon", "LB", [("Beirut", 33.8938, 35.5018)]),
    w!("Iraq", "IQ", [("Baghdad", 33.3152, 44.3661), ("Basra", 30.5085, 47.7835), ("Erbil", 36.1901, 44.0091)]),
    w!("Iran", "IR", [("Tehran", 35.6892, 51.3890), ("Mashhad", 36.2605, 59.6168), ("Isfahan", 32.6546, 51.6680), ("Shiraz", 29.5918, 52.5837)]),
    w!("Pakistan", "PK", [("Karachi", 24.8607, 67.0011), ("Lahore", 31.5204, 74.3587), ("Islamabad", 33.6844, 73.0479)]),
    w!("India", "IN", [("Delhi", 28.6139, 77.2090), ("Mumbai", 19.0760, 72.8777), ("Hyderabad", 17.3850, 78.4867)]),
    w!("Bangladesh", "BD", [("Dhaka", 23.8103, 90.4125)]),
    w!("Malaysia", "MY", [("Kuala Lumpur", 3.1390, 101.6869), ("Johor Bahru", 1.4927, 103.7414), ("George Town", 5.4141, 100.3288)]),
    w!("Singapore", "SG", [("Singapore", 1.3521, 103.8198)]),
    w!("Brunei", "BN", [("Bandar Seri Begawan", 4.9031, 114.9398)]),
    w!("Algeria", "DZ", [("Algiers", 36.7538, 3.0588), ("Oran", 35.6971, -0.6308), ("Constantine", 36.3650, 6.6147)]),
    w!("Tunisia", "TN", [("Tunis", 36.8065, 10.1815), ("Sfax", 34.7406, 10.7603)]),
    w!("Libya", "LY", [("Tripoli", 32.8872, 13.1913), ("Benghazi", 32.1167, 20.0667)]),
    w!("Sudan", "SD", [("Khartoum", 15.5007, 32.5599)]),
    w!("Somalia", "SO", [("Mogadishu", 2.0469, 45.3182)]),
    w!("Nigeria", "NG", [("Lagos", 6.5244, 3.3792), ("Abuja", 9.0765, 7.3986), ("Kano", 12.0022, 8.5920)]),
    w!("Senegal", "SN", [("Dakar", 14.7167, -17.4677)]),
    w!("Mauritania", "MR", [("Nouakchott", 18.0735, -15.9582)]),
    w!("Palestine", "PS", [("Jerusalem", 31.7683, 35.2137), ("Gaza", 31.5017, 34.4668)]),
    w!("Syria", "SY", [("Damascus", 33.5138, 36.2765), ("Aleppo", 36.2021, 37.1343)]),
    w!("Yemen", "YE", [("Sanaa", 15.3694, 44.1910)]),
    w!("Afghanistan", "AF", [("Kabul", 34.5553, 69.2075)]),
    w!("Azerbaijan", "AZ", [("Baku", 40.4093, 49.8671)]),
    w!("Kazakhstan", "KZ", [("Astana", 51.1694, 71.4491), ("Almaty", 43.2389, 76.8897)]),
    w!("Uzbekistan", "UZ", [("Tashkent", 41.2995, 69.2401)]),
    w!("France", "FR", [("Paris", 48.8566, 2.3522), ("Marseille", 43.2965, 5.3698), ("Lyon", 45.7640, 4.8357)]),
    w!("Germany", "DE", [("Berlin", 52.5200, 13.4050), ("Munich", 48.1351, 11.5820), ("Cologne", 50.9375, 6.9603)]),
    w!("United Kingdom", "GB", [("London", 51.5074, -0.1278), ("Birmingham", 52.4862, -1.8904), ("Manchester", 53.4808, -2.2426)]),
    w!("Spain", "ES", [("Madrid", 40.4168, -3.7038), ("Barcelona", 41.3874, 2.1686)]),
    w!("Italy", "IT", [("Rome", 41.9028, 12.4964), ("Milan", 45.4642, 9.1900)]),
    w!("Netherlands", "NL", [("Amsterdam", 52.3676, 4.9041), ("Rotterdam", 51.9244, 4.4777)]),
    w!("Belgium", "BE", [("Brussels", 50.8503, 4.3517)]),
    w!("Sweden", "SE", [("Stockholm", 59.3293, 18.0686)]),
    w!("Russia", "RU", [("Moscow", 55.7558, 37.6173), ("Kazan", 55.7887, 49.1221)]),
    w!("United States", "US", [("New York", 40.7128, -74.0060), ("Los Angeles", 34.0522, -118.2437), ("Chicago", 41.8781, -87.6298), ("Houston", 29.7604, -95.3698), ("Dearborn", 42.3223, -83.1763)]),
    w!("Canada", "CA", [("Toronto", 43.6532, -79.3832), ("Montreal", 45.5017, -73.5673), ("Vancouver", 49.2827, -123.1207)]),
    w!("Brazil", "BR", [("Sao Paulo", -23.5558, -46.6396)]),
    w!("Australia", "AU", [("Sydney", -33.8688, 151.2093), ("Melbourne", -37.8136, 144.9631)]),
    w!("South Africa", "ZA", [("Cape Town", -33.9249, 18.4241), ("Johannesburg", -26.2041, 28.0473)]),
    w!("Kenya", "KE", [("Nairobi", -1.2921, 36.8219)]),
    w!("China", "CN", [("Beijing", 39.9042, 116.4074), ("Shanghai", 31.2304, 121.4737)]),
    w!("Japan", "JP", [("Tokyo", 35.6762, 139.6503)]),
    w!("Philippines", "PH", [("Manila", 14.5995, 120.9842)]),
    w!("Thailand", "TH", [("Bangkok", 13.7563, 100.5018)]),
    w!("Bosnia and Herzegovina", "BA", [("Sarajevo", 43.8563, 18.4131)]),
    w!("Albania", "AL", [("Tirana", 41.3275, 19.8187)]),
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CRef {
    Builtin(usize), // 0 Indonesia, 1 Morocco
    World(usize),
}

/// Every selectable country, alphabetical: (name, ISO code, where its cities come from).
pub fn countries() -> Vec<(&'static str, &'static str, CRef)> {
    let mut v: Vec<(&'static str, &'static str, CRef)> = vec![("Indonesia", "ID", CRef::Builtin(0)), ("Morocco", "MA", CRef::Builtin(1))];
    for (i, w) in WORLD.iter().enumerate() {
        v.push((w.name, w.cc, CRef::World(i)));
    }
    v.sort_by_key(|c| c.0);
    v
}

pub enum Target {
    Builtin(usize, usize),
    World(usize, usize),
}

pub struct PickCity {
    pub name: String,
    pub sub: String,
    pub target: Target,
}

pub fn cities_of(c: CRef) -> Vec<PickCity> {
    match c {
        CRef::Builtin(k) => all_cities(k)
            .into_iter()
            .map(|(p, ci)| PickCity { name: PROVINCES[p].cities[ci].name.to_string(), sub: PROVINCES[p].name.to_string(), target: Target::Builtin(p, ci) })
            .collect(),
        CRef::World(wi) => WORLD[wi].cities.iter().enumerate().map(|(i, c)| PickCity { name: c.0.to_string(), sub: WORLD[wi].name.to_string(), target: Target::World(wi, i) }).collect(),
    }
}

/// ISO code of the place currently chosen.
pub fn current_cc(s: &Settings) -> String {
    if s.pray_custom {
        s.pray_cc.clone()
    } else if country_of(s.prayer_prov) == 1 {
        "MA".into()
    } else {
        "ID".into()
    }
}

/// Index into `countries()` of the current place's country, if it is one of the listed ones.
pub fn current_country(s: &Settings) -> Option<usize> {
    let cc = current_cc(s);
    countries().iter().position(|c| c.1 == cc)
}

pub fn is_selected(s: &Settings, t: &Target) -> bool {
    match t {
        Target::Builtin(p, c) => !s.pray_custom && s.prayer_prov == *p && s.prayer_city == *c,
        Target::World(wi, ci) => {
            let c = &WORLD[*wi].cities[*ci];
            s.pray_custom && s.pray_name == format!("{}, {}", c.0, WORLD[*wi].cc)
        }
    }
}

pub fn apply_target(s: &mut Settings, t: &Target) {
    match t {
        Target::Builtin(p, c) => {
            s.prayer_prov = *p;
            s.prayer_city = *c;
            s.pray_custom = false;
        }
        Target::World(wi, ci) => {
            let c = &WORLD[*wi].cities[*ci];
            s.pray_custom = true;
            s.pray_name = format!("{}, {}", c.0, WORLD[*wi].cc);
            s.pray_lat = c.1;
            s.pray_lon = c.2;
            s.pray_cc = WORLD[*wi].cc.to_string();
            s.pray_tz = String::new();
            ensure(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(m: f64) -> String {
        fmt(m)
    }
    fn city(name: &str) -> &'static City {
        for p in PROVINCES {
            for c in p.cities {
                if c.name == name {
                    return c;
                }
            }
        }
        panic!("no {name}")
    }
    fn close(a: f64, b: &str, what: &str) {
        let (h, m) = b.split_once(':').unwrap();
        let want = h.parse::<f64>().unwrap() * 60.0 + m.parse::<f64>().unwrap();
        assert!((a - want).abs() <= 2.0, "{what}: got {} want {b}", hm(a));
    }

    #[test]
    fn morocco_matches_aladhan_method_21() {
        // reference: api.aladhan.com method 21 (Morocco), Casablanca 2026-11-08, clock at UTC+0
        let (t, rise) = times_tz(city("Casablanca"), &MOROCCO, 0.0, 2026, 11, 8, &[0; 5]);
        close(t[0], "05:26", "fajr");
        close(rise, "06:55", "sunrise");
        close(t[1], "12:19", "dhuhr");
        close(t[2], "15:12", "asr");
        close(t[3], "17:37", "maghrib");
        close(t[4], "18:53", "isha");
        // Rabat 2026-07-20, clock at UTC+1
        let (t, _) = times_tz(city("Rabat"), &MOROCCO, 1.0, 2026, 7, 20, &[0; 5]);
        close(t[0], "04:44", "rabat fajr");
        close(t[1], "13:39", "rabat dhuhr");
        close(t[2], "17:19", "rabat asr");
        close(t[3], "20:42", "rabat maghrib");
        close(t[4], "22:10", "rabat isha");
    }

    #[test]
    fn country_list() {
        let c = countries();
        assert!(c.len() > 50);
        assert!(c.windows(2).all(|w| w[0].0 <= w[1].0));
        for w in WORLD {
            assert_eq!(w.cc.len(), 2);
            for ci in w.cities {
                assert!(ci.1.abs() <= 90.0 && ci.2.abs() <= 180.0);
            }
        }
        let mut s = Settings::default();
        apply_target(&mut s, &Target::Builtin(0, 0));
        assert_eq!(current_cc(&s), "ID");
        assert!(current_country(&s).is_some());
    }

    #[test]
    fn lists_and_search() {
        assert!(all_cities(1).len() >= 40);
        assert!(all_cities(0).len() > 50);
        let r = search("casa", 5);
        assert_eq!(PROVINCES[r[0].0].cities[r[0].1].name, "Casablanca");
        assert_eq!(country_of(r[0].0), 1);
        assert!(!search("morocco", 8).is_empty());
        assert_eq!(names(0)[0], "Subuh");
        assert_eq!(names(r[0].0)[0], "Fajr");
    }

    #[test]
    fn worldwide_lookup_live() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        online_request("Istanbul");
        let mut geo = vec![];
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            geo = online_results("Istanbul");
            if !geo.is_empty() {
                break;
            }
        }
        assert!(!geo.is_empty(), "geocoder returned nothing");
        let g = &geo[0];
        assert_eq!(g.cc, "TR");
        let mut s = Settings::default();
        s.pray_custom = true;
        s.pray_name = g.label();
        s.pray_lat = g.lat;
        s.pray_lon = g.lon;
        s.pray_cc = g.cc.clone();
        ensure(&s);
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if CACHE.lock().unwrap().is_some() {
                break;
            }
        }
        let (t, rise, city) = today(&s);
        println!("{} fajr {} sunrise {} dhuhr {} asr {} maghrib {} isha {} (offset {:?})", city.name, fmt(t[0]), fmt(rise), fmt(t[1]), fmt(t[2]), fmt(t[3]), fmt(t[4]), PLACE_OFF.load(Ordering::Relaxed));
        assert!(t[0] < rise && rise < t[1] && t[1] < t[2] && t[2] < t[3] && t[3] < t[4]);
        assert!(CACHE.lock().unwrap().is_some(), "exact table never arrived");
        assert_eq!(aladhan_method("TR").0, 13);
    }
}
