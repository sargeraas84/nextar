//! nextar-gui — a modern desktop front-end for the nextar engine.
//!
//! Views: create (drop files/folders, codec, level, encryption, recovery),
//! extract, inspect (archive preview), and repair. Jobs run on background
//! threads and report progress through a shared [`ProgressState`] the UI
//! polls every frame. The brand logo is drawn with the painter — no image
//! assets needed.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use nextar::archive;
use nextar::format::{ArchiveHeader, Index};
use nextar::pipeline::CreateOptions;
use nextar::progress::ProgressState;

// ------------------------------------------------------------- palette
/// Theme-aware UI palette. The app is a synthwave UI in both Windows themes,
/// but surfaces and text flip between deep purple (dark mode) and
/// lavender-white (light mode), cross-fading with the same eased tween as
/// the logo tile. The getters (`bg()`, `text()`, …) always read the current
/// blended palette, which `refresh_palette()` recomputes once per frame.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    bg2: Color32,
    surface: Color32,
    surface2: Color32,
    border: Color32,
    text: Color32,
    text2: Color32,
    text3: Color32,
    accent: Color32,
    accent2: Color32,
    accent3: Color32,
    ok: Color32,
    err: Color32,
    active: Color32,
}

const UI_DARK: Palette = Palette {
    bg: Color32::from_rgb(0x07, 0x0a, 0x11), // near-black glass void
    bg2: Color32::from_rgb(0x0c, 0x11, 0x1b),
    surface: Color32::from_rgb(0x11, 0x17, 0x24), // glass panel
    surface2: Color32::from_rgb(0x19, 0x21, 0x31),
    border: Color32::from_rgb(0x24, 0x2e, 0x42), // hairline
    text: Color32::from_rgb(0xf4, 0xf6, 0xfb),
    text2: Color32::from_rgb(0xa4, 0xae, 0xc4),
    text3: Color32::from_rgb(0x64, 0x70, 0x88),
    accent: Color32::from_rgb(0x5f, 0xf2, 0xff), // ice cyan
    accent2: Color32::from_rgb(0x8b, 0x7b, 0xff), // violet
    accent3: Color32::from_rgb(0xff, 0x5f, 0xd7), // pink
    ok: Color32::from_rgb(0x4c, 0xdf, 0x9e),
    err: Color32::from_rgb(0xff, 0x5f, 0x7a),
    active: Color32::from_rgb(0x0d, 0x12, 0x1d),
};

const UI_LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf4, 0xf6, 0xfa), // cool silver-white
    bg2: Color32::from_rgb(0xea, 0xee, 0xf5),
    surface: Color32::from_rgb(0xff, 0xff, 0xff),
    surface2: Color32::from_rgb(0xe3, 0xe8, 0xf1),
    border: Color32::from_rgb(0xd3, 0xdb, 0xe7),
    text: Color32::from_rgb(0x18, 0x1d, 0x2a),
    text2: Color32::from_rgb(0x4d, 0x58, 0x6c),
    text3: Color32::from_rgb(0x8a, 0x94, 0xa8),
    accent: Color32::from_rgb(0x0a, 0x97, 0xbf), // deep cyan on white
    accent2: Color32::from_rgb(0x66, 0x4b, 0xe8), // violet
    accent3: Color32::from_rgb(0xd4, 0x2e, 0xb6), // pink
    ok: Color32::from_rgb(0x0f, 0x9d, 0x6f),
    err: Color32::from_rgb(0xd9, 0x37, 0x55),
    active: Color32::from_rgb(0xe1, 0xe6, 0xef),
};

static CURRENT_PALETTE: Mutex<Palette> = Mutex::new(UI_DARK);

fn current_palette() -> Palette {
    *CURRENT_PALETTE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Recompute the UI palette from the theme tween (same eased blend as the
/// logo tile). Called once per frame before painting.
fn refresh_palette() {
    let b = theme_blend();
    let mut g = CURRENT_PALETTE.lock().unwrap_or_else(|p| p.into_inner());
    *g = blend_ui(UI_DARK, UI_LIGHT, b);
}

fn blend_ui(a: Palette, b: Palette, t: f32) -> Palette {
    Palette {
        bg: blend_color(a.bg, b.bg, t),
        bg2: blend_color(a.bg2, b.bg2, t),
        surface: blend_color(a.surface, b.surface, t),
        surface2: blend_color(a.surface2, b.surface2, t),
        border: blend_color(a.border, b.border, t),
        text: blend_color(a.text, b.text, t),
        text2: blend_color(a.text2, b.text2, t),
        text3: blend_color(a.text3, b.text3, t),
        accent: blend_color(a.accent, b.accent, t),
        accent2: blend_color(a.accent2, b.accent2, t),
        accent3: blend_color(a.accent3, b.accent3, t),
        ok: blend_color(a.ok, b.ok, t),
        err: blend_color(a.err, b.err, t),
        active: blend_color(a.active, b.active, t),
    }
}

fn bg() -> Color32 { current_palette().bg }
fn bg2() -> Color32 { current_palette().bg2 }
fn surface() -> Color32 { current_palette().surface }
fn surface2() -> Color32 { current_palette().surface2 }
fn border() -> Color32 { current_palette().border }
fn text() -> Color32 { current_palette().text }
fn text2() -> Color32 { current_palette().text2 }
fn text3() -> Color32 { current_palette().text3 }
fn accent() -> Color32 { current_palette().accent }
fn accent2() -> Color32 { current_palette().accent2 }
fn accent3() -> Color32 { current_palette().accent3 }
fn ok() -> Color32 { current_palette().ok }
fn err() -> Color32 { current_palette().err }
fn active() -> Color32 { current_palette().active }
fn neon_cyan() -> Color32 { current_palette().accent }
fn neon_pink() -> Color32 { current_palette().accent3 }

const SPLASH_DURATION: f32 = 1.55;

fn grad_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = if t < 0.5 { (accent(), accent2()) } else { (accent2(), accent3()) };
    let u = if t < 0.5 { t * 2.0 } else { (t - 0.5) * 2.0 };
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * u).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

fn parse_block(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('k') | Some('K') => (&s[..s.len() - 1], 1024u64),
        Some('m') | Some('M') => (&s[..s.len() - 1], 1024u64 * 1024),
        Some('g') | Some('G') => (&s[..s.len() - 1], 1024u64 * 1024 * 1024),
        _ => (s, 1u64),
    };
    let n: u64 = num.trim().parse().map_err(|_| format!("invalid block size '{s}'"))?;
    let v = n * mult;
    if !(512..=64 * 1024 * 1024).contains(&v) {
        return Err(format!("block size must be between 512 and 64 MiB (got {s})"));
    }
    Ok(v as u32)
}

// ------------------------------------------------------------- logo
/// Scale a color's alpha (for splash fades).
fn alpha(c: Color32, a: f32) -> Color32 {
    let a = a.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a).round() as u8)
}

fn lerp8(x: u8, y: u8, t: f32) -> u8 {
    (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8
}

fn lerpf(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn smoothstep(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------- logo palette
/// Light/dark tile + mark colors for the brand lockup. The GUI picks the
/// palette from the Windows apps theme (light vs dark) and swaps live.
/// The mark is clean vector geometry: a glass tile with a hairline bezel
/// carrying three converging chevron planes (violet → indigo → cyan) that
/// feed a bright core node with a soft glow — files and streams compressed
/// into one compact, intelligent point.
#[derive(Clone, Copy)]
struct LogoPalette {
    tile_a: Color32,
    tile_b: Color32,
    tile_c: Color32,
    tile_mag: Color32, // soft pink glass reflection at the bottom
    layer_a: Color32,  // outer plane — violet
    layer_b: Color32,  // mid plane — indigo
    layer_c: Color32,  // inner plane — cyan
    core: Color32,     // bright core node
    glow: Color32,     // cyan core glow (with alpha)
    bezel: Color32,    // crisp hairline tile bezel (with alpha)
}

const LIGHT_PALETTE: LogoPalette = LogoPalette {
    tile_a: Color32::from_rgb(0xfb, 0xfd, 0xff),
    tile_b: Color32::from_rgb(0xe0, 0xee, 0xf9),
    tile_c: Color32::from_rgb(0xbf, 0xd9, 0xf0),
    tile_mag: Color32::from_rgb(0xff, 0x9e, 0xe6),
    layer_a: Color32::from_rgb(0x6b, 0x33, 0xb8), // deep violet (contrast on white)
    layer_b: Color32::from_rgb(0x2f, 0x5f, 0xc8), // indigo
    layer_c: Color32::from_rgb(0x00, 0x8f, 0xc7), // cyan
    core: Color32::from_rgb(0x00, 0xa8, 0xdd),    // cyan core node
    glow: Color32::from_rgba_unmultiplied_const(0x00, 0xb3, 0xe6, 120),
    bezel: Color32::from_rgba_unmultiplied_const(0x00, 0xd9, 0xff, 230), // neon cyan ring
};

const DARK_PALETTE: LogoPalette = LogoPalette {
    tile_a: Color32::from_rgb(0x0e, 0x1b, 0x38),
    tile_b: Color32::from_rgb(0x14, 0x2b, 0x52),
    tile_c: Color32::from_rgb(0x1c, 0x3a, 0x6a),
    tile_mag: Color32::from_rgb(0xff, 0x2b, 0xd6),
    layer_a: Color32::from_rgb(0x8b, 0x5c, 0xf6), // electric violet
    layer_b: Color32::from_rgb(0x5a, 0x7c, 0xf8), // neon indigo
    layer_c: Color32::from_rgb(0x37, 0xe6, 0xff), // electric cyan
    core: Color32::from_rgb(0x5e, 0xf2, 0xff),    // ice cyan core node
    glow: Color32::from_rgba_unmultiplied_const(0x5e, 0xf2, 0xff, 150),
    bezel: Color32::from_rgba_unmultiplied_const(0x5e, 0xf2, 0xff, 235), // neon ice-cyan ring
};

/// Windows apps theme, cached briefly so we don't hit the registry every
/// frame. 0 = light, 1 = dark (`AppsUseLightTheme`).
static THEME_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
const THEME_POLL: Duration = Duration::from_secs(3);

/// User-pinned appearance (Settings view). `Follow` tracks the Windows
/// theme; `Dark`/`Light` pin the theme independent of the OS. Persisted to
/// `%LOCALAPPDATA%\nextar\settings.json` so it survives relaunches.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeOverride {
    #[default]
    Follow,
    Dark,
    Light,
}

/// User settings, persisted to `%LOCALAPPDATA%\nextar\settings.json`.
/// Unset fields (None) fall back to the engine defaults. `appearance` is
/// read by every window (GUI, splash, shell mode, installer wizard); the
/// create defaults seed the Create view's fields on launch.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
struct Settings {
    #[serde(default)]
    appearance: ThemeOverride,
    /// Default codec for the Create view ("zstd" | "lzma2" | "store").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codec: Option<String>,
    /// Default compression level (zstd 0..22, lzma2 0..9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    level: Option<i32>,
    /// Default block size ("512K" | "1M" | …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    block: Option<String>,
    /// Default worker thread count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    threads: Option<usize>,
    /// Default recovery parity blocks (0 = off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<u16>,
    /// Recently opened / created archives (most recent first, newest six).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recent: Option<Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: ThemeOverride::Follow,
            codec: None,
            level: None,
            block: None,
            threads: None,
            recovery: None,
            recent: None,
        }
    }
}

/// Global copy of the settings, loaded once at startup and updated when the
/// user changes anything in the Settings view (so the splash, shell mode
/// and main UI all share it without threading state through every painter).
static SETTINGS: Mutex<Settings> = Mutex::new(Settings {
    appearance: ThemeOverride::Follow,
    codec: None,
    level: None,
    block: None,
    threads: None,
    recovery: None,
    recent: None,
});

/// Set when the settings file exists but couldn't be parsed — the Settings
/// view then offers a one-click reset instead of silently using defaults.
static SETTINGS_CORRUPT: Mutex<bool> = Mutex::new(false);

/// Settings file location: `%LOCALAPPDATA%\nextar\settings.json` (the same
/// folder the installer uses), falling back to `./nextar-settings.json`
/// when LOCALAPPDATA isn't set.
fn settings_path() -> PathBuf {
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        Path::new(&la).join("nextar").join("settings.json")
    } else {
        PathBuf::from("nextar-settings.json")
    }
}

fn save_settings_at(p: &Path, s: &Settings) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(s)?)?;
    Ok(())
}

fn load_settings_at(p: &Path) -> Option<Settings> {
    let raw = std::fs::read_to_string(p).ok()?;
    serde_json::from_str::<Settings>(&raw).ok()
}

/// Write the current global settings to disk (best-effort).
fn save_settings() {
    let s = SETTINGS.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let _ = save_settings_at(&settings_path(), &s);
}

/// Load the persisted settings into the global at startup. A missing file is
/// a fresh install; a present-but-unreadable file is flagged as corrupt so
/// the Settings view can offer to reset it.
fn load_settings() {
    let p = settings_path();
    match load_settings_at(&p) {
        Some(s) => {
            *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = s;
        }
        None => {
            if p.exists() {
                *SETTINGS_CORRUPT.lock().unwrap_or_else(|p| p.into_inner()) = true;
            }
        }
    }
}

fn theme_override() -> ThemeOverride {
    SETTINGS.lock().map(|g| g.appearance).unwrap_or(ThemeOverride::Follow)
}

fn settings_corrupt() -> bool {
    SETTINGS_CORRUPT.lock().map(|g| *g).unwrap_or(false)
}

/// Backup path for a settings file (same directory, timestamped name), so a
/// reset never destroys the user's only copy.
fn corrupt_backup_path(p: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    p.with_extension(format!("json.corrupt-{ts}"))
}

/// Back up the settings file and delete the original. Returns the backup
/// path, or None when no file existed.
fn backup_and_remove_settings() -> Option<PathBuf> {
    let p = settings_path();
    if !p.exists() {
        return None;
    }
    let backup = corrupt_backup_path(&p);
    let _ = std::fs::copy(&p, &backup);
    let _ = std::fs::remove_file(&p);
    Some(backup)
}

/// Delete the unreadable settings file (keeping a backup) and fall back to
/// defaults, clearing the corruption flag so the view stops offering reset.
fn reset_corrupt_settings() {
    let _ = backup_and_remove_settings();
    *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = Settings::default();
    *SETTINGS_CORRUPT.lock().unwrap_or_else(|p| p.into_inner()) = false;
}

/// Apply a user-chosen appearance override (Settings view) and persist it.
/// The next `windows_dark_mode()` call notices and starts the eased
/// cross-fade through the existing theme tween.
fn set_theme_override(o: ThemeOverride) {
    SETTINGS.lock().unwrap_or_else(|p| p.into_inner()).appearance = o;
    save_settings();
}

// ---- create-view defaults (seeded from settings, live in the global) ----
fn settings_create_codec() -> String {
    SETTINGS
        .lock()
        .ok()
        .and_then(|g| g.codec.clone())
        .unwrap_or_else(|| "zstd".to_string())
}

fn settings_create_level() -> i32 {
    SETTINGS.lock().ok().and_then(|g| g.level).unwrap_or(3)
}

fn settings_create_block() -> String {
    SETTINGS
        .lock()
        .ok()
        .and_then(|g| g.block.clone())
        .unwrap_or_else(|| "1M".to_string())
}

fn settings_create_threads() -> usize {
    SETTINGS.lock().ok().and_then(|g| g.threads).unwrap_or_else(num_cpus::get)
}

fn settings_create_recovery() -> u16 {
    SETTINGS.lock().ok().and_then(|g| g.recovery).unwrap_or(0)
}

/// Apply the create-view defaults from the Settings view and persist them.
fn set_create_defaults(codec: String, level: i32, block: String, threads: usize, recovery: u16) {
    {
        let mut g = SETTINGS.lock().unwrap_or_else(|p| p.into_inner());
        g.codec = Some(codec);
        g.level = Some(level);
        g.block = Some(block);
        g.threads = Some(threads);
        g.recovery = Some(recovery);
    }
    save_settings();
}

/// Recently opened / created archives (most recent first, newest six).
fn recent_archives() -> Vec<String> {
    SETTINGS.lock().ok().and_then(|g| g.recent.clone()).unwrap_or_default()
}

/// Remember an archive the user opened or produced, persisting it in
/// settings.json (callable from worker threads — it uses the global).
fn push_recent_archive(path: &str) {
    {
        let mut g = SETTINGS.lock().unwrap_or_else(|p| p.into_inner());
        let mut v = g.recent.clone().unwrap_or_default();
        v.retain(|x| x != path);
        v.insert(0, path.to_string());
        v.truncate(6);
        g.recent = Some(v);
    }
    save_settings();
}

/// Forget the recent-archives list (persisted).
fn clear_recent() {
    SETTINGS.lock().unwrap_or_else(|p| p.into_inner()).recent = None;
    save_settings();
}

/// Pure precedence rule (testable): env pin > user override > registry.
fn effective_dark(env_pin: Option<bool>, over: ThemeOverride, registry: bool) -> bool {
    if let Some(p) = env_pin {
        return p;
    }
    match over {
        ThemeOverride::Dark => true,
        ThemeOverride::Light => false,
        ThemeOverride::Follow => registry,
    }
}

/// Parse the `NEXTAR_LOGO_THEME` dev/CI pin (dark|light) into a bool.
fn env_pin(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

/// User preference cache for the Windows "Animation effects" toggle.
static REDUCED_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
const REDUCED_POLL: Duration = Duration::from_secs(5);

/// Does the user want fewer animations? Gates the logo entrance / hover
/// micro-motion and the progress-bar pulse. Read from the Windows
/// "Animation effects" toggle (`SystemDisableAnimations`), with a
/// `NEXTAR_REDUCED_MOTION=1` dev/CI pin. Missing key → animations on.
fn reduced_motion() -> bool {
    if let Ok(v) = std::env::var("NEXTAR_REDUCED_MOTION") {
        return v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on");
    }
    #[cfg(windows)]
    {
        if let Ok(mut g) = REDUCED_CACHE.lock() {
            if let Some((at, v)) = *g {
                if at.elapsed() < REDUCED_POLL {
                    return v;
                }
            }
            let v = read_reduced_motion();
            *g = Some((Instant::now(), v));
            return v;
        }
    }
    false
}

#[cfg(windows)]
fn read_reduced_motion() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Accessibility") {
        Ok(k) => k
            .get_value::<u32, _>("SystemDisableAnimations")
            .map(|v| v != 0)
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(windows)]
fn windows_dark_mode() -> bool {
    // Dev/test override (also lets CI pin the palette on machines without
    // the Personalize registry key): NEXTAR_LOGO_THEME=dark|light.
    let pin = std::env::var("NEXTAR_LOGO_THEME").ok().and_then(|v| env_pin(&v));
    // User setting from the Settings view pins the theme independent of the OS.
    let over = theme_override();
    if pin.is_some() || over != ThemeOverride::Follow {
        return effective_dark(pin, over, false);
    }
    // Follow: read the registry, cached briefly so we don't hit it per frame.
    if let Ok(mut g) = THEME_CACHE.lock() {
        if let Some((at, dark)) = *g {
            if at.elapsed() < THEME_POLL {
                return dark;
            }
        }
        let dark = read_registry_dark();
        *g = Some((Instant::now(), dark));
        dark
    } else {
        false
    }
}

