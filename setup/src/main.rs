//! nextar-setup — professional per-user installer.
//!
//! Double-clicking launches a GUI wizard:
//!
//!   Welcome → License → Destination folder (with Browse…) → Options →
//!   Install → Finished (Launch checkbox + open-install-folder)
//!
//! It embeds the release `nextar.exe` + `nextar-gui.exe`, copies them into
//! the chosen folder (default `%LOCALAPPDATA%\nextar`, no admin needed), and
//! registers per-user Explorer integration under `HKCU\Software\Classes`:
//!
//!   * right-click any file/folder → "Compress to .next" / "Compress to .next and email"
//!   * right-click a folder        → "Extract .next… here" (pick an archive)
//!   * right-click a .next archive → "Open in nextar" / "Extract here" / "Repair with .nvol…"
//!   * double-click a .next archive → opens in nextar-gui (Inspect)
//!
//! Plus a proper uninstall entry (Settings → Apps), optional Start-Menu and
//! desktop shortcuts, and a GUI uninstaller (`--uninstall`).
//!
//! Automation flags (kept for scripts/tests):
//!   --dry-run   print what would be done without changing anything
//!   --quiet     no confirmation dialogs
//!   --prefix    install into <dir> instead of %LOCALAPPDATA%\nextar

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

const VERSION: &str = "0.2.1";
const NEXAR_EXE: &[u8] = include_bytes!("../../target/release/nextar.exe");
const GUI_EXE: &[u8] = include_bytes!("../../target/release/nextar-gui.exe");

/// End-user license agreement shown on the License page.
const LICENSE_TEXT: &str = "\
NEXTAR END-USER LICENSE AGREEMENT

Version 1.0 — August 15, 2026

