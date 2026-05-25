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
//! **AAD 绑定 key name**：encrypt/decrypt 把 `key_name`（如 `cred:abc:secret`）作
//! AEAD associated data 喂进去。这样**密文跟它所属的 key 绑死**：
//!   - 不带 AAD 的旧设计，attacker 只要能写 SQLite 就能 cut-and-paste —— 把
//!     `cred:A:secret` 的密文复制到 `cred:B:secret` 行，用户登录 B profile 时
//!     解出来是 A 的密码（AEAD tag 还能通过，因为 master key 相同）。
//!   - 加 AAD 之后，cross-key 替换会让 decrypt 的 tag 验证失败（不同 AAD），
//!     用户会得到明确的 `secret_decrypt_failed_or_wrong_key`，不会被静默欺骗。
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

pub fn encrypt(
    master_key: &[u8; MASTER_KEY_LEN],
    key_name: &str,
    plaintext: &[u8],
) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{
        aead::{Aead, Payload},
        ChaCha20Poly1305, KeyInit, Nonce,
    };

    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| AppError::other("secret_rng_failed", json!({ "err": e.to_string() })))?;
    let cipher = ChaCha20Poly1305::new(master_key.into());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let payload = Payload {
        msg: plaintext,
        aad: key_name.as_bytes(),
    };
    let ct_with_tag = cipher
        .encrypt(nonce, payload)
        .map_err(|_| AppError::other("secret_encrypt_failed", json!({})))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ct_with_tag.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ct_with_tag);
    Ok(format!("{}{}", ENC_PREFIX, STANDARD.encode(&blob)))
}

pub fn decrypt(
    master_key: &[u8; MASTER_KEY_LEN],
    key_name: &str,
    stored: &str,
) -> AppResult<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{
        aead::{Aead, Payload},
        ChaCha20Poly1305, KeyInit, Nonce,
    };

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
    let payload = Payload {
        msg: ct_with_tag,
        aad: key_name.as_bytes(),
    };
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), payload)
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

    const AAD: &str = "cred:test:secret";

    #[test]
    fn roundtrip_ascii() {
        let k = key();
        let blob = encrypt(&k, AAD, b"hello world").unwrap();
        let back = decrypt(&k, AAD, &blob).unwrap();
        assert_eq!(back, b"hello world");
    }

    #[test]
    fn roundtrip_empty() {
        // AEAD 必须能加解密空 plaintext（仍带 16B tag）
        let k = key();
        let blob = encrypt(&k, AAD, b"").unwrap();
        let back = decrypt(&k, AAD, &blob).unwrap();
        assert_eq!(back, b"");
    }

    #[test]
    fn roundtrip_long_payload() {
        // 模拟 RSA 4096 PEM 约 3300 字符长度
        let k = key();
        let plaintext = "x".repeat(3500);
        let blob = encrypt(&k, AAD, plaintext.as_bytes()).unwrap();
        let back = decrypt(&k, AAD, &blob).unwrap();
        assert_eq!(back, plaintext.as_bytes());
    }

    #[test]
    fn wrong_key_rejected_by_aead() {
        let k1 = key();
        let mut k2 = key();
        k2[0] ^= 0xff;
        let blob = encrypt(&k1, AAD, b"secret").unwrap();
        let err = decrypt(&k2, AAD, &blob).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    /// 核心安全护栏：cross-key cut-and-paste 攻击必须被 AEAD tag 验证拒绝。
    /// 场景：attacker 把 `cred:A:secret` 行的密文 raw 复制到 `cred:B:secret` 行
    /// （能改 DB 但读不到 master key 的威胁模型）。decrypt 时 AAD 不一致 → tag fail。
    #[test]
    fn cross_key_swap_rejected() {
        let k = key();
        let blob = encrypt(&k, "cred:A:secret", b"password-of-A").unwrap();
        // attacker 用 B 的 AAD 解 A 的密文 — 必须拒绝
        let err = decrypt(&k, "cred:B:secret", &blob).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn different_aad_yields_different_ciphertext() {
        // 同 key 同明文，不同 AAD，密文必不同；否则 AAD 没起作用。
        let k = key();
        let a = encrypt(&k, "k1", b"same").unwrap();
        let b = encrypt(&k, "k2", b"same").unwrap();
        // body（密文部分）的差异源于 tag 不同（AAD 进 GHASH/Poly1305）
        // 简化断言：用 k1 的 AAD 解 b 应失败
        assert!(decrypt(&k, "k1", &b).is_err());
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let k = key();
        let blob = encrypt(&k, AAD, b"hello world").unwrap();
        let body = blob.strip_prefix("enc:v1:").unwrap();
        let mut raw = STANDARD.decode(body).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = format!("enc:v1:{}", STANDARD.encode(&raw));
        let err = decrypt(&k, AAD, &tampered).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn tampered_nonce_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let k = key();
        let blob = encrypt(&k, AAD, b"hi").unwrap();
        let body = blob.strip_prefix("enc:v1:").unwrap();
        let mut raw = STANDARD.decode(body).unwrap();
        raw[0] ^= 0x01;
        let tampered = format!("enc:v1:{}", STANDARD.encode(&raw));
        let err = decrypt(&k, AAD, &tampered).unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn ciphertext_differs_across_invocations() {
        // 同 key 同明文加密两次：nonce 随机 → 密文必不同
        let k = key();
        let a = encrypt(&k, AAD, b"same").unwrap();
        let b = encrypt(&k, AAD, b"same").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&k, AAD, &a).unwrap(), b"same");
        assert_eq!(decrypt(&k, AAD, &b).unwrap(), b"same");
    }

    #[test]
    fn unknown_prefix_rejected() {
        // 没 "enc:v1:" 前缀 → 视为明文/未知格式，明确报错而不是乱解
        let k = key();
        let err = decrypt(&k, AAD, "rawplaintext").unwrap_err();
        assert_eq!(err.code(), "secret_format_unknown");
    }

    #[test]
    fn invalid_base64_rejected() {
        let k = key();
        let err = decrypt(&k, AAD, "enc:v1:!!!not base64!!!").unwrap_err();
        assert_eq!(err.code(), "secret_b64_decode_failed");
    }

    #[test]
    fn blob_too_short_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        // 合法 blob 下限 = 28 字节（nonce 12 + tag 16）；< 28 必拒
        let raw = vec![0u8; 20];
        let bad = format!("enc:v1:{}", STANDARD.encode(&raw));
        let k = key();
        let err = decrypt(&k, AAD, &bad).unwrap_err();
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