#[cfg(windows)]
fn read_registry_dark() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    match hkcu.open_subkey(path) {
        Ok(k) => k
            .get_value::<u32, _>("AppsUseLightTheme")
            .map(|v| v == 0)
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn windows_dark_mode() -> bool {
    effective_dark(None, theme_override(), false)
}

/// Cross-fade between the light and dark tiles when the Windows theme
/// changes, instead of an instant swap. `theme_blend()` returns 0.0 (light)
/// → 1.0 (dark), advancing the tween on every call (painters call it each
/// frame; the UI requests continuous repaint while it's mid-flight).
static THEME_TWEEN: Mutex<Option<ThemeTween>> = Mutex::new(None);
const THEME_TRANSITION: Duration = Duration::from_millis(450);

struct ThemeTween {
    start: Instant,
    start_blend: f32,
    target: bool, // false = light, true = dark
}

impl ThemeTween {
    /// Blend at eased progress `p` (0..1 within the transition window).
    fn current_at(&self, p: f32) -> f32 {
        let e = smoothstep(p.clamp(0.0, 1.0), 0.0, 1.0);
        if self.target {
            self.start_blend + (1.0 - self.start_blend) * e
        } else {
            self.start_blend * (1.0 - e)
        }
    }

    fn current(&self) -> f32 {
        let p = self.start.elapsed().as_secs_f32() / THEME_TRANSITION.as_secs_f32();
        self.current_at(p)
    }
}

fn theme_blend() -> f32 {
    let dark = windows_dark_mode();
    let mut g = THEME_TWEEN.lock().unwrap_or_else(|p| p.into_inner());
    if g.is_none() {
        // First call: snap to the detected theme (no boot animation).
        *g = Some(ThemeTween {
            start: Instant::now(),
            start_blend: if dark { 1.0 } else { 0.0 },
            target: dark,
        });
        return if dark { 1.0 } else { 0.0 };
    }
    if let Some(t) = g.as_ref() {
        if t.target != dark {
            // Theme flipped: morph from wherever the blend currently is.
            let cur = t.current();
            *g = Some(ThemeTween {
                start: Instant::now(),
                start_blend: cur,
                target: dark,
            });
        }
    }
    g.as_ref().map(|t| t.current()).unwrap_or(0.0)
}

fn theme_transitioning() -> bool {
    let b = theme_blend();
    b > 0.001 && b < 0.999
}

fn logo_palette() -> LogoPalette {
    blend_palettes(LIGHT_PALETTE, DARK_PALETTE, theme_blend())
}

fn blend_color(a: Color32, b: Color32, t: f32) -> Color32 {
    Color32::from_rgba_premultiplied(
        lerp8(a.r(), b.r(), t),
        lerp8(a.g(), b.g(), t),
        lerp8(a.b(), b.b(), t),
        lerp8(a.a(), b.a(), t),
    )
}

fn blend_palettes(a: LogoPalette, b: LogoPalette, t: f32) -> LogoPalette {
    LogoPalette {
        tile_a: blend_color(a.tile_a, b.tile_a, t),
        tile_b: blend_color(a.tile_b, b.tile_b, t),
        tile_c: blend_color(a.tile_c, b.tile_c, t),
        tile_mag: blend_color(a.tile_mag, b.tile_mag, t),
        layer_a: blend_color(a.layer_a, b.layer_a, t),
        layer_b: blend_color(a.layer_b, b.layer_b, t),
        layer_c: blend_color(a.layer_c, b.layer_c, t),
        core: blend_color(a.core, b.core, t),
        glow: blend_color(a.glow, b.glow, t),
        bezel: blend_color(a.bezel, b.bezel, t),
    }
}

/// Draw the brand lockup: a circular glass tile with a neon-cyan ring
/// carrying the convergence core — three nested chevron planes (violet →
/// indigo → cyan) that fold inward and feed a bright core node. The tile
/// swaps between frosted white (light mode) and deep navy (dark mode) to
/// follow the Windows theme; the mark geometry mirrors the icon generator
/// exactly.
fn draw_logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    draw_logo_at(ui.painter(), rect, 1.0);
}

/// The glass tile + neon ring only (the mark is composed separately during
/// the boot animation so its layers can fold in).
fn draw_logo_tile(p: &egui::Painter, rect: egui::Rect, fade: f32) {
    let w = rect.width();
    let pal = logo_palette();
    let center = rect.center();
    let radius = 0.44 * w; // circle inscribed in the 6%-inset content box
    p.add(circle_tile_mesh(rect, radius, |t| alpha(tile_grad(t, pal), fade)));
    p.circle_stroke(center, radius, Stroke::new((0.018 * w).max(1.2), alpha(pal.bezel, fade)));
}

fn draw_logo_at(p: &egui::Painter, rect: egui::Rect, fade: f32) {
    draw_logo_tile(p, rect, fade);
    converge_mark(p, rect, logo_palette(), fade, 1.0);
}

/// A perfect circular tile filled with a smooth vertical gradient,
/// tessellated as a mesh with per-vertex colors sampled from `color_at(0..1)`.
/// Mirrors the icon generator's circular-tile SDF so the tile is a clean
/// circle at every size (sidebar, Home, splash, shell, wizard).
fn circle_tile_mesh(rect: egui::Rect, radius: f32, color_at: impl Fn(f32) -> Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    let c = rect.center();
    let r = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5);
    let inv_h = 1.0 / rect.height().max(f32::EPSILON);

    let v = |mesh: &mut egui::Mesh, x: f32, y: f32| {
        let idx = mesh.vertices.len() as u32;
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(x, y),
            uv: egui::epaint::WHITE_UV,
            color: color_at(((y - rect.top()) * inv_h).clamp(0.0, 1.0)),
        });
        idx
    };
    let center = v(&mut mesh, c.x, c.y);
    let n = 48usize;
    let ring0 = (mesh.vertices.len()) as u32;
    for k in 0..n {
        let a = std::f32::consts::TAU * k as f32 / n as f32;
        v(&mut mesh, c.x + r * a.cos(), c.y + r * a.sin());
    }
    // fan from the center; winding matches the chrome mesh convention
    for k in 0..n {
        mesh.indices.extend_from_slice(&[
            center,
            ring0 + k as u32,
            ring0 + ((k + 1) % n) as u32,
        ]);
    }
    mesh
}

/// Tile gradient (top → bottom): frosted white → pale cyan → cool blue for
/// light mode, deep navy glass for dark mode; soft pink reflection at the
/// bottom edge.
fn tile_grad(t: f32, pal: LogoPalette) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let c = if t < 0.5 {
        Color32::from_rgb(
            lerp8(pal.tile_a.r(), pal.tile_b.r(), t * 2.0),
            lerp8(pal.tile_a.g(), pal.tile_b.g(), t * 2.0),
            lerp8(pal.tile_a.b(), pal.tile_b.b(), t * 2.0),
        )
    } else {
        Color32::from_rgb(
            lerp8(pal.tile_b.r(), pal.tile_c.r(), (t - 0.5) * 2.0),
            lerp8(pal.tile_b.g(), pal.tile_c.g(), (t - 0.5) * 2.0),
            lerp8(pal.tile_b.b(), pal.tile_c.b(), (t - 0.5) * 2.0),
        )
    };
    let u = (t - 1.12) / 0.30;
    let mag = (-u * u).exp() * 0.22;
    Color32::from_rgb(
        lerp8(c.r(), pal.tile_mag.r(), mag),
        lerp8(c.g(), pal.tile_mag.g(), mag),
        lerp8(c.b(), pal.tile_mag.b(), mag),
    )
}

/// One converging chevron plane: half-reach, top y, apex y, half-width.
/// Unit coords, y down; planes are drawn outer → inner (violet → cyan),
/// each narrower and deeper than the last, feeding the core node.
const PLANES: [(f32, f32, f32, f32); 3] = [
    (0.300, 0.240, 0.520, 0.030),
    (0.215, 0.360, 0.610, 0.028),
    (0.130, 0.470, 0.680, 0.026),
];

/// Core node position + radius (unit coords, y down).
const CORE: (f32, f32, f32) = (0.5, 0.735, 0.050);

/// The convergence-core mark: three nested chevron planes fold inward
/// (violet → indigo → cyan) and feed a bright core node with a soft glow —
/// files, folders and data streams compressed into one intelligent point.
/// Unit coords mirror the icon generator exactly; the tile is the 6%-inset
/// content box (0.06 + 0.88u). `build` (0..1) staggers the planes outer →
/// inner so the boot moment looks like layers folding into place.
fn converge_mark(p: &egui::Painter, rect: egui::Rect, pal: LogoPalette, fade: f32, build: f32) {
    let w = rect.width();
    let h = rect.height();
    let ux = |v: f32| rect.left() + (0.06 + 0.88 * v) * w;
    let uy = |t: f32| rect.top() + (0.06 + 0.88 * t) * h;
    let colors = [pal.layer_a, pal.layer_b, pal.layer_c];
    for (k, (reach, top, apex, hw)) in PLANES.iter().enumerate() {
        let kf = fade * smoothstep((build - 0.14 * k as f32) / 0.5, 0.0, 1.0);
        if kf <= 0.002 {
            continue;
        }
        let l = egui::pos2(ux(0.5 - reach), uy(*top));
        let a = egui::pos2(ux(0.5), uy(*apex));
        let r = egui::pos2(ux(0.5 + reach), uy(*top));
        let stroke = Stroke::new((2.0 * hw * 0.88 * w).max(1.0), alpha(colors[k], kf));
        p.add(egui::Shape::line(vec![l, a, r], stroke));
    }
    // core node: soft glow halo + bright dot + white inner spark
    let kf = fade * smoothstep((build - 0.25) / 0.5, 0.0, 1.0);
    if kf > 0.002 {
        let core = egui::pos2(ux(CORE.0), uy(CORE.1));
        let r = CORE.2 * 0.88 * w;
        p.circle_filled(core, r * 1.7, alpha(pal.glow, 0.30 * kf));
        p.circle_filled(core, r * 1.35, alpha(pal.glow, 0.40 * kf));
        p.circle_filled(core, r, alpha(pal.core, kf));
        p.circle_filled(egui::pos2(core.x, core.y - r * 0.25), r * 0.38, alpha(Color32::WHITE, 0.9 * kf));
    }
}

/// The boot-moment fragments: scattered digital particles converge into the
/// core (the "files being compressed" read). `t` is seconds since launch.
fn draw_converging_particles(p: &egui::Painter, rect: egui::Rect, t: f32, pal: LogoPalette) {
    let w = rect.width();
    let h = rect.height();
    let ux = |v: f32| rect.left() + (0.06 + 0.88 * v) * w;
    let uy = |v: f32| rect.top() + (0.06 + 0.88 * v) * h;
    let core = egui::pos2(ux(CORE.0), uy(CORE.1));
    const FRAG: [(f32, f32); 8] = [
        (0.16, 0.16), (0.84, 0.16), (0.20, 0.46), (0.80, 0.46),
        (0.28, 0.74), (0.72, 0.74), (0.42, 0.10), (0.58, 0.10),
    ];
    for (i, (fx, fy)) in FRAG.iter().enumerate() {
        let delay = i as f32 * 0.035;
        let u = ((t - delay) / 0.30).clamp(0.0, 1.0);
        if u <= 0.0 || u >= 1.0 {
            continue;
        }
        let e = smoothstep(u, 0.0, 1.0);
        let start = egui::pos2(ux(*fx), uy(*fy));
        let pos = egui::pos2(lerpf(start.x, core.x, e), lerpf(start.y, core.y, e));
        let a = (u * (1.0 - u)) * 4.0;
        let col = if i % 2 == 0 { pal.layer_c } else { pal.layer_a };
        p.circle_filled(pos, (0.016 * w).max(1.2), alpha(col, a * 0.9));
    }
}

/// One quad (4 vertices, fan-triangulated) with per-vertex colors.
fn quad(mesh: &mut egui::Mesh, pts: [egui::Pos2; 4], colors: [Color32; 4]) {
    let v0 = mesh.vertices.len() as u32;
    for (i, pt) in pts.iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *pt,
            uv: egui::epaint::WHITE_UV,
            color: colors[i],
        });
    }
    mesh.indices.extend_from_slice(&[v0, v0 + 1, v0 + 2, v0, v0 + 2, v0 + 3]);
}

/// A wide, soft-edged light band for the logo sweep. The band is subdivided
/// into vertical strips with a gaussian alpha falloff across its width so the
/// edges feather out — reads as light, not a hard stripe. `echo` draws the
/// faint uniform-cyan trail behind the main band; otherwise it's the
/// cyan→violet gradient band (cyan on the leading edge, the direction of
/// travel).
fn draw_sweep_band(p: &egui::Painter, box_r: egui::Rect, cx: f32, slope: f32, fade: f32, echo: bool) {
    if fade <= 0.0 {
        return;
    }
    let cyan = neon_cyan();
    let violet = accent2();
    let half = box_r.width() * if echo { 0.10 } else { 0.16 };
    let n = 16usize;
    let mut sm = egui::Mesh::default();
    for i in 0..n {
        let u0 = -1.0 + 2.0 * i as f32 / n as f32;
        let u1 = -1.0 + 2.0 * (i + 1) as f32 / n as f32;
        let um = (u0 + u1) * 0.5;
        let g = (-(um * um) / (2.0 * 0.42 * 0.42)).exp();
        let col = if echo {
            cyan
        } else {
            blend_color(violet, cyan, 0.5 + 0.5 * um)
        };
        let a = (if echo { 0.18 } else { 0.50 }) * g * fade;
        let x0 = cx + u0 * half;
        let x1 = cx + u1 * half;
        quad(
            &mut sm,
            [
                egui::pos2(x0, box_r.top()),
                egui::pos2(x1, box_r.top()),
                egui::pos2(x1 + slope, box_r.bottom()),
                egui::pos2(x0 + slope, box_r.bottom()),
            ],
            [alpha(col, a), alpha(col, a), alpha(col, a), alpha(col, a)],
        );
    }
    p.with_clip_rect(box_r).add(sm);
}

/// Gradient-colored text (per character) for the wordmark.
fn grad_text(s: &str, size: f32, strong: bool) -> egui::WidgetText {
    let mut job = egui::text::LayoutJob::default();
    let n = s.chars().count().max(1);
    for (i, c) in s.chars().enumerate() {
        job.append(
            &c.to_string(),
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(size),
                color: grad_color(i as f32 / (n - 1) as f32),
                extra_letter_spacing: if strong { 1.0 } else { 0.0 },
                ..Default::default()
            },
        );
    }
    job.into()
}

/// Per-letter staggered fade-in wordmark (mirrors the splash) for the Home
/// hero lockup. `t` is seconds since the view opened so it replays every
/// time Home is entered; reduced motion renders the static gradient.
fn draw_wordmark_stagger(ui: &mut egui::Ui, t: f32, size: f32) {
    let word = "NEXTAR";
    let font_id = egui::FontId::proportional(size);
    let tracking = 4.0;
    let glyphs: Vec<(char, f32)> = word
        .chars()
        .map(|c| {
            (
                c,
                ui.painter()
                    .layout_no_wrap(c.to_string(), font_id.clone(), Color32::WHITE)
                    .size()
                    .x,
            )
        })
        .collect();
    let total_w: f32 =
        glyphs.iter().map(|(_, w)| *w).sum::<f32>() + tracking * (glyphs.len() as f32 - 1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total_w, size + 8.0), egui::Sense::hover());
    let p = ui.painter();
    let base_y = rect.center().y;
    let mut x = rect.left();
    for (i, (ch, w)) in glyphs.iter().enumerate() {
        let e = if reduced_motion() {
            1.0
        } else {
            smoothstep(((t - i as f32 * 0.06) / 0.18).clamp(0.0, 1.0), 0.0, 1.0)
        };
        let ly = base_y - (1.0 - e) * 4.0;
        p.text(
            egui::pos2(x, ly),
            egui::Align2::LEFT_CENTER,
            ch.to_string(),
            font_id.clone(),
            alpha(grad_color(i as f32 / (glyphs.len() - 1) as f32), e),
        );
        x += w + tracking;
    }
}

// ------------------------------------------------------------- retro widgets
/// Glass progress bar: rounded track, smooth cyan→violet gradient fill
/// with a soft glow and a pulsing bright head (modern, matches the site).
fn led_bar(ui: &mut egui::Ui, pct: f32, label: &str, animated: bool) {
    let width = ui.available_width().min(560.0);
    let height = 14.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height + 22.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, Vec2::new(width, height));
    let p = ui.painter();
    let r = CornerRadius::same(7);
    // track: glass well with a hairline
    p.rect_filled(bar, r, surface2());
    p.rect_stroke(bar, r, Stroke::new(1.0, border()), egui::StrokeKind::Inside);
    let t = pct.clamp(0.0, 1.0);
    if t > 0.001 {
        let fill = egui::Rect::from_min_size(bar.min, Vec2::new((width * t).max(height), height));
        // soft glow under the fill
        p.rect_filled(fill.expand2(Vec2::new(0.0, 5.0)), r, alpha(grad_color(0.3), 0.16));
        // smooth horizontal gradient fill
        let n = 24usize;
        for i in 0..n {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            let seg = egui::Rect::from_min_max(
                egui::pos2(fill.left() + t0 * fill.width(), fill.top()),
                egui::pos2(fill.left() + t1 * fill.width(), fill.bottom()),
            );
            let c = grad_color((t0 + t1) * 0.5);
            let cr = if i == 0 {
                CornerRadius { nw: 7, ne: 0, sw: 7, se: 0 }
            } else if i == n - 1 {
                CornerRadius { nw: 0, ne: 7, sw: 0, se: 7 }
            } else {
                CornerRadius::ZERO
            };
            p.rect_filled(seg, cr, c);
        }
        if animated && !reduced_motion() {
            // pulsing bright head
            let pulse = ((ui.input(|i| i.time) * 2.2).sin() * 0.5 + 0.5) as f32;
            let head = egui::Rect::from_min_size(
                egui::pos2(fill.right() - height * 0.55, bar.top() + 2.0),
                Vec2::new(height * 0.55, height - 4.0),
            );
            p.rect_filled(head, CornerRadius::same(5), Color32::from_rgba_unmultiplied(255, 255, 255, (70.0 + 120.0 * pulse) as u8));
        }
    }
    p.text(
        egui::pos2(rect.left(), bar.bottom() + 12.0),
        egui::Align2::LEFT_CENTER,
        format!("{:.0}% · {label}", (pct * 100.0).round()),
        egui::FontId::proportional(11.5),
        text2(),
    );
}

fn dim(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f).round() as u8,
        (c.g() as f32 * f).round() as u8,
        (c.b() as f32 * f).round() as u8,
    )
}

/// Small vector icons for the nav and action surfaces (painted strokes —
/// no emoji, no image assets, scales cleanly at every size).
#[derive(Clone, Copy)]
enum Icon {
    Home,
    Create,
    Extract,
    Inspect,
    Repair,
    Settings,
}

fn draw_icon(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: Color32, w: f32) {
    let s = Stroke::new(w, color);
    let c = rect.center();
    match icon {
        Icon::Home => {
            p.line_segment([egui::pos2(rect.left(), c.y - 3.0), egui::pos2(c.x, rect.top() + 1.0)], s);
            p.line_segment([egui::pos2(c.x, rect.top() + 1.0), egui::pos2(rect.right(), c.y - 3.0)], s);
            p.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 2.5, c.y - 3.0),
                    egui::pos2(rect.right() - 2.5, rect.bottom() - 1.0),
                ),
                CornerRadius::same(1),
                s,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Create => {
            p.line_segment([egui::pos2(rect.left() + 2.0, c.y), egui::pos2(rect.right() - 2.0, c.y)], s);
            p.line_segment([egui::pos2(c.x, rect.top() + 2.0), egui::pos2(c.x, rect.bottom() - 2.0)], s);
        }
        Icon::Extract => {
            p.rect_stroke(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 1.0, rect.top() + 1.0),
                    egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0),
                ),
                CornerRadius::same(2),
                s,
                egui::StrokeKind::Inside,
            );
            let ay0 = rect.bottom() - 4.0;
            let ay1 = rect.top() + 5.0;
            p.line_segment([egui::pos2(c.x, ay0), egui::pos2(c.x, ay1)], s);
            p.line_segment([egui::pos2(c.x - 3.0, ay1 + 3.0), egui::pos2(c.x, ay1)], s);
            p.line_segment([egui::pos2(c.x + 3.0, ay1 + 3.0), egui::pos2(c.x, ay1)], s);
        }
        Icon::Inspect => {
            p.circle_stroke(egui::pos2(c.x - 2.0, c.y - 2.0), rect.width() * 0.30, s);
            p.line_segment(
                [egui::pos2(c.x + 2.0, c.y + 2.0), egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0)],
                s,
            );
        }
        Icon::Repair => {
            let top = rect.top() + 1.0;
            let bot = rect.bottom() - 1.0;
            let pts = [
                egui::pos2(rect.left() + 1.0, top + 3.0),
                egui::pos2(c.x, top),
                egui::pos2(rect.right() - 1.0, top + 3.0),
                egui::pos2(rect.right() - 2.0, c.y),
                egui::pos2(c.x, bot),
                egui::pos2(rect.left() + 2.0, c.y),
            ];
            p.add(egui::Shape::closed_line(pts.to_vec(), s));
            p.line_segment([egui::pos2(c.x - 4.0, c.y - 0.5), egui::pos2(c.x - 1.0, c.y + 2.5)], s);
            p.line_segment([egui::pos2(c.x - 1.0, c.y + 2.5), egui::pos2(c.x + 4.0, c.y - 2.5)], s);
        }
        Icon::Settings => {
            for (i, &u) in [0.28f32, 0.5, 0.72].iter().enumerate() {
                let y = rect.top() + rect.height() * u;
                p.line_segment([egui::pos2(rect.left() + 1.0, y), egui::pos2(rect.right() - 1.0, y)], s);
                let kx = match i {
                    0 => rect.left() + rect.width() * 0.30,
                    1 => rect.left() + rect.width() * 0.62,
                    _ => rect.left() + rect.width() * 0.42,
                };
                p.circle_filled(egui::pos2(kx, y), 1.7, color);
            }
        }
    }
}