This End-User License Agreement (\"Agreement\") is a legal agreement between
you (an individual or a single entity) and Michael Rieger, the creator of
nextar (\"nextar\", \"we\", or \"us\") governing your use of the nextar
software, including its installer, documentation, and accompanying
components (the \"Software\").
By installing, copying, or otherwise using the Software, you agree to be
bound by this Agreement. If you do not agree, do not install or use the
Software.

1. License Grant.
Subject to your compliance with this Agreement, nextar grants you a
personal, non-exclusive, non-transferable, revocable, limited license to
install and use the Software on your own devices for personal or commercial
purposes. You may make a reasonable number of backup copies for archival
purposes only.

2. Ownership.
The Software, including its code, user interface, logos, trademarks, and
documentation, is owned by Michael Rieger (creator of nextar) and protected
by copyright and other intellectual property laws. This Agreement grants a
license to use the Software; it does not transfer ownership.

3. Open-Source Components.
The Software incorporates compression, cryptography, and error-correction
libraries distributed under open-source licenses. Those components remain
governed by their respective licenses. Nothing in this Agreement limits
your rights under those licenses.

4. Restrictions.
You may not (a) sell, rent, lease, sublicense, or commercially redistribute
the Software; (b) reverse engineer, decompile, or disassemble the Software,
except as permitted by applicable law; (c) remove or alter any proprietary
notices; or (d) use the Software to process data in violation of applicable
law.

5. Your Data.
The Software processes files only on the devices you choose, and only for
the compression, encryption, extraction, and repair operations you
initiate. It does not upload, transmit, or share your files or passwords
with nextar or any third party. You are responsible for the files you
process, for safeguarding your passwords, and for keeping backups of
important data.

6. Updates.
nextar may provide updates or upgrades, which may replace earlier versions
while preserving your stored preferences. This Agreement applies to all
updates and upgrades unless they are accompanied by separate terms.

7. No Warranty.
THE SOFTWARE IS PROVIDED \"AS IS\" AND \"AS AVAILABLE\", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT. NEXTAR DOES NOT
WARRANT THAT THE SOFTWARE WILL BE ERROR-FREE, UNINTERRUPTED, OR THAT
COMPRESSED OR REPAIRED DATA WILL BE FREE FROM LOSS OR CORRUPTION.

8. Limitation of Liability.
TO THE MAXIMUM EXTENT PERMITTED BY LAW, NEXTAR SHALL NOT BE LIABLE FOR ANY
INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR EXEMPLARY DAMAGES,
INCLUDING LOSS OF DATA, LOSS OF PROFITS, OR BUSINESS INTERRUPTION, ARISING
OUT OF OR RELATED TO THE SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
SUCH DAMAGES. NEXTAR'S TOTAL LIABILITY SHALL NOT EXCEED THE AMOUNT YOU PAID
FOR THE SOFTWARE, IF ANY.

9. Termination.
This license continues until terminated. It terminates automatically if you
breach any term of this Agreement. You may terminate it at any time by
uninstalling the Software and deleting all copies. Sections 2, 5, 7, 8, and
10 survive termination.

10. Governing Law.
This Agreement is governed by the laws of the jurisdiction in which
nextar's licensor is established, without regard to conflict-of-law
principles. Any dispute shall be resolved in the competent courts of that
jurisdiction.

By installing or using the Software, you confirm that you have read,
understood, and agree to be bound by this Agreement.";

// Registry roots.
const CLASSES: &str = "Software\\Classes";
const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
const PROGID: &str = "nextar-gui";
const MENU_COMPRESS: &str = "*\\shell\\NextarCompress";
const MENU_EMAIL: &str = "*\\shell\\NextarEmail"; // compress + attach in the mail client
// `*` covers files only — folders are a separate namespace, so the compress
// verbs are registered again under Directory\shell.
const MENU_COMPRESS_DIR: &str = "Directory\\shell\\NextarCompress";
const MENU_EMAIL_DIR: &str = "Directory\\shell\\NextarEmail";
const MENU_EXTRACT: &str = "SystemFileAssociations\\.next\\shell\\NextarExtract";
const MENU_EXTRACT_INTO: &str = "Directory\\shell\\NextarExtractInto"; // right-click a folder
const MENU_OPEN: &str = "SystemFileAssociations\\.next\\shell\\NextarOpen";
const MENU_REPAIR: &str = "SystemFileAssociations\\.next\\shell\\NextarRepair";

// --------------------------------------------------------------- palette
/// Theme-aware UI palette: deep purple in Windows dark mode, lavender-white
/// in light mode, cross-fading with the same eased tween as the logo tile
/// (mirrors nextar-gui). Getter functions always read the current blend.
#[derive(Clone, Copy)]
struct Palette {
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
    bg2: Color32::from_rgb(0x07, 0x0a, 0x11), // near-black glass void
    surface: Color32::from_rgb(0x11, 0x17, 0x24),
    surface2: Color32::from_rgb(0x19, 0x21, 0x31),
    border: Color32::from_rgb(0x24, 0x2e, 0x42),
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
    bg2: Color32::from_rgb(0xea, 0xee, 0xf5), // cool silver-white
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

// ------------------------------------------------------------ actions
#[derive(Clone)]
enum Root {
    Classes,
    Uninstall,
}

enum Act {
    File { path: PathBuf, data: Vec<u8> },
    Set { root: Root, path: String, name: String, value: String },
    SetDword { root: Root, path: String, name: String, value: u32 },
    Delete { root: Root, path: String },
    /// A `.lnk` shortcut; `args` are fixed arguments (Explorer's Send To
    /// appends the dropped paths after them).
    Shortcut { lnk: PathBuf, target: PathBuf, icon: PathBuf, args: String },
    RemoveFile { path: PathBuf },
}

fn root_label(r: &Root) -> &'static str {
    match r {
        Root::Classes => "HKCU\\Software\\Classes",
        Root::Uninstall => "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    }
}

fn act_short(a: &Act) -> String {
    match a {
        Act::File { path, .. } => format!("write  {}", path.display()),
        Act::Set { root, path, name, value } => {
            if name.is_empty() {
                format!("set    {}\\{} = {}", root_label(root), path, value)
            } else {
                format!("set    {}\\{} [{}] = {}", root_label(root), path, name, value)
            }
        }
        Act::SetDword { root, path, name, value } => {
            format!("set    {}\\{} [{}] = {}", root_label(root), path, name, value)
        }
        Act::Delete { root, path } => format!("delete {}\\{}", root_label(root), path),
        Act::Shortcut { lnk, .. } => format!("shortcut {}", lnk.display()),
        Act::RemoveFile { path } => format!("remove {}", path.display()),
    }
}

/// Open a registry root, creating the key (and parents) if needed.
fn reg_key(hkcu: &RegKey, root: &Root, path: &str, write: bool) -> Result<RegKey> {
    let base_path = match root {
        Root::Classes => CLASSES,
        Root::Uninstall => UNINSTALL,
    };
    // The base key may not exist on a fresh profile (CI runners often lack
    // HKCU\...\Uninstall), so create it (RegCreateKeyEx also creates any
    // missing parents) instead of failing the whole install.
    let flags = KEY_READ | if write { KEY_WRITE } else { 0 };
    let (base, _) = hkcu
        .create_subkey_with_flags(base_path, flags)
        .with_context(|| format!("opening {base_path}"))?;
    if write {
        let (key, _) = base
            .create_subkey(path)
            .with_context(|| format!("creating {base_path}\\{path}"))?;
        Ok(key)
    } else {
        base.open_subkey(path).with_context(|| format!("opening {base_path}\\{path}"))
    }
}

fn create_lnk(lnk: &Path, target: &Path, icon: &Path, args: &str) -> Result<()> {
    let ps = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.IconLocation='{},0';$s.WorkingDirectory='{}';$s.Arguments='{}';$s.Save()",
        lnk.display().to_string().replace('\'', "''"),
        target.display().to_string().replace('\'', "''"),
        icon.display().to_string().replace('\'', "''"),
        target.parent().map(|p| p.display().to_string()).unwrap_or_default().replace('\'', "''"),
        args.replace('\'', "''"),
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .with_context(|| format!("creating shortcut {}", lnk.display()))?;
    Ok(())
}

/// Execute an action list (or print it with `--dry-run`). `cancel` lets the
/// GUI stop the worker between actions; a `Cancelled`-style short circuit
/// keeps a partial install from looking like a full one.
fn run(actions: &[Act], dry: bool, cancel: &AtomicBool) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for a in actions {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled by user");
        }
        if dry {
            println!("  {}", act_short(a));
            continue;
        }
        match a {
            Act::File { path, data } => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, data)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            Act::Set { root, path, name, value } => {
                let key = reg_key(&hkcu, root, path, true)?;
                key.set_value(name, value)?;
            }
            Act::SetDword { root, path, name, value } => {
                let key = reg_key(&hkcu, root, path, true)?;
                key.set_value(name, value)?;
            }
            Act::Delete { root, path } => {
                // Tolerate an already-missing base key: uninstall should not
                // fail when the registry entries are already gone.
                let base_path = match root {
                    Root::Classes => CLASSES,
                    Root::Uninstall => UNINSTALL,
                };
                if let Ok(base) = hkcu.open_subkey_with_flags(base_path, KEY_READ | KEY_WRITE) {
                    let _ = base.delete_subkey_all(path);
                }
            }
            Act::Shortcut { lnk, target, icon, args } => {
                if let Some(parent) = lnk.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                create_lnk(lnk, target, icon, args)?;
            }
            Act::RemoveFile { path } => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Ok(())
}

/// Invalidate Explorer's association/verb cache so newly registered
/// right-click menus appear immediately (and stale ones disappear on
/// uninstall). Without SHCNE_ASSOCCHANGED, Explorer can keep serving an
/// old context-menu cache until it restarts — the classic "I installed it
/// but the right-click option isn't there" bug.
fn notify_shell() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        // windows-sys types the constant u32 but the API takes i32
        // (0x0800_0000 fits, so the cast is lossless).
        SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
    }
    // Flush the icon cache too so verb icons update promptly.
    let _ = Command::new("ie4uinit.exe").args(["-show"]).creation_flags(0x0800_0000).spawn();
}

fn default_install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nextar")
}

/// Read the installed nextar version from the per-user Uninstall key (if
/// any), so the wizard can offer an in-place upgrade instead of a fresh
/// install. Settings and user archives are untouched by upgrades.
fn detect_existing_version() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(format!("{UNINSTALL}\\nextar"))
        .ok()
        .and_then(|k| k.get_value::<String, _>("DisplayVersion").ok())
}

/// Open a folder in Explorer (best effort).
fn open_folder(dir: &Path) {
    let _ = Command::new("explorer.exe").arg(dir).creation_flags(0x0800_0000).spawn();
}

