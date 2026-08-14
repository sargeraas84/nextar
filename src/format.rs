//! The `.NEXT` archive binary format.
//!
//! Layout of an archive file:
//!
//! ```text
//! ┌─────────────────────────────┐
//! │ Header (60 bytes)           │  magic, flags, codec, salt, index location
//! ├─────────────────────────────┤
//! │ Data block 0                │  28-byte block header + payload
//! │ Data block 1                │
//! │ ...                         │
//! │ Data block N-1              │
//! ├─────────────────────────────┤
//! │ Index block                 │  magic + JSON metadata + CRC
//! └─────────────────────────────┘
//! ```
//!
//! The header is written first with a placeholder index location and patched
//! once the index (written last, like a ZIP central directory) is complete.
//! See `docs/FORMAT.md` for the full field-by-field specification.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// File magic: `NEXTAR` + version byte + pad.
pub const MAGIC: [u8; 8] = *b"NEXTAR\x01\x00";
/// Block header magic (`NB1` + version byte), little-endian u32.
pub const BLOCK_MAGIC: u32 = 0x0131_424E;
/// Index block magic.
pub const INDEX_MAGIC: [u8; 8] = *b"NIDX\x01\x00\x00\x00";
/// Recovery volume file magic.
pub const VOL_MAGIC: [u8; 8] = *b"NEXVOL1\x00";
/// Marker before the index copy inside a volume file.
pub const VOL_INDEX_MARKER: [u8; 8] = *b"NIDXVOL1";
/// Volume file end magic.
pub const VOL_END: [u8; 8] = *b"NEXVEND\x00";

/// Set when payloads are encrypted with XChaCha20-Poly1305.
pub const FLAG_ENCRYPTED: u32 = 1 << 0;
/// Set when recovery volumes were generated (RS parity).
pub const FLAG_RECOVERY: u32 = 1 << 1;

/// Total size of the on-disk archive header.
pub const HEADER_LEN: u64 = 60;
/// Total size of one on-disk block header.
pub const BLOCK_HEADER_LEN: u64 = 28;

/// Compression codecs (per-block codec byte).
pub const CODE_STORE: u8 = 0;
pub const CODE_ZSTD: u8 = 1;
pub const CODE_LZMA2: u8 = 2;

pub fn codec_name(code: u8) -> &'static str {
    match code {
        CODE_STORE => "store",
        CODE_ZSTD => "zstd",
        CODE_LZMA2 => "lzma2",
        _ => "unknown",
    }
}

pub fn codec_from_name(name: &str) -> Result<u8> {
    match name.to_ascii_lowercase().as_str() {
        "store" | "none" | "copy" => Ok(CODE_STORE),
        "zstd" | "zstandard" => Ok(CODE_ZSTD),
        "lzma2" | "lzma" | "xz" => Ok(CODE_LZMA2),
        other => bail!("unknown codec '{other}' (expected zstd, lzma2 or store)"),
    }
}

// ─────────────────────────── archive header ───────────────────────────

/// Byte layout (60 bytes, all integers little-endian):
///
/// | offset | size | field                                             |
/// |--------|------|---------------------------------------------------|
/// | 0      | 8    | magic `NEXTAR\x01`                                |
/// | 8      | 2    | format version (1)                                |
/// | 10     | 4    | flags (bit 0 = encrypted, bit 1 = recovery)        |
/// | 14     | 1    | default codec                                     |
/// | 15     | 1    | default compression level                         |
/// | 16     | 4    | chunk / block size in bytes                       |
/// | 20     | 2    | RS segment size (data blocks per segment)         |
/// | 22     | 2    | RS parity blocks per segment                      |
/// | 24     | 16   | archive salt (random; seeds block nonces)         |
/// | 40     | 4    | CRC32 of bytes 8..40                              |
/// | 44     | 8    | index block offset                               |
/// | 52     | 8    | index block length                               |
#[derive(Debug, Clone, Copy)]
pub struct ArchiveHeader {
    pub version: u16,
    pub flags: u32,
    pub codec: u8,
    pub level: u8,
    pub block_size: u32,
    pub segment_size: u16,
    pub parity: u16,
    pub salt: [u8; 16],
    pub index_offset: u64,
    pub index_len: u64,
}