/// Primary action button (Create / Extract / Repair): a cyan gradient pill
/// with a painted icon, a soft glow on hover, press-down physics, and a
/// busy state (pulsing dot + ellipsis label). While busy the button is
/// non-interactive; callers also guard with `!self.busy()` so a running
/// job can never be re-triggered.
fn action_button(ui: &mut egui::Ui, label: &str, icon: Option<Icon>, size: f32, busy: bool) -> egui::Response {
    let txt_c = Color32::from_rgb(0x04, 0x12, 0x1a);
    let icon_s = size * 1.25;
    let gap = 9.0;
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), egui::FontId::proportional(size), txt_c);
    let w = galley.size().x + 40.0 + if icon.is_some() { icon_s + gap } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, galley.size().y + 20.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered() && !busy;
        let pressed = resp.is_pointer_button_down_on() && !busy;
        let shift = if pressed { 1.2 } else { 0.0 };
        let r = rect.translate(Vec2::new(0.0, shift));
        let corner = CornerRadius::same(255);
        p.rect_filled(
            r.translate(Vec2::new(0.0, 2.5 - shift)),
            corner,
            Color32::from_rgba_unmultiplied(0, 0, 0, if hovered { 90 } else { 45 }),
        );
        if busy {
            p.rect_filled(r, corner, surface());
            p.rect_stroke(r, corner, Stroke::new(1.0, border()), egui::StrokeKind::Inside);
            let pulse = ((ui.input(|i| i.time) * 2.6).sin() * 0.5 + 0.5) as f32;
            p.circle_filled(
                egui::pos2(r.left() + 24.0, r.center().y),
                4.0,
                alpha(grad_color(0.2), 0.35 + 0.65 * pulse),
            );
            p.circle_filled(egui::pos2(r.left() + 24.0, r.center().y), 8.0, alpha(grad_color(0.2), 0.12));
            p.text(
                egui::pos2(r.left() + 36.0, r.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{label}…"),
                egui::FontId::proportional(size),
                text2(),
            );
            return resp;
        }
        let body = if pressed { dim(grad_color(0.14), 0.85) } else { grad_color(0.14) };
        p.rect_filled(r, corner, body);
        let stroke_c = Color32::from_rgba_unmultiplied(255, 255, 255, if hovered { 115 } else { 55 });
        p.rect_stroke(r, corner, Stroke::new(1.0, stroke_c), egui::StrokeKind::Inside);
        if hovered && !pressed {
            p.rect_filled(r.expand2(Vec2::new(5.0, 5.0)), corner, alpha(neon_cyan(), 0.10));
        }
        if resp.has_focus() {
            p.rect_stroke(r.expand2(Vec2::new(2.0, 2.0)), corner, Stroke::new(1.5, alpha(neon_cyan(), 0.9)), egui::StrokeKind::Inside);
        }
        let mut x = r.center().x - (galley.size().x + if icon.is_some() { icon_s + gap } else { 0.0 }) * 0.5;
        if let Some(ic) = icon {
            let icon_r = egui::Rect::from_center_size(egui::pos2(x + icon_s * 0.5, r.center().y), Vec2::splat(icon_s));
            draw_icon(&p, icon_r, ic, Color32::from_rgb(0x04, 0x12, 0x1a), 1.8);
            x += icon_s + gap;
        }
        p.text(
            egui::pos2(x + galley.size().x * 0.5, r.center().y),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(size),
            if pressed { dim(txt_c, 0.85) } else { txt_c },
        );
    }
    resp
}

/// Glass quick-action card (Home): icon chip, title, description and a
/// chevron, with hover lift + cyan glow. Painted directly so it can fade
/// and slide in on view entry (`enter` is the eased 0..1 entrance).
fn action_card(ui: &mut egui::Ui, icon: Icon, title: &str, desc: &str, col: Color32, enter: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(176.0, 106.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter();
    let a = smoothstep(enter, 0.0, 1.0);
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();
    let r = rect.translate(Vec2::new(0.0, (1.0 - a) * 14.0 - if hovered { 2.0 } else { 0.0 }));
    let corner = CornerRadius::same(14);
    p.rect_filled(
        r.translate(Vec2::new(0.0, 3.0)),
        corner,
        Color32::from_rgba_unmultiplied(0, 0, 0, if hovered { 70 } else { 40 }),
    );
    p.rect_filled(r, corner, alpha(if pressed { dim(bg2(), 0.88) } else { bg2() }, a));
    p.rect_stroke(
        r,
        corner,
        Stroke::new(if hovered { 1.5 } else { 1.0 }, if hovered { alpha(col, 0.85) } else { alpha(border(), a) }),
        egui::StrokeKind::Inside,
    );
    p.line_segment(
        [egui::pos2(r.left() + 7.0, r.top() + 1.5), egui::pos2(r.right() - 7.0, r.top() + 1.5)],
        Stroke::new(1.0, alpha(Color32::from_rgb(255, 255, 255), 0.10 * a)),
    );
    let chip = egui::Rect::from_min_size(egui::pos2(r.left() + 14.0, r.top() + 14.0), Vec2::splat(34.0));
    p.rect_filled(chip, CornerRadius::same(10), alpha(col, 0.13 * a));
    p.rect_stroke(chip, CornerRadius::same(10), Stroke::new(1.0, alpha(col, 0.55 * a)), egui::StrokeKind::Inside);
    draw_icon(&p, egui::Rect::from_center_size(chip.center(), Vec2::splat(17.0)), icon, alpha(col, a), 1.8);
    p.text(
        egui::pos2(r.left() + 14.0, chip.bottom() + 14.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(15.0),
        alpha(text(), a),
    );
    p.text(
        egui::pos2(r.left() + 14.0, r.bottom() - 14.0),
        egui::Align2::LEFT_CENTER,
        desc,
        egui::FontId::proportional(11.0),
        alpha(text3(), a),
    );
    let chev = alpha(if hovered { col } else { text3() }, a);
    p.line_segment([egui::pos2(r.right() - 21.0, r.center().y - 3.5), egui::pos2(r.right() - 16.0, r.center().y)], Stroke::new(1.6, chev));
    p.line_segment([egui::pos2(r.right() - 16.0, r.center().y), egui::pos2(r.right() - 21.0, r.center().y + 3.5)], Stroke::new(1.6, chev));
    if resp.has_focus() {
        p.rect_stroke(r.expand2(Vec2::new(2.0, 2.0)), corner, Stroke::new(1.5, alpha(neon_cyan(), 0.9)), egui::StrokeKind::Inside);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// The Home centerpiece: a large glass drop target with corner brackets,
/// a down-arrow chip and a softly pulsing hairline. Clicking opens the
/// picker; actual drops are handled by `handle_drops`.
fn home_drop_zone(ui: &mut egui::Ui) -> egui::Response {
    let w = ui.available_width().min(560.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 148.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter();
    let hovered = resp.hovered();
    let pulse = ((ui.input(|i| i.time) * 1.6).sin() * 0.5 + 0.5) as f32;
    let corner = CornerRadius::same(18);
    p.rect_filled(rect, corner, alpha(surface(), if hovered { 1.0 } else { 0.86 }));
    p.rect_stroke(
        rect,
        corner,
        Stroke::new(
            if hovered { 1.8 } else { 1.2 },
            alpha(if hovered { neon_cyan() } else { border() }, if hovered { 0.9 } else { 0.45 + 0.3 * pulse }),
        ),
        egui::StrokeKind::Inside,
    );
    // corner brackets: a premium focus ring
    let o = 12.0f32;
    let l = 16.0f32;
    let br = Stroke::new(2.0, alpha(neon_cyan(), 0.75));
    p.line_segment([egui::pos2(rect.left() + o, rect.top() + o + l), egui::pos2(rect.left() + o, rect.top() + o)], br);
    p.line_segment([egui::pos2(rect.left() + o, rect.top() + o), egui::pos2(rect.left() + o + l, rect.top() + o)], br);
    p.line_segment([egui::pos2(rect.right() - o - l, rect.top() + o), egui::pos2(rect.right() - o, rect.top() + o)], br);
    p.line_segment([egui::pos2(rect.right() - o, rect.top() + o), egui::pos2(rect.right() - o, rect.top() + o + l)], br);
    p.line_segment([egui::pos2(rect.left() + o, rect.bottom() - o - l), egui::pos2(rect.left() + o, rect.bottom() - o)], br);
    p.line_segment([egui::pos2(rect.left() + o, rect.bottom() - o), egui::pos2(rect.left() + o + l, rect.bottom() - o)], br);
    p.line_segment([egui::pos2(rect.right() - o - l, rect.bottom() - o), egui::pos2(rect.right() - o, rect.bottom() - o)], br);
    p.line_segment([egui::pos2(rect.right() - o, rect.bottom() - o), egui::pos2(rect.right() - o, rect.bottom() - o - l)], br);
    // center chip + copy
    let chip = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 46.0), Vec2::splat(40.0));
    p.rect_filled(chip, CornerRadius::same(12), alpha(grad_color(0.2), 0.14));
    p.rect_stroke(chip, CornerRadius::same(12), Stroke::new(1.0, alpha(grad_color(0.2), 0.5)), egui::StrokeKind::Inside);
    let ax = chip.center().x;
    p.line_segment([egui::pos2(ax, chip.top() + 7.0), egui::pos2(ax, chip.bottom() - 7.0)], Stroke::new(2.0, alpha(neon_cyan(), 0.9)));
    p.line_segment([egui::pos2(ax - 4.5, chip.bottom() - 11.5), egui::pos2(ax, chip.bottom() - 7.0)], Stroke::new(2.0, alpha(neon_cyan(), 0.9)));
    p.line_segment([egui::pos2(ax + 4.5, chip.bottom() - 11.5), egui::pos2(ax, chip.bottom() - 7.0)], Stroke::new(2.0, alpha(neon_cyan(), 0.9)));
    p.text(
        egui::pos2(rect.center().x, chip.bottom() + 20.0),
        egui::Align2::CENTER_CENTER,
        "Drop files & folders here",
        egui::FontId::proportional(15.0),
        text(),
    );
    p.text(
        egui::pos2(rect.center().x, chip.bottom() + 38.0),
        egui::Align2::CENTER_CENTER,
        "click to browse — .next archives open in Inspect",
        egui::FontId::proportional(11.5),
        text3(),
    );
    if resp.has_focus() {
        p.rect_stroke(rect.expand2(Vec2::new(2.0, 2.0)), corner, Stroke::new(1.5, alpha(neon_cyan(), 0.9)), egui::StrokeKind::Inside);
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Sidebar navigation item: painted pill with a vector icon, a gradient
/// accent bar on the selected entry, and hover / press feedback.
fn nav_item(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 34.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let r = CornerRadius::same(10);
        let accent_c = grad_color(0.15);
        if selected {
            p.rect_filled(rect, r, alpha(accent_c, 0.14));
            p.rect_stroke(rect, r, Stroke::new(1.0, alpha(accent_c, 0.45)), egui::StrokeKind::Inside);
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 3.0, rect.top() + 8.0),
                    egui::pos2(rect.left() + 5.0, rect.bottom() - 8.0),
                ),
                CornerRadius::same(2),
                accent_c,
            );
        } else if hovered {
            p.rect_filled(rect, r, alpha(text2(), 0.07));
            p.rect_stroke(rect, r, Stroke::new(1.0, alpha(border(), 0.6)), egui::StrokeKind::Inside);
        }
        if pressed {
            p.rect_filled(rect, r, alpha(active(), 0.5));
        }
        let ic = if selected { neon_cyan() } else if hovered { text() } else { text2() };
        let icon_r = egui::Rect::from_center_size(egui::pos2(rect.left() + 19.0, rect.center().y), Vec2::splat(15.0));
        draw_icon(&p, icon_r, icon, ic, 1.6);
        p.text(
            egui::pos2(rect.left() + 33.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            if selected { text() } else { text2() },
        );
        if resp.has_focus() {
            p.rect_stroke(rect.expand2(Vec2::new(2.0, 2.0)), r, Stroke::new(1.5, alpha(neon_cyan(), 0.9)), egui::StrokeKind::Inside);
        }
    }
    resp
}

/// View header: an icon chip + title + subtitle with the gradient
/// underline — the same signature on every view for a coherent feel.
fn view_heading(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str) {
    let row = ui
        .horizontal(|ui| {
            let (chip_r, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
            let p = ui.painter();
            let c = grad_color(0.15);
            p.rect_filled(chip_r, CornerRadius::same(10), alpha(c, 0.14));
            p.rect_stroke(chip_r, CornerRadius::same(10), Stroke::new(1.0, alpha(c, 0.5)), egui::StrokeKind::Inside);
            draw_icon(&p, egui::Rect::from_center_size(chip_r.center(), Vec2::splat(17.0)), icon, neon_cyan(), 1.8);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(title).size(21.0).strong().color(text()));
                ui.label(RichText::new(subtitle).size(12.0).color(text3()));
            });
        })
        .response
        .rect;
    // gradient underline under the whole row
    let y = row.bottom() + 4.0;
    let x1 = (row.left() + ui.available_width()).min(row.left() + 320.0);
    let p = ui.painter();
    let n = 24usize;
    for i in 0..n {
        let a = row.left() + (x1 - row.left()) * (i as f32 / n as f32);
        let b = row.left() + (x1 - row.left()) * ((i + 1) as f32 / n as f32);
        p.line_segment(
            [egui::pos2(a, y), egui::pos2(b, y)],
            Stroke::new(2.0, grad_color(i as f32 / (n - 1) as f32)),
        );
    }
    ui.add_space(12.0);
}


// ------------------------------------------------------------- app state
#[derive(Clone, Copy, PartialEq)]
enum View {
    Home,
    Create,
    Extract,
    Inspect,
    Repair,
    Settings,
}

struct Job {
    state: Arc<ProgressState>,
    rx: Receiver<Result<String>>,
}

/// Entrance + hover animation state for one logo instance (hero vs sidebar).
#[derive(Clone, Copy)]
struct LogoAnim {
    born: Instant,
    hover: f32,
}

impl Default for LogoAnim {
    fn default() -> Self {
        Self { born: Instant::now(), hover: 0.0 }
    }
}

/// In-pane text preview for a stored file in the Inspect view.
enum PreviewState {
    None,
    Loading,
    Ready { text: String, truncated: bool },
    Error(String),
}

struct GuiApp {
    view: View,
    // create
    create_inputs: Vec<PathBuf>,
    create_output: String,
    create_output_manual: bool, // true once the user edits the output field
    create_codec: String,
    create_level: i32,
    create_block: String,
    create_password: String,
    create_recovery: u16,
    create_threads: usize,
    create_force: bool,
    // extract
    extract_archive: String,
    extract_output: String,
    extract_password: String,
    // inspect
    inspect_archive: String,
    inspect_data: Option<(ArchiveHeader, Index)>,
    inspect_error: Option<String>,
    /// Show the one-click "Extract here" banner (freshly dropped/opened archive).
    extract_banner: bool,
    /// In-pane text preview of a stored file (see `start_preview`).
    preview: PreviewState,
    preview_entry: Option<String>,
    preview_rx: Option<mpsc::Receiver<std::result::Result<(String, bool), String>>>,
    // repair
    repair_archive: String,
    repair_volume: String,
    repair_output: String,
    repair_force: bool,
    // appearance
    theme_override: ThemeOverride,
    // logo animation + view transitions
    hero_logo: LogoAnim,
    side_logo: LogoAnim,
    view_at: Instant,
    // home
    last_drop: Option<(Instant, String, View)>,
    recent: Vec<String>,
    // create defaults (Settings view)
    settings_codec: String,
    settings_level: i32,
    settings_block: String,
    settings_block_error: bool,
    settings_threads: usize,
    settings_recovery: u16,
    // job plumbing
    job: Option<Job>,
    last_result: Option<std::result::Result<String, String>>,
    /// Last title sent to the OS window (dedupe for `ViewportCommand::Title`).
    window_title: String,
}

impl Default for GuiApp {
    fn default() -> Self {
        Self {
            view: View::Home,
            create_inputs: Vec::new(),
            create_output: String::new(),
            create_output_manual: false,
            // Seed the Create view from the persisted defaults.
            create_codec: settings_create_codec(),
            create_level: settings_create_level(),
            create_block: settings_create_block(),
            create_password: String::new(),
            create_recovery: settings_create_recovery(),
            create_threads: settings_create_threads(),
            create_force: false,
            extract_archive: String::new(),
            extract_output: String::new(),
            extract_password: String::new(),
            inspect_archive: String::new(),
            inspect_data: None,
            inspect_error: None,
            extract_banner: false,
            preview: PreviewState::None,
            preview_entry: None,
            preview_rx: None,
            repair_archive: String::new(),
            repair_volume: String::new(),
            repair_output: String::new(),
            repair_force: false,
            theme_override: theme_override(),
            hero_logo: LogoAnim::default(),
            side_logo: LogoAnim::default(),
            view_at: Instant::now(),
            last_drop: None,
            recent: recent_archives(),
            settings_codec: settings_create_codec(),
            settings_level: settings_create_level(),
            settings_block: settings_create_block(),
            settings_block_error: false,
            settings_threads: settings_create_threads(),
            settings_recovery: settings_create_recovery(),
            job: None,
            last_result: None,
            window_title: "nextar".to_string(),
        }
    }
}

fn default_output(inputs: &[PathBuf]) -> PathBuf {
    if let Some(first) = inputs.first() {
        // 7-Zip style: the archive is named after the first selected item
        // and sits next to it, even when multiple items are selected.
        let mut p = first.clone();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        p.set_file_name(format!("{name}.next"));
        p
    } else {
        PathBuf::from("archive.next")
    }
}