/// Verify the payload landed intact (exact byte size) before reporting a
/// successful install, so a truncated copy never looks like a good install.
fn validate_install(prefix: &Path) -> Result<()> {
    for (name, expected) in [("nextar.exe", NEXAR_EXE.len()), ("nextar-gui.exe", GUI_EXE.len())] {
        let p = prefix.join(name);
        let got = std::fs::metadata(&p)
            .with_context(|| format!("verifying {}", p.display()))?
            .len() as usize;
        if got != expected {
            bail!("verification failed for {name}: expected {expected} bytes, found {got}");
        }
    }
    Ok(())
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

// --------------------------------------------------------- install
#[derive(Clone, Copy)]
pub struct InstallOpts {
    pub context_menu: bool,
    pub association: bool,
    pub start_menu: bool,
    pub desktop: bool,
}

impl Default for InstallOpts {
    fn default() -> Self {
        Self { context_menu: true, association: true, start_menu: true, desktop: false }
    }
}

fn shortcut_paths() -> (PathBuf, PathBuf) {
    // Start-Menu Programs dir + Desktop (both per-user, no admin needed).
    let start_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");
    let desktop = std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Desktop"))
        .unwrap_or_else(|| PathBuf::from("."));
    (start_dir.join("nextar.lnk"), desktop.join("nextar.lnk"))
}

fn install_actions(prefix: &Path, opts: &InstallOpts, no_shell: bool) -> Result<Vec<Act>> {
    let exe = prefix.join("nextar.exe");
    let gui = prefix.join("nextar-gui.exe");
    let setup = prefix.join("nextar-setup.exe");

    let mut acts = Vec::new();

    // 1) payload
    acts.push(Act::File { path: exe.clone(), data: NEXAR_EXE.to_vec() });
    acts.push(Act::File { path: gui.clone(), data: GUI_EXE.to_vec() });
    if let Ok(self_path) = std::env::current_exe() {
        if self_path != setup {
            if let Ok(data) = std::fs::read(&self_path) {
                acts.push(Act::File { path: setup.clone(), data });
            }
        }
    }

    // --no-shell: payload only — skip every registry key, shortcut, and
    // Explorer verb so the installer E2E can exercise the payload lifecycle
    // in isolation without touching global shell state.
    if no_shell {
        return Ok(acts);
    }

    // 2) context menus (all through the GUI's `--run` shell mode: no console flash)
    if opts.context_menu {
        let gui_q = format!("\"{}\"", gui.display());
        let gui_icon = format!("{},0", gui_q);

        acts.push(Act::Set { root: Root::Classes, path: MENU_COMPRESS.to_string(), name: String::new(), value: "Compress to .next".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_COMPRESS}\\command"), name: String::new(), value: format!("{gui_q} --run create \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_COMPRESS}\\Icon"), name: String::new(), value: gui_icon.clone() });

        acts.push(Act::Set { root: Root::Classes, path: MENU_EXTRACT.to_string(), name: String::new(), value: "Extract here".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EXTRACT}\\command"), name: String::new(), value: format!("{gui_q} --run extract --here \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EXTRACT}\\Icon"), name: String::new(), value: gui_icon.clone() });

        // right-click a *folder*: pick a .next archive and extract into it
        acts.push(Act::Set { root: Root::Classes, path: MENU_EXTRACT_INTO.to_string(), name: String::new(), value: "Extract .next… here".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EXTRACT_INTO}\\command"), name: String::new(), value: format!("{gui_q} --run extract-into \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EXTRACT_INTO}\\Icon"), name: String::new(), value: gui_icon.clone() });

        // `*` (above) covers files only — folders get the same compress
        // verbs through Directory\shell so right-clicking a folder offers
        // "Compress to .next" / "… and email" too.
        acts.push(Act::Set { root: Root::Classes, path: MENU_COMPRESS_DIR.to_string(), name: String::new(), value: "Compress to .next".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_COMPRESS_DIR}\\command"), name: String::new(), value: format!("{gui_q} --run create \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_COMPRESS_DIR}\\Icon"), name: String::new(), value: gui_icon.clone() });

        acts.push(Act::Set { root: Root::Classes, path: MENU_EMAIL_DIR.to_string(), name: String::new(), value: "Compress to .next and email".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EMAIL_DIR}\\command"), name: String::new(), value: format!("{gui_q} --run create-email \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EMAIL_DIR}\\Icon"), name: String::new(), value: gui_icon.clone() });

        // right-click any file/folder → compress, then hand the archive to
        // the default mail client (MAPI) with an Explorer fallback
        acts.push(Act::Set { root: Root::Classes, path: MENU_EMAIL.to_string(), name: String::new(), value: "Compress to .next and email".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EMAIL}\\command"), name: String::new(), value: format!("{gui_q} --run create-email \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_EMAIL}\\Icon"), name: String::new(), value: gui_icon.clone() });

        // right-click a .next archive → open it in the GUI (same as double-click)
        acts.push(Act::Set { root: Root::Classes, path: MENU_OPEN.to_string(), name: String::new(), value: "Open in nextar".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_OPEN}\\command"), name: String::new(), value: format!("{gui_q} \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_OPEN}\\Icon"), name: String::new(), value: gui_icon.clone() });

        acts.push(Act::Set { root: Root::Classes, path: MENU_REPAIR.to_string(), name: String::new(), value: "Repair with .nvol…".into() });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_REPAIR}\\command"), name: String::new(), value: format!("{gui_q} --run repair \"%1\"") });
        acts.push(Act::Set { root: Root::Classes, path: format!("{MENU_REPAIR}\\Icon"), name: String::new(), value: gui_icon.clone() });
    }

    // 3) .next file association → open in the GUI (only if not already taken)
    let mut assoc_done = false;
    if opts.association {
        let classes = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(CLASSES, KEY_READ)?;
        let taken = classes
            .open_subkey(".next")
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
            .map(|v| v != PROGID)
            .unwrap_or(false);
        if taken {
            println!("  note: .next is already associated with another app — leaving it;");
            println!("        right-click menus are still registered via SystemFileAssociations.");
        } else {
            let gui_q = format!("\"{}\"", gui.display());
            let gui_icon = format!("{},0", gui_q);
            acts.push(Act::Set { root: Root::Classes, path: ".next".into(), name: String::new(), value: PROGID.into() });
            acts.push(Act::Set { root: Root::Classes, path: PROGID.into(), name: String::new(), value: "nextar archive".into() });
            acts.push(Act::Set { root: Root::Classes, path: format!("{PROGID}\\shell\\open\\command"), name: String::new(), value: format!("{gui_q} \"%1\"") });
            acts.push(Act::Set { root: Root::Classes, path: format!("{PROGID}\\DefaultIcon"), name: String::new(), value: gui_icon });
            assoc_done = true;
        }
    }

    // 4) shortcuts
    if opts.start_menu || opts.desktop {
        let (start_lnk, desktop_lnk) = shortcut_paths();
        if opts.start_menu {
            acts.push(Act::Shortcut { lnk: start_lnk, target: gui.clone(), icon: gui.clone(), args: String::new() });
        }
        if opts.desktop {
            acts.push(Act::Shortcut { lnk: desktop_lnk, target: gui.clone(), icon: gui.clone(), args: String::new() });
        }
    }

    // 4b) Send To: Explorer appends the selected files as extra arguments,
    // so the shortcut targets `--run create` (multi-select friendly).
    if opts.context_menu {
        let sendto = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Microsoft")
            .join("Windows")
            .join("SendTo");
        acts.push(Act::Shortcut {
            lnk: sendto.join("Compress to .next.lnk"),
            target: gui.clone(),
            icon: gui.clone(),
            args: "--run create".into(),
        });
    }

    // 5) uninstall entry
    let exe_q = format!("\"{}\"", exe.display());
    let icon = format!("{},0", exe_q);
    let est_size: u32 = (NEXAR_EXE.len() as u32 + GUI_EXE.len() as u32) / 1024;
    acts.push(Act::SetDword { root: Root::Uninstall, path: "nextar".into(), name: "EstimatedSize".into(), value: est_size });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "DisplayName".into(), value: "nextar".into() });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "DisplayVersion".into(), value: VERSION.into() });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "Publisher".into(), value: "Michael Rieger".into() });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "InstallLocation".into(), value: prefix.display().to_string() });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "DisplayIcon".into(), value: icon.clone() });
    acts.push(Act::Set { root: Root::Uninstall, path: "nextar".into(), name: "UninstallString".into(), value: format!("\"{}\" --uninstall", setup.display()) });
    let _ = assoc_done;

    Ok(acts)
}

