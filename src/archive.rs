//! High-level archive operations: building the file list from disk,
//! extracting an archive back to disk, listing, verifying, and repairing
//! corrupted archives with recovery volumes.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use walkdir::WalkDir;

use crate::compress;
use crate::crypto::Crypto;
use crate::format::{self, ArchiveHeader, BlockEntry, FileEntry, Index};
use crate::pipeline::{self, CreateOptions, CreateStats, InputFile};
use crate::progress::{Progress, ProgressState};
use crate::recovery;
use crate::term;

// ─────────────────────────── positional I/O ───────────────────────────

/// Positional read (safe for concurrent readers sharing one file handle).
#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    f.read_at(buf, offset)
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;
    f.seek_read(buf, offset)
}

fn read_exact_at(f: &File, buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = read_at(f, &mut buf[filled..], offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        filled += n;
        offset += n as u64;
    }
    Ok(())
}

// ─────────────────────────── header + index ───────────────────────────

pub fn read_head_index(file: &File, path: &Path) -> Result<(ArchiveHeader, Index, Vec<u8>)> {
    let mut hb = [0u8; format::HEADER_LEN as usize];
    read_exact_at(file, &mut hb, 0).with_context(|| format!("reading header of {}", path.display()))?;
    let header = ArchiveHeader::from_bytes(&hb)?;
    let mut ib = vec![0u8; header.index_len as usize];
    read_exact_at(file, &mut ib, header.index_offset).context("reading index block")?;
    let index = format::index_from_bytes(&ib)?;
    Ok((header, index, ib))
}

fn check_password(header: &ArchiveHeader, index: &Index, password: Option<&str>) -> Result<Option<Arc<Crypto>>> {
    if !header.encrypted() {
        return Ok(None);
    }
    let p = password.context("archive is encrypted; provide --password")?;
    let crypto = Arc::new(Crypto::derive(p, &header.salt)?);
    if let Some(vh) = &index.verifier_hex {
        let v = format::unhex(vh)?;
        if !crypto.check_verifier(&v) {
            bail!("wrong password");
        }
    }
    Ok(Some(crypto))
}

// ─────────────────────────── walking inputs ───────────────────────────

pub fn walk_inputs(inputs: &[PathBuf], excludes: &[PathBuf]) -> Result<Vec<InputFile>> {
    let excludes: Vec<PathBuf> = excludes.iter().map(|p| absolutize(p)).collect();
    let mut out: Vec<InputFile> = Vec::new();

    for input in inputs {
        let abs = absolutize(input);
        if !abs.exists() {
            bail!("input does not exist: {}", input.display());
        }
        if abs.is_file() {
            let name = abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| abs.to_string_lossy().into_owned());
            out.push(entry_for(&abs, &name)?);
        } else {
            for entry in WalkDir::new(&abs).follow_links(false).sort_by_file_name() {
                let entry = entry.with_context(|| format!("walking {}", abs.display()))?;
                let path = entry.path();
                if path == abs {
                    continue;
                }
                if excludes.iter().any(|e| e == path) {
                    continue;
                }
                let rel = path.strip_prefix(&abs).unwrap().to_string_lossy().replace('\\', "/");
                let name = format!("{}/{}", abs.file_name().unwrap().to_string_lossy(), rel);
                out.push(entry_for(path, &name)?);
            }
        }
    }

    out.sort_by(|a, b| a.entry.path.cmp(&b.entry.path));
    Ok(out)
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(p)
    }
}

fn entry_for(path: &Path, stored: &str) -> Result<InputFile> {
    let md = fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let (kind, size, link) = if md.file_type().is_symlink() {
        let target = fs::read_link(path)
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default();
        ("symlink".to_string(), 0u64, Some(target))
    } else if md.is_dir() {
        ("dir".to_string(), 0u64, None)
    } else {
        ("file".to_string(), md.len(), None)
    };
    let mode = file_mode(&md);
    let (mt, mtn) = mtime(&md);
    Ok(InputFile {
        entry: FileEntry {
            path: stored.to_string(),
            kind,
            mode,
            size,
            mtime: mt,
            mtime_ns: mtn,
            link,
            blocks: Vec::new(),
        },
        local: path.to_path_buf(),
    })
}

