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
    bg: Color32::from_rgb(0x0a, 0x06, 0x14), // deep synthwave void
    bg2: Color32::from_rgb(0x13, 0x0c, 0x24),
    surface: Color32::from_rgb(0x1c, 0x12, 0x31),
    surface2: Color32::from_rgb(0x2b, 0x1d, 0x4b),
    border: Color32::from_rgb(0x3a, 0x2a, 0x63),
    text: Color32::from_rgb(0xf1, 0xec, 0xff),
    text2: Color32::from_rgb(0xb8, 0xa8, 0xd9),
    text3: Color32::from_rgb(0x6f, 0x5d, 0x99),
    accent: Color32::from_rgb(0x00, 0xff, 0xf7), // neon cyan
    accent2: Color32::from_rgb(0x9b, 0x5c, 0xff), // neon violet
    accent3: Color32::from_rgb(0xff, 0x2b, 0xd6), // hot pink
    ok: Color32::from_rgb(0x2e, 0xff, 0xb0),
    err: Color32::from_rgb(0xff, 0x4d, 0x6d),
    active: Color32::from_rgb(0x10, 0x0e, 0x1e),
};

const UI_LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf4, 0xf1, 0xfa), // lavender-white
    bg2: Color32::from_rgb(0xea, 0xe5, 0xf4),
    surface: Color32::from_rgb(0xff, 0xfe, 0xff),
    surface2: Color32::from_rgb(0xe6, 0xdf, 0xf2),
    border: Color32::from_rgb(0xd3, 0xc9, 0xe8),
    text: Color32::from_rgb(0x21, 0x1a, 0x3a),
    text2: Color32::from_rgb(0x58, 0x49, 0x7d),
    text3: Color32::from_rgb(0x94, 0x87, 0xb5),
    accent: Color32::from_rgb(0x00, 0xb3, 0xc9), // deeper cyan for contrast on white
    accent2: Color32::from_rgb(0x77, 0x45, 0xe8), // violet
    accent3: Color32::from_rgb(0xe0, 0x19, 0xa8), // pink
    ok: Color32::from_rgb(0x0f, 0xa8, 0x71),
    err: Color32::from_rgb(0xd9, 0x26, 0x4b),
    active: Color32::from_rgb(0xe8, 0xe0, 0xf6),
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

fn smoothstep(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------- logo palette
/// Light/dark tile + chrome colors for the brand lockup. The GUI picks the
/// palette from the Windows apps theme (light vs dark) and swaps live.
/// The mark is clean vector geometry: the tile is a smooth glass gradient
/// with a hairline bezel, the chevrons are mitered polygons with one
/// along-bar gradient each (base → tip).
#[derive(Clone, Copy)]
struct LogoPalette {
    tile_a: Color32,
    tile_b: Color32,
    tile_c: Color32,
    tile_mag: Color32, // soft pink glass reflection at the bottom
    back_a: Color32,   // back chevron steel (base end)
    back_b: Color32,   // back chevron tip end
    front_a: Color32,  // front chevron steel (base end)
    front_b: Color32,  // front chevron tip end (bright chrome)
    lit: Color32,      // cyan lit-chrome edge along the front chevron (with alpha)
    bezel: Color32,    // crisp hairline tile bezel (with alpha)
}

const LIGHT_PALETTE: LogoPalette = LogoPalette {
    tile_a: Color32::from_rgb(0xfb, 0xfd, 0xff),
    tile_b: Color32::from_rgb(0xe0, 0xee, 0xf9),
    tile_c: Color32::from_rgb(0xbf, 0xd9, 0xf0),
    tile_mag: Color32::from_rgb(0xff, 0x9e, 0xe6),
    back_a: Color32::from_rgb(0x21, 0x36, 0x4e), // dim, receding cool steel
    back_b: Color32::from_rgb(0x66, 0xa0, 0xbe), // muted cyan steel tip
    front_a: Color32::from_rgb(0x19, 0x23, 0x37), // deep cool gunmetal base
    front_b: Color32::from_rgb(0xd8, 0xe2, 0xec), // bright cool silver tip
    lit: Color32::from_rgba_unmultiplied_const(0x9d, 0xee, 0xff, 150),
    bezel: Color32::from_rgba_unmultiplied_const(0x00, 0xd9, 0xff, 230), // neon cyan ring
};

const DARK_PALETTE: LogoPalette = LogoPalette {
    tile_a: Color32::from_rgb(0x0e, 0x1b, 0x38),
    tile_b: Color32::from_rgb(0x14, 0x2b, 0x52),
    tile_c: Color32::from_rgb(0x1c, 0x3a, 0x6a),
    tile_mag: Color32::from_rgb(0xff, 0x2b, 0xd6),
    back_a: Color32::from_rgb(0x1d, 0x33, 0x4c), // dim, receding cool steel
    back_b: Color32::from_rgb(0x5c, 0x8f, 0xad), // muted cyan steel tip
    front_a: Color32::from_rgb(0x2c, 0x3a, 0x50), // cool gunmetal base
    front_b: Color32::from_rgb(0xf4, 0xf8, 0xfc), // white-hot chrome tip
    lit: Color32::from_rgba_unmultiplied_const(0x8a, 0xe8, 0xff, 170),
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
});

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

