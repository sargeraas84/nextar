//! Cryptography.
//!
//! * **Key derivation:** Argon2id (OWASP-recommended parameters:
//!   64 MiB, t=3, p=1) stretches a user password into 64 bytes: the first
//!   32 form the block-encryption key, the last 32 the password *verifier* key.
//! * **Encryption:** XChaCha20-Poly1305 (AEAD — authenticated encryption
//!   providing both confidentiality and integrity) over every block payload.
//! * **Block nonces:** deterministic per block — `SHA-256(salt ‖ block_id)[..24]`.
//!   Block ids are unique within an archive, so nonces never repeat, and they
//!   need no storage, which keeps headers rebuildable during repair.
//! * **AAD:** each block's authenticated data binds the ciphertext to its
//!   block id and codec, so blocks cannot be reordered or re-coded.
//!
//! The JSON index is deliberately *not* encrypted so archives can be listed
//! without a password; the payloads (file contents) are always protected.

use anyhow::{bail, Result};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;
pub const KEY_LEN: usize = 32;

/// Argon2id memory cost in KiB (64 MiB).
const ARGON2_M_COST: u32 = 64 * 1024;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

const VERIFIER_AAD: &[u8] = b"nextar-verifier-v1";
const VERIFIER_MSG_LEN: usize = 32;

#[derive(Clone)]
pub struct Crypto {
    key: [u8; KEY_LEN],
    verifier_key: [u8; KEY_LEN],
}

impl Crypto {
    /// Derive keys from a password and the archive salt (Argon2id).
    pub fn derive(password: &str, salt: &[u8; SALT_LEN]) -> Result<Self> {
        if password.is_empty() {
            bail!("empty password");
        }
        let params = argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(64))
            .map_err(|e| anyhow::anyhow!("invalid Argon2 parameters: {e}"))?;
        let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut out = [0u8; 64];
        argon
            .hash_password_into(password.as_bytes(), salt, &mut out)
            .map_err(|e| anyhow::anyhow!("Argon2id key derivation failed: {e}"))?;
        let mut key = [0u8; KEY_LEN];
        let mut verifier_key = [0u8; KEY_LEN];
        key.copy_from_slice(&out[0..KEY_LEN]);
        verifier_key.copy_from_slice(&out[KEY_LEN..64]);
        Ok(Crypto { key, verifier_key })
    }

    /// Deterministic per-block nonce: SHA-256(archive_salt ‖ block_id LE)[..24].
    pub fn block_nonce(salt: &[u8], block_id: u64) -> [u8; NONCE_LEN] {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(block_id.to_le_bytes());
        let digest = hasher.finalize();
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&digest[..NONCE_LEN]);
        nonce
    }

    /// Encrypt a block payload. `aad` binds the ciphertext to block id + codec.
    pub fn encrypt_block(&self, salt: &[u8], block_id: u64, codec: u8, plain: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::block_nonce(salt, block_id);
        let aad = block_aad(block_id, codec);
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: &aad })
            .map_err(|_| anyhow::anyhow!("encryption failed"))
    }

    /// Decrypt a block payload, authenticating id + codec via AAD.
    pub fn decrypt_block(&self, salt: &[u8], block_id: u64, codec: u8, ct: &[u8]) -> Result<Vec<u8>> {
        let nonce = Self::block_nonce(salt, block_id);
        let aad = block_aad(block_id, codec);
        let cipher = XChaCha20Poly1305::new((&self.key).into());
        cipher
            .decrypt(XNonce::from_slice(&nonce), Payload { msg: ct, aad: &aad })
            .map_err(|_| anyhow::anyhow!("decryption failed (wrong password or corrupted data)"))
    }

    /// Build the password verifier: AEAD of 32 zero bytes under the verifier key.
    pub fn verifier(&self) -> Result<Vec<u8>> {
        let cipher = XChaCha20Poly1305::new((&self.verifier_key).into());
        let msg = vec![0u8; VERIFIER_MSG_LEN];
        cipher
            .encrypt(XNonce::from_slice(&[0u8; NONCE_LEN]), Payload { msg: &msg, aad: VERIFIER_AAD })
            .map_err(|_| anyhow::anyhow!("verifier creation failed"))
    }

    /// Check a stored verifier against this derived key.
    pub fn check_verifier(&self, stored: &[u8]) -> bool {
        let cipher = XChaCha20Poly1305::new((&self.verifier_key).into());
        match cipher.decrypt(XNonce::from_slice(&[0u8; NONCE_LEN]), Payload { msg: stored, aad: VERIFIER_AAD }) {
            Ok(pt) => pt.len() == VERIFIER_MSG_LEN && pt.iter().all(|&b| b == 0),
            Err(_) => false,
        }
    }
}

/// Authenticated data binding a ciphertext to its position and codec.
fn block_aad(block_id: u64, codec: u8) -> [u8; 9] {
    let mut aad = [0u8; 9];
    aad[0..8].copy_from_slice(&block_id.to_le_bytes());
    aad[8] = codec;
    aad
}