#[cfg(unix)]
fn file_mode(md: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    md.mode()
}

#[cfg(windows)]
fn file_mode(md: &fs::Metadata) -> u32 {
    if md.is_dir() {
        0o755
    } else if md.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn mtime(md: &fs::Metadata) -> (i64, u32) {
    match md.modified() {
        Ok(t) => match t.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => (d.as_secs() as i64, d.subsec_nanos()),
            Err(_) => (0, 0),
        },
        Err(_) => (0, 0),
    }
}

// ─────────────────────────── create ───────────────────────────

pub fn create(inputs: &[PathBuf], out: &Path, mut opts: CreateOptions) -> Result<CreateStats> {
    if inputs.is_empty() {
        bail!("no input paths given");
    }
    if opts.threads == 0 {
        opts.threads = num_cpus::get();
    }
    if opts.parity > 0 && opts.segment_size + opts.parity > recovery::MAX_TOTAL_SHARDS {
        bail!(
            "segment size + parity must be <= {} (got {} + {})",
            recovery::MAX_TOTAL_SHARDS,
            opts.segment_size,
            opts.parity
        );
    }
    let vol_path = if opts.parity > 0 { Some(volume_path_for(out)) } else { None };
    let mut excludes = vec![out.to_path_buf()];
    if let Some(vp) = &vol_path {
        excludes.push(vp.clone());
    }
    let file_list = walk_inputs(inputs, &excludes)?;
    let stats = pipeline::create(file_list, out, vol_path.as_deref(), &opts)?;
    Ok(stats)
}

pub fn volume_path_for(archive: &Path) -> PathBuf {
    let name = archive
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    archive.with_file_name(format!("{name}.nvol"))
}

// ─────────────────────────── extract ───────────────────────────

pub struct ExtractStats {
    pub files: usize,
    pub dirs: usize,
    pub symlinks: usize,
    pub bytes: u64,
}

/// If every entry lives under one shared top-level directory, return it.
///
/// Folder archives store paths as `<folder>/<rel>` (the folder itself is not
/// an entry), while an archive of a single loose file stores just the bare
/// file name — so we require entries *inside* the top level before stripping.
fn common_root_dir(files: &[FileEntry]) -> Option<String> {
    let top = files.first()?.path.split('/').next()?;
    if top.is_empty() {
        return None;
    }
    let all_share = files.iter().all(|f| f.path.split('/').next() == Some(top));
    let has_children = files.iter().any(|f| f.path.len() > top.len());
    (all_share && has_children).then(|| top.to_string())
}