fn install(prefix: &Path, dry: bool, quiet: bool, no_shell: bool) -> Result<()> {
    let opts = InstallOpts::default();
    let acts = install_actions(prefix, &opts, no_shell)?;
    println!("nextar setup v{VERSION} — installing to {}", prefix.display());
    if dry {
        println!("  (dry run — nothing will be changed)");
    }
    run(&acts, dry, &AtomicBool::new(false))?;
    if !dry {
        validate_install(prefix)?;
        if !no_shell {
            notify_shell();
        }
    }
    if !dry {
        println!();
        println!("  ✓ nextar.exe + nextar-gui.exe + nextar-setup.exe");
        if !no_shell {
            if opts.context_menu {
                println!("  ✓ right-click any file/folder → “Compress to .next” / “Compress to .next and email”");
                println!("  ✓ right-click a folder → “Extract .next… here”");
                println!("  ✓ right-click a .next archive → “Open in nextar” / “Extract here” / “Repair with .nvol…”");
                println!("  ✓ Send To → “Compress to .next” (multi-select)");
            }
            if opts.association {
                println!("  ✓ double-click a .next archive → opens in nextar-gui");
            }
            println!("  ✓ uninstall entry registered (Settings → Apps → nextar)");
        }
        println!();
        if no_shell {
            println!("done — payload installed (no shell integration; --no-shell).");
        } else {
            println!("done — right-click anything in Explorer to use nextar.");
            println!("remove it anytime with: \"{}\" --uninstall", prefix.join("nextar-setup.exe").display());
        }
        if !quiet {
            let text = format!(
                "nextar is now installed — no admin needed.\n\n\
                 • Right-click any file or folder → \"Compress to .next\" / \"Compress to .next and email\"\n\
                 • Send To → \"Compress to .next\" (works with multiple selections)\n\
                 • Right-click a .next archive → \"Open in nextar\" / \"Extract here\" / \"Repair with .nvol…\"\n\
                 • Double-click a .next archive → opens in nextar-gui\n\n\
                 Installed to:\n{}\n\n\
                 Remove anytime via Settings → Apps → nextar.",
                prefix.display()
            );
            msgbox("nextar installed", &text);
        }
    }
    Ok(())
}

// ------------------------------------------------------------ uninstall
fn uninstall_actions(prefix: &Path, no_shell: bool) -> Vec<Act> {
    // --no-shell: remove only the payload files and leave registry keys and
    // shortcuts untouched (a --no-shell install never created them).
    if no_shell {
        return vec![
            Act::RemoveFile { path: prefix.join("nextar.exe") },
            Act::RemoveFile { path: prefix.join("nextar-gui.exe") },
        ];
    }
    let mut acts = vec![Act::Delete { root: Root::Classes, path: MENU_COMPRESS.to_string() }];
    acts.push(Act::Delete { root: Root::Classes, path: MENU_EMAIL.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_COMPRESS_DIR.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_EMAIL_DIR.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_EXTRACT.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_EXTRACT_INTO.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_OPEN.to_string() });
    acts.push(Act::Delete { root: Root::Classes, path: MENU_REPAIR.to_string() });

    // remove the .next association only if it still points at us
    let classes = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(CLASSES, KEY_READ);
    if let Ok(classes) = classes {
        if let Ok(k) = classes.open_subkey(".next") {
            if let Ok(v) = k.get_value::<String, _>("") {
                if v == PROGID {
                    acts.push(Act::Delete { root: Root::Classes, path: ".next".into() });
                }
            }
        }
    }
    acts.push(Act::Delete { root: Root::Classes, path: PROGID.to_string() });
    acts.push(Act::Delete { root: Root::Uninstall, path: "nextar".into() });

    // shortcuts we may have created (Start Menu, Desktop, Send To)
    let (start_lnk, desktop_lnk) = shortcut_paths();
    acts.push(Act::RemoveFile { path: start_lnk });
    acts.push(Act::RemoveFile { path: desktop_lnk });
    let sendto = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Microsoft")
        .join("Windows")
        .join("SendTo")
        .join("Compress to .next.lnk");
    acts.push(Act::RemoveFile { path: sendto });

    // payload files (leave the running setup exe for the delayed cleanup)
    acts.push(Act::RemoveFile { path: prefix.join("nextar.exe") });
    acts.push(Act::RemoveFile { path: prefix.join("nextar-gui.exe") });
    acts
}

fn uninstall(prefix: &Path, dry: bool, quiet: bool, no_shell: bool) -> Result<()> {
    let acts = uninstall_actions(prefix, no_shell);
    println!("nextar uninstall — removing from {}", prefix.display());
    if dry {
        println!("  (dry run — nothing will be changed)");
    }
    run(&acts, dry, &AtomicBool::new(false))?;

    if !dry {
        if !no_shell {
            notify_shell();
        }
        let self_ok = std::env::current_exe().map(|s| s == prefix.join("nextar-setup.exe")).unwrap_or(false);
        if self_ok {
            // The running exe locks itself; a detached PowerShell removes the
            // whole folder shortly after we exit.
            let dir = prefix.display().to_string().replace('\'', "''");
            let ps = format!(
                "Start-Sleep -Milliseconds 1500; Remove-Item -LiteralPath '{}' -Recurse -Force -ErrorAction SilentlyContinue",
                dir
            );
            match Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps])
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .spawn()
            {
                Ok(_) => println!("  ✓ scheduled folder cleanup"),
                Err(e) => println!("  ! could not schedule folder cleanup: {e}"),
            }
        } else {
            let _ = std::fs::remove_file(prefix.join("nextar-setup.exe"));
            let _ = std::fs::remove_dir(prefix);
        }
        println!();
        if !no_shell {
            println!("  ✓ context menus removed");
            println!("  ✓ file association removed");
            println!("  ✓ uninstall entry removed");
        }
        println!("done — nextar has been removed.");
        if !quiet {
            msgbox("nextar removed", "nextar has been uninstalled.\n\nRight-click menus and the .next file association are gone.");
        }
    }
    Ok(())
}

