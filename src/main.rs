//! `nextar` — a next-generation archiver.
//!
//! * Zstd "fast" tier and LZMA2 "ultra" tier compression
//! * Argon2id + XChaCha20-Poly1305 authenticated encryption
//! * Reed-Solomon recovery volumes that heal corrupted archives
//! * Fully parallel read → compress → encrypt → write pipeline

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use nextar::{archive, format, pipeline, term};
use nextar::format::codec_from_name;
use nextar::recovery::DEFAULT_SEGMENT_SIZE;

#[derive(Parser)]
#[command(
    name = "nextar",
    version,
    about = "A next-generation archiver: fast, secure, self-healing",
    long_about = "nextar — a next-generation archiver that aims to outperform WinRAR and 7-Zip.\n\
                  Zstd (fast) and LZMA2 (ultra) compression, Argon2id + XChaCha20-Poly1305\n\
                  encryption, Reed-Solomon recovery volumes, fully multi-threaded."
)]
struct Cli {
    /// Disable colored output (also honors the NO_COLOR env var)
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an archive (alias: a)
    #[command(alias = "a")]
    Create(CreateArgs),
    /// Extract an archive (alias: x)
    #[command(alias = "x")]
    Extract(ExtractArgs),
    /// List archive contents (alias: l)
    #[command(alias = "l")]
    List(ListArgs),
    /// Show archive header information
    Info(InfoArgs),
    /// Verify every block's checksum (and AEAD, with a password)
    Verify(VerifyArgs),
    /// Repair a corrupted archive using its recovery volume
    Repair(RepairArgs),
}

#[derive(clap::Args)]
struct CreateArgs {
    /// Files and/or directories to archive (multiple allowed)
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Output archive path (default: <input>.next)
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// Compression codec: zstd (fast) | lzma2 (ultra) | store
    #[arg(short = 'c', long, default_value = "zstd")]
    codec: String,
    /// Compression level (zstd: 1-22, lzma2: 0-9)
    #[arg(short = 'l', long)]
    level: Option<i32>,
    /// Encrypt with this password (Argon2id + XChaCha20-Poly1305)
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// Chunk (block) size in bytes; suffixes K/M/G allowed
    #[arg(short = 'b', long, default_value = "1M")]
    block_size: String,
    /// Parity blocks per Reed-Solomon segment (0 = no recovery volumes)
    #[arg(short = 'r', long, default_value_t = 0)]
    recovery: u16,
    /// Data blocks per RS segment (default 128; recovery only)
    #[arg(short = 's', long, default_value_t = DEFAULT_SEGMENT_SIZE as u16)]
    segment_size: u16,
    /// Worker threads (default: all cores)
    #[arg(short = 't', long)]
    threads: Option<usize>,
    /// Overwrite the output archive if it exists
    #[arg(short = 'f', long)]
    force: bool,
    /// Suppress progress output
    #[arg(short = 'q', long)]
    quiet: bool,
}

#[derive(clap::Args)]
struct ExtractArgs {
    /// Archive to extract
    archive: PathBuf,
    /// Output directory (default: current directory)
    #[arg(short = 'o', long, default_value = ".")]
    output: PathBuf,
    /// Extract into a folder named after the archive, next to it
    /// (used by the Explorer right-click menu)
    #[arg(long)]
    here: bool,
    /// Password (if the archive is encrypted)
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// Worker threads
    #[arg(short = 't', long)]
    threads: Option<usize>,
    /// Suppress progress output
    #[arg(short = 'q', long)]
    quiet: bool,
}

#[derive(clap::Args)]
struct ListArgs {
    /// Archive to list
    archive: PathBuf,
    /// Long format (mode, size, path)
    #[arg(short = 'l', long)]
    long: bool,
}

#[derive(clap::Args)]
struct InfoArgs {
    /// Archive to inspect
    archive: PathBuf,
}