impl GuiApp {
    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn spawn_job(&mut self, state: Arc<ProgressState>) -> mpsc::Sender<Result<String>> {
        let (tx, rx) = mpsc::channel();
        self.job = Some(Job { state, rx });
        self.last_result = None;
        tx
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(res) => {
                    self.last_result = Some(match res {
                        Ok(msg) => Ok(msg),
                        Err(e) => Err(format!("{e:#}")),
                    });
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                }
            }
        }
    }

    /// Keep the output field in sync with the selected inputs: when the user
    /// hasn't typed their own path, show the computed default (named after the
    /// first selected item, 7-Zip style) so it's visible and editable.
    fn refresh_create_output(&mut self) {
        if !self.create_output_manual {
            self.create_output = default_output(&self.create_inputs).display().to_string();
        }
    }

    /// Switch views and stamp the transition time (Home cards replay their
    /// entrance tween whenever the view changes).
    fn set_view(&mut self, v: View) {
        if self.view != v {
            self.view = v;
            self.view_at = Instant::now();
        }
    }

    // ------------------------------------------------ logo animation
    /// Home hero logo: fade + zoom entrance (95% → 100%), a vertical
    /// bottom→top reveal of the mark, one cyan light sweep, a soft static
    /// glow, and a smooth hover micro-lift. All motion is skipped when the
    /// user has reduced motion enabled (Windows "Animation effects").
    fn draw_logo_hero(&mut self, ui: &mut egui::Ui, size: f32) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        let p = ui.painter();
        let dt = ui.input(|i| i.stable_dt).max(0.0);
        let target = if resp.hovered() { 1.0 } else { 0.0 };
        self.hero_logo.hover += (target - self.hero_logo.hover) * (1.0 - (-dt / 0.15).exp());
        let hover = self.hero_logo.hover;
        if reduced_motion() {
            draw_logo_at(p, rect, 1.0);
            return resp;
        }
        let t = self.hero_logo.born.elapsed().as_secs_f32();
        // entrance: fade + zoom 96% → 100%
        let enter = smoothstep(t / 0.35, 0.0, 1.0);
        let scale = 0.96 + 0.04 * enter;
        let lift = hover * 3.0;
        let grow = 1.0 + hover * 0.03;
        let r = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.center().y - lift),
            Vec2::splat(size * scale * grow),
        );
        // soft glow behind the tile (a touch stronger while hovered)
        p.circle_filled(rect.center(), rect.width() * 0.58, alpha(neon_cyan(), 0.04 + 0.035 * hover));
        p.circle_filled(rect.center(), rect.width() * 0.5, alpha(accent2(), 0.05 + 0.03 * hover));
        let pal = logo_palette();
        // 1) scattered data fragments converge into the core (0.0–0.55 s)
        draw_converging_particles(p, r, t, pal);
        // 2) the planes fold in, outer → inner, feeding the core (0.25–0.8 s)
        let build = smoothstep((t - 0.25) / 0.45, 0.0, 1.0);
        draw_logo_tile(p, r, enter);
        converge_mark(p, r, pal, enter, build);
        // 3) energy pulse: a ring expands from the core (0.75–1.0 s)
        let pulse_t = ((t - 0.75) / 0.25).clamp(0.0, 1.0);
        if pulse_t > 0.0 && pulse_t < 1.0 {
            let e = smoothstep(pulse_t, 0.0, 1.0);
            let core = egui::pos2(
                r.left() + (0.06 + 0.88 * CORE.0) * r.width(),
                r.top() + (0.06 + 0.88 * CORE.1) * r.height(),
            );
            p.circle_stroke(core, lerpf(0.06, 0.46, e) * r.width(), Stroke::new(1.4, alpha(neon_cyan(), (1.0 - e) * 0.6)));
        }
        // 4) cyan → violet light sweep across the mark (0.9–1.35 s): a wide,
        //    soft-edged gradient band with a faint cyan echo trailing behind
        //    it, so it reads as light rather than a hard stripe
        let sweep_t = ((t - 0.9) / 0.45).clamp(0.0, 1.0);
        if sweep_t > 0.0 && sweep_t < 1.0 {
            let box_r = egui::Rect::from_min_max(
                egui::pos2(r.left() + 0.06 * r.width(), r.top() + 0.06 * r.height()),
                egui::pos2(r.right() - 0.06 * r.width(), r.bottom() - 0.06 * r.height()),
            );
            let fade = (sweep_t * (1.0 - sweep_t)) * 4.0;
            let cx = lerpf(box_r.left() - box_r.width() * 0.4, box_r.right() + box_r.width() * 0.4, sweep_t);
            let slope = box_r.height() * 0.3;
            // faint cyan trail behind the main band
            draw_sweep_band(p, box_r, cx - box_r.width() * 0.13, slope, fade * 0.45, true);
            // main gradient band
            draw_sweep_band(p, box_r, cx, slope, fade, false);
        }
        // 5) post-boot idle pulse on the core node's glow (after the
        //    entrance, a slow breathing halo so the mark feels alive while
        //    the app sits on the Home view)
        let idle = smoothstep((t - 1.45) / 0.5, 0.0, 1.0);
        if idle > 0.0 {
            let core = egui::pos2(
                r.left() + (0.06 + 0.88 * CORE.0) * r.width(),
                r.top() + (0.06 + 0.88 * CORE.1) * r.height(),
            );
            let cr = CORE.2 * 0.88 * r.width();
            let pulse = (t * 1.3).sin() * 0.5 + 0.5;
            p.circle_filled(core, cr * (1.7 + 0.25 * pulse), alpha(pal.glow, (0.10 + 0.12 * pulse) * idle));
            p.circle_filled(core, cr * (1.35 + 0.20 * pulse), alpha(pal.glow, (0.08 + 0.08 * pulse) * idle));
        }
        resp
    }

    /// Sidebar logo: a subtle hover micro-lift only (the splash and the
    /// Home hero own the big entrance moment).
    fn draw_logo_side(&mut self, ui: &mut egui::Ui, size: f32) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        let dt = ui.input(|i| i.stable_dt).max(0.0);
        let target = if resp.hovered() { 1.0 } else { 0.0 };
        self.side_logo.hover += (target - self.side_logo.hover) * (1.0 - (-dt / 0.15).exp());
        let lift = self.side_logo.hover * 2.0;
        let r = if reduced_motion() { rect } else { rect.translate(Vec2::new(0.0, -lift)) };
        draw_logo_at(ui.painter(), r, 1.0);
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx
            .input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if dropped.is_empty() {
            return;
        }
        match self.view {
            View::Home | View::Create => {
                // Context-aware: dropping a .next archive routes to Inspect
                // (and primes Extract); anything else routes to Create.
                if let Some(arch) = dropped
                    .iter()
                    .find(|p| p.extension().map(|e| e == "next").unwrap_or(false))
                {
                    self.inspect_archive = arch.display().to_string();
                    self.extract_archive = arch.display().to_string();
                    self.inspect_data = None;
                    self.inspect_error = None;
                    self.load_inspect();
                    self.extract_banner = true;
                    self.last_drop = Some((
                        Instant::now(),
                        "archive loaded — opened in Inspect".to_string(),
                        View::Inspect,
                    ));
                    self.set_view(View::Inspect);
                    return;
                }
                let n = dropped.len();
                for p in dropped {
                    if !self.create_inputs.contains(&p) {
                        self.create_inputs.push(p);
                    }
                }
                self.refresh_create_output();
                self.last_drop = Some((
                    Instant::now(),
                    format!("{n} item{} added — ready to create", if n == 1 { "" } else { "s" }),
                    View::Create,
                ));
                self.set_view(View::Create);
            }
            View::Extract => {
                if let Some(p) = dropped.first() {
                    if p.is_file() {
                        self.extract_archive = p.display().to_string();
                        self.last_drop = Some((
                            Instant::now(),
                            "archive set — ready to extract".to_string(),
                            View::Extract,
                        ));
                    }
                }
            }
            View::Inspect => {
                if let Some(p) = dropped.first() {
                    if p.is_file() {
                        self.inspect_archive = p.display().to_string();
                        self.inspect_data = None;
                        self.inspect_error = None;
                        self.load_inspect();
                        self.last_drop = Some((
                            Instant::now(),
                            "archive loaded — opened in Inspect".to_string(),
                            View::Inspect,
                        ));
                    }
                }
            }
            View::Repair => {
                for p in dropped {
                    let s = p.display().to_string();
                    if p.extension().map(|e| e == "nvol").unwrap_or(false) {
                        self.repair_volume = s;
                    } else {
                        self.repair_archive = s;
                    }
                }
                self.last_drop = Some((
                    Instant::now(),
                    "archive / volume set — ready to repair".to_string(),
                    View::Repair,
                ));
            }
            View::Settings => {}
        }
        ctx.request_repaint();
    }

    fn load_inspect(&mut self) {
        let path = self.inspect_archive.trim().to_string();
        self.inspect_data = None;
        self.inspect_error = None;
        self.extract_banner = false;
        self.preview = PreviewState::None;
        self.preview_entry = None;
        self.preview_rx = None;
        if path.is_empty() {
            return;
        }
        match std::fs::File::open(&path) {
            Ok(file) => match archive::read_head_index(&file, Path::new(&path)) {
                Ok((header, index, _)) => {
                    self.inspect_data = Some((header, index));
                    push_recent_archive(&path);
                }
                Err(e) => self.inspect_error = Some(format!("{e:#}")),
            },
            Err(e) => self.inspect_error = Some(format!("{e}")),
        }
    }

    // ------------------------------------------------ job runners
    fn start_create(&mut self) {
        if self.create_inputs.is_empty() {
            self.last_result = Some(Err("add at least one file or folder".to_string()));
            return;
        }
        let inputs = self.create_inputs.clone();
        let output = if self.create_output.trim().is_empty() {
            default_output(&inputs)
        } else {
            PathBuf::from(self.create_output.trim())
        };
        if output.exists() && !self.create_force {
            self.last_result = Some(Err(format!(
                "{} already exists — tick “Overwrite” to replace it",
                output.display()
            )));
            return;
        }
        let codec = match self.create_codec.as_str() {
            "zstd" => nextar::format::CODE_ZSTD,
            "lzma2" => nextar::format::CODE_LZMA2,
            "store" => nextar::format::CODE_STORE,
            other => {
                self.last_result = Some(Err(format!("unknown codec '{other}'")));
                return;
            }
        };
        let level = self.create_level.clamp(0, 22);
        let block_size = match parse_block(&self.create_block) {
            Ok(b) => b,
            Err(e) => {
                self.last_result = Some(Err(e));
                return;
            }
        };
        let password = if self.create_password.is_empty() {
            None
        } else {
            Some(self.create_password.clone())
        };
        let recovery = self.create_recovery;
        let threads = self.create_threads.max(1);
        let quiet = true;
        let state = Arc::new(ProgressState::new(0, "archiving"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let opts = CreateOptions {
                codec,
                level,
                block_size,
                password,
                threads,
                segment_size: 128,
                parity: recovery as usize,
                quiet,
                progress: Some(state.clone()),
            };
            let started = Instant::now();
            let res = archive::create(&inputs, &output, opts).map(|s| {
                push_recent_archive(&output.display().to_string());
                let mut msg = format!(
                    "archive created · {} → {} ({} in, {} out, ratio {:.2}×, {:.2}s)",
                    inputs
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" + "),
                    output.display(),
                    human(s.total_bytes_read),
                    human(s.archive_size),
                    s.total_bytes_read as f64 / s.archive_size.max(1) as f64,
                    started.elapsed().as_secs_f64()
                );
                if s.volume_size > 0 {
                    msg.push_str(&format!(
                        "\nrecovery volume: {} ({} blocks, {} parity/segment)",
                        human(s.volume_size),
                        s.block_count,
                        recovery
                    ));
                }
                msg
            });
            let _ = tx.send(res);
        });
    }

    fn start_extract(&mut self) {
        let archive_path = self.extract_archive.trim().to_string();
        if archive_path.is_empty() {
            self.last_result = Some(Err("pick an archive to extract".to_string()));
            return;
        }
        let out = if self.extract_output.trim().is_empty() {
            ".".to_string()
        } else {
            self.extract_output.trim().to_string()
        };
        let password = if self.extract_password.is_empty() {
            None
        } else {
            Some(self.extract_password.clone())
        };
        let state = Arc::new(ProgressState::new(0, "extracting"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let res = archive::extract(
                Path::new(&archive_path),
                Path::new(&out),
                password.as_deref(),
                num_cpus::get(),
                true,
                false,
                Some(state.clone()),
            )
            .map(|s| {
                push_recent_archive(&archive_path);
                format!(
                    "extracted {} files · {} dirs · {} symlinks ({} bytes) → {} in {:.2}s",
                    s.files,
                    s.dirs,
                    s.symlinks,
                    human(s.bytes),
                    out,
                    started.elapsed().as_secs_f64()
                )
            });
            let _ = tx.send(res);
        });
    }

    /// One-click banner action: extract the inspected archive into a folder
    /// named after it, right next to it (7-Zip "Extract here" semantics,
    /// same as the Explorer right-click command).
    fn start_extract_here(&mut self) {
        let archive_path = self.inspect_archive.trim().to_string();
        if archive_path.is_empty() {
            self.last_result = Some(Err("pick an archive to extract".to_string()));
            return;
        }
        let arch = PathBuf::from(&archive_path);
        let parent = arch.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = arch
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extracted".to_string());
        let out = parent.join(stem);
        let password = if self.extract_password.is_empty() {
            None
        } else {
            Some(self.extract_password.clone())
        };
        let out_disp = out.display().to_string();
        let state = Arc::new(ProgressState::new(0, "extracting"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let res = archive::extract(
                Path::new(&archive_path),
                &out,
                password.as_deref(),
                num_cpus::get(),
                true,
                true, // strip_root: contents land directly in the <stem>\ folder
                Some(state.clone()),
            )
            .map(|s| {
                push_recent_archive(&archive_path);
                format!(
                    "extracted {} files · {} dirs · {} symlinks ({} bytes) → {} in {:.2}s",
                    s.files,
                    s.dirs,
                    s.symlinks,
                    human(s.bytes),
                    out_disp,
                    started.elapsed().as_secs_f64()
                )
            });
            let _ = tx.send(res);
        });
    }

    fn start_verify(&mut self) {
        let archive_path = self.inspect_archive.trim().to_string();
        if archive_path.is_empty() {
            self.last_result = Some(Err("pick an archive to verify".to_string()));
            return;
        }
        let state = Arc::new(ProgressState::new(0, "verifying"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let res = archive::verify(Path::new(&archive_path), None, true, Some(state.clone())).and_then(|s| {
                if s.bad == 0 {
                    Ok(format!(
                        "verified {} of {} blocks — all ok · {:.2}s",
                        s.good,
                        s.total,
                        started.elapsed().as_secs_f64()
                    ))
                } else {
                    Err(anyhow::anyhow!(
                        "{} of {} blocks corrupt — use the Repair view",
                        s.bad,
                        s.total
                    ))
                }
            });
            let _ = tx.send(res);
        });
    }

    /// Attach the loaded archive to a new email via the same MAPI path the
    /// shell "Compress to .next and email" action uses (Explorer fallback
    /// when no mail client is configured).
    fn start_email(&mut self) {
        let archive_path = self.inspect_archive.trim().to_string();
        if archive_path.is_empty() {
            self.last_result = Some(Err("pick an archive to email".to_string()));
            return;
        }
        let state = Arc::new(ProgressState::new(0, "opening mail client"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let msg = mail_attach(Path::new(&archive_path));
            let _ = tx.send(Ok(msg));
        });
    }

    /// Load one stored file's contents (its blocks only, straight from the
    /// footer index — no full-archive pass) and show it in the preview pane.
    fn start_preview(&mut self, entry: &str) {
        self.preview_entry = Some(entry.to_string());
        let archive_path = self.inspect_archive.trim().to_string();
        if archive_path.is_empty() {
            self.preview = PreviewState::Error("pick an archive to preview".into());
            return;
        }
        let password = if self.extract_password.is_empty() {
            None
        } else {
            Some(self.extract_password.clone())
        };
        let (tx, rx) = mpsc::channel();
        self.preview = PreviewState::Loading;
        self.preview_rx = Some(rx);
        let entry = entry.to_string();
        std::thread::spawn(move || {
            let res = archive::read_file_bytes(Path::new(&archive_path), &entry, password.as_deref())
                .map_err(|e| format!("{e:#}"))
                .and_then(|bytes| {
                    if bytes.contains(&0u8) {
                        return Err("binary file — preview not available".to_string());
                    }
                    const CAP: usize = 128 * 1024;
                    let truncated = bytes.len() > CAP;
                    let text =
                        String::from_utf8_lossy(&bytes[..bytes.len().min(CAP)]).into_owned();
                    Ok((text, truncated))
                });
            let _ = tx.send(res);
        });
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.preview_rx else { return };
        match rx.try_recv() {
            Ok(Ok((text, truncated))) => {
                self.preview = PreviewState::Ready { text, truncated };
                self.preview_rx = None;
            }
            Ok(Err(e)) => {
                self.preview = PreviewState::Error(e);
                self.preview_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint(),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.preview = PreviewState::Error("preview failed".into());
                self.preview_rx = None;
            }
        }
    }

    fn start_repair(&mut self) {
        let archive_path = self.repair_archive.trim().to_string();
        let volume = self.repair_volume.trim().to_string();
        if archive_path.is_empty() {
            self.last_result = Some(Err("pick the corrupted archive".to_string()));
            return;
        }
        if volume.is_empty() {
            self.last_result = Some(Err("pick the .nvol recovery volume".to_string()));
            return;
        }
        let output = if self.repair_output.trim().is_empty() {
            repaired_path_for(Path::new(&archive_path))
        } else {
            PathBuf::from(self.repair_output.trim())
        };
        if output.exists() && !self.repair_force {
            self.last_result = Some(Err(format!(
                "{} already exists — tick “Overwrite” to replace it",
                output.display()
            )));
            return;
        }
        let state = Arc::new(ProgressState::new(0, "repairing"));
        let tx = self.spawn_job(state.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let res = archive::repair(
                Path::new(&archive_path),
                Path::new(&volume),
                &output,
                true,
                Some(state.clone()),
            )
            .map(|s| {
                push_recent_archive(&output.display().to_string());
                format!(
                    "repaired {} of {} blocks → {} ({} bytes) in {:.2}s",
                    s.repaired,
                    s.total_blocks,
                    output.display(),
                    human(s.out_size),
                    started.elapsed().as_secs_f64()
                )
            });
            let _ = tx.send(res);
        });
    }

    // ------------------------------------------------ shared widgets
    fn progress_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            let s = &job.state;
            let pct = s.pct() / 100.0;
            let done = human(s.done_bytes());
            let total = human(s.total());
            let speed = if s.elapsed() > 0.0 {
                human((s.done_bytes() as f64 / s.elapsed()) as u64)
            } else {
                human(0)
            };
            led_bar(ui, pct, &format!("{} · {} / {} · {}/s", s.label(), done, total, speed), true);
            ctx.request_repaint();
        }
    }

    fn show_result(&mut self, ui: &mut egui::Ui) {
        let Some(res) = &self.last_result else { return };
        let (color, icon) = match res {
            Ok(_) => (ok(), "✓"),
            Err(_) => (err(), "✕"),
        };
        let msg = match res {
            Ok(m) => m.clone(),
            Err(m) => m.clone(),
        };
        let w = ui.available_width().min(620.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 44.0), egui::Sense::hover());
        let p = ui.painter();
        let corner = CornerRadius::same(10);
        p.rect_filled(rect, corner, alpha(surface(), 0.92));
        p.rect_stroke(rect, corner, Stroke::new(1.0, alpha(color, 0.55)), egui::StrokeKind::Inside);
        // left accent bar + icon chip
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.left() + 2.0, rect.top() + 8.0), egui::pos2(rect.left() + 4.0, rect.bottom() - 8.0)),
            CornerRadius::same(2),
            color,
        );
        let chip = egui::Rect::from_center_size(egui::pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(20.0));
        p.circle_filled(chip.center(), 10.0, alpha(color, 0.16));
        p.text(chip.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(12.0), color);
        p.text(
            egui::pos2(rect.left() + 38.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            msg,
            egui::FontId::proportional(12.5),
            text2(),
        );
        ui.add_space(6.0);
    }

    /// The bottom pane of the Inspect view: text preview of the selected
    /// stored file (streamed from its blocks only, no full-archive pass).
    fn preview_pane(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut dismiss = false;
        match &self.preview {
            PreviewState::None => return,
            PreviewState::Loading => {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(bg2())
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let blink = ui.input(|i| i.time) % 1.0 < 0.5;
                            let led = if blink { neon_cyan() } else { dim(neon_cyan(), 0.35) };
                            let (led_r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(led_r.center(), 3.5, led);
                            ui.label(
                                RichText::new(format!(
                                    "previewing {}…",
                                    self.preview_entry.as_deref().unwrap_or("")
                                ))
                                .size(12.5)
                                .color(text2()),
                            );
                        });
                    });
                ctx.request_repaint(); // keep the LED blinking + channel polled
            }
            PreviewState::Error(e) => {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(surface())
                    .stroke(Stroke::new(1.0, err()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("❌ {e}")).color(err()).size(12.5));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("✕").clicked() {
                                    dismiss = true;
                                }
                            });
                        });
                    });
            }
            PreviewState::Ready { text: preview_text, truncated } => {
                let name = self.preview_entry.clone().unwrap_or_default();
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(bg2())
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("📄").color(text3()));
                            ui.label(RichText::new(&name).size(12.5).strong().color(text()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("✕").clicked() {
                                    dismiss = true;
                                }
                            });
                        });
                        egui::ScrollArea::vertical()
                            .max_height(150.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(preview_text.as_str()).monospace().size(11.5).color(text2()),
                                );
                            });
                        if *truncated {
                            ui.label(
                                RichText::new("… preview truncated to 128 KiB — extract to read the full file")
                                    .size(11.0)
                                    .color(text3()),
                            );
                        }
                    });
            }
        }
        if dismiss {
            self.preview = PreviewState::None;
            self.preview_entry = None;
        }
    }

    // ------------------------------------------------ views
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            self.draw_logo_side(ui, 36.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(grad_text("NEXTAR", 18.0, true));
                ui.label(RichText::new("v0.1.0").size(10.0).color(text3()));
            });
        });
        ui.add_space(20.0);
        ui.label(RichText::new("WORKSPACE").size(9.5).color(text3()).strong());
        ui.add_space(6.0);
        let nav = [
            (View::Home, Icon::Home, "Home"),
            (View::Create, Icon::Create, "Create"),
            (View::Extract, Icon::Extract, "Extract"),
            (View::Inspect, Icon::Inspect, "Inspect"),
            (View::Repair, Icon::Repair, "Repair"),
            (View::Settings, Icon::Settings, "Settings"),
        ];
        for (view, icon, label) in nav {
            let selected = self.view == view;
            if nav_item(ui, icon, label, selected).clicked() {
                self.set_view(view);
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(8.0);
            let busy = self.busy();
            let op = self.job.as_ref().map(|j| j.state.label().to_string());
            egui::Frame::new()
                .fill(if busy { alpha(surface(), 0.8) } else { surface() })
                .stroke(Stroke::new(
                    1.0,
                    if busy { alpha(neon_cyan(), 0.35) } else { border() },
                ))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (dot_r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                        let dot_c = if busy {
                            let pulse = ((ui.input(|i| i.time) * 3.0).sin() * 0.5 + 0.5) as f32;
                            alpha(neon_cyan(), 0.4 + 0.6 * pulse)
                        } else {
                            ok()
                        };
                        ui.painter().circle_filled(dot_r.center(), 3.5, dot_c);
                        if busy {
                            ui.painter().circle_filled(dot_r.center(), 6.0, alpha(neon_cyan(), 0.15));
                        }
                        let title = if busy {
                            format!("working… · {}", op.as_deref().unwrap_or("job"))
                        } else {
                            "ready · 100% local".to_string()
                        };
                        ui.label(
                            RichText::new(title)
                                .size(11.5)
                                .color(if busy { alpha(text2(), 0.75) } else { text2() }),
                        );
                    });
                    ui.add_space(2.0);
                    let dim_a = if busy { 0.55 } else { 1.0 };
                    ui.label(RichText::new("zstd · lzma2 · argon2id").size(9.5).color(alpha(text3(), dim_a)));
                    ui.label(RichText::new("xchacha20 · reed-solomon").size(9.5).color(alpha(text3(), dim_a)));
                });
        });
    }

    fn view_home(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let t = self.view_at.elapsed().as_secs_f32();
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            // horizontal lockup: logo left, wordmark + tagline right — the
            // same brand pairing the splash shows, now in-app too. Scales
            // down when the window is narrow so it stays centered/balanced.
            let narrow = ui.available_width() < 360.0;
            let logo_s = if narrow { 62.0 } else { 84.0 };
            let word_s = if narrow { 20.0 } else { 26.0 };
            let tag_s = if narrow { 10.5 } else { 12.5 };
            ui.horizontal(|ui| {
                self.draw_logo_hero(ui, logo_s);
                ui.add_space(if narrow { 12.0 } else { 16.0 });
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    draw_wordmark_stagger(ui, t, word_s);
                    ui.add_space(2.0);
                    ui.label(RichText::new("fast · secure · self-healing archives").size(tag_s).color(text2()));
                });
            });
            ui.add_space(14.0);

            // centerpiece: the drag & drop stage
            let dz = home_drop_zone(ui);
            if dz.clicked() {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    for f in files {
                        if !self.create_inputs.contains(&f) {
                            self.create_inputs.push(f);
                        }
                    }
                    self.refresh_create_output();
                    let n = self.create_inputs.len();
                    self.last_drop = Some((
                        Instant::now(),
                        format!("{n} item{} added — ready to create", if n == 1 { "" } else { "s" }),
                        View::Create,
                    ));
                    self.set_view(View::Create);
                }
            }

            // drop feedback banner (auto-expires after 5 s)
            let mut go: Option<View> = None;
            if let Some((at, msg, v)) = &self.last_drop {
                if at.elapsed() < Duration::from_secs(5) {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    ui.add_space(12.0);
                    let w = ui.available_width().min(560.0);
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 42.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, CornerRadius::same(10), alpha(surface(), 0.92));
                    p.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, alpha(ok(), 0.6)), egui::StrokeKind::Inside);
                    p.circle_filled(egui::pos2(rect.left() + 22.0, rect.center().y), 10.0, alpha(ok(), 0.18));
                    p.text(egui::pos2(rect.left() + 22.0, rect.center().y), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(12.0), ok());
                    p.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, msg, egui::FontId::proportional(12.5), text2());
                    let btn_r = egui::Rect::from_min_max(
                        egui::pos2(rect.right() - 96.0, rect.top() + 7.0),
                        egui::pos2(rect.right() - 12.0, rect.bottom() - 7.0),
                    );
                    let btn = ui.interact(btn_r, ui.id().with("drop-go"), egui::Sense::click());
                    let bp = ui.painter();
                    bp.rect_filled(
                        btn_r,
                        CornerRadius::same(255),
                        if btn.hovered() { alpha(grad_color(0.2), 0.95) } else { alpha(grad_color(0.2), 0.75) },
                    );
                    bp.text(
                        btn_r.center(),
                        egui::Align2::CENTER_CENTER,
                        "Open →",
                        egui::FontId::proportional(12.0),
                        Color32::from_rgb(0x04, 0x12, 0x1a),
                    );
                    if btn.clicked() {
                        go = Some(*v);
                    }
                }
            }

            ui.add_space(18.0);
            ui.label(RichText::new("QUICK ACTIONS").size(10.0).color(text3()).strong());
            ui.add_space(8.0);
            let card_w = 176.0;
            let gap = 14.0;
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - card_w * 3.0 - gap * 2.0) * 0.5);
                let actions = [
                    (View::Create, Icon::Create, "Create", "compress files & folders", neon_cyan()),
                    (View::Extract, Icon::Extract, "Extract", "restore an archive", accent2()),
                    (View::Repair, Icon::Repair, "Repair", "heal corruption with .nvol", neon_pink()),
                ];
                for (i, &(view, icon, title, desc, col)) in actions.iter().enumerate() {
                    let enter = if reduced_motion() {
                        1.0
                    } else {
                        smoothstep(t - 0.1 - i as f32 * 0.07, 0.0, 0.22)
                    };
                    if action_card(ui, icon, title, desc, col, enter).clicked() {
                        go = Some(view);
                    }
                }
            });

            // recent archives (this machine, persisted in settings.json)
            if !self.recent.is_empty() {
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("RECENT ARCHIVES").size(10.0).color(text3()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("clear")
                            .on_hover_text("Forget the recent list")
                            .clicked()
                        {
                            clear_recent();
                            self.recent = recent_archives();
                        }
                    });
                });
                ui.add_space(6.0);
                let items = self.recent.clone();
                egui::ScrollArea::horizontal()
                    .max_height(58.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for path in items {
                                let name = Path::new(&path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.clone());
                                let btn = ui.add(
                                    egui::Button::new(RichText::new(format!("📦 {name}")).size(12.0).color(text2()))
                                        .corner_radius(CornerRadius::same(8)),
                                );
                                if btn.clicked() {
                                    self.inspect_archive = path.clone();
                                    self.extract_archive = path.clone();
                                    self.inspect_data = None;
                                    self.inspect_error = None;
                                    self.load_inspect();
                                    self.extract_banner = true;
                                    go = Some(View::Inspect);
                                }
                            }
                        });
                    });
            }

            ui.add_space(14.0);
            ui.label(RichText::new("100% local · encrypted · self-healing").size(11.0).color(text3()));
            if let Some(v) = go {
                self.last_drop = None;
                self.set_view(v);
            }
        });
    }

    fn drop_zone(&mut self, ui: &mut egui::Ui, label: &str) {
        egui::Frame::new()
            .fill(surface())
            .stroke(Stroke::new(1.5, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).color(text3()).size(12.5));
                    if ui.button("+ files").clicked() {
                        if let Some(files) = rfd::FileDialog::new().pick_files() {
                            for f in files {
                                if !self.create_inputs.contains(&f) {
                                    self.create_inputs.push(f);
                                }
                            }
                            self.refresh_create_output();
                        }
                    }
                    if ui.button("+ folder").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            if !self.create_inputs.contains(&dir) {
                                self.create_inputs.push(dir);
                            }
                            self.refresh_create_output();
                        }
                    }
                });
            });
    }

    fn view_create(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        view_heading(ui, Icon::Create, "Create archive", "Compress files and folders with zstd or lzma2 — optionally encrypted and self-healing.");

        self.drop_zone(ui, "Drop files & folders here, or add them:");
        if !self.create_inputs.is_empty() {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut remove: Option<usize> = None;
                for (i, p) in self.create_inputs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("📄").color(text3()));
                        ui.label(RichText::new(p.display().to_string()).color(text2()).size(12.5));
                        if ui.small_button("🗑").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.create_inputs.remove(i);
                    self.refresh_create_output();
                }
            });
        }
        ui.add_space(10.0);

        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("create-options")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Output");
                        let out_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.create_output)
                                .hint_text("e.g. backup.next (default: <input>.next)")
                                .desired_width(320.0),
                        );
                        if out_resp.changed() {
                            self.create_output_manual = !self.create_output.trim().is_empty();
                        }
                        ui.end_row();

                        ui.label("Codec");
                        egui::ComboBox::from_id_salt("codec")
                            .selected_text(
                                match self.create_codec.as_str() {
                                    "zstd" => "zstd — fast (default)",
                                    "lzma2" => "lzma2 — maximum compression",
                                    "store" => "store — no compression",
                                    other => other,
                                }
                                .to_string(),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.create_codec, "zstd".to_string(), "zstd — fast (default)");
                                ui.selectable_value(&mut self.create_codec, "lzma2".to_string(), "lzma2 — maximum compression");
                                ui.selectable_value(&mut self.create_codec, "store".to_string(), "store — no compression");
                            });
                        ui.end_row();

                        ui.label("Level");
                        ui.horizontal(|ui| {
                            let max = if self.create_codec == "lzma2" { 9 } else { 22 };
                            ui.add(egui::Slider::new(&mut self.create_level, 0..=max).text(""));
                            ui.label(RichText::new(format!("{}/{}", self.create_level, max)).color(text3()).size(12.0));
                        });
                        ui.end_row();

                        ui.label("Block size");
                        ui.add(egui::TextEdit::singleline(&mut self.create_block).hint_text("1M").desired_width(80.0));
                        ui.end_row();

                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut self.create_password).password(true).hint_text("optional — AES-256-GCM-class auth (XChaCha20-Poly1305)").desired_width(320.0));
                        ui.end_row();

                        ui.label("Recovery");
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut self.create_recovery, 0..=16).text("parity blocks"));
                            ui.label(RichText::new("writes a .nvol volume to heal corruption").color(text3()).size(12.0));
                        });
                        ui.end_row();

                        ui.label("Threads");
                        ui.add(egui::Slider::new(&mut self.create_threads, 1..=64).text(format!("/ {} cores", num_cpus::get())));
                        ui.end_row();

                        ui.label("Overwrite");
                        ui.checkbox(&mut self.create_force, "replace the output if it exists");
                        ui.end_row();
                    });
            });
        ui.add_space(10.0);

        self.progress_bar(ui, ctx);
        self.show_result(ui);

        ui.horizontal(|ui| {
            if !self.busy() && action_button(ui, "Create archive", Some(Icon::Create), 14.0, self.busy()).clicked() {
                self.start_create();
            }
            if ui.button("Clear").on_hover_text("Reset the file list").clicked() {
                self.create_inputs.clear();
                self.last_result = None;
            }
        });
    }

    fn view_extract(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        view_heading(ui, Icon::Extract, "Extract archive", "Restore files, folders, permissions, symlinks and timestamps from a .next archive.");

        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Archive").size(12.0).color(text3()));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.extract_archive).hint_text("path to .next archive — drop it here").desired_width(340.0));
                    if ui.button("browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("nextar archive", &["next"]).pick_file() {
                            self.extract_archive = f.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("Output folder").size(12.0).color(text3()));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.extract_output).hint_text("current folder").desired_width(340.0));
                    if ui.button("browse…").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.extract_output = d.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("Password").size(12.0).color(text3()));
                ui.add(egui::TextEdit::singleline(&mut self.extract_password).password(true).hint_text("only if the archive is encrypted").desired_width(340.0));
            });
        ui.add_space(10.0);

        self.progress_bar(ui, ctx);
        self.show_result(ui);

        if !self.busy() && action_button(ui, "Extract", Some(Icon::Extract), 14.0, self.busy()).clicked() {
            self.start_extract();
        }
    }

    fn view_inspect(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        view_heading(ui, Icon::Inspect, "Inspect archive", "Preview an archive's header, contents and health before extracting.");

        // One-click "Extract here" banner for freshly dropped/opened archives.
        if self.extract_banner && !self.busy() {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0xff, 0x2b, 0xd6, 10))
                .stroke(Stroke::new(1.2, Color32::from_rgba_unmultiplied(0xff, 0x2b, 0xd6, 130)))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let blink = ui.input(|i| i.time) % 1.0 < 0.5;
                        let led = if blink { neon_pink() } else { dim(neon_pink(), 0.35) };
                        let (led_r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                        ui.painter().circle_filled(led_r.center(), 3.5, led);
                        ui.label(
                            RichText::new("Archive ready — extract it right next to itself?")
                                .size(13.0)
                                .strong()
                                .color(text()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if action_button(ui, "Extract here", None, 12.0, self.busy()).clicked() {
                                self.extract_banner = false;
                                self.start_extract_here();
                            }
                            if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                                self.extract_banner = false;
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.inspect_archive).hint_text("path to .next archive — drop it here").desired_width(340.0));
            if ui.button("browse…").clicked() {
                if let Some(f) = rfd::FileDialog::new().add_filter("nextar archive", &["next"]).pick_file() {
                    self.inspect_archive = f.display().to_string();
                    self.inspect_data = None;
                    self.inspect_error = None;
                    self.load_inspect();
                }
            }
            if ui.button("Load").clicked() {
                self.load_inspect();
            }
            if ui.button("Verify").clicked() && !self.busy() {
                self.start_verify();
            }
            let email_enabled = self.inspect_data.is_some() && !self.busy();
            if ui
                .add_enabled(email_enabled, egui::Button::new("📧 Email"))
                .on_hover_text("Attach the loaded archive to a new email (uses your default mail app)")
                .clicked()
            {
                self.start_email();
            }
        });
        ui.add_space(8.0);

        self.progress_bar(ui, ctx);
        self.show_result(ui);

        if let Some(emsg) = &self.inspect_error {
            ui.label(RichText::new(format!("❌ {emsg}")).color(err()));
            return;
        }
        let Some((header, index)) = &self.inspect_data else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let (ic_r, _) = ui.allocate_exact_size(Vec2::splat(46.0), egui::Sense::hover());
                let p = ui.painter();
                p.circle_filled(ic_r.center(), 23.0, alpha(accent2(), 0.08));
                draw_icon(
                    &p,
                    egui::Rect::from_center_size(ic_r.center(), Vec2::splat(23.0)),
                    Icon::Inspect,
                    alpha(accent2(), 0.6),
                    1.6,
                );
                ui.add_space(10.0);
                ui.label(RichText::new("No archive loaded").size(15.0).strong().color(text2()));
                ui.add_space(2.0);
                ui.label(
                    RichText::new("Drop a .next file anywhere, or use Browse / Load above.")
                        .size(12.0)
                        .color(text3()),
                );
            });
            return;
        };

        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("inspect-meta").num_columns(4).spacing([18.0, 6.0]).show(ui, |ui| {
                    let kv = |ui: &mut egui::Ui, k: &str, v: String| {
                        ui.label(RichText::new(k).color(text3()).size(12.0));
                        ui.label(RichText::new(v).color(text()).size(12.5));
                    };
                    kv(ui, "codec", format!("{} · level {}", index.codec, index.level));
                    kv(ui, "block size", human(index.block_size as u64));
                    kv(ui, "encrypted", if index.encrypted { "yes".into() } else { "no".into() });
                    kv(ui, "recovery", if header.recovery() { format!("{}/{}", header.segment_size, header.parity) } else { "off".into() });
                    ui.end_row();
                    let files = index.files.iter().filter(|f| f.kind == "file").count();
                    let dirs = index.files.iter().filter(|f| f.kind == "dir").count();
                    let symlinks = index.files.iter().filter(|f| f.kind == "symlink").count();
                    let data: u64 = index.files.iter().filter(|f| f.kind == "file").map(|f| f.size).sum();
                    kv(ui, "entries", format!("{files} files · {dirs} dirs · {symlinks} symlinks"));
                    kv(ui, "logical size", human(data));
                    kv(ui, "blocks", index.blocks.len().to_string());
                    kv(ui, "made by", index.created_by.clone());
                    ui.end_row();
                });
            });
        ui.add_space(8.0);

        // Reserve room for the preview pane when it's open so the list
        // shrinks instead of overlapping.
        let preview_open = !matches!(self.preview, PreviewState::None);
        let mut clicked: Option<String> = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 56.0 - if preview_open { 210.0 } else { 0.0 })
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for f in &index.files {
                    let depth = f.path.matches('/').count();
                    let indent = 14.0 * depth as f32;
                    let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                    let icon = match f.kind.as_str() {
                        "dir" => "📁",
                        "symlink" => "🔗",
                        _ => "📄",
                    };
                    let size = if f.kind == "file" { human(f.size) } else { String::new() };
                    let is_file = f.kind == "file";
                    let selected = self.preview_entry.as_deref() == Some(f.path.as_str());
                    // painter-drawn row: full-width click target, hover +
                    // selected highlight, right-aligned size / link
                    let row_w = ui.available_width();
                    let (row_r, resp) = ui.allocate_exact_size(Vec2::new(row_w, 26.0), egui::Sense::click());
                    let p = ui.painter();
                    let bg = if selected {
                        alpha(neon_cyan(), 0.08)
                    } else if resp.hovered() {
                        alpha(text2(), 0.06)
                    } else {
                        Color32::TRANSPARENT
                    };
                    if bg != Color32::TRANSPARENT {
                        p.rect_filled(row_r, CornerRadius::same(6), bg);
                    }
                    if selected {
                        p.rect_stroke(
                            row_r,
                            CornerRadius::same(6),
                            Stroke::new(1.0, alpha(neon_cyan(), 0.3)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let text_y = row_r.center().y;
                    let display = format!("{name}{}", if f.kind == "dir" { "/" } else { "" });
                    let name_c = if selected { accent() } else if resp.hovered() { text() } else { text2() };
                    p.text(
                        egui::pos2(row_r.left() + indent + 12.0, text_y),
                        egui::Align2::LEFT_CENTER,
                        icon,
                        egui::FontId::proportional(12.0),
                        if selected { neon_cyan() } else { text3() },
                    );
                    p.text(
                        egui::pos2(row_r.left() + indent + 32.0, text_y),
                        egui::Align2::LEFT_CENTER,
                        display,
                        egui::FontId::proportional(12.5),
                        name_c,
                    );
                    if let Some(t) = &f.link {
                        p.text(
                            egui::pos2(row_r.right() - 10.0, text_y),
                            egui::Align2::RIGHT_CENTER,
                            format!("→ {t}"),
                            egui::FontId::proportional(11.5),
                            text3(),
                        );
                    }
                    if !size.is_empty() {
                        let extra = if f.link.is_some() { 84.0 } else { 0.0 };
                        p.text(
                            egui::pos2(row_r.right() - 10.0 - extra, text_y),
                            egui::Align2::RIGHT_CENTER,
                            size,
                            egui::FontId::proportional(11.5),
                            text3(),
                        );
                    }
                    if is_file && resp.on_hover_text(&f.path).clicked() {
                        clicked = Some(f.path.clone());
                    }
                }
            });
        // Status strip under the file list: total entries, logical size, ratio.
        {
            let logical: u64 = index.files.iter().filter(|f| f.kind == "file").map(|f| f.size).sum();
            let (orig, stored): (u64, u64) = index
                .blocks
                .iter()
                .fold((0, 0), |(o, s), b| (o + u64::from(b.orig_len), s + u64::from(b.stored_len)));
            let ratio_txt = if orig > 0 && stored > 0 {
                format!("{:.2} : 1", orig as f64 / stored as f64)
            } else {
                "—".to_string()
            };
            egui::Frame::new()
                .fill(bg2())
                .stroke(Stroke::new(1.0, border()))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let stat = |ui: &mut egui::Ui, k: &str, v: &str, accent: bool| {
                            let c = if accent { neon_cyan() } else { text2() };
                            ui.label(RichText::new(k).size(11.0).color(text3()));
                            ui.label(RichText::new(v).size(12.0).strong().color(c));
                        };
                        stat(ui, "entries", &index.files.len().to_string(), false);
                        ui.separator();
                        stat(ui, "logical size", &human(logical), false);
                        ui.separator();
                        stat(ui, "ratio", &ratio_txt, true);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{} blocks", index.blocks.len()))
                                    .size(11.0)
                                    .color(text3()),
                            );
                        });
                    });
                });
        }
        ui.add_space(6.0);

        if let Some(p) = clicked {
            self.start_preview(&p);
        }

        self.preview_pane(ui, ctx);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Extract this archive…").clicked() {
                self.extract_archive = self.inspect_archive.clone();
                self.set_view(View::Extract);
            }
        });
    }

    fn view_settings(&mut self, ui: &mut egui::Ui) {
        view_heading(ui, Icon::Settings, "Settings", "Appearance, create defaults, and the Windows theme integration.");

        // Corrupted settings: offer a one-click reset instead of silently
        // running on defaults.
        if settings_corrupt() {
            egui::Frame::new()
                .fill(surface2())
                .stroke(Stroke::new(1.0, err()))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Your settings file couldn't be read — it may be corrupted.")
                                .size(13.0)
                                .strong()
                                .color(text()),
                        );
                        ui.label(
                            RichText::new("nextar is running on defaults. Reset to a clean settings file? Your current file will be backed up first.")
                                .size(12.0)
                                .color(text2()),
                        );
                        ui.add_space(8.0);
                        if ui.add(egui::Button::new("Reset settings")).clicked() {
                            reset_corrupt_settings();
                            self.theme_override = ThemeOverride::Follow;
                            self.settings_codec = "zstd".to_string();
                            self.settings_level = 3;
                            self.settings_block = "1M".to_string();
                            self.settings_threads = num_cpus::get();
                            self.settings_recovery = 0;
                            self.settings_block_error = false;
                        }
                    });
                });
            ui.add_space(12.0);
        }

        // Live preview: the tile shows the palette the choice will produce
        // (the same eased cross-fade used everywhere in the app).
        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    draw_logo(ui, 64.0);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        let b = theme_blend();
                        let label = if b >= 0.999 {
                            "deep navy glass — dark".to_string()
                        } else if b <= 0.001 {
                            "frosted glass — light".to_string()
                        } else {
                            "cross-fading…".to_string()
                        };
                        ui.label(RichText::new(format!("Preview: {label}")).size(13.0).strong().color(text()));
                        let sys = if windows_dark_mode() { "dark" } else { "light" };
                        ui.label(RichText::new(format!("Windows apps theme: {sys}")).size(12.0).color(text3()));
                    });
                });
            });
        ui.add_space(12.0);

        ui.label(RichText::new("Appearance").size(12.0).color(text3()));
        ui.add_space(6.0);
        let opts = [
            (ThemeOverride::Follow, "Follow Windows", "swap automatically with the OS theme"),
            (ThemeOverride::Dark, "Always dark", "pin the deep synthwave look"),
            (ThemeOverride::Light, "Always light", "pin the frosted light look"),
        ];
        let mut chosen: Option<ThemeOverride> = None;
        ui.horizontal(|ui| {
            let card_w = 212.0;
            for (opt, title, desc) in opts {
                let selected = self.theme_override == opt;
                let resp = egui::Frame::new()
                    .fill(if selected { surface2() } else { surface() })
                    .stroke(Stroke::new(
                        if selected { 1.6 } else { 1.0 },
                        if selected { neon_cyan() } else { border() },
                    ))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(card_w - 28.0);
                        ui.horizontal(|ui| {
                            let (led_r, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                led_r.center(),
                                4.0,
                                if selected { neon_cyan() } else { text3() },
                            );
                            ui.label(
                                RichText::new(title)
                                    .size(14.5)
                                    .strong()
                                    .color(if selected { neon_cyan() } else { text() }),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(desc).size(11.5).color(text3()));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_stroke(
                        resp.rect,
                        CornerRadius::same(12),
                        Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 255, 247, 120)),
                        egui::StrokeKind::Inside,
                    );
                }
                if resp.clicked() {
                    chosen = Some(opt);
                }
            }
        });
        if let Some(opt) = chosen {
            if opt != self.theme_override {
                self.theme_override = opt;
                set_theme_override(opt);
            }
        }
        ui.add_space(18.0);

        // ---- create defaults: seed the Create view on the next launch ----
        ui.label(RichText::new("Create defaults").size(12.0).color(text3()));
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("settings-create")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Default codec");
                        let before = self.settings_codec.clone();
                        egui::ComboBox::from_id_salt("settings-codec")
                            .selected_text(
                                match self.settings_codec.as_str() {
                                    "zstd" => "zstd — fast (default)",
                                    "lzma2" => "lzma2 — maximum compression",
                                    "store" => "store — no compression",
                                    other => other,
                                }
                                .to_string(),
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.settings_codec, "zstd".to_string(), "zstd — fast (default)");
                                ui.selectable_value(&mut self.settings_codec, "lzma2".to_string(), "lzma2 — maximum compression");
                                ui.selectable_value(&mut self.settings_codec, "store".to_string(), "store — no compression");
                            });
                        if self.settings_codec != before {
                            // lzma2 levels cap at 9; keep the stored default valid.
                            if self.settings_codec == "lzma2" && self.settings_level > 9 {
                                self.settings_level = 9;
                            }
                            self.save_create_defaults();
                        }
                        ui.end_row();

                        ui.label("Default level");
                        let max = if self.settings_codec == "lzma2" { 9 } else { 22 };
                        let resp = ui.add(
                            egui::Slider::new(&mut self.settings_level, 0..=max).text("level"),
                        );
                        if resp.changed() {
                            self.save_create_defaults();
                        }
                        ui.label(RichText::new(format!("{}/{}", self.settings_level, max)).color(text3()).size(12.0));
                        ui.end_row();

                        ui.label("Default block size");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.settings_block)
                                .hint_text("e.g. 1M, 4M, 512K")
                                .desired_width(120.0),
                        );
                        if resp.changed() {
                            match parse_block(&self.settings_block) {
                                Ok(_) => {
                                    self.settings_block_error = false;
                                    self.save_create_defaults();
                                }
                                Err(_) => self.settings_block_error = true,
                            }
                        }
                        if self.settings_block_error {
                            ui.label(RichText::new("invalid — 512 B to 64 MiB").color(err()).size(11.5));
                        } else {
                            ui.label(RichText::new("512 B to 64 MiB").color(text3()).size(11.5));
                        }
                        ui.end_row();

                        ui.label("Default threads");
                        let resp = ui.add(
                            egui::Slider::new(&mut self.settings_threads, 1..=64)
                                .text(format!("/ {} cores", num_cpus::get())),
                        );
                        if resp.changed() {
                            self.save_create_defaults();
                        }
                        ui.end_row();

                        ui.label("Recovery by default");
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::Slider::new(&mut self.settings_recovery, 0..=16).text("parity blocks"),
                            );
                            if resp.changed() {
                                self.save_create_defaults();
                            }
                            ui.label(
                                RichText::new(if self.settings_recovery == 0 {
                                    "off — no .nvol volume".to_string()
                                } else {
                                    "writes a .nvol volume on create".to_string()
                                })
                                .color(text3())
                                .size(12.0),
                            );
                        });
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Applied to the Create view on the next launch (the current archive is untouched).")
                        .size(11.0)
                        .color(text3()),
                );
            });
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("settings saved to {}", settings_path().display()))
                .size(11.0)
                .color(text3()),
        );
    }

    fn save_create_defaults(&self) {
        set_create_defaults(
            self.settings_codec.clone(),
            self.settings_level,
            self.settings_block.clone(),
            self.settings_threads,
            self.settings_recovery,
        );
    }

    fn view_repair(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        view_heading(ui, Icon::Repair, "Repair archive", "Heal a corrupted or partially downloaded archive using its .nvol recovery volume.");

        egui::Frame::new()
            .fill(bg2())
            .stroke(Stroke::new(1.0, border()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Corrupted archive").size(12.0).color(text3()));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.repair_archive).hint_text("drop the archive here").desired_width(340.0));
                    if ui.button("browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("nextar archive", &["next"]).pick_file() {
                            self.repair_archive = f.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("Recovery volume (.nvol)").size(12.0).color(text3()));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.repair_volume).hint_text("drop the volume here").desired_width(340.0));
                    if ui.button("browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("recovery volume", &["nvol"]).pick_file() {
                            self.repair_volume = f.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("Output").size(12.0).color(text3()));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.repair_output).hint_text("default: <name>.repaired.next").desired_width(340.0));
                    if ui.button("browse…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().set_file_name("repaired.next").save_file() {
                            self.repair_output = f.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.checkbox(&mut self.repair_force, "overwrite the output if it exists");
            });
        ui.add_space(10.0);

        self.progress_bar(ui, ctx);
        self.show_result(ui);

        if !self.busy() && action_button(ui, "Repair", Some(Icon::Repair), 14.0, self.busy()).clicked() {
            self.start_repair();
        }
    }
}