pub fn extract(
    archive_path: &Path,
    out_dir: &Path,
    password: Option<&str>,
    threads: usize,
    quiet: bool,
    strip_root: bool,
    progress: Option<Arc<ProgressState>>,
) -> Result<ExtractStats> {
    let file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let (header, index, _) = read_head_index(&file, archive_path)?;
    let crypto = check_password(&header, &index, password)?;

    let offsets = Arc::new(recovery::block_offsets(&index.blocks));
    let archive = Arc::new(file);
    fs::create_dir_all(out_dir)?;

    let total: u64 = index.files.iter().map(|f| f.size).sum();
    let mut progress = match progress {
        Some(ps) => Progress::from_shared(Arc::clone(&ps), total),
        None => Progress::new(total, "extracting", quiet),
    };
    progress.start();

    // "Extract here" (right-click menu): when the archive holds exactly one
    // top-level directory, drop it so the contents land directly in the
    // <stem>\ folder instead of <stem>\<top>\… (7-Zip "Extract to" behavior).
    let files_owned: Vec<FileEntry> = if strip_root {
        match common_root_dir(&index.files) {
            Some(top) => index
                .files
                .iter()
                .filter_map(|f| {
                    if f.path == top {
                        return None; // the root dir itself
                    }
                    let mut e = f.clone();
                    e.path = f.path.strip_prefix(&format!("{top}/" )).unwrap_or(&f.path).to_string();
                    Some(e)
                })
                .collect(),
            None => index.files.clone(),
        }
    } else {
        index.files.clone()
    };

    let files = Arc::new(files_owned);
    let blocks = Arc::new(index.blocks.clone());
    let salt = header.salt;
    let next = Arc::new(AtomicUsize::new(0));
    let stats_lock = Arc::new(std::sync::Mutex::new((0usize, 0usize, 0usize, 0u64)));
    let n = threads.max(1).min(files.len().max(1));

    let results = std::thread::scope(|s| -> Vec<Result<()>> {
        let mut handles = Vec::new();
        for _ in 0..n {
            let archive = Arc::clone(&archive);
            let offsets = Arc::clone(&offsets);
            let files = Arc::clone(&files);
            let blocks = Arc::clone(&blocks);
            let next = Arc::clone(&next);
            let crypto = crypto.clone();
            let stats_lock = Arc::clone(&stats_lock);
            let progress = &progress;
            handles.push(s.spawn(move || -> Result<()> {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= files.len() {
                        break;
                    }
                    let bytes = extract_one(&files[i], &blocks, &archive, &offsets, out_dir, crypto.as_deref(), &salt, progress)?;
                    let mut st = stats_lock.lock().unwrap();
                    st.3 += bytes;
                    match files[i].kind.as_str() {
                        "dir" => st.1 += 1,
                        "symlink" => st.2 += 1,
                        _ => st.0 += 1,
                    }
                }
                Ok(())
            }));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in results {
        r?;
    }
    progress.finish();

    // Restore directory timestamps last (children must exist first).
    for entry in files.iter().filter(|e| e.kind == "dir") {
        let p = safe_join(out_dir, &entry.path)?;
        restore_mtime(&p, entry);
    }

    let st = stats_lock.lock().unwrap();
    Ok(ExtractStats { files: st.0, dirs: st.1, symlinks: st.2, bytes: st.3 })
}

/// Read one stored file's full contents into memory without extracting the
/// whole archive (the footer index lets us jump straight to its blocks).
/// Used by the GUI's Inspect preview pane.
pub fn read_file_bytes(archive_path: &Path, entry_path: &str, password: Option<&str>) -> Result<Vec<u8>> {
    let file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let (header, index, _) = read_head_index(&file, archive_path)?;
    let crypto = check_password(&header, &index, password)?;
    let entry = index
        .files
        .iter()
        .find(|f| f.path == entry_path)
        .with_context(|| format!("no entry '{entry_path}' in archive"))?;
    if entry.kind != "file" {
        bail!("'{entry_path}' is not a regular file");
    }
    let offsets = recovery::block_offsets(&index.blocks);
    let mut out = Vec::with_capacity(entry.size.min(64 * 1024 * 1024) as usize);
    for &id in &entry.blocks {
        let be = &index.blocks[id as usize];
        let payload = read_block_payload(&file, offsets[id as usize], be)?;
        let plain = match &crypto {
            Some(c) => c.decrypt_block(&header.salt, be.id, be.codec, &payload)?,
            None => payload,
        };
        let data = compress::decompress(be.codec, &plain, be.orig_len as usize)?;
        out.extend_from_slice(&data);
    }
    Ok(out)
}

fn extract_one(
    entry: &FileEntry,
    blocks: &[BlockEntry],
    archive: &File,
    offsets: &[u64],
    out_dir: &Path,
    crypto: Option<&Crypto>,
    salt: &[u8],
    progress: &Progress,
) -> Result<u64> {
    let out_path = safe_join(out_dir, &entry.path)?;
    let mut written = 0u64;
    match entry.kind.as_str() {
        "dir" => {
            fs::create_dir_all(&out_path)?;
        }
        "symlink" => {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Some(target) = &entry.link {
                let _ = fs::remove_file(&out_path);
                let _ = make_symlink(target, &out_path);
            }
        }
        "file" => {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&out_path)?;
            for &id in &entry.blocks {
                let be = &blocks[id as usize];
                let payload = read_block_payload(archive, offsets[id as usize], be)?;
                let plain = match &crypto {
                    Some(c) => c.decrypt_block(salt, be.id, be.codec, &payload)?,
                    None => payload,
                };
                let data = compress::decompress(be.codec, &plain, be.orig_len as usize)?;
                out.write_all(&data)?;
                written += data.len() as u64;
                progress.add(data.len() as u64);
            }
            out.flush()?;
            restore_meta(&out_path, entry);
        }
        other => bail!("unknown entry kind '{other}'"),
    }
    Ok(written)
}

// ─────────────────────────── block reading ───────────────────────────

fn read_block_payload(archive: &File, offset: u64, be: &BlockEntry) -> Result<Vec<u8>> {
    read_block_payload_opt(archive, offset, be)?.ok_or_else(|| anyhow::anyhow!("block {} is corrupt or missing", be.id))
}

/// Read one block's payload, verifying header magic/id and the payload CRC.
/// Returns `Ok(None)` when the block is corrupt or missing (used by repair).
fn read_block_payload_opt(archive: &File, offset: u64, be: &BlockEntry) -> Result<Option<Vec<u8>>> {
    let mut hb = [0u8; format::BLOCK_HEADER_LEN as usize];
    if read_exact_at(archive, &mut hb, offset).is_err() {
        return Ok(None);
    }
    let hdr = match format::BlockHeader::from_bytes(&hb) {
        Ok(h) => h,
        Err(_) => return Ok(None),
    };
    if hdr.block_id != be.id {
        return Ok(None);
    }
    if hdr.stored_len != be.stored_len || hdr.orig_len != be.orig_len {
        return Ok(None);
    }
    let mut payload = vec![0u8; be.stored_len as usize];
    if read_exact_at(archive, &mut payload, offset + format::BLOCK_HEADER_LEN).is_err() {
        return Ok(None);
    }
    if crc32fast::hash(&payload) != be.crc {
        return Ok(None);
    }
    Ok(Some(payload))
}

// ─────────────────────────── list / info ───────────────────────────

pub fn list(archive_path: &Path, long: bool) -> Result<()> {
    let file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let (header, index, _) = read_head_index(&file, archive_path)?;
    println!("{} {}", term::grad("nextar"), term::bold("archive"));
    println!("  {} {}", term::dim("path  "), term::path(archive_path.display().to_string()));
    println!(
        "  {}  codec {} · level {} · block {} · {} · recovery {}{}",
        term::dim("spec  "),
        term::bold(&index.codec),
        term::bold(index.level.to_string()),
        term::bold(human(index.block_size as u64)),
        if index.encrypted {
            term::bold("encrypted".to_string())
        } else {
            term::dim("plaintext".to_string())
        },
        if header.recovery() {
            term::bold(format!("{}/{}", header.segment_size, header.parity))
        } else {
            term::dim("off".to_string())
        },
        if header.recovery() {
            format!(" ({} blocks/segment, {} parity)", header.segment_size, header.parity)
        } else {
            String::new()
        }
    );
    println!("  {}  {} entries · {} data blocks", term::dim("count "), term::bold(index.files.len().to_string()), term::bold(index.blocks.len().to_string()));
    for f in &index.files {
        if long {
            let mode = format!("{:o}", f.mode);
            let size = if f.kind == "file" { f.size.to_string() } else { "-".to_string() };
            println!(
                "{} {}  {}",
                term::dim(&mode),
                term::dim(&size),
                term::path(&f.path)
            );
        } else {
            println!("{}", term::path(&f.path));
        }
    }
    Ok(())
}

pub fn info(archive_path: &Path) -> Result<()> {
    let file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let (header, index, _) = read_head_index(&file, archive_path)?;
    println!("{} (v{})", term::grad("nextar archive"), header.version);
    println!("  {} : {}", term::dim("codec           "), term::bold(format!("{} (level {})", index.codec, index.level)));
    println!("  {} : {}", term::dim("block size      "), term::bold(human(index.block_size as u64)));
    println!(
        "  {} : {}",
        term::dim("encrypted       "),
        if header.encrypted() {
            term::bold("yes".to_string())
        } else {
            term::dim("no".to_string())
        }
    );
    if header.recovery() {
        println!(
            "  {} : {}",
            term::dim("recovery        "),
            term::bold(format!("{} data/segment, {} parity", header.segment_size, header.parity))
        );
    } else {
        println!("  {} : {}", term::dim("recovery        "), term::dim("none"));
    }
    println!(
        "  {} : {}",
        term::dim("files           "),
        term::bold(index.files.iter().filter(|f| f.kind == "file").count().to_string())
    );
    println!(
        "  {} : {}",
        term::dim("dirs            "),
        term::bold(index.files.iter().filter(|f| f.kind == "dir").count().to_string())
    );
    println!(
        "  {} : {}",
        term::dim("symlinks        "),
        term::bold(index.files.iter().filter(|f| f.kind == "symlink").count().to_string())
    );
    println!(
        "  {} : {}",
        term::dim("data blocks     "),
        term::bold(index.blocks.len().to_string())
    );
    let data_bytes: u64 = index.files.iter().filter(|f| f.kind == "file").map(|f| f.size).sum();
    println!("  {} : {}", term::dim("logical size    "), term::bold(human(data_bytes)));
    println!("  {} : {}", term::dim("created by      "), index.created_by);
    Ok(())
}

// ─────────────────────────── verify ───────────────────────────

pub struct VerifyStats {
    pub total: usize,
    pub good: usize,
    pub bad: usize,
}

pub fn verify(
    archive_path: &Path,
    password: Option<&str>,
    quiet: bool,
    progress: Option<Arc<ProgressState>>,
) -> Result<VerifyStats> {
    let file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let (header, index, _) = read_head_index(&file, archive_path)?;
    let crypto = check_password(&header, &index, password)?;
    let salt = header.salt;
    let offsets = recovery::block_offsets(&index.blocks);
    let total = index.blocks.len();

    let mut progress = match progress {
        Some(ps) => Progress::from_shared(Arc::clone(&ps), total as u64),
        None => Progress::new(total as u64, "verifying", quiet),
    };
    progress.start();

    let mut good = 0usize;
    let mut bad = 0usize;
    for (i, be) in index.blocks.iter().enumerate() {
        let ok = match read_block_payload_opt(&file, offsets[i], be) {
            Ok(Some(payload)) => {
                let crc_ok = crc32fast::hash(&payload) == be.crc;
                if crc_ok && header.encrypted() {
                    if let Some(c) = &crypto {
                        c.decrypt_block(&salt, be.id, be.codec, &payload).is_ok()
                    } else {
                        crc_ok
                    }
                } else {
                    crc_ok
                }
            }
            _ => false,
        };
        if ok {
            good += 1;
        } else {
            bad += 1;
            eprintln!("  block {} corrupt", be.id);
        }
        progress.add(1);
    }
    progress.finish();
    Ok(VerifyStats { total, good, bad })
}

// ─────────────────────────── repair ───────────────────────────

pub struct RepairStats {
    pub total_blocks: usize,
    pub repaired: usize,
    pub out_size: u64,
}

pub fn repair(
    archive_path: &Path,
    vol_path: &Path,
    out_path: &Path,
    quiet: bool,
    progress: Option<Arc<ProgressState>>,
) -> Result<RepairStats> {
    let archive_file = File::open(archive_path).with_context(|| format!("opening archive {}", archive_path.display()))?;
    let volume = recovery::read_volume(vol_path)?;
    let (mut header, index, index_bytes) = match read_head_index(&archive_file, archive_path) {
        Ok(x) => x,
        Err(_) => {
            // Archive header/index damaged: fall back to the volume's copies.
            let ib = volume
                .index_copy
                .as_ref()
                .context("archive index unreadable and volume has no index copy")?;
            let idx = format::index_from_bytes(ib).context("volume index copy is also unreadable")?;
            (volume.archive_header, idx, ib.clone())
        }
    };
    recovery::validate_pair(&header, &volume)?;

    let k = volume.k;
    let m = volume.m;
    let offsets = recovery::block_offsets(&index.blocks);
    let total_blocks = index.blocks.len();
    let segs = total_blocks.div_ceil(k);

    let mut out = File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    out.write_all(&header.to_bytes())?;

    let mut progress = match progress {
        Some(ps) => Progress::from_shared(Arc::clone(&ps), total_blocks as u64),
        None => Progress::new(total_blocks as u64, "repairing", quiet),
    };
    progress.start();
    let mut repaired = 0usize;

    for s in 0..segs {
        let start = s * k;
        let end = (start + k).min(total_blocks);
        let seg = volume
            .segments
            .get(&(s as u32))
            .with_context(|| format!("volume is missing segment {s}"))?;
        if seg.data_count != end - start {
            bail!("volume data count for segment {s} does not match archive");
        }

        let mut lanes: Vec<Option<Vec<u8>>> = Vec::with_capacity(end - start);
        for (rel, id) in (start..end).enumerate() {
            let off = offsets[start + rel];
            match read_block_payload_opt(&archive_file, off, &index.blocks[id]) {
                Ok(Some(p)) => lanes.push(Some(p)),
                _ => {
                    lanes.push(None);
                    repaired += 1;
                }
            }
        }

        let rebuilt = recovery::reconstruct_segment(k, m, lanes.clone(), &seg.parity)?;
        for i in 0..(end - start) {
            let be = index.blocks[start + i];
            let payload = match (&lanes[i], &rebuilt[i]) {
                (Some(p), _) => p.clone(),
                (None, Some(p)) => p.clone(),
                (None, None) => unreachable!("reconstruct fills every lane"),
            };
            let payload = &payload[..be.stored_len as usize];
            if crc32fast::hash(payload) != be.crc {
                bail!(
                    "rebuilt block {} still fails its checksum (parity inconsistent?)",
                    be.id
                );
            }
            let mut flags = 0u8;
            if header.encrypted() {
                flags |= format::BLOCK_FLAG_ENCRYPTED;
            }
            let hdr = format::BlockHeader {
                flags,
                codec: be.codec,
                block_id: be.id,
                orig_len: be.orig_len,
                stored_len: be.stored_len,
                crc: be.crc,
            };
            out.write_all(&hdr.to_bytes())?;
            out.write_all(payload)?;
        }
        progress.add((end - start) as u64);
    }

    // Write the index copy + patch the header.
    let index_offset = out.stream_position()?;
    out.write_all(&index_bytes)?;
    header.index_offset = index_offset;
    header.index_len = index_bytes.len() as u64;
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&header.to_bytes())?;
    out.flush()?;
    progress.finish();

    Ok(RepairStats { total_blocks, repaired, out_size: out.metadata()?.len() })
}

