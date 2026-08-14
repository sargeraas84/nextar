# nextar — System Architecture

`nextar` is a next-generation archiver built in Rust. It combines the best
modern open-source technologies to target the four pillars the project brief
defines: **speed & ratio**, **security**, **resilience**, and **concurrency**.

| Pillar      | Technology chosen                                                                 |
|-------------|-----------------------------------------------------------------------------------|
| Speed       | Zstandard (zstd) — default "fast" tier, ~GB/s on commodity hardware                |
| Ratio       | LZMA2 (liblzma, the 7-Zip engine) — "ultra" tier                                   |
| Security    | Argon2id (KDF) + XChaCha20-Poly1305 (authenticated encryption)                    |
| Resilience  | Reed-Solomon erasure coding → `.nvol` recovery volumes that rebuild lost data     |
| Concurrency | Multi-stage worker pipeline: read ‖ compress ‖ encrypt ‖ write on every core      |

## 1. Module breakdown

```
src/
├── lib.rs         Engine root — shared by both binaries
├── main.rs        CLI (clap): create / extract / list / info / verify / repair
├── bin/nextar-gui.rs  Desktop app (egui/eframe): create, extract, inspect, repair
├── archive.rs     Orchestration: filesystem walking, extraction, verify, repair
├── pipeline.rs    The parallel compression pipeline (producer → workers → writer)
├── compress.rs    Compression engines: zstd + lzma2 (+ store fallback)
├── crypto.rs      Argon2id key derivation + XChaCha20-Poly1305 AEAD
├── recovery.rs    Reed-Solomon segments, parity shards, volume file I/O
├── format.rs      .NEXT binary format: headers, index, serialization
├── progress.rs    Progress reporter (terminal bar or shared state for the GUI)
└── term.rs        ANSI styling + brand colors for terminal output
```

```
              ┌────────────────────────────────────────────────┐
              │                     CLI (main.rs)              │
              └───────┬──────────┬───────────┬────────┬────────┘
                      │          │           │        │
              ┌───────▼───┐ ┌────▼────┐ ┌────▼────┐ ┌─▼──────────┐
              │ archive.rs │ │ crypto  │ │ compress│ │ recovery   │
              │ walk/extract││ .rs     │ │ .rs     │ │ .rs (RS)   │
              │ verify/repair│          │           │            │
              └───────┬───┘ └────┬────┘ └────┬────┘ └─┬──────────┘
                      │          │           │        │
              ┌───────▼──────────▼───────────▼────────▼─────────┐
              │               pipeline.rs                        │
              │  producer → N workers (compress+encrypt) → writer │
              └─────────────────────────────────────────────────┘
                      │
              ┌───────▼───────┐
              │   format.rs    │   .NEXT binary layout (docs/FORMAT.md)
              └───────────────┘
```

### Responsibilities

* **`archive.rs` (File I/O + orchestration)** — walks input trees (walkdir,
  symlink-safe, output-path exclusion), builds the JSON index, and drives
  extraction across worker threads. Also implements `verify` (per-block CRC +
  AEAD check) and `repair` (RS rebuild, see §4).
* **`pipeline.rs` (Compression Manager)** — the parallel create path: one
  producer reads chunks, N workers compress/encrypt/checksum, one writer
  orders blocks and (optionally) computes RS parity. Bounded channels give
  backpressure so memory stays flat.
* **`compress.rs`** — thin, codec-agnostic layer over `zstd` (bulk) and
  `xz2`/liblzma (LZMA2 in an xz container). Empty chunks and incompressible
  chunks fall back to "store" automatically.
* **`crypto.rs`** — Argon2id (64 MiB, t=3, p=1) stretches the password into a
  32-byte key + 32-byte verifier key. Every block payload is encrypted with
  XChaCha20-Poly1305 using a deterministic per-block nonce
  (`SHA-256(salt ‖ block_id)`) and an AAD binding id + codec.
* **`recovery.rs`** — segments blocks into groups of `k` (default 128),
  computes `m` Reed-Solomon parity shards per segment, and reads/writes
  `.nvol` volume files (which also carry header + index copies).
* **`format.rs`** — the .NEXT binary format: 60-byte header, 28-byte block
  headers, JSON index with the block table (see `docs/FORMAT.md`).