// ------------------------------------------------------------ native msgbox
fn msgbox(title: &str, text: &str) {
    use std::os::windows::ffi::OsStrExt;
    let wide = |s: &str| {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let t = wide(title);
    let x = wide(text);
    unsafe {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(h: isize, text: *const u16, caption: *const u16, ty: u32) -> i32;
        }
        MessageBoxW(0, x.as_ptr(), t.as_ptr(), 0x40); // MB_ok() | MB_ICONINFORMATION
    }
}

// ====================================================================
// GUI installer wizard
// ====================================================================

fn grad_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = if t < 0.5 { (accent(), accent2()) } else { (accent2(), accent3()) };
    let u = if t < 0.5 { t * 2.0 } else { (t - 0.5) * 2.0 };
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * u).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
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

/// Scale a color's alpha.
fn alpha(c: Color32, a: f32) -> Color32 {
    let a = a.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a).round() as u8)
}

// ------------------------------------------------------------- logo palette
/// Light/dark tile + mark colors for the brand lockup, mirroring
/// nextar-gui. The wizard picks the palette from the Windows apps theme.
/// Clean vector look: smooth glass tile + hairline bezel carrying three
/// converging chevron planes (violet → indigo → cyan) that feed a bright
/// core node with a soft glow.
#[derive(Clone, Copy)]
struct LogoPalette {
    tile_a: Color32,
    tile_b: Color32,
    tile_c: Color32,
    tile_mag: Color32,
    layer_a: Color32,
    layer_b: Color32,
    layer_c: Color32,
    core: Color32,
    glow: Color32, // cyan core glow (with alpha)
    bezel: Color32,
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

/// Windows apps theme, cached briefly (0 = light, 1 = dark).
static THEME_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
const THEME_POLL: Duration = Duration::from_secs(3);

/// User-pinned appearance from the GUI's Settings view (shared
/// `%LOCALAPPDATA%\nextar\settings.json`). The wizard never writes it — it
/// only reads it so the installer matches the app's pinned theme.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeOverride {
    #[default]
    Follow,
    Dark,
    Light,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Settings {
    #[serde(default)]
    appearance: ThemeOverride,
}

/// Read the appearance override pinned in the GUI's settings file, if any.
/// Cheap enough to call on each theme poll (the file is a few bytes).
fn settings_override() -> ThemeOverride {
    let path = match std::env::var("LOCALAPPDATA") {
        Ok(la) => PathBuf::from(la).join("nextar").join("settings.json"),
        Err(_) => return ThemeOverride::Follow,
    };
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<Settings>(&raw)
            .map(|s| s.appearance)
            .unwrap_or(ThemeOverride::Follow),
        Err(_) => ThemeOverride::Follow,
    }
}

fn windows_dark_mode() -> bool {
    // The GUI's Settings view can pin the theme independent of the OS.
    match settings_override() {
        ThemeOverride::Dark => return true,
        ThemeOverride::Light => return false,
        ThemeOverride::Follow => {}
    }
    if let Ok(mut g) = THEME_CACHE.lock() {
        if let Some((at, dark)) = *g {
            if at.elapsed() < THEME_POLL {
                return dark;
            }
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        let dark = match hkcu.open_subkey(path) {
            Ok(k) => k
                .get_value::<u32, _>("AppsUseLightTheme")
                .map(|v| v == 0)
                .unwrap_or(false),
            Err(_) => false,
        };
        *g = Some((Instant::now(), dark));
        dark
    } else {
        false
    }
}

/// User preference cache for the Windows "Animation effects" toggle.
static REDUCED_CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
const REDUCED_POLL: Duration = Duration::from_secs(5);

/// Does the user want fewer animations? Gates the wizard's logo entrance.
/// Read from the Windows "Animation effects" toggle
/// (`SystemDisableAnimations`), with a `NEXTAR_REDUCED_MOTION=1` dev/CI pin.
/// Missing key → animations on (mirrors nextar-gui).
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
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Accessibility") {
        Ok(k) => k
            .get_value::<u32, _>("SystemDisableAnimations")
            .map(|v| v != 0)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Cross-fade between the light and dark tiles when the Windows theme
/// changes, instead of an instant swap (mirrors nextar-gui). `theme_blend()`
/// returns 0.0 (light) → 1.0 (dark); the wizard keeps repainting while it's
/// mid-flight.
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
/// follow the Windows theme (mirrors nextar-gui and the icon generator).
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

/// The wizard's boot moment (mirrors nextar-gui's hero logo): a one-shot
/// fade + zoom entrance, converging data fragments, the planes folding in,
/// an energy pulse and a cyan → violet light sweep. Skips to a static logo
/// when the user has reduced motion enabled (Windows "Animation effects").
fn draw_logo_boot(ui: &mut egui::Ui, size: f32, born: Instant) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    if reduced_motion() {
        draw_logo_at(p, rect, 1.0);
        return;
    }
    let t = born.elapsed().as_secs_f32();
    let enter = smoothstep(t / 0.35, 0.0, 1.0);
    let scale = 0.96 + 0.04 * enter;
    let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size * scale));
    p.circle_filled(rect.center(), rect.width() * 0.58, alpha(neon_cyan(), 0.04));
    p.circle_filled(rect.center(), rect.width() * 0.5, alpha(accent2(), 0.05));
    let pal = logo_palette();
    draw_converging_particles(p, r, t, pal);
    let build = smoothstep((t - 0.25) / 0.45, 0.0, 1.0);
    draw_logo_tile(p, r, enter);
    converge_mark(p, r, pal, enter, build);
    // energy pulse: a ring expands from the core (0.75–1.0 s)
    let pulse_t = ((t - 0.75) / 0.25).clamp(0.0, 1.0);
    if pulse_t > 0.0 && pulse_t < 1.0 {
        let e = smoothstep(pulse_t, 0.0, 1.0);
        let core = egui::pos2(
            r.left() + (0.06 + 0.88 * CORE.0) * r.width(),
            r.top() + (0.06 + 0.88 * CORE.1) * r.height(),
        );
        p.circle_stroke(core, lerpf(0.06, 0.46, e) * r.width(), Stroke::new(1.4, alpha(neon_cyan(), (1.0 - e) * 0.6)));
    }
    // cyan → violet light sweep across the mark (0.9–1.35 s): a wide,
    // soft-edged gradient band with a faint cyan echo trailing behind it,
    // so it reads as light rather than a hard stripe
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
    let ring0 = mesh.vertices.len() as u32;
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
/// Unit coords, y down; planes are drawn outer → inner (violet → cyan).
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