impl eframe::App for GuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Follow the Windows light/dark theme: refresh the UI palette + egui
        // style every frame (the palette is tweened), and while a cross-fade
        // is mid-flight keep repainting so the morph is smooth.
        refresh_palette();
        apply_titlebar(frame);
        apply_window_corners(frame);
        configure_theme(&ctx);
        ctx.request_repaint_after(THEME_POLL);
        if theme_transitioning() {
            ctx.request_repaint();
        }
        self.handle_drops(&ctx);
        self.poll_job(&ctx);
        self.poll_preview(&ctx);
        // Keep frames flowing while the Home hero logo entrance and the
        // view-entry card tweens play (otherwise the app only repaints on
        // interaction or the 3 s theme poll, freezing mid-animation).
        if !reduced_motion()
            && (self.hero_logo.born.elapsed() < Duration::from_millis(1500)
                || self.view_at.elapsed() < Duration::from_millis(450))
        {
            ctx.request_repaint();
        }
        // The Home hero's core node keeps a slow idle pulse after boot, so
        // keep frames flowing at a gentle cadence while Home is visible.
        if !reduced_motion() && self.view == View::Home {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        // Mirror the active job into the native title bar ("nextar —
        // archiving 43%") so progress stays visible even unfocused. Sent
        // only on change — eframe applies the OS title, and repaints already
        // flow while a job runs.
        let title = match &self.job {
            Some(j) => format!("nextar — {} {:.0}%", j.state.label(), j.state.pct()),
            None => "nextar".to_string(),
        };
        if title != self.window_title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        // Taskbar progress fill (green) while a job runs, cleared when idle.
        update_taskbar_progress(frame, self.job.as_ref().map(|j| (j.state.done_bytes(), j.state.total())));
        // Keyboard-first navigation: 1-6 switch views (sidebar order),
        // Ctrl+N/E/R jump straight to the archive actions. Skipped while a
        // text field is focused so typing is never swallowed.
        if ctx.memory(|m| m.focused()).is_none() {
            let cmd = ctx.input(|i| i.modifiers.command);
            let key = |k: egui::Key| ctx.input(|i| i.key_pressed(k));
            if !cmd {
                if key(egui::Key::Num1) {
                    self.set_view(View::Home);
                } else if key(egui::Key::Num2) {
                    self.set_view(View::Create);
                } else if key(egui::Key::Num3) {
                    self.set_view(View::Extract);
                } else if key(egui::Key::Num4) {
                    self.set_view(View::Inspect);
                } else if key(egui::Key::Num5) {
                    self.set_view(View::Repair);
                } else if key(egui::Key::Num6) {
                    self.set_view(View::Settings);
                }
            } else if key(egui::Key::N) {
                self.set_view(View::Create);
            } else if key(egui::Key::E) {
                self.set_view(View::Extract);
            } else if key(egui::Key::R) {
                self.set_view(View::Repair);
            }
        }

        egui::Panel::left("nav")
            .exact_size(208.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(bg2()).inner_margin(Margin::same(14)))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg()).inner_margin(Margin::same(20)))
            .show(ui, |ui| match self.view {
                View::Home => self.view_home(ui),
                View::Create => self.view_create(ui, &ctx),
                View::Extract => self.view_extract(ui, &ctx),
                View::Inspect => self.view_inspect(ui, &ctx),
                View::Repair => self.view_repair(ui, &ctx),
                View::Settings => self.view_settings(ui),
            });

    }
}

