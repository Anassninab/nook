# Nook — a living notch for Windows

A from-scratch Rust implementation of a "notch" overlay: a pill at the top of the screen that
grows on hover into a small dashboard (music + synced lyrics, calendar, focus timer, water,
to-do, prayer times, file shelf, widgets), plus HUD popups for volume / Caps Lock / input
language and battery / Wi-Fi / Bluetooth events.

Behaviour (sizes, hover delay, spring overshoot) was matched by observing a reference app's
public behaviour. **No code, assets, icons or strings were copied from it**, and the name,
mascot and settings UI are original.

## Build / run

```
cargo build --release
target\release\nook.exe
```

Quit with the tray icon (or right-click the notch) → *Quit Nook*. Settings: right-click → *Settings…*.
Data lives in `%APPDATA%\Nook` (plain `key=value` / line files).

## Architecture

| file | role |
|---|---|
| `main.rs` | window class, single-instance mutex, startup |
| `gfx.rs` | D3D11 + DXGI composition swap chain + Direct2D/DirectWrite; notch silhouette, text, bitmaps |
| `spring.rs` | damped springs (ω≈12.6, ζ≈0.7 → ~5 % overshoot, ~350 ms to peak) |
| `app.rs` | notch state machine, hover intent, input, chrome (header, ring, dots), keyboard hook, clipboard, tray |
| `pages.rs` | page bodies (media, widgets, calendar, focus, water, to-do, prayer, shelf), toasts, HUD, reminder services |
| `media.rs` | Windows media-session worker (title, art, timeline, transport) |
| `lyrics.rs` | LRCLIB client + LRC parser |
| `prayer.rs` | Kemenag RI prayer-time maths + Indonesian province/city table |
| `settings_ui.rs` | settings window and add-task dialog (immediate-mode toolkit, own visual style) |
| `store.rs`, `config.rs` | persistence |
| `netmon.rs` | Wi-Fi and Bluetooth connect/disconnect events |

## Debug switches (environment variables)

* `NOOK_DEBUG_Y=<px>` – shift the overlay down (to run beside another notch app)
* `NOOK_DEBUG_PAGE=<n>` – start expanded and held open on page *n*
* `NOOK_DEBUG_SETTINGS=1`, `NOOK_DEBUG_TAB=<n>` – open the settings window on tab *n*

## What the second pass added

* Bottom **icon tab bar** that replaces the page dots when you hover the lower strip (notch grows 20 px)
* Page titles ("Todo**List**", "Prayer *Jakarta*", …), larger type and vector transport icons
* **TodoList** page: done/total counter, Add / Edit task dialog, task panel
* **Prayer** page: next prayer, countdown, progress, sun arc with the five times
* **Lyrics view**: compact player + scrolling lyric panel, header hides, theme glow along the bottom
* **Lyric bar**: while music plays the collapsed notch widens and shows the current synced line
* **Theme**: glow colour from the artwork (or Violet / Blue / Pink / Green / Off)
* Debug: `NOOK_DEBUG_LYRICS`, `NOOK_DEBUG_TABS`, `NOOK_DEBUG_LYRIC="text"`, `NOOK_DEBUG_NOHIDE`

## Known gaps

* Dragging files *out of* the shelf is not implemented (click opens, × removes; dropping files in works).
* Brightness HUD is not implemented.
* World-clock offsets are fixed (no daylight-saving rules); Islamic countdown dates are approximate.
* Prayer-city table covers province capitals and a few larger cities, not every kabupaten/kota.
* Glass blur is drawn by a small acrylic backing window that tracks the notch each frame, so during fast
  animation it can trail the notch by a frame.

## Third pass

* **Window never blocks other apps**: the overlay window is resized every frame to just the visible notch (plus a small
  shadow margin); the swap chain stays canvas-sized and a DirectComposition visual offset shows the right slice
  (`App::fit_window`, `Gfx::set_view_offset`). Before, a transparent 740x250 rectangle swallowed clicks aimed at
  whatever was under the top of the screen, e.g. Chrome's tab strip.
* **Panel**: solid black again. Always-visible dock with a capsule that glides to the current page (no side arrows, no
  hover-revealed tab bar), header aligned to the content edge, redesigned media page (ambient glow, round play button,
  friendly source name) and a five-line lyrics view.