fn dim(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f).round() as u8,
        (c.g() as f32 * f).round() as u8,
        (c.b() as f32 * f).round() as u8,
    )
}

// ---------------------------------------------------------- wizard widgets
/// Small painted vector icons (mirrors nextar-gui) for the wizard's primary
/// actions — no emoji, no image assets.
#[derive(Clone, Copy)]
enum Icon {
    Create,
    ArrowRight,
    Check,
    Trash,
}

fn draw_icon(p: &egui::Painter, rect: egui::Rect, icon: Icon, color: Color32, w: f32) {
    let s = Stroke::new(w, color);
    let c = rect.center();
    match icon {
        Icon::Create => {
            p.line_segment([egui::pos2(rect.left() + 2.0, c.y), egui::pos2(rect.right() - 2.0, c.y)], s);
            p.line_segment([egui::pos2(c.x, rect.top() + 2.0), egui::pos2(c.x, rect.bottom() - 2.0)], s);
        }
        Icon::ArrowRight => {
            p.line_segment([egui::pos2(rect.left() + 1.0, c.y), egui::pos2(rect.right() - 2.0, c.y)], s);
            p.line_segment([egui::pos2(rect.right() - 5.0, c.y - 3.0), egui::pos2(rect.right() - 2.0, c.y)], s);
            p.line_segment([egui::pos2(rect.right() - 5.0, c.y + 3.0), egui::pos2(rect.right() - 2.0, c.y)], s);
        }
        Icon::Check => {
            p.line_segment([egui::pos2(rect.left() + 1.0, c.y), egui::pos2(c.x - 1.0, rect.bottom() - 2.0)], s);
            p.line_segment([egui::pos2(c.x - 1.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 1.0, rect.top() + 2.0)], s);
        }
        Icon::Trash => {
            let top = rect.top() + 1.0;
            let bot = rect.bottom() - 1.0;
            // handle + lid
            p.line_segment([egui::pos2(c.x - 2.0, top + 2.0), egui::pos2(c.x - 2.0, top)], s);
            p.line_segment([egui::pos2(c.x + 2.0, top + 2.0), egui::pos2(c.x + 2.0, top)], s);
            p.line_segment([egui::pos2(rect.left() + 2.0, top + 3.5), egui::pos2(rect.right() - 2.0, top + 3.5)], s);
            // body (trapezoid)
            let pts = [
                egui::pos2(rect.left() + 2.0, top + 5.0),
                egui::pos2(rect.right() - 2.0, top + 5.0),
                egui::pos2(rect.right() - 3.5, bot),
                egui::pos2(rect.left() + 3.5, bot),
            ];
            p.add(egui::Shape::closed_line(pts.to_vec(), s));
        }
    }
}

