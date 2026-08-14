//! Reed-Solomon recovery volumes.
//!
//! Every archive is divided into *segments* of `k` data blocks. When recovery
//! is enabled, `m` parity shards are computed per segment and stored in a
//! separate volume file (`<archive>.nvol`). Any `m` lost or corrupted blocks
//! within a segment can be rebuilt from the survivors — the archive heals
//! itself the way a RAID-6 array or PAR2 set does.
//!
//! Parity is computed over the *stored* bytes (compressed + encrypted
//! payloads), so repair needs no password and never touches plaintext.
//!
//! The volume file additionally carries a copy of the archive header and the
//! JSON index, so an archive truncated mid-download can still be repaired
//! (the index copy substitutes for the missing tail).
//!
//! Volume file layout:
//!
//! ```text
//! magic "NEXVOL1" (8) | version u16 | k u16 | m u16 | seg_count u32
//! archive header copy (60 bytes)
//! per segment: seg_id u32 | data_count u32 | shard_size u32 | m × shard_size bytes
//! "NIDXVOL1" (8) | index_len u64 | index bytes | index_crc u32
//! "NEXVEND" (8)
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use reed_solomon_erasure::galois_8::ReedSolomon;

use crate::format::{self, ArchiveHeader, BLOCK_HEADER_LEN};

/// Default data blocks per RS segment (power of two, galois-8 friendly).
pub const DEFAULT_SEGMENT_SIZE: usize = 128;
/// Upper bound: galois_8 supports at most 256 total shards.
pub const MAX_TOTAL_SHARDS: usize = 256;

/// Parity computed for one segment.
pub struct SegmentParity {
    pub seg_id: u32,
    /// Number of real data blocks in the segment (<= k; last segment may be short).
    pub data_count: usize,
    /// All data shards are padded to this size for the RS math.
    pub shard_size: usize,
    /// `m` parity shards, each `shard_size` bytes.
    pub parity: Vec<Vec<u8>>,
}

fn validate_params(k: usize, m: usize) -> Result<()> {
    if k < 2 {
        bail!("segment size must be >= 2 (got {k})");
    }
    if m < 1 {
        bail!("parity must be >= 1 for recovery");
    }
    if k + m > MAX_TOTAL_SHARDS {
        bail!("segment size + parity must be <= {MAX_TOTAL_SHARDS} (got {k} + {m})");
    }
    Ok(())
}

/// Compute `m` parity shards for one segment of `k` lanes from the real
/// payloads present (`payloads.len() <= k`). Missing lanes are synthesized
/// as zero shards so the last, short segment still uses the same `k` codeword.
pub fn encode_segment(k: usize, m: usize, payloads: &[Vec<u8>]) -> Result<SegmentParity> {
    validate_params(k, m)?;
    let data_count = payloads.len();
    if data_count > k {
        bail!("segment has more data blocks than the segment size");
    }
    let shard_size = payloads.iter().map(|p| p.len()).max().unwrap_or(0);

    let rs = ReedSolomon::new(k, m).context("reed-solomon setup failed")?;
    // Lanes: 0..data_count = real payloads (padded), data_count..k = zeros.
    let mut shards: Vec<Vec<u8>> = payloads
        .iter()
        .map(|p| {
            let mut v = p.clone();
            v.resize(shard_size, 0);
            v
        })
        .chain(std::iter::repeat_with(|| vec![0u8; shard_size]).take(k - data_count))
        .chain(std::iter::repeat_with(|| vec![0u8; shard_size]).take(m))
        .collect();
    rs.encode(&mut shards).context("reed-solomon encoding failed")?;

    let parity = shards[k..k + m].to_vec();
    Ok(SegmentParity { seg_id: 0, data_count, shard_size, parity })
}

