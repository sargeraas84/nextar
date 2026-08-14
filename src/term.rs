//! Terminal styling: brand palette, ANSI helpers, and the gradient logo.
//!
//! Color is enabled by default only when stdout/stderr are terminals; it is
//! disabled by the `NO_COLOR` environment variable or the `--no-color` flag
//! (which calls [`init`]), and forced on by `CLICOLOR_FORCE=1`.

use std::io::IsTerminal;
use std::sync::OnceLock;

// Brand palette (indigo → violet → fuchsia).
pub const C_ACCENT: (u8, u8, u8) = (99, 102, 241); // #6366f1
pub const C_ACCENT2: (u8, u8, u8) = (168, 85, 247); // #a855f7
pub const C_ACCENT3: (u8, u8, u8) = (236, 72, 153); // #ec4899
pub const C_OK: (u8, u8, u8) = (52, 211, 153); // #34d399
pub const C_WARN: (u8, u8, u8) = (251, 191, 36); // #fbbf24
pub const C_ERR: (u8, u8, u8) = (248, 113, 113); // #f87171
pub const C_DIM: (u8, u8, u8) = (120, 127, 140);
pub const C_PATH: (u8, u8, u8) = (96, 165, 250); // #60a5fa

static COLOR: OnceLock<bool> = OnceLock::new();

/// Force-disable color (from `--no-color`). If not called, color is decided
/// lazily from `NO_COLOR` / `CLICOLOR_FORCE` / terminal detection.
pub fn init(no_color: bool) {
    if no_color {
        let _ = COLOR.set(false);
    }
}

/// Whether ANSI color should be emitted.
pub fn color() -> bool {
    *COLOR.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        if std::env::var_os("CLICOLOR_FORCE").is_some() {
            return true;
        }
        std::io::stdout().is_terminal() && std::io::stderr().is_terminal()
    })
}

fn rgb_fg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// Wrap `s` in a truecolor foreground escape (no-op when color is off).
pub fn paint(c: (u8, u8, u8), s: impl std::fmt::Display) -> String {
    if !color() {
        return s.to_string();
    }
    format!("{}{}\x1b[0m", rgb_fg(c), s)
}

pub fn bold(s: impl std::fmt::Display) -> String {
    if !color() {
        return s.to_string();
    }
    format!("\x1b[1m{}\x1b[0m", s)
}

pub fn dim(s: impl std::fmt::Display) -> String {
    paint(C_DIM, s)
}

pub fn ok(s: impl std::fmt::Display) -> String {
    paint(C_OK, s)
}

pub fn err(s: impl std::fmt::Display) -> String {
    paint(C_ERR, s)
}

pub fn warn(s: impl std::fmt::Display) -> String {
    paint(C_WARN, s)
}

pub fn path(s: impl std::fmt::Display) -> String {
    paint(C_PATH, s)
}

/// Brand gradient across the given text, one color stop per character.
pub fn grad(s: impl std::fmt::Display) -> String {
    let s = s.to_string();
    if !color() {
        return s;
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len().max(1);
    let mut out = String::with_capacity(s.len() + 24);
    for (i, c) in chars.iter().enumerate() {
        let t = i as f32 / (n - 1) as f32;
        out.push_str(&paint(stop(t), *c));
    }
    out
}

/// Interpolate along indigo → violet → fuchsia.
fn stop(t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = if t < 0.5 {
        (C_ACCENT, C_ACCENT2)
    } else {
        (C_ACCENT2, C_ACCENT3)
    };
    let u = if t < 0.5 { t * 2.0 } else { (t - 0.5) * 2.0 };
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * u).round() as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * u).round() as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * u).round() as u8,
    )
}

// "NEXTAR" in a small figlet-style face (5 rows × 48 cols).
const LOGO_ROWS: [&str; 5] = [
    " _   _  _____ __  __  _____     _     ____  ",
    "| \\ | | | ____| \\ \\/ /  |_   _|    / \\    |  _ \\ ",
    "|  \\| | |  _|    \\  /     | |     / _ \\   | |_) |",
    "| |\\  | | |___   /  \\     | |    / ___ \\  |  _ < ",
    "|_| \\_| |_____| /_/\\_\\    |_|   /_/   \\_\\ |_| \\_\\",
];

/// The full logo banner: gradient wordmark + tagline. Empty-safe (no ANSI
/// when color is off).
pub fn banner() -> String {
    let mut out = String::new();
    let mut idx = 0usize;
    let mut total = 0usize;
    for row in LOGO_ROWS {
        total += row.chars().filter(|c| *c != ' ').count();
    }
    if color() {
        for row in LOGO_ROWS {
            for c in row.chars() {
                if c == ' ' {
                    out.push(' ');
                } else {
                    let t = idx as f32 / (total.max(1) - 1) as f32;
                    out.push_str(&paint(stop(t), c));
                    idx += 1;
                }
            }
            out.push('\n');
        }
    } else {
        for row in LOGO_ROWS {
            out.push_str(row);
            out.push('\n');
        }
    }
    out.push_str(&dim("  next-generation archiver  ·  zstd + lzma2  ·  argon2id + xchacha20-poly1305  ·  reed-solomon\n"));
    out
}