/// Primary wizard button (mirrors nextar-gui): gradient pill with a painted
/// icon, hover glow, press-down physics and a busy state. `danger` swaps the
/// fill to the error red for the Uninstall action.
fn action_button(ui: &mut egui::Ui, label: &str, icon: Option<Icon>, size: f32, busy: bool, danger: bool) -> egui::Response {
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
            p.circle_filled(egui::pos2(r.left() + 24.0, r.center().y), 4.0, alpha(grad_color(0.2), 0.35 + 0.65 * pulse));
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
        let fill = if danger { err() } else { grad_color(0.14) };
        let body = if pressed { dim(fill, 0.85) } else { fill };
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
            draw_icon(p, icon_r, ic, Color32::from_rgb(0x04, 0x12, 0x1a), 1.8);
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

/// Subtle inset top highlight for glass cards (mirrors nextar-gui).
fn paint_card_top(p: &egui::Painter, rect: egui::Rect, radius: u8, a: f32) {
    let inset = radius as f32 + 3.0;
    p.line_segment(
        [
            egui::pos2(rect.left() + inset, rect.top() + 1.0),
            egui::pos2(rect.right() - inset, rect.top() + 1.0),
        ],
        Stroke::new(1.0, alpha(Color32::from_rgb(255, 255, 255), 0.07 * a)),
    );
}

// ---------------------------------------------------------- titlebar theme
/// Make the wizard's native title bar follow the app palette (immersive
/// dark-mode flag + caption/text/border colors on Win11, which fail
/// gracefully on Win10). Change-detected per window, so the chrome only
/// updates when the palette moves — including the eased cross-fade.
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
/// OS-default square corners there. Applied once per window (keyed by HWND).
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

/// Embed Space Grotesk (the marketing site's face) as the proportional
/// font for regular/semibold/bold, keeping the defaults as fallback.
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
    style.visuals.extreme_bg_color = bg2();
    style.visuals.code_bg_color = surface();
    style.visuals.override_text_color = Some(text());
    style.visuals.selection.bg_fill = accent2();
    style.visuals.selection.stroke = Stroke::new(1.0, neon_cyan());
    style.visuals.hyperlink_color = neon_cyan();
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
    ctx.set_style_of(egui::Theme::Dark, style);
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

enum Page {
    Welcome,
    License,
    Destination,
    Options,
    Installing,
    Uninstall,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Install,
    Uninstall,
}

struct Wizard {
    mode: Mode,
    page: Page,
    prefix: String,
    opts: InstallOpts,
    launch: bool,
    /// License page gate: the user must tick the box to proceed.
    agree: bool,
    /// Version already installed (read from the Uninstall key), if any.
    existing: Option<String>,
    /// Set by the Cancel button while the worker runs.
    cancel: Arc<AtomicBool>,
    /// Timestamp of the window opening — drives the one-shot logo entrance.
    logo_born: Instant,
    // install/uninstall worker
    rx: Option<Receiver<Result<String>>>,
    logs: Vec<String>,
    done: Option<Result<String>>,
    started: bool,
}

impl Wizard {
    fn new(mode: Mode) -> Self {
        // The uninstaller is launched from Settings → Apps via
        // "<install>\nextar-setup.exe" --uninstall, so self-locate from the
        // running exe when no explicit folder was picked (handles custom
        // install locations). Install mode starts at the default folder.
        let prefix = if mode == Mode::Uninstall {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(default_install_dir)
        } else {
            default_install_dir()
        };
        Self {
            mode,
            page: Page::Welcome,
            prefix: prefix.display().to_string(),
            opts: InstallOpts::default(),
            launch: true,
            agree: false,
            existing: if mode == Mode::Install { detect_existing_version() } else { None },
            cancel: Arc::new(AtomicBool::new(false)),
            logo_born: Instant::now(),
            rx: None,
            logs: Vec::new(),
            done: None,
            started: false,
        }
    }

    fn begin(&mut self, tx: Sender<Result<String>>) {
        let prefix = PathBuf::from(self.prefix.trim());
        let opts = self.opts;
        let mode = self.mode;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.done = None;
        self.logs.clear();
        std::thread::spawn(move || {
            let res: Result<String> = (|| {
                match mode {
                    Mode::Install => {
                        let acts = install_actions(&prefix, &opts, false)?;
                        run(&acts, false, &cancel)?;
                        validate_install(&prefix)?;
                        Ok(format!(
                            "nextar installed in {}\n\n{} files · {}",
                            prefix.display(),
                            acts.iter().filter(|a| matches!(a, Act::File { .. })).count(),
                            human((NEXAR_EXE.len() + GUI_EXE.len()) as u64),
                        ))
                    }
                    Mode::Uninstall => {
                        let acts = uninstall_actions(&prefix, false);
                        run(&acts, false, &cancel)?;
                        Ok("nextar has been uninstalled.".to_string())
                    }
                }
            })();
            let _ = tx.send(res);
        });
        self.started = true;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(res) => {
                    self.done = Some(res);
                    self.rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
        }
    }
}

impl eframe::App for Wizard {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Follow the Windows light/dark theme: refresh the UI palette + egui
        // style every frame (the palette is tweened), keep a lazy 3 s poll so
        // the logo tile and surfaces stay in sync, and repaint every frame
        // while a cross-fade is mid-flight (lightweight — otherwise the
        // wizard only repaints on interaction).
        refresh_palette();
        apply_titlebar(frame);
        apply_window_corners(frame);
        configure_theme(&ctx);
        ctx.request_repaint_after(THEME_POLL);
        if theme_transitioning() {
            ctx.request_repaint();
        }
        self.poll(&ctx);

        ui.vertical_centered(|ui| {
            ui.add_space(18.0);
            // horizontal lockup: logo left, wordmark + version right — the
            // same brand pairing as the app's Home hero
            ui.horizontal(|ui| {
                draw_logo_boot(ui, 64.0, self.logo_born);
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.label(grad_text("NEXTAR", 24.0, true));
                    ui.label(RichText::new(format!("setup v{VERSION}")).size(12.0).color(text3()));
                    ui.label(RichText::new("by Michael Rieger").size(10.0).color(text3()));
                });
            });
            ui.add_space(8.0);
        });
        // keep frames flowing while the one-shot boot logo animation plays,
        // then go back to repainting only on interaction
        if self.logo_born.elapsed() < Duration::from_millis(1100) {
            ctx.request_repaint();
        }

        match self.page {
            Page::Welcome => {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("A next-generation archiver: fast · secure · self-healing").size(14.0).color(text2()));
                    ui.add_space(6.0);
                    ui.label(RichText::new("zstd + lzma2 compression · Argon2id + XChaCha20-Poly1305 encryption · Reed-Solomon recovery").size(11.5).color(text3()));
                    if let Some(prev) = &self.existing {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(format!("nextar {prev} is already installed — this will upgrade to v{VERSION}, keeping your settings."))
                                .size(11.5)
                                .color(accent()),
                        );
                    }
                    ui.add_space(22.0);
                    let label = if self.existing.is_some() { "Upgrade nextar" } else { "Install nextar" };
                    if action_button(ui, label, Some(Icon::Create), 15.0, false, false).clicked() {
                        self.page = Page::License;
                    }
                });
            }
            Page::License => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("License agreement").size(15.0).strong());
                    ui.add_space(8.0);
                });
                egui::ScrollArea::vertical().max_height(210.0).show(ui, |ui| {
                    ui.label(RichText::new(LICENSE_TEXT).size(11.5).color(text2()));
                });
                ui.add_space(10.0);
                ui.checkbox(&mut self.agree, "I accept the terms of the license agreement");
                if !self.agree {
                    ui.add_space(2.0);
                    ui.label(RichText::new("Accept the terms to continue.").size(11.0).color(text3()));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("← Back").on_hover_text("Return to the welcome page").clicked() {
                        self.page = Page::Welcome;
                    }
                    if action_button(ui, "Next", Some(Icon::ArrowRight), 13.0, false, false).clicked() && self.agree {
                        self.page = Page::Destination;
                    }
                });
            }
            Page::Destination => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Choose where to install nextar").size(15.0).strong());
                    ui.add_space(10.0);
                });
                ui.horizontal(|ui| {
                    ui.label("Destination");
                    ui.add(egui::TextEdit::singleline(&mut self.prefix).desired_width(360.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            self.prefix = dir.display().to_string();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Per-user install — no administrator needed.").size(11.5).color(text3()));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("← Back").on_hover_text("Return to the license agreement").clicked() {
                        self.page = Page::License;
                    }
                    if action_button(ui, "Next", Some(Icon::ArrowRight), 13.0, false, false).clicked()
                        && !self.prefix.trim().is_empty()
                    {
                        self.page = Page::Options;
                    }
                });
            }
            Page::Options => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Installation options").size(15.0).strong());
                    ui.add_space(10.0);
                });
                let card = egui::Frame::new()
                    .fill(surface())
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(16))
                    .show(ui, |ui| {
                        ui.checkbox(&mut self.opts.context_menu, "Right-click “Compress to .next” / “Extract here” menus");
                        ui.checkbox(&mut self.opts.association, "Open .next archives in nextar-gui on double-click");
                        ui.checkbox(&mut self.opts.start_menu, "Add a Start Menu shortcut");
                        ui.checkbox(&mut self.opts.desktop, "Add a desktop shortcut");
                    });
                paint_card_top(ui.painter(), card.response.rect, 12, 1.0);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("← Back").on_hover_text("Return to the destination folder").clicked() {
                        self.page = Page::Destination;
                    }
                    let install_label = if self.existing.is_some() { "Upgrade" } else { "Install" };
                    if action_button(ui, install_label, Some(Icon::Create), 13.0, false, false).clicked() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.begin(tx);
                        self.rx = Some(rx);
                        self.page = Page::Installing;
                    }
                });
            }
            Page::Installing => {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(if self.mode == Mode::Install { "Installing nextar…" } else { "Removing nextar…" }).size(15.0).strong());
                    ui.add_space(16.0);
                });
                if let Some(done) = &self.done {
                    match done {
                        Ok(msg) => {
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("✔  done").color(ok()).size(14.0).strong());
                                ui.add_space(4.0);
                                ui.label(RichText::new(msg).size(12.0).color(text2()));
                                ui.add_space(12.0);
                            });
                            if self.mode == Mode::Install {
                                ui.checkbox(&mut self.launch, "Launch nextar-gui when finished");
                                ui.add_space(6.0);
                            }
                            ui.vertical_centered(|ui| {
                                ui.horizontal(|ui| {
                                    if self.mode == Mode::Install
                                        && ui.button("Open installation folder")
                                            .on_hover_text("Open the folder in Explorer")
                                            .clicked()
                                    {
                                        open_folder(Path::new(self.prefix.trim()));
                                    }
                                    if action_button(ui, "Finish", Some(Icon::Check), 13.0, false, false).clicked() {
                                        if self.mode == Mode::Install && self.launch {
                                            let gui = PathBuf::from(self.prefix.trim()).join("nextar-gui.exe");
                                            let _ = Command::new(gui).creation_flags(0x0800_0000).spawn();
                                        }
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    }
                                });
                            });
                        }
                        Err(e) => {
                            let cancelled = self.cancel.load(Ordering::Relaxed);
                            ui.vertical_centered(|ui| {
                                if cancelled {
                                    ui.label(RichText::new("Installation cancelled.").size(13.0).strong().color(text2()));
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Run the installer again to complete it.").size(11.5).color(text3()));
                                } else {
                                    ui.label(RichText::new(format!("❌  {e:#}")).color(err()).size(13.0));
                                }
                                ui.add_space(8.0);
                                if ui.button("Close").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        }
                    }
                } else {
                    // indeterminate progress while the worker runs
                    let t = ui.input(|i| i.time) as f32;
                    let phase = (t * 2.0).fract();
                    let bar = egui::ProgressBar::new(phase).fill(accent()).desired_width(340.0);
                    ui.vertical_centered(|ui| {
                        ui.add(bar);
                        ui.add_space(4.0);
                        ui.label(RichText::new("Copying files · registering Explorer integration…").size(11.5).color(text3()));
                    });
                    if self.mode == Mode::Install {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            if ui.button("Cancel").on_hover_text("Stop the installation").clicked() {
                                self.cancel.store(true, Ordering::Relaxed);
                            }
                        });
                    }
                    ctx.request_repaint();
                }
            }
            Page::Uninstall => {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Uninstall nextar").size(15.0).strong());
                    ui.add_space(10.0);
                });
                ui.horizontal(|ui| {
                    ui.label("Installation");
                    ui.add(egui::TextEdit::singleline(&mut self.prefix).desired_width(360.0));
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").on_hover_text("Leave nextar installed").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if action_button(ui, "Uninstall", Some(Icon::Trash), 13.0, false, true).clicked() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.begin(tx);
                        self.rx = Some(rx);
                        self.page = Page::Installing;
                    }
                });
            }
        }
    }
}