/// Rebuild corrupted/missing data shards for one segment.
///
/// `lanes` has one entry per real data block: `Some(payload)` when the block
/// is intact, `None` when it is corrupted or missing. `parity` holds the `m`
/// volume shards. Returns the rebuilt payloads for the *erased* lanes only,
/// in lane order.
pub fn reconstruct_segment(
    k: usize,
    m: usize,
    lanes: Vec<Option<Vec<u8>>>,
    parity: &[Vec<u8>],
) -> Result<Vec<Option<Vec<u8>>>> {
    validate_params(k, m)?;
    let data_count = lanes.len();
    if data_count > k {
        bail!("segment has more data blocks than the segment size");
    }
    if parity.len() != m {
        bail!("volume parity count mismatch (expected {m}, got {})", parity.len());
    }
    let shard_size = lanes
        .iter()
        .flatten()
        .map(|p| p.len())
        .chain(parity.iter().map(|p| p.len()))
        .max()
        .unwrap_or(0);

    let rs = ReedSolomon::new(k, m).context("reed-solomon setup failed")?;
    let mut received: Vec<Option<Vec<u8>>> = lanes
        .iter()
        .map(|l| {
            l.as_ref().map(|p| {
                let mut v = p.clone();
                v.resize(shard_size, 0);
                v
            })
        })
        // synthesize zero shards for the short final segment
        .chain(std::iter::repeat_with(|| Some(vec![0u8; shard_size])).take(k - data_count))
        .chain(parity.iter().cloned().map(Some))
        .collect();

    rs.reconstruct(&mut received).context("reed-solomon reconstruction failed")?;

    let rebuilt = received[..data_count]
        .iter()
        .zip(lanes.iter())
        .map(|(recv, orig)| match (recv, orig) {
            (Some(_), Some(_)) => None, // intact, untouched
            (Some(v), None) => Some(v.clone()),
            (None, _) => unreachable!("reconstruct fills every shard"),
        })
        .collect();
    Ok(rebuilt)
}

// ─────────────────────────── volume file I/O ───────────────────────────

pub struct VolumeWriter {
    file: File,
    k: usize,
    m: usize,
    segments: u32,
    header: Option<[u8; format::HEADER_LEN as usize]>,
    index: Option<Vec<u8>>,
}

impl VolumeWriter {
    pub fn k(&self) -> usize {
        self.k
    }

    pub fn m(&self) -> usize {
        self.m
    }

    pub fn create(path: &Path, k: usize, m: usize) -> Result<Self> {
        validate_params(k, m)?;
        let mut file = File::create(path).with_context(|| format!("creating volume file {}", path.display()))?;
        file.write_all(&format::VOL_MAGIC)?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&(k as u16).to_le_bytes())?;
        file.write_all(&(m as u16).to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?; // seg_count placeholder
        Ok(VolumeWriter { file, k, m, segments: 0, header: None, index: None })
    }

    /// Store a copy of the archive header (rescues truncated archives).
    /// Written immediately, right after the volume header, before any segments.
    pub fn set_header(&mut self, header: &ArchiveHeader) -> Result<()> {
        self.file.write_all(&header.to_bytes())?;
        self.header = Some(header.to_bytes());
        Ok(())
    }

    pub fn add_segment(&mut self, seg: &SegmentParity) -> Result<()> {
        let file = &mut self.file;
        file.write_all(&seg.seg_id.to_le_bytes())?;
        file.write_all(&(seg.data_count as u32).to_le_bytes())?;
        file.write_all(&(seg.shard_size as u32).to_le_bytes())?;
        for shard in &seg.parity {
            file.write_all(shard)?;
        }
        self.segments += 1;
        Ok(())
    }

    /// Append the index copy (written after all blocks complete).
    pub fn set_index(&mut self, index_bytes: &[u8]) -> Result<()> {
        self.index = Some(index_bytes.to_vec());
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        if self.header.is_none() {
            bail!("volume finished without an archive header copy");
        }
        let file = &mut self.file;
        let index = self
            .index
            .take()
            .ok_or_else(|| anyhow::anyhow!("volume finished without an index copy"))?;
        file.write_all(&format::VOL_INDEX_MARKER)?;
        file.write_all(&(index.len() as u64).to_le_bytes())?;
        file.write_all(&index)?;
        file.write_all(&crc32fast::hash(&index).to_le_bytes())?;
        file.write_all(&format::VOL_END)?;
        // patch seg_count (written as a placeholder at offset 14)
        let seg_count = self.segments.to_le_bytes();
        file.seek(std::io::SeekFrom::Start(14))?;
        file.write_all(&seg_count)?;
        file.flush()?;
        Ok(())
    }
}

pub struct Volume {
    pub k: usize,
    pub m: usize,
    pub archive_header: ArchiveHeader,
    pub segments: HashMap<u32, SegmentParity>,
    pub index_copy: Option<Vec<u8>>,
}

