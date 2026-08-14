//! nextar engine — shared by the `nextar` CLI and `nextar-gui` desktop app.
//!
//! * Zstd "fast" tier and LZMA2 "ultra" tier compression
//! * Argon2id + XChaCha20-Poly1305 authenticated encryption
//! * Reed-Solomon recovery volumes that heal corrupted archives
//! * Fully parallel read → compress → encrypt → write pipeline

pub mod archive;
pub mod compress;
pub mod crypto;
pub mod format;
pub mod pipeline;
pub mod progress;
pub mod recovery;
pub mod term;