impl ArchiveHeader {
    pub fn new(codec: u8, level: u8, block_size: u32, segment_size: u16, parity: u16, encrypted: bool) -> Self {
        let mut flags = 0u32;
        if encrypted {
            flags |= FLAG_ENCRYPTED;
        }
        if parity > 0 {
            flags |= FLAG_RECOVERY;
        }
        let mut salt = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);
        ArchiveHeader {
            version: 1,
            flags,
            codec,
            level,
            block_size,
            segment_size,
            parity,
            salt,
            index_offset: 0,
            index_len: 0,
        }
    }

    pub fn encrypted(&self) -> bool {
        self.flags & FLAG_ENCRYPTED != 0
    }

    pub fn recovery(&self) -> bool {
        self.flags & FLAG_RECOVERY != 0
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN as usize] {
        let mut b = [0u8; HEADER_LEN as usize];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..10].copy_from_slice(&self.version.to_le_bytes());
        b[10..14].copy_from_slice(&self.flags.to_le_bytes());
        b[14] = self.codec;
        b[15] = self.level;
        b[16..20].copy_from_slice(&self.block_size.to_le_bytes());
        b[20..22].copy_from_slice(&self.segment_size.to_le_bytes());
        b[22..24].copy_from_slice(&self.parity.to_le_bytes());
        b[24..40].copy_from_slice(&self.salt);
        let crc = crc32fast::hash(&b[8..40]);
        b[40..44].copy_from_slice(&crc.to_le_bytes());
        b[44..52].copy_from_slice(&self.index_offset.to_le_bytes());
        b[52..60].copy_from_slice(&self.index_len.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_LEN as usize {
            bail!("archive header truncated");
        }
        if b[0..8] != MAGIC {
            bail!("not a .next archive (bad magic)");
        }
        let expected = u32::from_le_bytes(b[40..44].try_into().unwrap());
        let actual = crc32fast::hash(&b[8..40]);
        if expected != actual {
            bail!("archive header checksum mismatch (corrupted header)");
        }
        Ok(ArchiveHeader {
            version: u16::from_le_bytes(b[8..10].try_into().unwrap()),
            flags: u32::from_le_bytes(b[10..14].try_into().unwrap()),
            codec: b[14],
            level: b[15],
            block_size: u32::from_le_bytes(b[16..20].try_into().unwrap()),
            segment_size: u16::from_le_bytes(b[20..22].try_into().unwrap()),
            parity: u16::from_le_bytes(b[22..24].try_into().unwrap()),
            salt: b[24..40].try_into().unwrap(),
            index_offset: u64::from_le_bytes(b[44..52].try_into().unwrap()),
            index_len: u64::from_le_bytes(b[52..60].try_into().unwrap()),
        })
    }
}

// ─────────────────────────── block header ───────────────────────────

/// Byte layout (28 bytes, little-endian):
///
/// | offset | size | field                                     |
/// |--------|------|-------------------------------------------|
/// | 0      | 4    | magic `NB1\x01`                            |
/// | 4      | 1    | flags (bit 0 = payload encrypted)          |
/// | 5      | 1    | codec (0 store / 1 zstd / 2 lzma2)         |
/// | 6      | 8    | block id (global sequence)                 |
/// | 14     | 4    | original (plaintext) length                |
/// | 18     | 4    | stored (on-disk) length                    |
/// | 22     | 4    | CRC32 of the stored payload                |
/// | 26     | 2    | reserved                                   |
#[derive(Debug, Clone, Copy)]
pub struct BlockHeader {
    pub flags: u8,
    pub codec: u8,
    pub block_id: u64,
    pub orig_len: u32,
    pub stored_len: u32,
    pub crc: u32,
}

pub const BLOCK_FLAG_ENCRYPTED: u8 = 1 << 0;