/// Load the persisted settings into the global at startup.
fn load_settings() {
    if let Some(s) = load_settings_at(&settings_path()) {
        *SETTINGS.lock().unwrap_or_else(|p| p.into_inner()) = s;
    }
}

fn theme_override() -> ThemeOverride {
    SETTINGS.lock().map(|g| g.appearance).unwrap_or(ThemeOverride::Follow)
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
        back_a: blend_color(a.back_a, b.back_a, t),
        back_b: blend_color(a.back_b, b.back_b, t),
        front_a: blend_color(a.front_a, b.front_a, t),
        front_b: blend_color(a.front_b, b.front_b, t),
        lit: blend_color(a.lit, b.lit, t),
        bezel: blend_color(a.bezel, b.bezel, t),
    }
}

/// Draw the brand lockup: glass tile + heavy chrome " >> " fast-forward
/// chevrons (dim cyan back pair, hero chrome front pair). The tile swaps
/// between frosted white (light mode) and deep navy (dark mode) to follow
/// the Windows theme.
fn draw_logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    draw_logo_at(ui.painter(), rect, 1.0);
}

fn draw_logo_at(p: &egui::Painter, rect: egui::Rect, fade: f32) {
    let w = rect.width();
    let h = rect.height();
    let pal = logo_palette();
    let center = rect.center();
    let radius = 0.44 * w; // circle inscribed in the 6%-inset content box

    // ---- glass tile: a perfect circle filled with the smooth vertical
    //      gradient via a per-vertex mesh (a per-band `rect_filled` would
    //      clamp the corner radius and read as square; the mesh keeps the
    //      circle at every size) ----
    p.add(circle_tile_mesh(rect, radius, |t| alpha(tile_grad(t, pal), fade)));
    // thin neon cyan ring around the tile edge (matches the lit-chrome
    // accent), scaling with size so it stays visible on the small tiles
    p.circle_stroke(center, radius, Stroke::new((0.018 * w).max(1.2), alpha(pal.bezel, fade)));

    // ---- chrome fast-forward chevrons: crisp mitered vector polygons
    //      (geometry mirrors the icon generator — no texture, no glows).
    //      The chevrons are scaled up to fill the circle: the front tip
    //      lands at ~68% of the circle radius. ----
    let ux = |v: f32| rect.left() + (0.06 + 0.88 * v) * w;
    let uy = |t: f32| rect.top() + (0.06 + 0.88 * t) * h;
    let back = chevron_hex(
        egui::pos2(ux(0.0739), uy(0.3466)),
        egui::pos2(ux(0.0739), uy(0.6534)),
        egui::pos2(ux(0.3239), uy(0.5)),
        0.038 * w,
    );
    let front = chevron_hex(
        egui::pos2(ux(0.4830), uy(0.2898)),
        egui::pos2(ux(0.4830), uy(0.7102)),
        egui::pos2(ux(0.8352), uy(0.5)),
        0.075 * w,
    );
    let mut mesh = egui::Mesh::default();
    chrome_hex(&mut mesh, &back, pal.back_a, pal.back_b, fade);
    chrome_hex(&mut mesh, &front, pal.front_a, pal.front_b, fade);
    // subtle cyan lit-chrome edge along the front chevron's upper bars
    chrome_lit(&mut mesh, &front, 0.02 * w, pal.lit, fade);
    p.add(mesh);
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

/// Intersection of the line through `p` along `u` and the line through `q`
/// along `v` (miter-join math).
fn line_intersect(p: egui::Pos2, u: egui::Vec2, q: egui::Pos2, v: egui::Vec2) -> egui::Pos2 {
    let denom = u.x * v.y - u.y * v.x;
    if denom.abs() < 1e-6 {
        return p + u * 0.5;
    }
    let w = q - p;
    let t = (w.x * v.y - w.y * v.x) / denom;
    p + u * t
}

/// One chevron as a mitered hexagon `[am, o, bp, bm, i, ap]` plus the
/// gradient position at the notch and the bar inward normals (for the lit
/// edge). `a` top bar start, `b` bottom bar start, `t` where the
/// centerlines meet, `hw` half bar width. The two bars' flat ends meet at a
/// true mitered tip with no seams — exactly an SVG stroke with
/// `stroke-linejoin="miter"` (mirrors the icon generator).
struct Chevron {
    hex: [egui::Pos2; 6],
    t_i: f32,
    n1: egui::Vec2, // top bar: from its upper edge into the bar
    n2: egui::Vec2, // bottom bar: from its upper edge into the bar
}

fn chevron_hex(a: egui::Pos2, b: egui::Pos2, t: egui::Pos2, hw: f32) -> Chevron {
    let d1 = t - a;
    let d2 = t - b;
    let l1 = d1.length().max(f32::EPSILON);
    let l2 = d2.length().max(f32::EPSILON);
    let u1 = d1 / l1;
    let u2 = d2 / l2;
    let n1 = egui::vec2(-u1.y, u1.x);
    let n2 = egui::vec2(-u2.y, u2.x);
    let ap = a + n1 * hw; // top bar lower (inner) edge start
    let am = a - n1 * hw; // top bar upper (outer) edge start
    let bp = b + n2 * hw; // bottom bar lower (outer) edge start
    let bm = b - n2 * hw; // bottom bar upper (inner) edge start
    let o = line_intersect(am, u1, bp, u2); // tip — the two outer edges meet
    let i = line_intersect(ap, u1, bm, u2); // notch — the two inner edges meet
    let t_i = ((i - a).dot(u1) / l1).clamp(0.0, 1.0);
    Chevron { hex: [am, o, bp, bm, i, ap], t_i, n1, n2 }
}

/// Add one mitered chevron to the mesh: per-vertex gradient along the bars
/// (steel base → bright tip), fan-triangulated from the notch vertex.
fn chrome_hex(mesh: &mut egui::Mesh, ch: &Chevron, base: Color32, tip: Color32, fade: f32) {
    let mid = Color32::from_rgb(
        lerp8(base.r(), tip.r(), ch.t_i),
        lerp8(base.g(), tip.g(), ch.t_i),
        lerp8(base.b(), tip.b(), ch.t_i),
    );
    // vertex colors: [am, o, bp, bm, i, ap]
    let colors = [base, tip, base, base, mid, base];
    let v0 = mesh.vertices.len() as u32;
    for (i, pt) in ch.hex.iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *pt,
            uv: egui::epaint::WHITE_UV,
            color: alpha(colors[i], fade),
        });
    }
    // fan from the notch (index 4): (i, am, o), (i, o, bp), (i, bp, bm), (i, ap, am)
    mesh.indices.extend_from_slice(&[
        v0 + 4, v0, v0 + 1,
        v0 + 4, v0 + 1, v0 + 2,
        v0 + 4, v0 + 2, v0 + 3,
        v0 + 4, v0 + 5, v0,
    ]);
}

