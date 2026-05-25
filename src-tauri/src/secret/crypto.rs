//! 内部 secret 加密 — 用主密钥（OnceLock 缓存）直接 AEAD 加密 secret 入 DB。
//!
//! 跟 src/crypto.rs（GitHub Sync 备份）区别：
//!   - 这里：固定主密钥（一次性生成 + keychain/file 持久化 + 进程内 OnceLock 缓存），无 KDF
//!   - 备份：每次 export 用用户输入密码 + 新 salt，Argon2id KDF 派生 key
//!
//! Wire format:  "enc:v1:" + base64(nonce[12] || ciphertext_with_tag)
//!   - "enc:v1:" 前缀：未来 v2 换算法时旧密文仍可识别（拒绝并明确 v1，
//!     不会被误当 base64 噪声乱解），也能让肉眼或日志一眼分辨"是加密过的"
//!   - nonce 12 字节：ChaCha20-Poly1305 标准 (RFC 8439)
//!   - tag 16 字节：自动 append 在 ct 末尾（aead crate 默认行为）
//!
//! 选 ChaCha20-Poly1305 而不是 AES-256-GCM：
//!   - 项目已依赖 chacha20poly1305 (GitHub Sync 用)，避免引第二个加密算法
//!   - 软件实现快、移动端不依赖 AES-NI、跨平台一致

use serde_json::json;

use crate::error::{AppError, AppResult};

const ENC_PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const MIN_BLOB: usize = NONCE_LEN + TAG_LEN;
pub const MASTER_KEY_LEN: usize = 32;

pub fn encrypt(master_key: &[u8; MASTER_KEY_LEN], plaintext: &[u8]) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};

    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| AppError::other("secret_rng_failed", json!({ "err": e.to_string() })))?;
    let cipher = ChaCha20Poly1305::new(master_key.into());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct_with_tag = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| AppError::other("secret_encrypt_failed", json!({})))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ct_with_tag.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct_with_tag);
    Ok(format!("{}{}", ENC_PREFIX, STANDARD.encode(&blob)))
}

pub fn decrypt(master_key: &[u8; MASTER_KEY_LEN], stored: &str) -> AppResult<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};

    let body = stored
        .strip_prefix(ENC_PREFIX)
        .ok_or_else(|| AppError::other("secret_format_unknown", json!({})))?;
    let blob = STANDARD
        .decode(body)
        .map_err(|e| AppError::other("secret_b64_decode_failed", json!({ "err": e.to_string() })))?;
    if blob.len() < MIN_BLOB {
        return Err(AppError::other("secret_blob_too_short", json!({})));
    }
    let (nonce_bytes, ct_with_tag) = blob.split_at(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new(master_key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct_with_tag)
        .map_err(|_| AppError::other("secret_decrypt_failed_or_wrong_key", json!({})))
}

/// 判断 stored 是否是 v1 密文格式。迁移代码用：旧 keychain 值是明文，
/// 新 DB 值带 "enc:v1:" 前缀；用这个函数辨别即可幂等。
pub fn is_encrypted_v1(stored: &str) -> bool {
    stored.starts_with(ENC_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; MASTER_KEY_LEN] {
        let mut k = [0u8; MASTER_KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn roundtrip_ascii() {
        let k = key();
        let blob = encrypt(&k, b"hello world").unwrap();
        let back = decrypt(&k, &blob).unwrap();
        assert_eq!(back, b"hello world");
    }

    #[test]
    fn roundtrip_empty() {
        // AEAD 必须能加解密空 plaintext（仍带 16B tag）
        let k = key();
        let blob = encrypt(&k, b"").unwrap();
        let back = decrypt(&k, &blob).unwrap();
        assert_eq!(back, b"");
    }

    #[test]
    fn roundtrip_long_payload() {
        // 模拟 RSA 4096 PEM 约 3300 字符长度
        let k = key();
        let plaintext = "x".repeat(3500);
        let blob = encrypt(&k, plaintext.as_bytes()).unwrap();
        let back = decrypt(&k, &blob).unwrap();
        assert_eq!(back, plaintext.as_bytes());
    }

    #[test]
    fn wrong_key_rejected_by_aead() {
        let k1 = key();
        let mut k2 = key();
        k2[0] ^= 0xff;
        let blob = encrypt(&k1, b"secret").unwrap();
        let err = decrypt(&k2, &blob).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let k = key();
        let blob = encrypt(&k, b"hello world").unwrap();
        let body = blob.strip_prefix("enc:v1:").unwrap();
        let mut raw = STANDARD.decode(body).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = format!("enc:v1:{}", STANDARD.encode(&raw));
        let err = decrypt(&k, &tampered).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn tampered_nonce_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let k = key();
        let blob = encrypt(&k, b"hi").unwrap();
        let body = blob.strip_prefix("enc:v1:").unwrap();
        let mut raw = STANDARD.decode(body).unwrap();
        raw[0] ^= 0x01;
        let tampered = format!("enc:v1:{}", STANDARD.encode(&raw));
        let err = decrypt(&k, &tampered).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn ciphertext_differs_across_invocations() {
        // 同 key 同明文加密两次：nonce 随机 → 密文必不同
        let k = key();
        let a = encrypt(&k, b"same").unwrap();
        let b = encrypt(&k, b"same").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&k, &a).unwrap(), b"same");
        assert_eq!(decrypt(&k, &b).unwrap(), b"same");
    }

    #[test]
    fn unknown_prefix_rejected() {
        // 没 "enc:v1:" 前缀 → 视为明文/未知格式，明确报错而不是乱解
        let k = key();
        let err = decrypt(&k, "rawplaintext").unwrap_err();
        assert_eq!(err.code(), "secret_format_unknown");
    }

    #[test]
    fn invalid_base64_rejected() {
        let k = key();
        let err = decrypt(&k, "enc:v1:!!!not base64!!!").unwrap_err();
        assert_eq!(err.code(), "secret_b64_decode_failed");
    }

    #[test]
    fn blob_too_short_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        // 合法 blob 下限 = 28 字节（nonce 12 + tag 16）；< 28 必拒
        let raw = vec![0u8; 20];
        let bad = format!("enc:v1:{}", STANDARD.encode(&raw));
        let k = key();
        let err = decrypt(&k, &bad).unwrap_err();
        assert_eq!(err.code(), "secret_blob_too_short");
    }

    #[test]
    fn is_encrypted_v1_recognizes_format() {
        assert!(is_encrypted_v1("enc:v1:abc"));
        assert!(!is_encrypted_v1("plaintext"));
        assert!(!is_encrypted_v1("enc:v2:abc"));
        assert!(!is_encrypted_v1(""));
    }
}
