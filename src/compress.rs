//! Compression engines.
//!
//! Two tiers as designed in the project brief:
//! * **Zstandard** (`zstd`) — the default "fast" tier. Extremely fast with
//!   good ratios; levels 1..=22.
//! * **LZMA2** (`lzma2`) — the "maximum/ultra" tier via liblzma (xz2),
//!   the same engine 7-Zip uses; levels 0..=9.
//!
//! Compression happens per chunk (block), which keeps the pipeline fully
//! parallel and streaming-friendly.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};

use crate::format::{CODE_LZMA2, CODE_STORE, CODE_ZSTD};

/// Compress a chunk. Returns `Some(compressed)` if the codec is applied,
/// `None` if the data is empty (empty chunks are stored with zero length).
pub fn compress(codec: u8, level: i32, data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let out = match codec {
        CODE_STORE => data.to_vec(),
        CODE_ZSTD => zstd::bulk::compress(data, level).context("zstd compression failed")?,
        CODE_LZMA2 => lzma2_compress(data, level)?,
        other => bail!("unknown codec {other}"),
    };
    Ok(out)
}

/// Decompress a chunk whose original (plaintext) length is `capacity`.
pub fn decompress(codec: u8, data: &[u8], capacity: usize) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    match codec {
        CODE_STORE => Ok(data.to_vec()),
        CODE_ZSTD => zstd::bulk::decompress(data, capacity).context("zstd decompression failed"),
        CODE_LZMA2 => lzma2_decompress(data, capacity),
        other => bail!("unknown codec {other}"),
    }
}

fn lzma2_compress(data: &[u8], level: i32) -> Result<Vec<u8>> {
    let level = level.clamp(0, 9) as u32;
    let mut enc = xz2::write::XzEncoder::new(Vec::with_capacity(data.len() / 2), level);
    enc.write_all(data).context("lzma2 write failed")?;
    enc.finish().context("lzma2 compression failed")
}

fn lzma2_decompress(data: &[u8], capacity: usize) -> Result<Vec<u8>> {
    let mut dec = xz2::read::XzDecoder::new(data);
    let mut out = Vec::with_capacity(capacity);
    dec.read_to_end(&mut out).context("lzma2 decompression failed")?;
    Ok(out)
}