/// Subtle lit-chrome edge: a thin cyan strip along the front chevron's two
/// upper bars (the top bar's outer edge am→o and the bottom bar's inner
/// edge bm→o), drawn over the chrome so it reads as light catching the
/// metal. Mirrors the icon generator's per-pixel band.
fn chrome_lit(mesh: &mut egui::Mesh, ch: &Chevron, w: f32, lit: Color32, fade: f32) {
    lit_quad(mesh, ch.hex[0], ch.hex[1], ch.n1, w, lit, fade); // am→o, into the top bar
    lit_quad(mesh, ch.hex[3], ch.hex[1], ch.n2, w, lit, fade); // bm→o, into the bottom bar
}

/// One thin quad strip along the segment p0→p1, offset `w` into the bar
/// along `inward`.
fn lit_quad(mesh: &mut egui::Mesh, p0: egui::Pos2, p1: egui::Pos2, inward: egui::Vec2, w: f32, lit: Color32, fade: f32) {
    let n = inward.normalized() * w;
    let v0 = mesh.vertices.len() as u32;
    for pos in [p0, p1, p1 + n, p0 + n] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::epaint::WHITE_UV,
            color: alpha(lit, fade),
        });
    }
    mesh.indices.extend_from_slice(&[v0, v0 + 1, v0 + 2, v0, v0 + 2, v0 + 3]);
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