// ─────────────────────────── path & meta helpers ───────────────────────────

fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        bail!("unsafe path in archive: {rel}");
    }
    Ok(base.join(p))
}

fn restore_meta(path: &Path, entry: &FileEntry) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(entry.mode));
    }
    #[cfg(windows)]
    {
        // Best effort: map read-only permission bits to the read-only attribute.
        if entry.mode & 0o222 == 0 {
            let _ = set_readonly(path, true);
        }
    }
    restore_mtime(path, entry);
}

fn restore_mtime(path: &Path, entry: &FileEntry) {
    if let Some(t) = SystemTime::UNIX_EPOCH.checked_add(Duration::new(entry.mtime.max(0) as u64, entry.mtime_ns)) {
        // Directories can't be opened as files on Windows; ignore failures.
        if let Ok(f) = File::open(path) {
            let _ = f.set_modified(t);
        }
    }
}

#[cfg(windows)]
fn set_readonly(path: &Path, ro: bool) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let f = fs::OpenOptions::new()
        .write(true)
        .custom_flags(0x02000000 /* FILE_FLAG_BACKUP_SEMANTICS */)
        .open(path)?;
    let md = f.metadata()?;
    let mut perms = md.permissions();
    perms.set_readonly(ro);
    fs::set_permissions(path, perms)
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
        .or_else(|_| std::os::windows::fs::symlink_file(target, link))
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