#[derive(clap::Args)]
struct VerifyArgs {
    /// Archive to verify
    archive: PathBuf,
    /// Password (also authenticates every block with the AEAD)
    #[arg(short = 'p', long)]
    password: Option<String>,
    /// Suppress progress output
    #[arg(short = 'q', long)]
    quiet: bool,
}

#[derive(clap::Args)]
struct RepairArgs {
    /// Corrupted archive to repair
    archive: PathBuf,
    /// Recovery volume file (default: <archive>.nvol)
    #[arg(long)]
    volumes: Option<PathBuf>,
    /// Output archive path (default: <archive>.repaired)
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// Overwrite the output archive if it exists
    #[arg(short = 'f', long)]
    force: bool,
    /// Suppress progress output
    #[arg(short = 'q', long)]
    quiet: bool,
}

fn main() {
    // Show the logo banner above help/version output (and for a bare `nextar`).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let wants_banner = raw.is_empty()
        || raw
            .iter()
            .any(|a| matches!(a.as_str(), "-h" | "--help" | "-V" | "--version"));
    let no_color = raw.iter().any(|a| a == "--no-color");
    term::init(no_color);
    if wants_banner {
        print!("{}", term::banner());
    }

    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("{}", term::err(format!("nextar: error: {e:#}")));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Command::Create(a) => cmd_create(a),
        Command::Extract(a) => cmd_extract(a),
        Command::List(a) => cmd_list(a),
        Command::Info(a) => cmd_info(a),
        Command::Verify(a) => cmd_verify(a),
        Command::Repair(a) => cmd_repair(a),
    }
}

fn cmd_create(a: CreateArgs) -> Result<()> {
    let codec = codec_from_name(&a.codec)?;
    let level = a.level.unwrap_or(match codec {
        format::CODE_ZSTD => 3,
        format::CODE_LZMA2 => 6,
        _ => 0,
    });
    let block_size = parse_size(&a.block_size)?;
    if !(512..=64 * 1024 * 1024).contains(&block_size) {
        bail!("block size must be between 512 and 64 MiB (got {block_size})");
    }
    let output = match &a.output {
        Some(o) => o.clone(),
        None => {
            if let Some(first) = a.inputs.first() {
                // 7-Zip style: named after the first selected item, sitting
                // next to it, even when multiple items are selected.
                let mut p = first.clone();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                p.set_file_name(format!("{name}.next"));
                p
            } else {
                PathBuf::from("archive.next")
            }
        }
    };
    if output.exists() && !a.force {
        bail!("{} already exists (use --force to overwrite)", output.display());
    }
    let threads = a.threads.unwrap_or_else(num_cpus::get);
    let opts = pipeline::CreateOptions {
        codec,
        level,
        block_size,
        password: a.password,
        threads,
        segment_size: a.segment_size as usize,
        parity: a.recovery as usize,
        quiet: a.quiet,
        progress: None,
    };
    let started = std::time::Instant::now();
    let stats = archive::create(&a.inputs, &output, opts)?;
    let elapsed = started.elapsed().as_secs_f64();
    println!("{} {}", term::ok("✓"), term::bold("archive created"));
    let inputs_str = a
        .inputs
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" + ");
    println!("  {}  {}", term::dim("archive"), term::path(inputs_str));
    println!("  {}  {}", term::dim("output "), term::bold(output.display().to_string()));
    println!(
        "  {}  {}  {}  {}  {}  {:.2}×",
        term::dim("input  "),
        term::bold(human(stats.total_bytes_read)),
        term::dim("out"),
        term::bold(human(stats.archive_size)),
        term::dim("ratio"),
        stats.total_bytes_read as f64 / stats.archive_size.max(1) as f64
    );
    if stats.volume_size > 0 {
        println!(
            "  {}  {}  ({} blocks, {} parity/segment)  ·  {:.2}s",
            term::dim("volume "),
            term::bold(human(stats.volume_size)),
            stats.block_count,
            a.recovery,
            elapsed
        );
    } else {
        println!("  {}  {:.2}s", term::dim("time   "), elapsed);
    }
    Ok(())
}

