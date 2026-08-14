//! The parallel **create** pipeline.
//!
//! ```text
//! ┌──────────┐   jobs    ┌─────────────────────┐   blocks    ┌──────────┐
//! │ producer │ ────────► │ N worker threads     │ ──────────► │ writer   │
//! │ (reads   │  bounded  │ compress → encrypt → │   bounded   │ (ordered │
//! │  files)  │  channel  │ crc                  │   channel   │  write)  │
//! └──────────┘           └─────────────────────┘             └──────────┘
//! ```
//!
//! * The **producer** reads file chunks and hands them to a bounded job queue.
//! * **Workers** (one per core) compress with zstd/lzma2, fall back to
//!   storing incompressible chunks raw, then encrypt (XChaCha20-Poly1305)
//!   and checksum each chunk.
//! * The **writer** consumes blocks out of order and writes them in block-id
//!   order. When recovery is enabled it groups blocks into RS segments,
//!   computes parity into the volume file, then writes the segment.
//!
//! Bounded channels give natural backpressure, so memory stays ~constant
//! regardless of input size.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use crossbeam_channel::Receiver;

use crate::compress;
use crate::crypto::Crypto;
use crate::format::{
    self, codec_name, ArchiveHeader, BlockEntry, BlockHeader, FileEntry, Index, CODE_STORE,
};
use crate::progress::{Progress, ProgressState};
use crate::recovery::{self, VolumeWriter};

pub struct CreateOptions {
    pub codec: u8,
    pub level: i32,
    pub block_size: u32,
    pub password: Option<String>,
    pub threads: usize,
    pub segment_size: usize,
    pub parity: usize,
    pub quiet: bool,
    /// Optional shared progress state (used by the GUI to draw a progress bar).
    pub progress: Option<std::sync::Arc<ProgressState>>,
}

pub struct CreateStats {
    pub total_bytes_read: u64,
    pub archive_size: u64,
    pub volume_size: u64,
    pub block_count: usize,
}

/// One input file: metadata entry (stored path) + local path to read from.
pub struct InputFile {
    pub entry: FileEntry,
    pub local: std::path::PathBuf,
}

struct ChunkJob {
    block_id: u64,
    data: Vec<u8>,
}

struct StoredBlock {
    block_id: u64,
    codec: u8,
    orig_len: u32,
    payload: Vec<u8>,
    crc: u32,
    encrypted: bool,
}