// ------------------------------------------------------------- retro widgets
/// Segmented LED progress bar (synthwave hardware look).
fn led_bar(ui: &mut egui::Ui, pct: f32, label: &str, animated: bool) {
    let width = ui.available_width().min(560.0);
    let height = 15.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height + 20.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, Vec2::new(width, height));
    let n = 30usize;
    let gap = 2.5;
    let seg_w = (width - gap * (n as f32 - 1.0)) / n as f32;
    let p = ui.painter();
    let full = (pct * n as f32).round().clamp(0.0, n as f32) as usize;
    let time = ui.input(|i| i.time);
    let head_blink = animated && (time % 0.5 < 0.25);
    for i in 0..n {
        let x0 = bar.left() + i as f32 * (seg_w + gap);
        let seg = egui::Rect::from_min_size(egui::pos2(x0, bar.top()), Vec2::new(seg_w, height));
        let r = CornerRadius::same(3);
        let on = i < full || (animated && i == full && head_blink);
        if on {
            let c = grad_color(i as f32 / (n - 1) as f32);
            p.rect_filled(
                seg.expand2(Vec2::new(1.2, 2.2)),
                r,
                Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 46),
            );
            p.rect_filled(seg, r, c);
            let core = egui::Rect::from_min_size(
                egui::pos2(seg.left(), seg.top() + 3.0),
                Vec2::new(seg_w, seg.height() - 6.0),
            );
            p.rect_filled(core, CornerRadius::same(2), Color32::from_rgba_unmultiplied(255, 255, 255, 110));
        } else {
            p.rect_filled(seg, r, Color32::from_rgba_unmultiplied(0xff, 0x2b, 0xd6, 14));
            p.rect_stroke(seg, r, Stroke::new(0.8, Color32::from_rgba_unmultiplied(0x00, 0xff, 0xf7, 24)), egui::StrokeKind::Inside);
        }
    }
    p.text(
        egui::pos2(rect.left(), bar.bottom() + 11.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::monospace(11.0),
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

/// Neon keycap button: gradient body, beveled edges, glow on hover,
/// physically pushed down while pressed.
fn neon_button(ui: &mut egui::Ui, label: &str, size: f32) -> egui::Response {
    let pad = Vec2::new(22.0, 10.0);
    let galley = ui
        .painter()
        .layout_no_wrap(format!("  {label}  "), egui::FontId::proportional(size), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let shift = if pressed { 1.5 } else { 0.0 };
        let r = rect.translate(Vec2::new(0.0, shift));
        let corner = CornerRadius::same(8);
        // drop shadow
        p.rect_filled(
            r.translate(Vec2::new(0.0, 3.0 - shift)),
            corner,
            Color32::from_rgba_unmultiplied(0, 0, 0, 110),
        );
        // gradient body (cyan → violet → pink)
        let strips = 14usize;
        for i in 0..strips {
            let t0 = i as f32 / strips as f32;
            let t1 = (i + 1) as f32 / strips as f32;
            let mut c = grad_color((t0 + t1) * 0.5);
            if pressed {
                c = dim(c, 0.7);
            }
            let cr = if i == 0 || i == strips - 1 { corner } else { CornerRadius::ZERO };
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(r.left(), r.top() + t0 * r.height() - 0.5),
                    egui::pos2(r.right(), r.top() + t1 * r.height() + 0.5),
                ),
                cr,
                c,
            );
        }
        // bevel: bright top edge, shaded bottom edge
        p.line_segment(
            [egui::pos2(r.left() + 5.0, r.top() + 1.0), egui::pos2(r.right() - 5.0, r.top() + 1.0)],
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, if hovered && !pressed { 170 } else { 85 })),
        );
        p.line_segment(
            [egui::pos2(r.left() + 5.0, r.bottom() - 1.0), egui::pos2(r.right() - 5.0, r.bottom() - 1.0)],
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 0, 0, 130)),
        );
        let outline = if pressed {
            neon_pink()
        } else if hovered {
            neon_cyan()
        } else {
            Color32::from_rgba_unmultiplied(0, 255, 247, 110)
        };
        p.rect_stroke(r, corner, Stroke::new(1.4, outline), egui::StrokeKind::Inside);
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            format!("  {label}  "),
            egui::FontId::proportional(size),
            if pressed { dim(text(), 0.8) } else { Color32::WHITE },
        );
    }
    resp
}

