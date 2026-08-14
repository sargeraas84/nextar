# The .NEXT Archive Format — Specification v1

This document describes the on-disk binary format used by `nextar` (version 1).
All multi-byte integers are **little-endian**. All checksums are CRC-32 (IEEE,
the same polynomial as `crc32fast`).

## 1. Archive file layout

```
┌─────────────────────────────────────────────────────────┐
│ Header                      (60 bytes)                   │
├─────────────────────────────────────────────────────────┤
│ Data block 0                (28-byte header + payload)   │
│ Data block 1                                             │
│ ...                                                      │
│ Data block N-1                                           │
├─────────────────────────────────────────────────────────┤
│ Index block                 (written last, ZIP-style)    │
└─────────────────────────────────────────────────────────┘
```

The index is written *after* every data block, then the header is patched with
the index offset/length. This keeps the header small and lets the data stream
first, exactly like a ZIP central directory.

### 1.1 Archive header (60 bytes)

| offset | size | field                     | meaning                                        |
|--------|------|---------------------------|------------------------------------------------|
| 0      | 8    | magic                     | `4E 45 58 54 41 52 01 00` ("NEXTAR" + v1)       |
| 8      | 2    | version                   | format version (1)                              |
| 10     | 4    | flags                     | bit 0 = encrypted, bit 1 = recovery             |
| 14     | 1    | default codec             | 0 store · 1 zstd · 2 lzma2                      |
| 15     | 1    | default level             | compression level of the default codec          |
| 16     | 4    | block size                | chunk size in bytes (data is split into chunks) |
| 20     | 2    | segment size              | RS data blocks per segment (0 if no recovery)   |
| 22     | 2    | parity                    | RS parity blocks per segment (0 if no recovery) |
| 24     | 16   | salt                      | random archive salt (seeds block nonces)        |
| 40     | 4    | header CRC32              | CRC32 of bytes **8..40**                         |
| 44     | 8    | index offset              | byte offset of the index block                  |
| 52     | 8    | index length              | byte length of the index block                  |

The header CRC covers the version, flags, codec, level, block size, segment
size, parity and salt — everything except the magic and the two index fields.
The index fields are patched at the end, so the CRC is recomputed then.

### 1.2 Data block (28-byte header + payload)

| offset | size | field      | meaning                                         |
|--------|------|------------|-------------------------------------------------|
| 0      | 4    | magic      | `4E 42 31 01` ("NB1" + v1)                       |
| 4      | 1    | flags      | bit 0 = payload is encrypted                     |
| 5      | 1    | codec      | 0 store · 1 zstd · 2 lzma2 (per-block override)  |
| 6      | 8    | block id   | global sequential block id                       |
| 14     | 4    | orig len   | plaintext (decompressed) length                  |
| 18     | 4    | stored len | payload length on disk                           |
| 22     | 4    | payload CRC| CRC32 of the stored payload bytes                |
| 26     | 2    | reserved   | zero                                            |

The payload is the compressed, then optionally encrypted, chunk bytes.
Encryption is authenticated (see §3), so the payload CRC is a cheap second
integrity layer that works even without the password (used by `verify` and
`repair`).

An empty chunk is stored as a block with `stored len = 0` and `codec = store`.
Incompressible chunks fall back to `codec = store` automatically (stored raw),
so compression never makes a chunk *larger*.

### 1.3 Index block

```
magic "NIDX\x01\x00\x00\x00" (8 bytes)
index JSON length             (u64)
index JSON                    (serde_json, see §2)
index JSON CRC32              (u32)
```

The index holds all metadata — file tree, permissions, timestamps and the
global block table — as a single JSON document.

## 2. Index JSON

```jsonc
{
  "created_by":  "nextar 0.1.0",
  "created_at":  1712345678,
  "encrypted":   false,
  "codec":       "zstd",          // "zstd" | "lzma2" | "store"
  "level":       3,
  "block_size":  1048576,
  "segment_size": 128,            // 0 when no recovery
  "parity":      8,               // 0 when no recovery
  "salt_hex":    null,            // 32 hex chars when encrypted
  "verifier_hex": null,           // password verifier when encrypted
  "files": [
    {
      "path":  "docs/readme.md",  // archive-relative, forward slashes
      "kind":  "file",            // "file" | "dir" | "symlink"
      "mode":  33188,             // Unix permission bits (best-effort on Windows)
      "size":  12345,
      "mtime": 1712345678,        // seconds since Unix epoch
      "mtime_ns": 123456,
      "link":  null,              // symlink target when kind == "symlink"
      "blocks": [0, 1, 2]         // block ids holding this file's chunks, in order
    }
  ],
  "blocks": [
    { "id": 0, "codec": 1, "orig_len": 1048576, "stored_len": 51234, "crc": 1234567890 }
  ]
}
```

### Design notes

* **Directory structure** — directories are stored as explicit `dir` entries,
  so empty directories survive extraction. Paths are stored with `/`
  separators, relative to the archive root, and are validated on extraction
  (no `..`, no absolute paths — zip-slip protection).