pub fn create(
    inputs: Vec<InputFile>,
    out_path: &Path,
    vol_path: Option<&Path>,
    opts: &CreateOptions,
) -> Result<CreateStats> {
    let block_size = opts.block_size as usize;
    let threads = opts.threads.max(1);

    // Salt + header first so the crypto keys (and every block nonce) derive
    // from a single random value.
    let mut header = ArchiveHeader::new(
        opts.codec,
        opts.level.clamp(0, 255) as u8,
        opts.block_size,
        opts.segment_size as u16,
        opts.parity as u16,
        opts.password.is_some(),
    );
    let salt = header.salt;
    let crypto: Option<Arc<Crypto>> = match &opts.password {
        Some(p) => Some(Arc::new(Crypto::derive(p, &salt)?)),
        None => None,
    };

    let total_bytes: u64 = inputs.iter().map(|i| i.entry.size).sum();
    let mut progress = match &opts.progress {
        Some(ps) => Progress::from_shared(std::sync::Arc::clone(ps), total_bytes),
        None => Progress::new(total_bytes, "archiving", opts.quiet),
    };
    progress.start();

    let mut out = File::create(out_path).with_context(|| format!("creating archive {}", out_path.display()))?;
    out.write_all(&header.to_bytes())?;

    let mut vol = if let Some(vp) = vol_path {
        let mut w = VolumeWriter::create(vp, opts.segment_size, opts.parity)?;
        w.set_header(&header)?;
        Some(w)
    } else {
        None
    };

    let (job_tx, job_rx) = crossbeam_channel::bounded(threads * 2);
    let (res_tx, res_rx) = crossbeam_channel::bounded(threads * 2);
    // Only the producer's sender keeps the job channel open; the original is
    // dropped here so workers see end-of-stream when the producer finishes.
    let prod_tx = job_tx.clone();
    drop(job_tx);

    let entries: Vec<FileEntry> = inputs.iter().map(|i| i.entry.clone()).collect();
    let locals: Vec<std::path::PathBuf> = inputs.iter().map(|i| i.local.clone()).collect();

    let err: Arc<Mutex<Option<anyhow::Error>>> = Arc::new(Mutex::new(None));
    let produced = Arc::new(AtomicU64::new(0));
    let block_lists: Arc<Mutex<Vec<Vec<u64>>>> = Arc::new(Mutex::new(Vec::new()));
    let block_table: Arc<Mutex<Vec<BlockEntry>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        // Producer: read files into chunks, assign block ids in order.
        let job_tx = prod_tx;
        let prod_err = Arc::clone(&err);
        let produced = Arc::clone(&produced);
        let block_lists = Arc::clone(&block_lists);
        let progress = &progress;
        let entries_ref = &entries;
        s.spawn(move || {
            let r = (|| -> Result<()> {
                let mut next_id = 0u64;
                let mut lists = Vec::with_capacity(entries_ref.len());
                for (fi, entry) in entries_ref.iter().enumerate() {
                    let mut list = Vec::new();
                    if entry.kind == "file" && entry.size > 0 {
                        let mut file =
                            File::open(&locals[fi]).with_context(|| format!("opening {}", locals[fi].display()))?;
                        let mut buf = vec![0u8; block_size];
                        loop {
                            let n = file.read(&mut buf).with_context(|| format!("reading {}", locals[fi].display()))?;
                            if n == 0 {
                                break;
                            }
                            buf.truncate(n);
                            job_tx
                                .send(ChunkJob { block_id: next_id, data: buf })
                                .context("job queue closed")?;
                            list.push(next_id);
                            next_id += 1;
                            progress.add(n as u64);
                            buf = vec![0u8; block_size];
                        }
                    }
                    lists.push(list);
                }
                *block_lists.lock().unwrap() = lists;
                produced.store(next_id, Ordering::SeqCst);
                Ok(())
            })();
            drop(job_tx);
            if let Err(e) = r {
                *prod_err.lock().unwrap() = Some(e);
            }
        });

        // Workers: compress -> encrypt -> checksum.
        for _ in 0..threads {
            let job_rx = job_rx.clone();
            let res_tx = res_tx.clone();
            let crypto = crypto.clone();
            let err = Arc::clone(&err);
            s.spawn(move || {
                let r = (|| -> Result<()> {
                    for job in &job_rx {
                        let orig_len = job.data.len() as u32;
                        let mut codec = opts.codec;
                        let mut payload = compress::compress(codec, opts.level, &job.data)?;
                        if !job.data.is_empty() && payload.len() >= job.data.len() {
                            payload = job.data;
                            codec = CODE_STORE;
                        }
                        if let Some(c) = &crypto {
                            payload = c.encrypt_block(&salt, job.block_id, codec, &payload)?;
                        }
                        let crc = crc32fast::hash(&payload);
                        res_tx.send(StoredBlock {
                            block_id: job.block_id,
                            codec,
                            orig_len,
                            payload,
                            crc,
                            encrypted: crypto.is_some(),
                        })?;
                    }
                    Ok(())
                })();
                if let Err(e) = r {
                    *err.lock().unwrap() = Some(e);
                }
            });
        }
        drop(res_tx);

        // Writer: ordered writes + RS segment parity. On error, record it and
        // drain the channel so workers never deadlock on a full queue.
        let r = run_writer(&res_rx, &mut out, opts.segment_size, vol.as_mut(), &block_table, progress);
        if let Err(e) = r {
            *err.lock().unwrap() = Some(e);
            for _ in &res_rx {
                // drain
            }
        }
    });

    if let Some(e) = err.lock().unwrap().take() {
        let _ = out.flush();
        return Err(e);
    }

    let block_table = Arc::try_unwrap(block_table)
        .map_err(|_| anyhow::anyhow!("internal: block table still shared"))?
        .into_inner()
        .unwrap();
    let produced_n = produced.load(Ordering::SeqCst);
    if block_table.len() as u64 != produced_n {
        bail!(
            "pipeline inconsistency: {} blocks produced, {} written",
            produced_n,
            block_table.len()
        );
    }

    let mut files = entries;
    let lists = Arc::try_unwrap(block_lists)
        .map_err(|_| anyhow::anyhow!("internal: block lists still shared"))?
        .into_inner()
        .unwrap();
    for (i, list) in lists.into_iter().enumerate() {
        files[i].blocks = list;
    }

    // Index + header patch (index goes last, like a ZIP central directory).
    let verifier_hex = match &crypto {
        Some(c) => Some(format::hex(&c.verifier()?)),
        None => None,
    };
    let index = Index {
        created_by: format!("nextar {}", env!("CARGO_PKG_VERSION")),
        created_at: now_unix(),
        encrypted: crypto.is_some(),
        codec: codec_name(opts.codec).to_string(),
        level: opts.level.clamp(0, 255) as u8,
        block_size: opts.block_size,
        segment_size: opts.segment_size as u16,
        parity: opts.parity as u16,
        salt_hex: crypto.as_ref().map(|_| format::hex(&salt)),
        verifier_hex,
        files,
        blocks: block_table,
    };
    let index_bytes = format::index_to_bytes(&index)?;
    let index_offset = out.stream_position()?;
    out.write_all(&index_bytes)?;
    header.index_offset = index_offset;
    header.index_len = index_bytes.len() as u64;
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&header.to_bytes())?;
    out.flush()?;
    let archive_size = out.metadata()?.len();

    let mut volume_size = 0;
    if let Some(v) = vol.take() {
        let mut v = v;
        v.set_index(&index_bytes)?;
        v.finish()?;
        if let Some(vp) = vol_path {
            volume_size = std::fs::metadata(vp).map(|m| m.len()).unwrap_or(0);
        }
    }

    progress.finish();
    Ok(CreateStats {
        total_bytes_read: total_bytes,
        archive_size,
        volume_size,
        block_count: index.blocks.len(),
    })
}