* **Speed**: hover-to-open 230 -> 110 ms, springs ~40 % stiffer, text-width/fit results cached, stroke styles created
  once, artwork shared by `Arc` instead of being copied every frame.
* **Lyrics**: LRCLIB exact + search, NetEase Cloud Music, lyrics.ovh; titles are split into every plausible
  (artist, title) pair; results are matched by track name and duration; session cache.
* **Media**: sticky source choice, click the artwork to switch source, artwork retried after a track change, real
  cover lookup when a browser only supplies its own logo.
* **Full screen**: hides only when the foreground window covers the notch's monitor edge to edge (`sys::fullscreen_busy`).
* Debug: `NOOK_DEBUG_CLICK=1` logs clicks to `%TEMP%
ook.log`.

## Fourth pass

* **Style**: Settings -> Appearance -> *Style* = Notch / Floating / **Island**. Island is modelled on the iOS Dynamic Island:
  a pill floating 10 px below the top edge, bouncy spring when it grows and a quick critically-damped one when it
  shrinks, content that settles in (scale + fade), and a **split** mode where the pill divides into a music blob and a
  detached timer dot (the same idea as the `Split` mode in jhaemin/dynamic-island). Springs: grow omega 13 / zeta 0.66,
  shrink omega 24 / zeta 0.95.
* **Pomodoro** page: phase + big time + status, "This round" timeline (1 2 3 4 + long break), gradient Start button, reset
  and skip. A running timer also shows in the collapsed pill.
* **Water** page: amount + reminder switch, next reminder time, progress, hourly chart with a 0/200/400 ml scale, and a
  Settings sub-page (bottle, goal, active hours, reminder / refill count, reset).
* **Shelf**: dotted drop zone with tray icon and hint.
* Header: waveform at the right, page arrows back in their own gutter.
* Debug: `NOOK_DEBUG_STYLE=island`, `NOOK_DEBUG_TIMER=1`, `NOOK_DEBUG_WATERCFG=1`.

## Fifth pass

* **Logo**: `assets/icon.ico` / `icon.png` / `logo128.png` are generated from the desktop `Nook.png` (black corners made transparent);
  used for the exe, tray, window titles and the Settings sidebar.
* **Auto-update** (`update.rs`): reads `latest.json` + `nook.exe` from `github.com/Anassninab/nook/releases/latest/download/`
  (plain file downloads, no API rate limit), accepts a build only if size, `MZ` header and SHA-256 match, then renames the running
  exe to `nook.old.exe`, copies the new one in, starts it with `--updated <version>` and quits. The new process waits for the
  single-instance lock, shows an "updated" toast and deletes the old file. Installs only when the notch is idle (not hovered, no timer
  running, no settings window). Settings, General, Updates has the toggle, status and *Check now* / *Restart & update*.
* **Releasing**: bump `version` in `Cargo.toml`, run `.\publish.ps1 -Notes "..."`.
* Test hooks: `NOOK_UPDATE_BASE`, `NOOK_UPDATE_FIRST_SECS`, `NOOK_UPDATE_EVERY_SECS`.

## Sixth pass

* **Tutorial** (`tour.rs`): first launch walks through every part with a dimmed panel, a white frame and a tooltip (Back / Next /
  Skip all). Highlights are measured from the live panel every frame, so they match in Notch and Island styles. Replay: Settings,
  About, or the tray menu. Flag: `tour_done` in settings. Debug: `NOOK_DEBUG_TOUR=<step>`.
* **Morocco** prayer times: Fajr 19 deg, Isha 17 deg, Dhuhr +5 min, Maghrib +5 min (same as Aladhan method 21; a unit test checks 11 values
  against it). 12 regions / 45 cities; the clock offset follows the PC (so the Ramadan UTC+0 shift is handled). Names switch to
  Fajr / Dhuhr / Asr / Maghrib / Isha.
* **City picker**: tap the city chip on the Prayer page for a country-tabbed list (mouse wheel scrolls); Settings, Prayer has a country
  switch and a type-to-search finder. Debug: `NOOK_DEBUG_PICK=<country>`, `NOOK_DEBUG_CITY=<province>,<city>`, `NOOK_DEBUG_SEARCH=<text>`.