/// Sidebar navigation item with a neon selection bar.
fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let text = RichText::new(label).size(14.0).color(if selected { neon_cyan() } else { text2() });
    let resp = ui.add(
        egui::Button::new(text)
            .fill(if selected { Color32::from_rgba_unmultiplied(0, 255, 247, 12) } else { Color32::TRANSPARENT })
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(6)),
    );
    if selected {
        let r = resp.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(r.left(), r.top() + 3.0), Vec2::new(3.0, r.height() - 6.0)),
            CornerRadius::same(2),
            neon_cyan(),
        );
    }
    resp
}

/// Neon gradient underline under view headings.
fn neon_heading(ui: &mut egui::Ui, title: &str) {
    let resp = ui.label(RichText::new(format!("> {title}")).size(20.0).strong().color(text()));
    let y = resp.rect.bottom() + 3.0;
    let x0 = resp.rect.left();
    let x1 = (x0 + ui.available_width()).min(440.0);
    let p = ui.painter();
    let n = 24usize;
    for i in 0..n {
        let a = x0 + (x1 - x0) * (i as f32 / n as f32);
        let b = x0 + (x1 - x0) * ((i + 1) as f32 / n as f32);
        p.line_segment(
            [egui::pos2(a, y), egui::pos2(b, y)],
            Stroke::new(2.0, grad_color(i as f32 / (n - 1) as f32)),
        );
    }
    ui.add_space(8.0);
}