* **Permissions & timestamps** — each entry carries Unix-style `mode` bits and
  `mtime` (seconds + nanoseconds). On extraction the mode is restored
  (chmod on Unix; read-only attribute mapping on Windows) and the mtime is
  set after the file is written.
* **Block table** — the flat `blocks` array (sorted by id) gives every
  block's length, codec and CRC. Because blocks are stored back-to-back right
  after the header, any block's byte offset is the cumulative sum of the
  preceding block sizes. This powers direct seeking during extraction and
  precise repair when a block's own header is damaged.
* **The index is not encrypted** — so archives can be listed without a
  password. Payloads (file contents) are always protected; the metadata is
  not (a documented trade-off, like WinRAR without "encrypt file names").
* **Block nonces are derived, not stored** — see §3.2.

## 3. Cryptography

### 3.1 Key derivation (Argon2id)

```
out[64] = Argon2id(password, salt=archive.salt, m=64 MiB, t=3, p=1)
key          = out[0..32]    → block encryption (XChaCha20-Poly1305)
verifier_key = out[32..64]   → password check
```

### 3.2 Block encryption (XChaCha20-Poly1305)

Every block payload is encrypted with XChaCha20-Poly1305 (AEAD).

```
nonce[24] = SHA-256(archive.salt ‖ block_id_le64)[0..24]
aad[9]    = block_id_le64 ‖ codec
ciphertext = XChaCha20Poly1305_encrypt(key, nonce, aad, plaintext)
```

* Nonces are **deterministic per block id**, so nothing needs storing in the
  header — which in turn means a repaired archive can rebuild headers
  bit-for-bit from the block table.
* The **AAD binds the ciphertext to its block id and codec**, so blocks
  cannot be reordered, duplicated or re-coded without failing authentication.
* An encrypted block's `stored len` = plaintext length + 16 (Poly1305 tag).

### 3.3 Password verifier

`verifier_hex` is the AEAD of 32 zero bytes under `verifier_key` with a fixed
zero nonce and AAD `"nextar-verifier-v1"`. It is a fast, deterministic
password check performed at extraction/verify time — a wrong password fails
here instead of mid-decompression.

## 4. Recovery volumes (.nvol)

The volume file is a *sibling* of the archive: `backup.next` →
`backup.next.nvol`. It is only created when `-r N` is passed.

```
magic "NEXVOL1\x00"            (8 bytes)
version                        (u16 = 1)
segment size k                 (u16, must match archive header)
parity m                       (u16, must match archive header)
segment count                  (u32, placeholder patched at the end)
archive header copy            (60 bytes — rescues a truncated archive)
per segment (segment count times):
    segment id                 (u32)
    data count                 (u32 — real data blocks; < k only for the last segment)
    shard size                 (u32 — max payload length in the segment)
    m parity shards            (m × shard size bytes)
"NIDXVOL1"                     (8 bytes)
index length                   (u64)
index bytes                    (raw index block, incl. magic + CRC)
index CRC32                    (u32)
"NEXVEND\x00"                  (8 bytes)
```

### Reed-Solomon segmentation

* The archive's blocks are grouped into segments of `k` consecutive block ids
  (default 128). Segment *s* covers ids `[s·k, (s+1)·k)`.
* For each segment, the `m` parity shards are computed with
  Reed-Solomon erasure coding (GF(2⁸)) over the segment's **stored payloads**
  (compressed + encrypted bytes — never plaintext).
* Shards are padded to the segment's largest payload, so all shards in a
  segment are equal length.
* The **last segment may be short** (`data count < k`); the missing lanes are
  synthesized as zero shards, so the same `k`-lane codeword is always used.
* Repair needs any `k` of the `k + m` shards per segment, so up to `m`
  corrupt **or missing** blocks per segment can be rebuilt. This heals both
  bit flips and partial downloads.
* Parity is computed over ciphertext, so **repair needs no password** and
  never touches plaintext.
* The volume carries copies of the header and index, so even an archive whose
  tail (including the index block) is gone can be reconstructed — run
  `nextar repair` first, then `nextar extract`.

### Why the overhead scales with the biggest block

Each segment stores `m` shards of the segment's maximum payload size, so a
volume costs roughly `m × block_size` bytes per segment (e.g. `-r 8` with
1 MiB blocks ≈ 8 MiB per 128 MiB of data ≈ 6.25%). For tiny archives this
fixed cost dominates — use a smaller `-r` or a smaller `-b` block size.

## 5. Limits & invariants

* `k + m ≤ 256` (GF(2⁸) field size). Default `k = 128`, so `m ≤ 128`.
* Block id, orig len, stored len and payload CRC in the index are
  authoritative; the on-disk block header mirrors them for standalone
  scanning and for a second integrity check.
* The archive header CRC covers bytes 8..40; the index JSON CRC covers the
  whole JSON document; each payload CRC covers the stored bytes. The AEAD tag
  additionally authenticates every encrypted payload against tampering.
