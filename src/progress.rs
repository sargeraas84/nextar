//! Progress reporting: a background thread redraws a colored progress bar
//! (percentage + bytes + speed) on stderr every 250 ms while a job runs.
//! Falls back to a plain text line when the terminal can't do ANSI color,
//! and to silence when stderr isn't a terminal or `--quiet` is set.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::term;

/// Shared progress state that a GUI can poll while a job runs. Create it with
/// [`Progress::from_shared`]; the engine sets the real total and updates the
/// counters as the job progresses.
pub struct ProgressState {
    total: AtomicU64,
    label: String,
    done: AtomicU64,
    finished: AtomicBool,
    started: Instant,
}

impl ProgressState {
    pub fn new(total: u64, label: &str) -> Self {
        ProgressState {
            total: AtomicU64::new(total),
            label: label.to_string(),
            done: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            started: Instant::now(),
        }
    }

    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Called by the engine once the real total is known (e.g. archive size
    /// or block count), since the GUI can't know it in advance.
    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn done_bytes(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn pct(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            (self.done.load(Ordering::Relaxed) as f32 / total as f32 * 100.0).min(100.0)
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
}

pub struct Progress {
    total: u64,
    done: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    label: String,
    quiet: bool,
    started: Instant,
    handle: Option<std::thread::JoinHandle<()>>,
    shared: Option<Arc<ProgressState>>,
}

impl Progress {
    pub fn new(total: u64, label: &str, quiet: bool) -> Self {
        let tty = std::io::stderr().is_terminal();
        let quiet = quiet || !tty || total == 0;
        Progress {
            total,
            done: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            label: label.to_string(),
            quiet,
            started: Instant::now(),
            handle: None,
            shared: None,
        }
    }

    /// Build a progress reporter that writes into a shared state object
    /// instead of the terminal — used by the desktop GUI. `total` is the
    /// real job size (bytes or blocks) once known.
    pub fn from_shared(state: Arc<ProgressState>, total: u64) -> Self {
        state.set_total(total);
        Progress {
            total,
            done: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            label: state.label.clone(),
            quiet: true,
            started: Instant::now(),
            handle: None,
            shared: Some(state),
        }
    }

    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
        if let Some(s) = &self.shared {
            s.done.fetch_add(n, Ordering::Relaxed);
        }
    }

    pub fn start(&mut self) {
        if self.quiet {
            return;
        }
        let done = Arc::clone(&self.done);
        let stop = Arc::clone(&self.stop);
        let total = self.total;
        let label = self.label.clone();
        let color = term::color();
        self.handle = Some(std::thread::spawn(move || {
            let mut last = 0u64;
            let mut last_time = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let d = done.load(Ordering::Relaxed);
                let now = Instant::now();
                let dt = now.duration_since(last_time).as_secs_f64();
                let speed = if dt > 0.0 { (d - last) as f64 / dt } else { 0.0 };
                last = d;
                last_time = now;
                let pct = if total > 0 { d as f64 * 100.0 / total as f64 } else { 100.0 };
                if color {
                    let width = 24usize;
                    let filled = ((pct / 100.0) * width as f64).round() as usize;
                    let fill = "█".repeat(filled);
                    let rest = "░".repeat(width - filled);
                    eprint!(
                        "\r\x1b[2K{label}: {} {} {pct:5.1}%  {} / {}  {}/s  ",
                        term::paint(term::C_ACCENT, &fill),
                        rest,
                        human(d),
                        human(total),
                        human(speed as u64)
                    );
                } else {
                    eprint!(
                        "\r{label}: {pct:5.1}%  {} / {}  {}/s  ",
                        human(d),
                        human(total),
                        human(speed as u64)
                    );
                }
            }
        }));
    }

    pub fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if let Some(s) = &self.shared {
            s.finished.store(true, Ordering::Relaxed);
        }
        if self.quiet {
            return;
        }
        let summary = format!(
            "{}  {}: {} in {:.1}s",
            term::ok("✓"),
            self.label,
            human(self.total),
            self.started.elapsed().as_secs_f64()
        );
        if term::color() {
            eprint!("\r\x1b[2K{summary}\n");
        } else {
            eprintln!("\r{summary:<72}");
        }
    }
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