fn run_writer(
    rx: &Receiver<StoredBlock>,
    out: &mut File,
    k: usize,
    mut vol: Option<&mut VolumeWriter>,
    block_table: &Arc<Mutex<Vec<BlockEntry>>>,
    progress: &Progress,
) -> Result<()> {
    let mut pending: BTreeMap<u64, StoredBlock> = BTreeMap::new();
    let mut segment: Vec<StoredBlock> = Vec::new();
    let mut next_id = 0u64;

    loop {
        match rx.recv() {
            Ok(b) => {
                pending.insert(b.block_id, b);
            }
            Err(_) => break,
        }
        while let Some(b) = pending.remove(&next_id) {
            if vol.is_some() {
                segment.push(b);
                if segment.len() == k {
                    finalize_segment(&mut segment, vol.as_mut().unwrap(), out, block_table, progress)?;
                    segment.clear();
                }
            } else {
                write_block(out, &b, block_table, progress)?;
            }
            next_id += 1;
        }
    }

    if !pending.is_empty() {
        bail!(
            "pipeline ended early: {} blocks missing from writer",
            pending.len()
        );
    }
    if vol.is_some() && !segment.is_empty() {
        finalize_segment(&mut segment, vol.as_mut().unwrap(), out, block_table, progress)?;
    }
    Ok(())
}

fn finalize_segment(
    segment: &mut Vec<StoredBlock>,
    vol: &mut VolumeWriter,
    out: &mut File,
    block_table: &Arc<Mutex<Vec<BlockEntry>>>,
    progress: &Progress,
) -> Result<()> {
    let k = vol.k();
    let m = vol.m();
    let payloads: Vec<Vec<u8>> = segment.iter().map(|b| b.payload.clone()).collect();
    let mut parity = recovery::encode_segment(k, m, &payloads)?;
    parity.seg_id = (segment[0].block_id as usize / k) as u32;
    vol.add_segment(&parity)?;
    for b in segment.iter() {
        write_block(out, b, block_table, progress)?;
    }
    Ok(())
}

fn write_block(
    out: &mut File,
    b: &StoredBlock,
    block_table: &Arc<Mutex<Vec<BlockEntry>>>,
    progress: &Progress,
) -> Result<()> {
    let mut flags = 0u8;
    if b.encrypted {
        flags |= format::BLOCK_FLAG_ENCRYPTED;
    }
    let hdr = BlockHeader {
        flags,
        codec: b.codec,
        block_id: b.block_id,
        orig_len: b.orig_len,
        stored_len: b.payload.len() as u32,
        crc: b.crc,
    };
    out.write_all(&hdr.to_bytes())?;
    out.write_all(&b.payload)?;
    block_table.lock().unwrap().push(BlockEntry {
        id: b.block_id,
        codec: b.codec,
        orig_len: b.orig_len,
        stored_len: hdr.stored_len,
        crc: b.crc,
    });
    progress.add(b.payload.len() as u64);
    Ok(())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