// ====================================================================
// entry point
// ====================================================================

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("nextar-setup v{VERSION}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("nextar-setup v{VERSION}");
        println!("usage: nextar-setup [--uninstall] [--prefix <dir>] [--dry-run] [--quiet] [--no-shell]");
        println!("  (no args)   open the GUI installer wizard");
        println!("  --uninstall open the GUI uninstaller");
        println!("  --prefix    install into <dir> instead of %LOCALAPPDATA%\\nextar");
        println!("  --dry-run   print what would be done without changing anything");
        println!("  --quiet     no confirmation dialogs (for scripts)");
        println!("  --no-shell  payload only: no registry keys, shortcuts, or Explorer verbs");
        return Ok(());
    }

    let dry = args.iter().any(|a| a == "--dry-run");
    let uninst = args.iter().any(|a| a == "--uninstall");
    let no_shell = args.iter().any(|a| a == "--no-shell");
    // --no-shell is a scripting flag: it implies no confirmation dialogs.
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q") || no_shell;
    // Normalize forward slashes (e.g. `--prefix "D:/stuff"` from a POSIX
    // shell): the verb commands embed this path, and Explorer refuses to
    // spawn an executable with mixed separators.
    let prefix = arg_value(&args, "--prefix")
        .map(|p| PathBuf::from(p.replace('/', "\\")))
        .unwrap_or_else(default_install_dir);

    // Console mode for scripts/tests: any explicit flag keeps us headless.
    if dry || quiet || no_shell || args.iter().any(|a| a == "--prefix") {
        if prefix.as_os_str().is_empty() {
            bail!("empty install prefix");
        }
        let res = if uninst { uninstall(&prefix, dry, quiet, no_shell) } else { install(&prefix, dry, quiet, no_shell) };
        if let Err(e) = &res {
            if !quiet {
                msgbox("nextar setup", &format!("Something went wrong:\n\n{e:#}\n\nSee the console output for details."));
            }
        }
        return res;
    }

    // GUI wizard (double-click, or --uninstall from Settings).
    let mode = if uninst { Mode::Uninstall } else { Mode::Install };
    let mut wizard = Wizard::new(mode);
    if uninst {
        wizard.page = Page::Uninstall;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nextar setup")
            .with_inner_size([560.0, 480.0])
            .with_min_inner_size([540.0, 460.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "nextar setup",
        options,
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(wizard))
        }),
    )?;
    Ok(())
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
        let same = |a: Palette, b: Palette| {
            a.bg2 == b.bg2 && a.surface == b.surface && a.surface2 == b.surface2 && a.border == b.border
                && a.text == b.text && a.text2 == b.text2 && a.text3 == b.text3 && a.accent == b.accent
                && a.accent2 == b.accent2 && a.accent3 == b.accent3 && a.ok == b.ok && a.err == b.err
                && a.active == b.active
        };
        assert!(same(blend_ui(UI_DARK, UI_LIGHT, 0.0), UI_DARK));
        assert!(same(blend_ui(UI_DARK, UI_LIGHT, 1.0), UI_LIGHT));
        // Readability at the endpoints: light text on dark surfaces, dark
        // text on light surfaces.
        assert!(UI_DARK.text.r() > UI_DARK.surface.r());
        assert!(UI_LIGHT.text.r() < UI_LIGHT.surface.r());
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
    fn no_shell_install_is_payload_only() {
        let acts = install_actions(Path::new("C:\\tmp\\nextar-test"), &InstallOpts::default(), true).unwrap();
        // nextar.exe + nextar-gui.exe + the copied nextar-setup.exe — no
        // registry, shortcut, association, or uninstall-entry actions.
        assert_eq!(acts.len(), 3);
        for a in &acts {
            assert!(
                matches!(a, Act::File { path, .. } if path.to_string_lossy().ends_with(".exe")),
                "expected only payload File actions, got: {}",
                act_short(a)
            );
        }
    }

    #[test]
    fn no_shell_uninstall_is_payload_only() {
        let acts = uninstall_actions(Path::new("C:\\tmp\\nextar-test"), true);
        assert_eq!(acts.len(), 2);
        for a in &acts {
            assert!(
                matches!(a, Act::RemoveFile { path } if path.to_string_lossy().ends_with("nextar.exe") || path.to_string_lossy().ends_with("nextar-gui.exe")),
                "expected only payload RemoveFile actions, got: {}",
                act_short(a)
            );
        }
    }
}