// ---------------------------------------------------------- titlebar theme
/// Make the native window title bar and decorations follow the app palette.
/// Sets the immersive dark-mode flag plus the caption/text/border colors
/// (Win11 22000+; the color attributes fail gracefully on Win10). A
/// per-window change detector means DWM is only called when the chrome
/// actually moves — including the eased cross-fade mid-transition.
#[cfg(windows)]
fn apply_titlebar(frame: &mut eframe::Frame) {
    use raw_window_handle::HasWindowHandle;
    let Ok(handle) = frame.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::Win32(w) = handle.as_raw() else { return };
    let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;

    let p = current_palette();
    let dark = theme_blend() > 0.5;
    let caption = colorref(p.surface);
    let text = colorref(p.text);
    let border = colorref(p.border);

    // Keyed by HWND so a new window in the same process (splash → main)
    // always gets the chrome applied, even if the palette didn't move.
    type Chrome = (isize, bool, u32, u32, u32);
    static LAST: Mutex<Option<Chrome>> = Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|l| l.into_inner());
    if *g == Some((w.hwnd.get(), dark, caption, text, border)) {
        return;
    }
    *g = Some((w.hwnd.get(), dark, caption, text, border));

    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR,
        DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };
    let size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `hwnd` is the live eframe window handle and every attribute
    // points at a stack local that outlives the call. Attributes unsupported
    // on older builds (Win10) return E_INVALIDARG, which we ignore.
    unsafe {
        let mode: u32 = u32::from(dark);
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &mode as *const u32 as *const _, size);
        DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR as u32, &caption as *const u32 as *const _, size);
        DwmSetWindowAttribute(hwnd, DWMWA_TEXT_COLOR as u32, &text as *const u32 as *const _, size);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &border as *const u32 as *const _, size);
    }
}

#[cfg(not(windows))]
fn apply_titlebar(_frame: &mut eframe::Frame) {}

/// Round the native window corners on Windows 11 (DWM corner preference).
/// Windows 10 doesn't support `DWMWA_WINDOW_CORNER_PREFERENCE` — the call
/// fails with E_INVALIDARG, which we ignore — so the window keeps the
/// OS-default square corners there. Applied once per window (keyed by HWND)
/// and independent of the light/dark palette, so the rounded glass chrome
/// matches the app in both themes without fighting the OS.
#[cfg(windows)]
fn apply_window_corners(frame: &mut eframe::Frame) {
    use raw_window_handle::HasWindowHandle;
    let Ok(handle) = frame.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::Win32(w) = handle.as_raw() else { return };
    let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;

    static DONE: Mutex<Option<isize>> = Mutex::new(None);
    let mut g = DONE.lock().unwrap_or_else(|l| l.into_inner());
    if *g == Some(w.hwnd.get()) {
        return;
    }
    *g = Some(w.hwnd.get());

    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    let pref = DWMWCP_ROUND as u32;
    let size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `hwnd` is the live eframe window handle and the attribute
    // points at a stack local that outlives the call. Unsupported on Win10.
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &pref as *const u32 as *const _, size);
    }
}

#[cfg(not(windows))]
fn apply_window_corners(_frame: &mut eframe::Frame) {}

/// Drive the Windows taskbar progress overlay from the live job state: a
/// green fill on the taskbar icon while a job runs, cleared when idle.
/// COM is initialized once at startup (see `main`); the `ITaskbarList3`
/// object is acquired lazily and cached for the app's lifetime. Any
/// failure degrades silently — the overlay is decorative and the in-app
/// progress bar always works.
#[cfg(windows)]
fn update_taskbar_progress(frame: &mut eframe::Frame, job: Option<(u64, u64)>) {
    use raw_window_handle::HasWindowHandle;
    use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows_sys::Win32::UI::Shell::{TaskbarList, TBPF_NORMAL, TBPF_NOPROGRESS};

    // windows-sys only ships class IDs, not interface IIDs, so
    // ITaskbarList3's GUID is spelled out (EA1AFB91-9E28-4B86-90E9-9E9F8A5EEFAF).
    const IID_ITASKBAR_LIST3: windows_sys::core::GUID =
        windows_sys::core::GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eefaf);

    let Ok(handle) = frame.window_handle() else { return };
    let raw_window_handle::RawWindowHandle::Win32(w) = handle.as_raw() else { return };
    let hwnd = w.hwnd.get();

    // Lazy COM object, cached as a raw pointer (usize keeps the static
    // Send); Some(0) means "acquisition failed — stop trying".
    static TASKBAR: Mutex<Option<usize>> = Mutex::new(None);
    let ptr = {
        let mut g = TASKBAR.lock().unwrap_or_else(|l| l.into_inner());
        if g.is_none() {
            let mut raw: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `TaskbarList` is the documented class ID, the IID
            // above is ITaskbarList3's, and the returned pointer is used
            // only through its vtable while the app lives.
            let hr = unsafe {
                CoCreateInstance(
                    &TaskbarList,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_ITASKBAR_LIST3,
                    &mut raw,
                )
            };
            if hr == 0 && !raw.is_null() {
                // HrInit (vtable slot 3) initializes the taskbar object.
                let hr_init: unsafe extern "system" fn(*mut core::ffi::c_void) -> i32 = unsafe {
                    let vtbl = *(raw as *const *const usize);
                    std::mem::transmute(*vtbl.add(3))
                };
                unsafe { let _ = hr_init(raw); }
                *g = Some(raw as usize);
            } else {
                *g = Some(0);
            }
        }
        match *g {
            Some(0) | None => return,
            Some(p) => p,
        }
    };

    // ITaskbarList3 vtable layout: IUnknown's 3 slots, then ITaskbarList's
    // 5 (HrInit..SetActiveAlt), then ITaskbarList2's 1 (MarkFullscreenWindow),
    // so SetProgressValue = slot 9 and SetProgressState = slot 10.
    let (set_progress_value, set_progress_state) = unsafe {
        let vtbl = *(ptr as *const *const usize);
        (
            std::mem::transmute::<usize, unsafe extern "system" fn(*mut core::ffi::c_void, isize, u64, u64) -> i32>(*vtbl.add(9)),
            std::mem::transmute::<usize, unsafe extern "system" fn(*mut core::ffi::c_void, isize, i32) -> i32>(*vtbl.add(10)),
        )
    };
    let taskbar = ptr as *mut core::ffi::c_void;

    // Only touch the OS when the visible state actually changes (keyed by
    // window + active flag + done/total, so idle frames send nothing).
    static LAST: Mutex<Option<(isize, bool, u64, u64)>> = Mutex::new(None);
    let next = match job {
        Some((done, total)) if total > 0 => (hwnd, true, done, total),
        _ => (hwnd, false, 0, 0),
    };
    {
        let mut ls = LAST.lock().unwrap_or_else(|l| l.into_inner());
        if *ls == Some(next) {
            return;
        }
        *ls = Some(next);
    }
    // SAFETY: `hwnd` is the live window handle and the COM object was
    // successfully created above (or we returned).
    unsafe {
        if next.1 {
            let _ = set_progress_state(taskbar, hwnd, TBPF_NORMAL);
            let _ = set_progress_value(taskbar, hwnd, next.2, next.3);
        } else {
            let _ = set_progress_state(taskbar, hwnd, TBPF_NOPROGRESS);
        }
    }
}