* **`progress.rs`** — a background thread redraws a % / bytes / speed line on
  stderr every 250 ms (auto-disabled when not a TTY or `-q`).

## 2. The compression pipeline

```
    files            chunks              stored blocks             disk
┌──────────┐   jobs   ┌─────────────┐  results  ┌──────────────┐
│ producer │ ───────► │ worker 0..N │ ────────► │ writer       │
│ (1 thread│ bounded  │ compress →  │  bounded  │ ordered      │
│  reads)  │ channel  │ encrypt →   │  channel  │ writes + RS  │
└──────────┘          │ crc         │           └──────────────┘
                      └─────────────┘
```

* **Producer** reads each file in `block_size` chunks (default 1 MiB),
  assigns global block ids, and hands chunks to a bounded job queue.
* **Workers** (one per core, `-t` to override) each pull a chunk, compress
  it, fall back to storing raw when compression doesn't help, encrypt it
  (if a password was given), CRC it, and push the result to the results queue.
* **Writer** consumes results out of order, buffers them in a `BTreeMap`, and
  writes in block-id order — so the on-disk layout is always sequential. With
  recovery enabled, it buffers whole RS segments and streams parity into the
  volume file as each segment completes.
* **Backpressure**: both queues are bounded (`2 × threads`), so a slow disk
  stalls the writer, which stalls the workers, which stalls the producer —
  memory use stays ~constant regardless of archive size. The worst case is
  one full RS segment in flight (~128 MiB at defaults).
* **Failure handling**: any stage error is recorded in a shared slot; the
  writer drains its queue before exiting so no thread deadlocks on a full
  channel. The whole operation then aborts cleanly.

**Extraction** uses the same worker-pool model but parallelizes *across
files* (each worker owns one output file, reads its blocks positionally from
the shared archive handle via `read_at`/`seek_read` — safe for concurrent
readers), and restores permissions/timestamps as files complete.

## 3. Cryptography design

* **Argon2id** with OWASP-recommended parameters (64 MiB, t=3, p=1) makes
  brute-forcing the password expensive; the random 16-byte archive salt
  prevents rainbow tables.
* **XChaCha20-Poly1305** provides authenticated encryption: tampered or
  reordered blocks fail decryption. The 24-byte nonce space and 192-bit
  nonce construction make random nonce collisions practically impossible —
  and we make nonces *deterministic* per block id anyway (see FORMAT.md §3.2),
  which also keeps headers rebuildable after repair.
* The **index is deliberately unencrypted** so `list` works without a
  password. File *contents* are always protected. (Encrypting file names is a
  natural future extension.)
* Recovery parity is computed over **ciphertext**, so `repair` can heal an
  encrypted archive without the password.

## 4. Resilience & repair

Each segment of `k` blocks produces `m` parity shards stored in the volume
file. Repair (`nextar repair`):

1. reads the archive header + index (falling back to the copies embedded in
   the volume when the tail of the archive is damaged or missing);
2. walks every block, verifying its payload CRC — a bad CRC or an
   unreadable/missing block is marked as an *erasure*;
3. reconstructs the erased shards with Reed-Solomon (needs only `k` of
   `k+m` shards per segment, so up to `m` lost blocks per segment heal);
4. verifies each rebuilt payload against the index CRC, then writes a fresh
   archive (temp file + rename) with the original index.

This handles both **bit flips** (detected by CRC) and **partial downloads**
(the tail is rebuilt from parity + the volume's index copy).

## 5. Known trade-offs and future work

* **Chunked compression** (per 1 MiB block) enables the parallel pipeline but
  costs a few percent of ratio versus whole-file compression. A solid-state
  "long-range" mode (zstd `--long`, larger blocks) is a natural next step.
* **Extraction parallelizes across files**; a single huge file extracts on
  one core. Positional `write_at` chunk writes (as already done for reads)
  would parallelize within a file too.
* **Volume cost** scales with `m × max-block-size` per segment — fine for
  full segments, wasteful for tiny archives (documented in FORMAT.md §4).
* The volume format v1 is a single file; multi-part volumes (PAR2-style) are
  a future extension.
* `verify` with a password additionally authenticates every block with the
  AEAD, catching tampering that a CRC-only check would miss.