fn cmd_extract(a: ExtractArgs) -> Result<()> {
    if !a.archive.exists() {
        bail!("archive not found: {}", a.archive.display());
    }
    let out_dir = if a.here {
        let parent = a.archive.parent().unwrap_or(std::path::Path::new("."));
        let stem = a
            .archive
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extracted".to_string());
        parent.join(stem)
    } else {
        a.output.clone()
    };
    let threads = a.threads.unwrap_or_else(num_cpus::get);
    let started = std::time::Instant::now();
    let stats = archive::extract(&a.archive, &out_dir, a.password.as_deref(), threads, a.quiet, a.here, None)?;
    println!(
        "{}  {} files · {} dirs · {} symlinks ({} bytes)",
        term::ok("✓ extracted"),
        term::bold(stats.files.to_string()),
        term::bold(stats.dirs.to_string()),
        term::bold(stats.symlinks.to_string()),
        term::bold(human(stats.bytes))
    );
    println!(
        "  {}  {}  ·  {} {:.2}s",
        term::dim("output"),
        term::path(out_dir.display().to_string()),
        term::dim("time"),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn cmd_list(a: ListArgs) -> Result<()> {
    archive::list(&a.archive, a.long)
}

fn cmd_info(a: InfoArgs) -> Result<()> {
    archive::info(&a.archive)
}

fn cmd_verify(a: VerifyArgs) -> Result<()> {
    let stats = archive::verify(&a.archive, a.password.as_deref(), a.quiet, None)?;
    if stats.bad > 0 {
        bail!(
            "verification failed: {} of {} blocks corrupt (use `nextar repair` with recovery volumes)",
            stats.bad,
            stats.total
        );
    }
    println!(
        "{}  verified {} of {} blocks — all ok",
        term::ok("✓"),
        term::bold(stats.good.to_string()),
        stats.total
    );
    Ok(())
}

fn cmd_repair(a: RepairArgs) -> Result<()> {
    let volumes = a.volumes.unwrap_or_else(|| archive::volume_path_for(&a.archive));
    if !volumes.exists() {
        bail!("recovery volume not found: {} (create archives with -r N to generate volumes)", volumes.display());
    }
    if !a.archive.exists() {
        bail!("archive not found: {}", a.archive.display());
    }
    let output = match &a.output {
        Some(o) => o.clone(),
        None => repaired_path_for(&a.archive),
    };
    if output.exists() && !a.force {
        bail!("{} already exists (use --force to overwrite)", output.display());
    }
    let started = std::time::Instant::now();
    let stats = archive::repair(&a.archive, &volumes, &output, a.quiet, None)?;
    println!(
        "{}  repaired {} of {} blocks → {}  ({} bytes) in {:.2}s",
        term::ok("✓"),
        term::bold(stats.repaired.to_string()),
        stats.total_blocks,
        term::path(output.display().to_string()),
        human(stats.out_size),
        started.elapsed().as_secs_f64()
    );
    if stats.repaired > 0 {
        println!("  {}  run `nextar verify {}` to confirm", term::warn("note:"), output.display());
    }
    Ok(())
}

/// `<name>.next` → `<name>.repaired.next`
fn repaired_path_for(archive: &PathBuf) -> PathBuf {
    let stem = archive.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = archive.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    archive.with_file_name(format!("{stem}.repaired{ext}"))
}

/// Parse sizes like "512", "1M", "256K", "2G" (binary suffixes).
fn parse_size(s: &str) -> Result<u32> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('k') | Some('K') => (&s[..s.len() - 1], 1024u64),
        Some('m') | Some('M') => (&s[..s.len() - 1], 1024u64 * 1024),
        Some('g') | Some('G') => (&s[..s.len() - 1], 1024u64 * 1024 * 1024),
        _ => (s, 1u64),
    };
    let n: u64 = num
        .trim()
        .parse()
        .with_context(|| format!("invalid size '{s}'"))?;
    let v = n * mult;
    if v > u32::MAX as u64 {
        bail!("size too large: {s}");
    }
    Ok(v as u32)
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