#[cfg(not(windows))]
fn update_taskbar_progress(_frame: &mut eframe::Frame, _job: Option<(u64, u64)>) {}

/// Pack an sRGB [`Color32`] as a Win32 COLORREF (0x00BBGGRR) for DWM.
fn colorref(c: Color32) -> u32 {
    u32::from(c.r()) | (u32::from(c.g()) << 8) | (u32::from(c.b()) << 16)
}

fn repaired_path_for(archive: &Path) -> PathBuf {
    let stem = archive.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = archive.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    archive.with_file_name(format!("{stem}.repaired{ext}"))
}

/// Embed Space Grotesk (the marketing site's face) as the proportional
/// font for regular/semibold/bold, keeping the defaults as fallback for
/// glyphs it lacks. Same weights map to the matching TTFs.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "sg-regular".into(),
        egui::FontData::from_static(include_bytes!("../../resources/fonts/SpaceGrotesk-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "sg-semibold".into(),
        egui::FontData::from_static(include_bytes!("../../resources/fonts/SpaceGrotesk-SemiBold.ttf")).into(),
    );
    fonts.font_data.insert(
        "sg-bold".into(),
        egui::FontData::from_static(include_bytes!("../../resources/fonts/SpaceGrotesk-Bold.ttf")).into(),
    );
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        list.insert(0, "sg-bold".into());
        list.insert(1, "sg-semibold".into());
        list.insert(2, "sg-regular".into());
    }
    ctx.set_fonts(fonts);
}

fn configure_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = bg2();
    style.visuals.window_fill = bg2();
    style.visuals.extreme_bg_color = bg();
    style.visuals.code_bg_color = surface();
    style.visuals.override_text_color = Some(text());
    style.visuals.selection.bg_fill = accent2();
    style.visuals.selection.stroke = Stroke::new(1.0, neon_cyan());
    style.visuals.hyperlink_color = neon_cyan();
    style.visuals.window_stroke = Stroke::new(1.0, border());
    style.visuals.window_corner_radius = CornerRadius::same(10);
    for w in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        w.bg_fill = surface();
        w.bg_stroke = Stroke::new(1.0, border());
        w.fg_stroke = Stroke::new(1.0, text2());
        w.corner_radius = CornerRadius::same(8);
    }
    style.visuals.widgets.hovered.bg_fill = surface2();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.5, alpha(neon_cyan(), 0.55));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text());
    style.visuals.widgets.active.bg_fill = active();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5, alpha(neon_cyan(), 0.85));
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, text());
    style.visuals.text_cursor = egui::style::TextCursorStyle {
        stroke: Stroke::new(1.0, text2()),
        preview: false,
        blink: true,
        on_duration: 0.5,
        off_duration: 0.5,
    };
    style.visuals.faint_bg_color = border();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(16.0, 8.0);
    style.spacing.slider_width = 190.0;
    ctx.set_style_of(egui::Theme::Dark, style);
}

// ---------------------------------------------------------------- shell mode
// `nextar-gui --run <op> <path>` — launched from the Explorer right-click
// menu. Opens a small always-on-top progress window (no console flash), runs
// the job through the shared engine, and closes on success.

enum ShellJob {
    /// One or more items (SendTo passes every selected file).
    Create(Vec<PathBuf>),
    /// Create the archive, then hand it to the default mail client.
    CreateEmail(Vec<PathBuf>),
    ExtractHere(PathBuf),
    /// "Extract .next… here" on a folder: the archive is picked by the user
    /// (the menu passes the *folder* as %1); filled in before the window opens.
    ExtractInto { folder: PathBuf, archive: PathBuf },
    Repair(PathBuf),
    Bad(String),
}

fn parse_shell_job(args: &[String]) -> ShellJob {
    match args.first().map(|s| s.as_str()) {
        Some("create") => {
            let inputs: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            if inputs.is_empty() {
                ShellJob::Bad("usage: --run create <path…>".into())
            } else {
                ShellJob::Create(inputs)
            }
        }
        Some("create-email") => {
            let inputs: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
            if inputs.is_empty() {
                ShellJob::Bad("usage: --run create-email <path…>".into())
            } else {
                ShellJob::CreateEmail(inputs)
            }
        }
        Some("extract") => {
            // args[1..] = ["extract", "--here", <archive>]
            if args.get(1).map(|a| a == "--here").unwrap_or(false) {
                match args.get(2) {
                    Some(p) => ShellJob::ExtractHere(PathBuf::from(p)),
                    None => ShellJob::Bad("usage: --run extract --here <archive>".into()),
                }
            } else {
                ShellJob::Bad("usage: --run extract --here <archive>".into())
            }
        }
        Some("extract-into") => match args.get(1) {
            Some(p) => ShellJob::ExtractInto { folder: PathBuf::from(p), archive: PathBuf::new() },
            None => ShellJob::Bad("usage: --run extract-into <folder>".into()),
        },
        Some("repair") => match args.get(1) {
            Some(p) => ShellJob::Repair(PathBuf::from(p)),
            None => ShellJob::Bad("usage: --run repair <archive>".into()),
        },
        _ => ShellJob::Bad("usage: --run create|create-email|extract --here|extract-into|repair <path>".into()),
    }
}

/// "Compress to .next and email": after the archive exists, hand it to the
/// default MAPI mail client (the same mechanism 7-Zip uses). mapi32.dll
/// ships with every Windows; when no MAPI provider is available the call
/// fails and we fall back to revealing the archive in Explorer.
/// Is a *plausible* default mail client configured? We only hand the
/// archive to MAPI when both signals agree: a non-browser `mailto` ProgId
/// (browsers hijack this when no mail app is chosen) AND a `Clients\Mail`
/// entry that isn't the legacy "Hotmail" stub. Calling MAPI without a real
/// provider hangs, so a false positive is worse than no email.
#[cfg(windows)]
fn has_mail_client() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let prog_id = hkcu
        .open_subkey(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\mailto\UserChoice")
        .and_then(|k| k.get_value::<String, _>("ProgId"))
        .unwrap_or_default();
    if prog_id.to_ascii_uppercase().contains("HTM") {
        return false; // a browser is standing in for the mail app
    }
    let mail = |hkcu: &RegKey, hklm: &RegKey| -> Option<String> {
        hkcu.open_subkey_with_flags(r"Software\Clients\Mail", winreg::enums::KEY_READ)
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| {
                hklm.open_subkey_with_flags(r"Software\Clients\Mail", winreg::enums::KEY_READ)
                    .and_then(|k| k.get_value::<String, _>(""))
                    .ok()
                    .filter(|n| !n.trim().is_empty())
            })
    };
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    match mail(&hkcu, &hklm) {
        Some(name) => !name.eq_ignore_ascii_case("Hotmail"),
        None => false,
    }
}

#[cfg(windows)]
fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(path).spawn();
}

#[cfg(windows)]
fn mail_attach(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;

    if !has_mail_client() {
        reveal_in_explorer(path);
        return "no mail client found — archive selected in Explorer".to_string();
    }

    #[repr(C)]
    struct MapiFileDescW {
        ul_reserved: u32,
        fl_flags: i32,
        n_position: u32,
        lpsz_path_name: *mut u16,
        lpsz_file_name: *mut u16,
        lp_file_type: *mut core::ffi::c_void,
    }
    #[repr(C)]
    struct MapiMessageW {
        ul_reserved: u32,
        lpsz_subject: *mut u16,
        lpsz_note_text: *mut u16,
        lpsz_message_type: *mut u16,
        lpsz_date_received: *mut u16,
        lpsz_conversation_id: *mut u16,
        fl_flags: i32,
        lp_originator: *mut u16,
        n_recip_count: u32,
        lp_recips: *mut core::ffi::c_void,
        n_file_count: u32,
        lp_files: *mut MapiFileDescW,
    }
    // mapi32.dll exports MAPISendMailW only by ordinal in the modern SDK
    // import lib, so resolve it at runtime (the DLL itself exports it by
    // name — this is the standard workaround).
    type Mapisendmailw = unsafe extern "system" fn(usize, usize, *mut MapiMessageW, i32, usize) -> u32;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let lib_name: Vec<u16> = "mapi32.dll".encode_utf16().chain(std::iter::once(0)).collect();
    let lib = unsafe { LoadLibraryW(lib_name.as_ptr()) };
    let proc_name: Vec<u8> = b"MAPISendMailW\0".to_vec();
    let proc = if lib.is_null() { None } else { unsafe { GetProcAddress(lib, proc_name.as_ptr()) } };
    let Some(proc) = proc else {
        return "no mail client found — archive selected in Explorer".to_string();
    };
    let send_mail: Mapisendmailw = unsafe { std::mem::transmute(proc) };

    const MAPI_DIALOG: i32 = 0x8;
    const MAPI_SUCCESS: u32 = 0;
    const MAPI_E_USER_ABORT: u32 = 1;

    let mut wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut wide_name: Vec<u16> = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut subject: Vec<u16> = "nextar archive".encode_utf16().chain(std::iter::once(0)).collect();

    // Run the call on its own thread with a grace window: a real client
    // returns promptly after the compose dialog opens, while a missing
    // provider would block forever — the process exit (window close) kills
    // the straggler either way, so the shell window never hangs.
    // The MAPI structs contain raw pointers, so they are built inside the
    // closure where the (Send) wide buffers are owned.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut file_desc = MapiFileDescW {
            ul_reserved: 0,
            fl_flags: 0,
            n_position: u32::MAX, // MAPI_POS: attach at the end
            lpsz_path_name: wide_path.as_mut_ptr(),
            lpsz_file_name: if wide_name.len() > 1 { wide_name.as_mut_ptr() } else { std::ptr::null_mut() },
            lp_file_type: std::ptr::null_mut(),
        };
        let mut msg = MapiMessageW {
            ul_reserved: 0,
            lpsz_subject: subject.as_mut_ptr(),
            lpsz_note_text: std::ptr::null_mut(),
            lpsz_message_type: std::ptr::null_mut(),
            lpsz_date_received: std::ptr::null_mut(),
            lpsz_conversation_id: std::ptr::null_mut(),
            fl_flags: 0,
            lp_originator: std::ptr::null_mut(),
            n_recip_count: 0,
            lp_recips: std::ptr::null_mut(),
            n_file_count: 1,
            lp_files: &mut file_desc,
        };
        // SAFETY: the structs match the documented MAPI layout (mapiform.h
        // / mapidefs.h); the wide buffers outlive the call (owned here);
        // MAPISendMailW only reads them during the call.
        let code = unsafe { send_mail(0, 0, &mut msg, MAPI_DIALOG, 0) };
        let _ = tx.send(code);
    });
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(MAPI_SUCCESS) => "email opened with the archive attached".to_string(),
        Ok(MAPI_E_USER_ABORT) => "email cancelled".to_string(),
        Ok(_) => {
            // MAPI provider refused: reveal the archive in Explorer so the
            // user can attach it manually.
            reveal_in_explorer(path);
            "mail client unavailable — archive selected in Explorer".to_string()
        }
        Err(_) => {
            // Still running: a real client has the compose window open
            // (leave it alone) or a broken provider would hang forever (the
            // window closing kills the thread). Either way the archive is
            // saved next to the source.
            "email opened in your mail app".to_string()
        }
    }
}

#[cfg(not(windows))]
fn mail_attach(path: &Path) -> String {
    format!("created — attach it manually ({})", path.display())
}

struct ShellApp {
    job: ShellJob,
    started: bool,
    state: Option<Arc<ProgressState>>,
    rx: Option<Receiver<Result<String>>>,
    result: Option<std::result::Result<String, String>>,
    close_timer: Option<f32>,
    /// Last title sent to the OS window (dedupe for `ViewportCommand::Title`).
    window_title: String,
}

impl ShellApp {
    fn label(&self) -> &str {
        match &self.job {
            ShellJob::Create(_) | ShellJob::CreateEmail(_) => "Compressing…",
            ShellJob::ExtractHere(_) | ShellJob::ExtractInto { .. } => "Extracting…",
            ShellJob::Repair(_) => "Repairing…",
            ShellJob::Bad(_) => "",
        }
    }

    fn start(&mut self) {
        let label = self.label().trim_end_matches('…');
        let state = Arc::new(ProgressState::new(0, label));
        let (tx, rx) = mpsc::channel();
        self.state = Some(state.clone());
        self.rx = Some(rx);
        let started = Instant::now();
        match &self.job {
            ShellJob::Create(inputs) | ShellJob::CreateEmail(inputs) => {
                let inputs = inputs.clone();
                let email = matches!(&self.job, ShellJob::CreateEmail(_));
                let output = default_output(&inputs);
                std::thread::spawn(move || {
                    let opts = CreateOptions {
                        codec: nextar::format::CODE_ZSTD,
                        level: 3,
                        block_size: 1024 * 1024,
                        password: None,
                        threads: num_cpus::get(),
                        segment_size: 128,
                        parity: 0,
                        quiet: true,
                        progress: Some(state.clone()),
                    };
                    let res = archive::create(&inputs, &output, opts).map(|s| {
                        let mail = if email { Some(mail_attach(&output)) } else { None };
                        let names = inputs
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(" + ");
                        let mut msg = format!(
                            "archive created · {} → {}\n{} in, {} out · {:.2}s",
                            names,
                            output.display(),
                            human(s.total_bytes_read),
                            human(s.archive_size),
                            started.elapsed().as_secs_f64()
                        );
                        if let Some(m) = mail {
                            msg.push_str(&format!("\n📧 {m}"));
                        }
                        msg
                    });
                    let _ = tx.send(res);
                });
            }
            ShellJob::ExtractInto { folder, archive } => {
                let (folder, archive) = (folder.clone(), archive.clone());
                std::thread::spawn(move || {
                    let res = archive::extract(&archive, &folder, None, num_cpus::get(), true, true, Some(state.clone())).map(
                        |s| {
                            format!(
                                "extracted {} files · {} dirs · {} symlinks ({} bytes) → {}\n{:.2}s",
                                s.files,
                                s.dirs,
                                s.symlinks,
                                human(s.bytes),
                                folder.display(),
                                started.elapsed().as_secs_f64()
                            )
                        },
                    );
                    let _ = tx.send(res);
                });
            }
            ShellJob::ExtractHere(archive) => {
                let archive = archive.clone();
                std::thread::spawn(move || {
                    let parent = archive.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let stem = archive
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "extracted".to_string());
                    let out = parent.join(stem);
                    let res = archive::extract(&archive, &out, None, num_cpus::get(), true, true, Some(state.clone())).map(
                        |s| {
                            format!(
                                "extracted {} files · {} dirs · {} symlinks ({} bytes) → {}\n{:.2}s",
                                s.files,
                                s.dirs,
                                s.symlinks,
                                human(s.bytes),
                                out.display(),
                                started.elapsed().as_secs_f64()
                            )
                        },
                    );
                    let _ = tx.send(res);
                });
            }
            ShellJob::Repair(archive) => {
                let archive = archive.clone();
                std::thread::spawn(move || {
                    let volume = nextar::archive::volume_path_for(&archive);
                    let out = repaired_path_for(&archive);
                    let res = archive::repair(&archive, &volume, &out, true, Some(state.clone())).map(|s| {
                        format!(
                            "repaired {} of {} blocks → {}\n{} bytes · {:.2}s",
                            s.repaired,
                            s.total_blocks,
                            out.display(),
                            human(s.out_size),
                            started.elapsed().as_secs_f64()
                        )
                    });
                    let _ = tx.send(res);
                });
            }
            ShellJob::Bad(_) => {}
        }
        self.started = true;
    }
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        refresh_palette();
        apply_titlebar(frame);
        apply_window_corners(frame);
        configure_theme(&ctx);
        if !self.started {
            self.start();
        }

        // poll the job
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(res) => {
                    self.result = Some(match res {
                        Ok(m) => Ok(m),
                        Err(e) => Err(format!("{e:#}")),
                    });
                    self.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint(),
                Err(mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        }
        // Mirror the live job (or its outcome) into the native title bar:
        // "nextar — extracting 62%", then "done"/"failed" on completion.
        let title = if let Some(res) = &self.result {
            match res {
                Ok(_) => "nextar — done".to_string(),
                Err(_) => "nextar — failed".to_string(),
            }
        } else if let Some(s) = &self.state {
            format!("nextar — {} {:.0}%", s.label(), s.pct())
        } else {
            "nextar".to_string()
        };
        if title != self.window_title {
            self.window_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 130.0) * 0.5);
                draw_logo(ui, 28.0);
                ui.label(RichText::new("NEXTAR").size(16.0).strong());
            });
            ui.add_space(6.0);

            if let ShellJob::Bad(msg) = &self.job {
                ui.label(RichText::new(msg).color(err()).size(13.0));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.label(RichText::new("closing…").color(text3()).size(11.0));
                });
                let t = self.close_timer.get_or_insert(0.0);
                *t += ui.input(|i| i.stable_dt);
                if *t > 4.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ctx.request_repaint();
                return;
            }

            if self.result.is_none() {
                if let Some(state) = &self.state {
                    let pct = state.pct() / 100.0;
                    let done = human(state.done_bytes());
                    let total = human(state.total());
                    let speed = if state.elapsed() > 0.0 {
                        human((state.done_bytes() as f64 / state.elapsed()) as u64)
                    } else {
                        human(0)
                    };
                    led_bar(ui, pct, &format!("{} · {} / {} · {}/s", self.label(), done, total, speed), true);
                    ctx.request_repaint();
                }
            } else if let Some(res) = &self.result {
                match res {
                    Ok(msg) => {
                        ui.label(RichText::new(format!("✔  {msg}")).color(ok()).size(12.5));
                        let t = self.close_timer.get_or_insert(0.0);
                        *t += ui.input(|i| i.stable_dt);
                        if *t > 1.5 {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ctx.request_repaint();
                    }
                    Err(msg) => {
                        ui.label(RichText::new(format!("❌  {msg}")).color(err()).size(12.5));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Close").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            // Shell actions are launched from Explorer; a
                            // window that never closes is worse than one that
                            // dismisses itself, so errors auto-close too
                            // (longer than success so the message is readable).
                            ui.label(RichText::new("closing…").color(text3()).size(11.0));
                        });
                        let t = self.close_timer.get_or_insert(0.0);
                        *t += ui.input(|i| i.stable_dt);
                        if *t > 4.0 {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ctx.request_repaint();
                    }
                }
            }
        });
    }
}