pub fn read_volume(path: &Path) -> Result<Volume> {
    let mut file = File::open(path).with_context(|| format!("opening volume file {}", path.display()))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    read_volume_bytes(&data)
}

pub fn read_volume_bytes(data: &[u8]) -> Result<Volume> {
    let mut pos = 0usize;
    let need = |pos: &mut usize, n: usize, what: &str| -> Result<()> {
        if *pos + n > data.len() {
            bail!("volume file truncated while reading {what}");
        }
        *pos += n;
        Ok(())
    };

    need(&mut pos, 8, "magic")?;
    if data[0..8] != format::VOL_MAGIC {
        bail!("not a .next volume file (bad magic)");
    }
    need(&mut pos, 2, "version")?;
    let version = u16::from_le_bytes(data[pos - 2..pos].try_into().unwrap());
    if version != 1 {
        bail!("unsupported volume version {version}");
    }
    need(&mut pos, 2, "k")?;
    let k = u16::from_le_bytes(data[pos - 2..pos].try_into().unwrap()) as usize;
    need(&mut pos, 2, "m")?;
    let m = u16::from_le_bytes(data[pos - 2..pos].try_into().unwrap()) as usize;
    need(&mut pos, 4, "seg_count")?;
    let seg_count = u32::from_le_bytes(data[pos - 4..pos].try_into().unwrap());
    validate_params(k, m)?;

    need(&mut pos, format::HEADER_LEN as usize, "archive header copy")?;
    let archive_header = ArchiveHeader::from_bytes(&data[pos - format::HEADER_LEN as usize..pos])?;

    let mut segments = HashMap::new();
    for _ in 0..seg_count {
        need(&mut pos, 4, "seg_id")?;
        let seg_id = u32::from_le_bytes(data[pos - 4..pos].try_into().unwrap());
        need(&mut pos, 4, "data_count")?;
        let data_count = u32::from_le_bytes(data[pos - 4..pos].try_into().unwrap()) as usize;
        need(&mut pos, 4, "shard_size")?;
        let shard_size = u32::from_le_bytes(data[pos - 4..pos].try_into().unwrap()) as usize;
        let mut parity = Vec::with_capacity(m);
        for _ in 0..m {
            need(&mut pos, shard_size, "parity shard")?;
            parity.push(data[pos - shard_size..pos].to_vec());
        }
        segments.insert(seg_id, SegmentParity { seg_id, data_count, shard_size, parity });
    }

    let mut index_copy = None;
    if pos + 8 <= data.len() && data[pos..pos + 8] == format::VOL_INDEX_MARKER {
        pos += 8;
        need(&mut pos, 8, "index len")?;
        let ilen = u64::from_le_bytes(data[pos - 8..pos].try_into().unwrap()) as usize;
        need(&mut pos, ilen, "index copy")?;
        let ibytes = data[pos - ilen..pos].to_vec();
        need(&mut pos, 4, "index crc")?;
        let expected = u32::from_le_bytes(data[pos - 4..pos].try_into().unwrap());
        let actual = crc32fast::hash(&ibytes);
        if expected == actual {
            index_copy = Some(ibytes);
        }
        if pos + 8 <= data.len() && data[pos..pos + 8] != format::VOL_END {
            bail!("volume file missing end marker");
        }
    }

    Ok(Volume { k, m, archive_header, segments, index_copy })
}

/// Byte offset of data block `id` within an archive, given the block table.
/// Blocks are stored back-to-back right after the header, so offsets are the
/// cumulative sum of preceding block sizes.
pub fn block_offsets(blocks: &[format::BlockEntry]) -> Vec<u64> {
    let mut offsets = Vec::with_capacity(blocks.len());
    let mut off = format::HEADER_LEN;
    for b in blocks {
        offsets.push(off);
        off += BLOCK_HEADER_LEN + b.stored_len as u64;
    }
    offsets
}

/// Validate that a volume matches its archive header (segment size + parity).
pub fn validate_pair(header: &ArchiveHeader, volume: &Volume) -> Result<()> {
    if volume.k != header.segment_size as usize {
        bail!(
            "volume segment size {} does not match archive {}",
            volume.k,
            header.segment_size
        );
    }
    if volume.m != header.parity as usize {
        bail!("volume parity {} does not match archive {}", volume.m, header.parity);
    }
    Ok(())
}