impl BlockHeader {
    pub fn to_bytes(&self) -> [u8; BLOCK_HEADER_LEN as usize] {
        let mut b = [0u8; BLOCK_HEADER_LEN as usize];
        b[0..4].copy_from_slice(&BLOCK_MAGIC.to_le_bytes());
        b[4] = self.flags;
        b[5] = self.codec;
        b[6..14].copy_from_slice(&self.block_id.to_le_bytes());
        b[14..18].copy_from_slice(&self.orig_len.to_le_bytes());
        b[18..22].copy_from_slice(&self.stored_len.to_le_bytes());
        b[22..26].copy_from_slice(&self.crc.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < BLOCK_HEADER_LEN as usize {
            bail!("block header truncated");
        }
        if u32::from_le_bytes(b[0..4].try_into().unwrap()) != BLOCK_MAGIC {
            bail!("bad block magic");
        }
        Ok(BlockHeader {
            flags: b[4],
            codec: b[5],
            block_id: u64::from_le_bytes(b[6..14].try_into().unwrap()),
            orig_len: u32::from_le_bytes(b[14..18].try_into().unwrap()),
            stored_len: u32::from_le_bytes(b[18..22].try_into().unwrap()),
            crc: u32::from_le_bytes(b[22..26].try_into().unwrap()),
        })
    }

}

// ─────────────────────────── index (JSON) ───────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileEntry {
    /// Archive-relative path, forward slashes, no leading `./`.
    pub path: String,
    /// "file" | "dir" | "symlink"
    pub kind: String,
    /// Unix-style permission bits (best-effort on Windows).
    pub mode: u32,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: u32,
    /// Symlink target, when kind == "symlink".
    pub link: Option<String>,
    /// Block ids holding this file's chunks, in order.
    pub blocks: Vec<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct BlockEntry {
    pub id: u64,
    pub codec: u8,
    pub orig_len: u32,
    pub stored_len: u32,
    pub crc: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Index {
    pub created_by: String,
    pub created_at: i64,
    pub encrypted: bool,
    pub codec: String,
    pub level: u8,
    pub block_size: u32,
    pub segment_size: u16,
    pub parity: u16,
    /// Hex-encoded Argon2 salt (16 bytes) when encrypted.
    pub salt_hex: Option<String>,
    /// Hex-encoded password verifier when encrypted.
    pub verifier_hex: Option<String>,
    /// Files and directories in the archive, sorted by path.
    pub files: Vec<FileEntry>,
    /// Global block table, sorted by id. Used for direct seeking and repair.
    pub blocks: Vec<BlockEntry>,
}

/// Serialize an index to the on-disk index block.
pub fn index_to_bytes(index: &Index) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(index).context("serializing index")?;
    let mut out = Vec::with_capacity(8 + 8 + json.len() + 4);
    out.extend_from_slice(&INDEX_MAGIC);
    out.extend_from_slice(&(json.len() as u64).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&crc32fast::hash(&json).to_le_bytes());
    Ok(out)
}

/// Parse an index block from raw bytes (header's index region).
pub fn index_from_bytes(b: &[u8]) -> Result<Index> {
    if b.len() < 8 + 8 + 4 {
        bail!("index block truncated");
    }
    if b[0..8] != INDEX_MAGIC {
        bail!("bad index magic (index block missing or corrupted)");
    }
    let len = u64::from_le_bytes(b[8..16].try_into().unwrap()) as usize;
    let json_start = 8 + 8;
    if b.len() < json_start + len + 4 {
        bail!("index block truncated (declared {len} bytes of JSON)");
    }
    let json = &b[json_start..json_start + len];
    let expected = u32::from_le_bytes(b[json_start + len..json_start + len + 4].try_into().unwrap());
    let actual = crc32fast::hash(json);
    if expected != actual {
        bail!("index block checksum mismatch (index corrupted)");
    }
    serde_json::from_slice(json).context("parsing index JSON")
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        bail!("odd-length hex string");
    }
    for i in (0..bytes.len()).step_by(2) {
        let hi = (bytes[i] as char).to_digit(16).context("invalid hex")?;
        let lo = (bytes[i + 1] as char).to_digit(16).context("invalid hex")?;
        out.push((hi * 16 + lo) as u8);
    }
    Ok(out)
}