// ---------------------------------------------------------------- splash window
/// The clean glass boot scene: zipper logo over a soft halo with an
/// orbiting accent dot and a gradient boot bar, fading in and back out
/// (matches the site's hero).
fn draw_splash_scene(ctx: &egui::Context, t: f32) {
    let screen = ctx.content_rect();
    let fade_in = (t / 0.35).clamp(0.0, 1.0);
    let fade_out = ((SPLASH_DURATION - t) / 0.30).clamp(0.0, 1.0);
    let a = (fade_in * fade_out).min(1.0);
    let cx = screen.center().x;
    let cy = screen.top() + screen.height() * 0.46;

    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("splash")));

    // background: glass void with a soft breathing halo
    p.rect_filled(screen, CornerRadius::ZERO, alpha(bg(), a));
    let halo_r = screen.width().min(screen.height()) * 0.34;
    let pulse = (t * 1.6).sin() * 0.5 + 0.5;
    p.circle_filled(egui::pos2(cx, cy), halo_r * (1.04 + pulse * 0.05), alpha(neon_cyan(), 0.05 * a));
    p.circle_filled(egui::pos2(cx, cy), halo_r * 0.90, alpha(accent2(), 0.06 * a));
    p.circle_filled(egui::pos2(cx, cy), halo_r * 0.55, alpha(neon_cyan(), 0.04 * a));

    // orbit ring + rotating accent dot (the site's orbit motif)
    let orbit_r = halo_r * 0.72;
    p.circle_stroke(
        egui::pos2(cx, cy),
        orbit_r,
        Stroke::new(1.0, alpha(border(), a)),
    );
    let ang = t * 0.9;
    let dot = egui::pos2(cx + ang.cos() * orbit_r, cy + ang.sin() * orbit_r);
    p.circle_filled(dot, 2.6, alpha(neon_cyan(), a));
    p.circle_filled(dot, 5.0, alpha(neon_cyan(), 0.25 * a));

    // convergence-core logo: fragments converge in as the mark rises
    let rise = (1.0 - smoothstep(fade_in, 0.0, 1.0)) * 10.0;
    let logo_s = (screen.width().min(screen.height()) * 0.22).clamp(96.0, 170.0);
    let logo_rect = egui::Rect::from_center_size(egui::pos2(cx, cy + rise), Vec2::splat(logo_s));
    draw_logo_tile(&p, logo_rect, a);
    let pal = logo_palette();
    if reduced_motion() {
        converge_mark(&p, logo_rect, pal, a, 1.0);
    } else {
        draw_converging_particles(&p, logo_rect, t, pal);
        let build = smoothstep((t - 0.12) / 0.5, 0.0, 1.0);
        converge_mark(&p, logo_rect, pal, a, build);
    }
    // wordmark: letter-spaced Space Grotesk, per-letter staggered fade-in
    // (each glyph eases up and in, then the whole word fades with the scene)
    let word = "NEXTAR";
    let fs = 26.0;
    let font_id = egui::FontId::proportional(fs);
    let tracking = 6.0;
    let base_y = logo_rect.bottom() + 24.0;
    let glyphs: Vec<(char, f32)> = word
        .chars()
        .map(|c| (c, p.layout_no_wrap(c.to_string(), font_id.clone(), Color32::WHITE).size().x))
        .collect();
    let total_w: f32 = glyphs.iter().map(|(_, w)| *w).sum::<f32>() + tracking * (glyphs.len() as f32 - 1.0);
    let mut x = cx - total_w * 0.5;
    for (i, (ch, w)) in glyphs.iter().enumerate() {
        let lt = ((t - 0.30 - i as f32 * 0.06) / 0.18).clamp(0.0, 1.0);
        let e = smoothstep(lt, 0.0, 1.0);
        let ly = base_y - (1.0 - e) * 4.0;
        p.text(
            egui::pos2(x, ly),
            egui::Align2::LEFT_CENTER,
            ch.to_string(),
            font_id.clone(),
            alpha(text(), a * e),
        );
        x += w + tracking;
    }
    p.text(
        egui::pos2(cx, logo_rect.bottom() + 44.0),
        egui::Align2::CENTER_CENTER,
        "fast · secure · self-healing",
        egui::FontId::proportional(12.0),
        alpha(text2(), a),
    );

    // glass boot bar with a gradient fill
    let bar_w = 240.0;
    let bar_y = screen.top() + screen.height() * 0.80;
    let bar = egui::Rect::from_min_size(egui::pos2(cx - bar_w * 0.5, bar_y), Vec2::new(bar_w, 4.0));
    p.rect_filled(bar, CornerRadius::same(2), alpha(surface2(), a));
    p.rect_stroke(bar, CornerRadius::same(2), Stroke::new(1.0, alpha(border(), a)), egui::StrokeKind::Inside);
    let fill_w = bar_w * (t / SPLASH_DURATION).clamp(0.0, 1.0);
    if fill_w > 0.5 {
        let fill = egui::Rect::from_min_size(bar.min, Vec2::new(fill_w, 4.0));
        let n = 16usize;
        for i in 0..n {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            let seg = egui::Rect::from_min_max(
                egui::pos2(fill.left() + t0 * fill.width(), fill.top()),
                egui::pos2(fill.left() + t1 * fill.width(), fill.bottom()),
            );
            let cr = if i == 0 {
                CornerRadius { nw: 2, ne: 0, sw: 2, se: 0 }
            } else if i == n - 1 {
                CornerRadius { nw: 0, ne: 2, sw: 0, se: 2 }
            } else {
                CornerRadius::ZERO
            };
            p.rect_filled(seg, cr, alpha(grad_color((t0 + t1) * 0.5), a));
        }
    }
    p.text(
        egui::pos2(cx, bar.bottom() + 14.0),
        egui::Align2::CENTER_CENTER,
        "zstd · lzma2 · argon2id · xchacha20-poly1305 · reed-solomon",
        egui::FontId::proportional(10.0),
        alpha(text3(), a),
    );
}

/// Small standalone boot window: plays the scene, then closes so the main
/// window can open (or skips immediately on click).
struct SplashApp {
    t: f32,
}

impl eframe::App for SplashApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        refresh_palette();
        apply_titlebar(frame);
        apply_window_corners(frame);
        configure_theme(&ctx);
        self.t += ui.input(|i| i.stable_dt);
        draw_splash_scene(&ctx, self.t);
        ctx.request_repaint();
        if self.t >= SPLASH_DURATION || ui.input(|i| i.pointer.any_click()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn main() -> eframe::Result {
    // Stable taskbar identity + COM for the taskbar progress overlay, once
    // per process. Failures are ignored — taskbar progress is a nicety.
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        let id: Vec<u16> = "Nextar.Nextar".encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        }
    }
    // Load the persisted settings before any window starts, so the splash,
    // shell mode, main UI and the Create-view defaults all apply from the
    // first frame.
    load_settings();
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--reset-settings` → headless recovery: back up and delete the settings
    // file, then exit without opening any window.
    if args.iter().any(|a| a == "--reset-settings") {
        match backup_and_remove_settings() {
            Some(backup) => println!("nextar: settings reset — backed up to {}", backup.display()),
            None => println!("nextar: no settings file at {}", settings_path().display()),
        }
        return Ok(());
    }

    // `--run` → Explorer right-click shell mode: a small progress window.
    if let Some(first) = args.first() {
        if first == "--run" {
            let mut job = parse_shell_job(&args[1..]);
            // "Extract .next… here" passes the *folder*; ask which archive
            // to extract before any window opens (cancel → exit quietly).
            if let ShellJob::ExtractInto { folder, archive: _ } = &job {
                match rfd::FileDialog::new()
                    .add_filter("nextar archives", &["next"])
                    .pick_file()
                {
                    Some(a) => job = ShellJob::ExtractInto { folder: folder.clone(), archive: a },
                    None => return Ok(()),
                }
            }
            let shell = ShellApp {
                job,
                started: false,
                state: None,
                rx: None,
                result: None,
                close_timer: None,
                window_title: "nextar".to_string(),
            };
            let options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title("nextar")
                    .with_inner_size([500.0, 190.0])
                    .with_min_inner_size([500.0, 190.0])
                    .with_resizable(false)
                    .with_always_on_top(),
                ..Default::default()
            };
            return eframe::run_native(
                "nextar",
                options,
                Box::new(|cc| {
                    configure_theme(&cc.egui_ctx);
                    Ok(Box::new(shell))
                }),
            );
        }
    }

    // Optional CLI arg: an archive path to open straight into the Inspect view
    // (used by the Explorer "Open with nextar" association).
    let mut app = GuiApp::default();
    if let Some(p) = args.first() {
        if Path::new(p).is_file() {
            app.inspect_archive = p.clone();
            app.view = View::Inspect;
            app.load_inspect();
            app.extract_banner = true;
        }
    }

    // Boot splash: a brief standalone synthwave window before the main UI.
    eframe::run_native(
        "nextar",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("nextar")
                .with_inner_size([480.0, 300.0])
                .with_min_inner_size([480.0, 300.0])
                .with_resizable(false)
                .with_decorations(false)
                .with_always_on_top(),
            ..Default::default()
        },
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(SplashApp { t: 0.0 }))
        }),
    )?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nextar")
            .with_inner_size([1020.0, 700.0])
            .with_min_inner_size([860.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "nextar",
        options,
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_snaps_at_ends() {
        let t = ThemeTween {
            start: Instant::now(),
            start_blend: 0.0,
            target: true,
        };
        assert_eq!(t.current_at(0.0), 0.0);
        assert_eq!(t.current_at(1.0), 1.0);
        let t2 = ThemeTween {
            start: Instant::now(),
            start_blend: 0.8,
            target: false,
        };
        assert_eq!(t2.current_at(0.0), 0.8);
        assert_eq!(t2.current_at(1.0), 0.0);
    }

    #[test]
    fn tween_is_monotonic_and_eased() {
        let t = ThemeTween {
            start: Instant::now(),
            start_blend: 0.0,
            target: true,
        };
        let mut prev = 0.0f32;
        for i in 0..=100 {
            let v = t.current_at(i as f32 / 100.0);
            assert!(v >= prev - 1e-6, "not monotonic at {i}: {v} < {prev}");
            prev = v;
        }
        assert!(prev > 0.99);
        // ease: quarter-way should be well under a quarter of the way there
        assert!(t.current_at(0.25) < 0.25);
    }

    #[test]
    fn mid_flip_continues_from_current_blend() {
        let t = ThemeTween {
            start: Instant::now(),
            start_blend: 0.0,
            target: true,
        };
        let half = t.current_at(0.5);
        // flipping target mid-flight starts a new segment from that blend
        let t2 = ThemeTween {
            start: Instant::now(),
            start_blend: half,
            target: false,
        };
        assert_eq!(t2.current_at(0.0), half);
        assert_eq!(t2.current_at(1.0), 0.0);
    }

    #[test]
    fn blend_palettes_interpolates_midpoint() {
        let mid = blend_palettes(LIGHT_PALETTE, DARK_PALETTE, 0.5);
        // lerp8 rounds to nearest, so half-sums of odd totals round up.
        let expect = |a: u8, b: u8| ((a as u16 + b as u16 + 1) / 2) as u8;
        assert_eq!(mid.tile_a.r(), expect(LIGHT_PALETTE.tile_a.r(), DARK_PALETTE.tile_a.r()));
        assert_eq!(mid.layer_b.g(), expect(LIGHT_PALETTE.layer_b.g(), DARK_PALETTE.layer_b.g()));
        assert_eq!(mid.bezel.a(), expect(LIGHT_PALETTE.bezel.a(), DARK_PALETTE.bezel.a()));
    }

    #[test]
    fn ui_blend_maps_endpoints_exactly() {
        // Every field of blend_ui must map the dark/light endpoints exactly.
        let same = |a: Palette, b: Palette| {
            a.bg == b.bg && a.bg2 == b.bg2 && a.surface == b.surface && a.surface2 == b.surface2
                && a.border == b.border && a.text == b.text && a.text2 == b.text2 && a.text3 == b.text3
                && a.accent == b.accent && a.accent2 == b.accent2 && a.accent3 == b.accent3
                && a.ok == b.ok && a.err == b.err && a.active == b.active
        };
        assert!(same(blend_ui(UI_DARK, UI_LIGHT, 0.0), UI_DARK));
        assert!(same(blend_ui(UI_DARK, UI_LIGHT, 1.0), UI_LIGHT));
        // Readability at the endpoints: light text on dark surfaces, dark
        // text on light surfaces.
        assert!(UI_DARK.text.r() > UI_DARK.surface.r());
        assert!(UI_LIGHT.text.r() < UI_LIGHT.surface.r());
    }

    #[test]
    fn override_precedence_env_then_setting_then_os() {
        // env pin beats the user setting and the registry
        assert!(effective_dark(Some(true), ThemeOverride::Light, false));
        assert!(!effective_dark(Some(false), ThemeOverride::Dark, true));
        // user setting beats the registry
        assert!(effective_dark(None, ThemeOverride::Dark, false));
        assert!(!effective_dark(None, ThemeOverride::Light, true));
        // follow falls back to the registry value
        assert!(effective_dark(None, ThemeOverride::Follow, true));
        assert!(!effective_dark(None, ThemeOverride::Follow, false));
        // env pin parsing
        assert_eq!(env_pin("dark"), Some(true));
        assert_eq!(env_pin("LIGHT"), Some(false));
        assert_eq!(env_pin("banana"), None);
    }

    #[test]
    fn settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("nextar-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        // missing file → None
        assert_eq!(load_settings_at(&path), None);
        // save + load the full settings (appearance + create defaults)
        let full = Settings {
            appearance: ThemeOverride::Dark,
            codec: Some("lzma2".to_string()),
            level: Some(9),
            block: Some("4M".to_string()),
            threads: Some(4),
            recovery: Some(8),
            recent: Some(vec!["C:\\a\\b.next".to_string()]),
        };
        save_settings_at(&path, &full).unwrap();
        assert_eq!(load_settings_at(&path), Some(full));
        // missing keys fall back to defaults (old/partial settings files)
        std::fs::write(&path, "{\"appearance\":\"light\"}").unwrap();
        let s = load_settings_at(&path).unwrap();
        assert_eq!(s.appearance, ThemeOverride::Light);
        assert_eq!(s.codec, None);
        assert_eq!(s.level, None);
        assert_eq!(s.block, None);
        assert_eq!(s.threads, None);
        assert_eq!(s.recovery, None);
        // garbage file → None
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_settings_at(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_migrate_from_older_schema() {
        let dir = std::env::temp_dir().join(format!("nextar-settings-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        // v1 schema: only the appearance pin existed. Its value must survive
        // the upgrade, and every field added later falls back to its default.
        std::fs::write(&path, "{\"appearance\":\"dark\"}").unwrap();
        let s = load_settings_at(&path).unwrap();
        assert_eq!(s.appearance, ThemeOverride::Dark, "v1 appearance value must be preserved");
        assert_eq!(s.codec, None);
        assert_eq!(s.level, None);
        assert_eq!(s.block, None);
        assert_eq!(s.threads, None);
        assert_eq!(s.recovery, None);
        assert_eq!(s.recent, None);

        // v2 schema: create defaults existed but `recent` did not. Existing
        // values survive; the newer `recent` field defaults to None.
        std::fs::write(
            &path,
            "{\"appearance\":\"light\",\"codec\":\"lzma2\",\"level\":9,\"block\":\"4M\",\"threads\":8,\"recovery\":16}",
        )
        .unwrap();
        let s = load_settings_at(&path).unwrap();
        assert_eq!(s.appearance, ThemeOverride::Light);
        assert_eq!(s.codec.as_deref(), Some("lzma2"));
        assert_eq!(s.level, Some(9));
        assert_eq!(s.block.as_deref(), Some("4M"));
        assert_eq!(s.threads, Some(8));
        assert_eq!(s.recovery, Some(16));
        assert_eq!(s.recent, None, "v2 files have no recent list");

        // forward compatibility: fields written by a *future* version are
        // ignored, and the settings still load with everything we know.
        std::fs::write(
            &path,
            "{\"appearance\":\"light\",\"future_field\":{\"x\":1},\"unknown\":true}",
        )
        .unwrap();
        let s = load_settings_at(&path).unwrap();
        assert_eq!(s.appearance, ThemeOverride::Light);
        assert_eq!(s.codec, None);
        assert_eq!(s.recent, None);

        // a migrated file re-saves as a complete current-schema file that
        // loads back identically.
        save_settings_at(&path, &s).unwrap();
        assert_eq!(load_settings_at(&path), Some(s));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_fixtures_migrate_and_preserve_values() {
        // Historical settings.json files (one per schema version) must load
        // with their original values preserved and newer fields defaulted.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings");
        let cases: &[(&str, Settings)] = &[
            (
                "v1-appearance-only.json",
                Settings {
                    appearance: ThemeOverride::Dark,
                    codec: None,
                    level: None,
                    block: None,
                    threads: None,
                    recovery: None,
                    recent: None,
                },
            ),
            (
                "v2-create-defaults.json",
                Settings {
                    appearance: ThemeOverride::Light,
                    codec: Some("lzma2".to_string()),
                    level: Some(9),
                    block: Some("4M".to_string()),
                    threads: Some(8),
                    recovery: Some(16),
                    recent: None,
                },
            ),
            (
                "v3-current.json",
                Settings {
                    appearance: ThemeOverride::Dark,
                    codec: Some("zstd".to_string()),
                    level: Some(5),
                    block: Some("1M".to_string()),
                    threads: Some(8),
                    recovery: Some(4),
                    recent: Some(vec!["C:\\Users\\alice\\Documents\\backup.next".to_string()]),
                },
            ),
        ];
        for (name, want) in cases {
            let p = dir.join(name);
            let s = load_settings_at(&p).unwrap_or_else(|| panic!("fixture {name} failed to load"));
            assert_eq!(&s, want, "fixture {name} migrated to an unexpected settings value");
        }
    }

    #[test]
    fn corrupt_backup_path_keeps_same_directory() {
        let p = Path::new("C:\\Users\\me\\AppData\\Local\\nextar\\settings.json");
        let b = corrupt_backup_path(p);
        assert_eq!(b.parent(), p.parent(), "backup must live next to the original");
        let name = b.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("settings.json.corrupt-"),
            "backup name should be timestamped, got {name}"
        );
        assert_ne!(name, "settings.json", "backup must not overwrite the original name");
    }

    #[test]
    fn colorref_packs_bgr() {
        // COLORREF is 0x00BBGGRR — blue in the high byte, red in the low.
        assert_eq!(colorref(Color32::from_rgb(0x11, 0x22, 0x33)), 0x0033_2211);
        assert_eq!(colorref(Color32::from_rgb(0xff, 0x00, 0x00)), 0x0000_00ff);
        assert_eq!(colorref(Color32::from_rgb(0x00, 0x00, 0xff)), 0x00ff_0000);
        // the exact chrome caption color dark-mode surface
        assert_eq!(colorref(UI_DARK.surface), 0x0024_1711);
    }

    #[test]
    fn circle_tile_mesh_covers_center_not_corners() {
        // The tile mesh must fill the circle: center and edge midpoints
        // covered, the four corners left empty, consistent winding.
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        let mesh = circle_tile_mesh(rect, 44.0, |_| Color32::from_rgb(255, 0, 0));
        assert_eq!(mesh.indices.len() % 3, 0);
        let tri = |i: usize| -> [egui::Pos2; 3] {
            [
                mesh.vertices[mesh.indices[i * 3] as usize].pos,
                mesh.vertices[mesh.indices[i * 3 + 1] as usize].pos,
                mesh.vertices[mesh.indices[i * 3 + 2] as usize].pos,
            ]
        };
        let cross = |t: [egui::Pos2; 3]| {
            (t[1].x - t[0].x) * (t[2].y - t[0].y) - (t[1].y - t[0].y) * (t[2].x - t[0].x)
        };
        let first = cross(tri(0));
        assert!(first > 0.0, "first triangle must be positive-cross");
        for i in 1..mesh.indices.len() / 3 {
            assert!(
                (cross(tri(i)) > 0.0) == (first > 0.0),
                "triangle {i} has inconsistent winding"
            );
        }
        let inside = |p: egui::Pos2| {
            (0..mesh.indices.len() / 3).any(|i| {
                let [a, b, c] = tri(i);
                let d1 = cross([a, b, p]);
                let d2 = cross([b, c, p]);
                let d3 = cross([c, a, p]);
                (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
            })
        };
        assert!(inside(egui::pos2(50.0, 50.0)), "center should be covered");
        assert!(inside(egui::pos2(50.0, 8.0)), "top of circle should be covered");
        assert!(inside(egui::pos2(8.0, 50.0)), "left of circle should be covered");
        assert!(!inside(egui::pos2(2.0, 2.0)), "top-left corner should be empty");
        assert!(!inside(egui::pos2(98.0, 2.0)), "top-right corner should be empty");
        assert!(!inside(egui::pos2(2.0, 98.0)), "bottom-left corner should be empty");
        assert!(!inside(egui::pos2(98.0, 98.0)), "bottom-right corner should be empty");
    }

    #[test]
    fn converge_planes_descend_toward_core() {
        // Planes must narrow and deepen toward the core, and the core must
        // sit just below the innermost apex so the fold reads as converging.
        for w in PLANES.windows(2) {
            let (r0, t0, a0, h0) = w[0];
            let (r1, t1, a1, h1) = w[1];
            assert!(r1 < r0, "reach must shrink inward");
            assert!(t1 > t0, "tops must descend");
            assert!(a1 > a0, "apexes must descend");
            assert!(h1 <= h0, "stroke must stay thin");
        }
        let (_, _, inner_apex, _) = PLANES[2];
        assert!(CORE.1 > inner_apex, "core sits below the innermost apex");
        assert!((0.02..0.1).contains(&CORE.2), "core radius is sane");
    }
}