/// Subtle CRT scanline + neon grid overlay across the whole window.
fn crt_overlay(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt")));
    // Scanlines + neon grid: white/cyan on the dark theme, indigo-tinted on
    // the light theme so the retro texture stays visible on light surfaces.
    let dark = theme_blend() > 0.5;
    let scan = if dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 5)
    } else {
        Color32::from_rgba_unmultiplied(0x2a, 0x1b, 0x4a, 6)
    };
    let grid_c = if dark {
        Color32::from_rgba_unmultiplied(0, 255, 247, 8)
    } else {
        Color32::from_rgba_unmultiplied(0x2a, 0x1b, 0x4a, 5)
    };
    let mut y = screen.top() + 1.0;
    while y < screen.bottom() {
        p.line_segment(
            [egui::pos2(screen.left(), y), egui::pos2(screen.right(), y)],
            Stroke::new(1.0, scan),
        );
        y += 3.0;
    }
    let g = 44.0;
    let mut x = screen.left() + g * 0.5;
    while x < screen.right() {
        p.line_segment(
            [egui::pos2(x, screen.top()), egui::pos2(x, screen.bottom())],
            Stroke::new(1.0, grid_c),
        );
        x += g;
    }
    let mut yy = screen.top() + g * 0.5;
    while yy < screen.bottom() {
        p.line_segment(
            [egui::pos2(screen.left(), yy), egui::pos2(screen.right(), yy)],
            Stroke::new(1.0, grid_c),
        );
        yy += g;
    }
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
            settings_codec: settings_create_codec(),
            settings_level: settings_create_level(),
            settings_block: settings_create_block(),
            settings_block_error: false,
            settings_threads: settings_create_threads(),
            settings_recovery: settings_create_recovery(),
            job: None,
            last_result: None,
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
                    self.view = View::Inspect;
                    return;
                }
                for p in dropped {
                    if !self.create_inputs.contains(&p) {
                        self.create_inputs.push(p);
                    }
                }
                self.refresh_create_output();
                self.view = View::Create;
            }
            View::Extract => {
                if let Some(p) = dropped.first() {
                    if p.is_file() {
                        self.extract_archive = p.display().to_string();
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
            }
            View::Settings => {}
        }
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
                Ok((header, index, _)) => self.inspect_data = Some((header, index)),
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
        let (color, prefix, msg) = match res {
            Ok(m) => (ok(), '✔', m.clone()),
            Err(m) => (err(), '❌', m.clone()),
        };
        egui::Frame::new()
            .fill(surface())
            .stroke(Stroke::new(1.0, color))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("{prefix}  {msg}")).color(color).size(13.0));
            });
        ui.add_space(4.0);
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
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            draw_logo(ui, 34.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.label(grad_text("nextar", 19.0, true));
                ui.label(RichText::new("v0.1.0").size(10.0).color(text3()));
            });
        });
        ui.add_space(18.0);
        let nav = [
            (View::Home, "Home"),
            (View::Create, "Create"),
            (View::Extract, "Extract"),
            (View::Inspect, "Inspect"),
            (View::Repair, "Repair"),
            (View::Settings, "⚙ Settings"),
        ];
        for (view, label) in nav {
            let selected = self.view == view;
            if nav_item(ui, label, selected).clicked() {
                self.view = view;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(8.0);
            ui.label(
                RichText::new("zstd · lzma2 · argon2id\nxchacha20-poly1305 · reed-solomon")
                    .size(10.0)
                    .color(text3()),
            );
            ui.label(RichText::new("100% local · no cloud").size(10.0).color(text3()));
        });
    }

    fn view_home(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(26.0);
            draw_logo(ui, 110.0);
            ui.add_space(10.0);
            ui.label(grad_text("NEXTAR", 30.0, true));
            ui.add_space(2.0);
            ui.label(
                RichText::new("the next-generation archiver — fast, secure, self-healing")
                    .size(14.0)
                    .color(text2()),
            );
            ui.add_space(30.0);
            let card_w = 190.0;
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - card_w * 3.0 - 24.0) * 0.5);
                for (view, title, desc) in [
                    (View::Create, "Create", "compress files & folders"),
                    (View::Extract, "Extract", "restore an archive"),
                    (View::Repair, "Repair", "heal corruption with .nvol"),
                ] {
                    let resp = egui::Frame::new()
                        .fill(surface())
                        .stroke(Stroke::new(1.0, border()))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(card_w - 32.0);
                            ui.label(RichText::new(title).size(17.0).strong().color(text()));
                            ui.add_space(4.0);
                            ui.label(RichText::new(desc).size(12.0).color(text3()));
                        })
                        .response
                        .interact(egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_stroke(
                            resp.rect,
                            CornerRadius::same(14),
                            Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 255, 247, 130)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if resp.clicked() {
                        self.view = view;
                    }
                }
            });
            ui.add_space(30.0);
            ui.label(RichText::new("— or drop files & folders anywhere —").size(12.0).color(text3()));
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
        neon_heading(ui, "Create archive");
        ui.label(RichText::new("Compress files and folders with zstd or lzma2 — optionally encrypted and self-healing.").size(12.5).color(text2()));
        ui.add_space(12.0);

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
            let can_run = !self.busy();
            if can_run && neon_button(ui, "Create archive", 14.0).clicked() {
                self.start_create();
            }
            if ui.button("Clear").clicked() {
                self.create_inputs.clear();
                self.last_result = None;
            }
        });
    }

    fn view_extract(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        neon_heading(ui, "Extract archive");
        ui.label(RichText::new("Restore files, folders, permissions, symlinks and timestamps from a .next archive.").size(12.5).color(text2()));
        ui.add_space(12.0);

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

        if !self.busy() && neon_button(ui, "Extract", 14.0).clicked() {
            self.start_extract();
        }
    }

    fn view_inspect(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        neon_heading(ui, "Inspect archive");
        ui.label(RichText::new("Preview an archive's header, contents and health before extracting.").size(12.5).color(text2()));
        ui.add_space(12.0);

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
                            if neon_button(ui, "Extract here", 12.0).clicked() {
                                self.extract_banner = false;
                                self.start_extract_here();
                            }
                            if ui.small_button("✕").clicked() {
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
            ui.label(RichText::new("Load an archive to see its contents.").color(text3()));
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
            .max_height(ui.available_height() - 30.0 - if preview_open { 210.0 } else { 0.0 })
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
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        ui.label(RichText::new(icon).color(text3()));
                        let label = ui
                            .add(
                                egui::Label::new(
                                    RichText::new(format!("{name}{}", if f.kind == "dir" { "/" } else { "" }))
                                        .color(if selected { accent() } else { text() })
                                        .size(12.5),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text(&f.path);
                        if is_file && label.clicked() {
                            clicked = Some(f.path.clone());
                        }
                        if !size.is_empty() {
                            ui.label(RichText::new(size).color(text3()).size(11.5));
                        }
                        if let Some(t) = &f.link {
                            ui.label(RichText::new(format!("→ {t}")).color(text3()).size(11.5));
                        }
                    });
                }
            });
        if let Some(p) = clicked {
            self.start_preview(&p);
        }

        self.preview_pane(ui, ctx);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Extract this archive…").clicked() {
                self.extract_archive = self.inspect_archive.clone();
                self.view = View::Extract;
            }
        });
    }

    fn view_settings(&mut self, ui: &mut egui::Ui) {
        neon_heading(ui, "Settings");
        ui.label(
            RichText::new("Appearance — follow the Windows theme, or pin the look independent of the OS.")
                .size(12.5)
                .color(text2()),
        );
        ui.add_space(12.0);

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
        neon_heading(ui, "Repair archive");
        ui.label(RichText::new("Heal a corrupted or partially downloaded archive using its Reed-Solomon .nvol recovery volume.").size(12.5).color(text2()));
        ui.add_space(12.0);

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

        if !self.busy() && neon_button(ui, "Repair", 14.0).clicked() {
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

        crt_overlay(&ctx);
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

/// Pack an sRGB [`Color32`] as a Win32 COLORREF (0x00BBGGRR) for DWM.
fn colorref(c: Color32) -> u32 {
    u32::from(c.r()) | (u32::from(c.g()) << 8) | (u32::from(c.b()) << 16)
}

fn repaired_path_for(archive: &Path) -> PathBuf {
    let stem = archive.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = archive.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    archive.with_file_name(format!("{stem}.repaired{ext}"))
}

fn configure_theme(ctx: &egui::Context) {
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
    for w in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        w.bg_fill = surface();
        w.bg_stroke = Stroke::new(1.0, border());
        w.fg_stroke = Stroke::new(1.0, text2());
        w.corner_radius = CornerRadius::same(6);
    }
    style.visuals.widgets.hovered.bg_fill = surface2();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.5, alpha(neon_cyan(), 0.63));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text());
    style.visuals.widgets.active.bg_fill = active();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.5, neon_pink());
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
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
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

        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 130.0) * 0.5);
                draw_logo(ui, 28.0);
                ui.label(RichText::new("nextar").size(16.0).strong());
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
/// The animated synthwave boot scene: chrome logo over a neon sun, a CRT
/// sweep scan, and a boot bar, fading in and back out.
fn draw_splash_scene(ctx: &egui::Context, t: f32) {
    let screen = ctx.content_rect();
    let fade_in = (t / 0.35).clamp(0.0, 1.0);
    let fade_out = ((SPLASH_DURATION - t) / 0.30).clamp(0.0, 1.0);
    let a = (fade_in * fade_out).min(1.0);
    let cx = screen.center().x;

    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("splash")));

    // background void
    p.rect_filled(screen, CornerRadius::ZERO, alpha(bg(), a));

    // synthwave sun behind the logo (glow discs + horizontal slats)
    let sun_c = egui::pos2(cx, screen.top() + screen.height() * 0.30);
    let sun_r = screen.width().min(screen.height()) * 0.30;
    p.circle_filled(sun_c, sun_r, alpha(Color32::from_rgba_unmultiplied(0xff, 0x2b, 0xd6, 16), a));
    p.circle_filled(sun_c, sun_r * 0.90, alpha(Color32::from_rgba_unmultiplied(0x9b, 0x5c, 0xff, 14), a));
    p.circle_filled(sun_c, sun_r * 0.68, alpha(Color32::from_rgba_unmultiplied(0x00, 0xff, 0xf7, 20), a));
    p.circle_filled(sun_c, sun_r * 0.40, alpha(Color32::from_rgba_unmultiplied(255, 255, 255, 26), a));
    let slats = 12usize;
    for i in 0..slats {
        let y = sun_c.y + sun_r * 0.45 + i as f32 * (sun_r * 0.55 / slats as f32);
        let half = sun_r * 0.95 * (1.0 - i as f32 / slats as f32 * 0.75);
        p.line_segment(
            [egui::pos2(cx - half, y), egui::pos2(cx + half, y)],
            Stroke::new(2.0, alpha(bg(), a)),
        );
    }

    // chrome logo + wordmark
    let logo_s = (screen.width().min(screen.height()) * 0.22).clamp(96.0, 170.0);
    let logo_rect = egui::Rect::from_center_size(
        egui::pos2(cx, screen.top() + screen.height() * 0.46),
        Vec2::splat(logo_s),
    );
    draw_logo_at(&p, logo_rect, a);
    p.text(
        egui::pos2(cx, logo_rect.bottom() + 26.0),
        egui::Align2::CENTER_CENTER,
        "NEXTAR",
        egui::FontId::proportional(26.0),
        alpha(text(), a),
    );
    p.text(
        egui::pos2(cx, logo_rect.bottom() + 46.0),
        egui::Align2::CENTER_CENTER,
        "fast · secure · self-healing",
        egui::FontId::proportional(12.0),
        alpha(text2(), a),
    );

    // boot bar filling across the splash duration
    let bar_w = 240.0;
    let bar_y = screen.top() + screen.height() * 0.80;
    let bar = egui::Rect::from_min_size(egui::pos2(cx - bar_w * 0.5, bar_y), Vec2::new(bar_w, 3.0));
    p.rect_filled(bar, CornerRadius::same(2), alpha(Color32::from_rgba_unmultiplied(0, 255, 247, 30), a));
    p.rect_filled(
        egui::Rect::from_min_size(bar.min, Vec2::new(bar_w * (t / SPLASH_DURATION).clamp(0.0, 1.0), 3.0)),
        CornerRadius::same(2),
        alpha(neon_cyan(), a),
    );
    p.text(
        egui::pos2(cx, bar.bottom() + 14.0),
        egui::Align2::CENTER_CENTER,
        "zstd · lzma2 · argon2id · xchacha20-poly1305 · reed-solomon",
        egui::FontId::monospace(10.0),
        alpha(text3(), a),
    );

    // CRT sweep scan line
    let sweep = (t * 0.9) % 1.0;
    let sy = screen.top() + sweep * screen.height();
    p.rect_filled(
        egui::Rect::from_min_size(egui::pos2(screen.left(), sy - 1.0), Vec2::new(screen.width(), 2.0)),
        CornerRadius::ZERO,
        alpha(Color32::from_rgba_unmultiplied(0, 255, 247, 55), a),
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
        crt_overlay(&ctx);
        ctx.request_repaint();
        if self.t >= SPLASH_DURATION || ui.input(|i| i.pointer.any_click()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn main() -> eframe::Result {
    // Load the persisted settings before any window starts, so the splash,
    // shell mode, main UI and the Create-view defaults all apply from the
    // first frame.
    load_settings();
    let args: Vec<String> = std::env::args().skip(1).collect();

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
        assert_eq!(mid.front_b.g(), expect(LIGHT_PALETTE.front_b.g(), DARK_PALETTE.front_b.g()));
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
    fn colorref_packs_bgr() {
        // COLORREF is 0x00BBGGRR — blue in the high byte, red in the low.
        assert_eq!(colorref(Color32::from_rgb(0x11, 0x22, 0x33)), 0x0033_2211);
        assert_eq!(colorref(Color32::from_rgb(0xff, 0x00, 0x00)), 0x0000_00ff);
        assert_eq!(colorref(Color32::from_rgb(0x00, 0x00, 0xff)), 0x00ff_0000);
        // the exact chrome caption color dark-mode surface
        assert_eq!(colorref(UI_DARK.surface), 0x0031_121c);
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
}
