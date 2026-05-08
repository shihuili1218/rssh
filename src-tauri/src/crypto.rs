use serde_json::json;

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// 配置备份加密 — v2 wire format
//
//   base64( version[1] || salt[16] || nonce[12] || ciphertext_with_tag )
//
//   version = 0x02
//   KDF     = Argon2id（RustCrypto `argon2` 0.5 默认参数：
//             m_cost=19456 KiB, t_cost=2, p_cost=1，符合 OWASP 推荐）
//   AEAD    = ChaCha20-Poly1305（key 32B, nonce 12B, tag 16B）
//
// v1（手搓 SHA256 KDF + 手搓流密码）已废弃，旧 blob 无法解密。
// ---------------------------------------------------------------------------

const V2: u8 = 0x02;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// 解密合法 blob 的最小字节数：版本 + salt + nonce + tag（空明文也带 16 字节 tag）。
const MIN_BLOB: usize = 1 + SALT_LEN + NONCE_LEN + TAG_LEN;

fn random(buf: &mut [u8]) -> AppResult<()> {
    getrandom::getrandom(buf)
        .map_err(|e| AppError::other("crypto_rng_failed", json!({ "err": e.to_string() })))
}

fn derive_key(password: &str, salt: &[u8]) -> AppResult<[u8; KEY_LEN]> {
    use argon2::Argon2;
    let mut key = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::other("crypto_kdf_failed", json!({ "err": e.to_string() })))?;
    Ok(key)
}

pub fn encrypt(json_text: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};

    let mut salt = [0u8; SALT_LEN];
    random(&mut salt)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    random(&mut nonce_bytes)?;

    let key = derive_key(password, &salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct_with_tag = cipher
        .encrypt(nonce, json_text.as_bytes())
        .map_err(|_| AppError::other("crypto_encrypt_failed", json!({})))?;

    let mut out = Vec::with_capacity(MIN_BLOB + json_text.len());
    out.push(V2);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct_with_tag);
    Ok(STANDARD.encode(&out))
}

pub fn decrypt(b64: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};

    let data = STANDARD
        .decode(b64)
        .map_err(|e| AppError::config("crypto_base64_decode_failed", json!({ "err": e.to_string() })))?;
    if data.len() < MIN_BLOB {
        return Err(AppError::config("crypto_invalid_payload", json!({})));
    }
    if data[0] != V2 {
        return Err(AppError::config(
            "crypto_unsupported_version",
            json!({ "version": data[0] }),
        ));
    }

    let salt = &data[1..1 + SALT_LEN];
    let nonce_bytes = &data[1 + SALT_LEN..1 + SALT_LEN + NONCE_LEN];
    let ct_with_tag = &data[1 + SALT_LEN + NONCE_LEN..];

    let key = derive_key(password, salt)?;
    let cipher = ChaCha20Poly1305::new((&key).into());
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct_with_tag)
        .map_err(|_| AppError::config("crypto_password_or_corrupted", json!({})))?;

    String::from_utf8(plain).map_err(|_| AppError::config("crypto_decrypt_not_utf8", json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test]
    fn roundtrip_ascii() {
        let plaintext = r#"{"name":"alpha","port":22}"#;
        let blob = encrypt(plaintext, "passw0rd!").unwrap();
        let recovered = decrypt(&blob, "passw0rd!").unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn roundtrip_unicode_and_long_payload() {
        // 多字节 UTF-8 + 长 payload，覆盖 AEAD 多 block 路径
        let plaintext = "中文测试 🦀 ".repeat(2000);
        let blob = encrypt(&plaintext, "p").unwrap();
        let recovered = decrypt(&blob, "p").unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn roundtrip_empty_payload() {
        let blob = encrypt("", "anything").unwrap();
        let recovered = decrypt(&blob, "anything").unwrap();
        assert_eq!(recovered, "");
    }

    #[test]
    fn wrong_password_rejected_by_aead() {
        let blob = encrypt("secret", "right").unwrap();
        let err = decrypt(&blob, "wrong").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn tampered_ciphertext_rejected_by_aead() {
        // 篡改 ciphertext 区段，AEAD tag 校验必须失败
        let blob = encrypt("hello world", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = STANDARD.encode(&raw);
        let err = decrypt(&tampered, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn tampered_nonce_rejected() {
        let blob = encrypt("hi", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        // nonce 区段在 [1+16, 1+16+12)，翻一个 bit
        raw[1 + 16] ^= 0x01;
        let tampered = STANDARD.encode(&raw);
        let err = decrypt(&tampered, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn tampered_salt_rejected() {
        // salt 改了 → 派生出不同 key → AEAD 校验必失败
        let blob = encrypt("hi", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        raw[1] ^= 0x01;
        let tampered = STANDARD.encode(&raw);
        let err = decrypt(&tampered, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn invalid_base64_rejected() {
        let err = decrypt("!!!not base64!!!", "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_base64_decode_failed");
    }

    #[test]
    fn payload_too_short_rejected() {
        // 合法 blob 下限 = 45 字节（version+salt+nonce+tag）；< 45 必拒
        let raw = vec![0u8; 40];
        let blob = STANDARD.encode(&raw);
        let err = decrypt(&blob, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_invalid_payload");
    }

    #[test]
    fn unsupported_version_rejected() {
        // 非 0x02 版本字节立刻拒绝。覆盖未来 v3 / v1 残留 / 随机噪声开头。
        let blob = encrypt("x", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        raw[0] = 0x01; // 假装是 v1
        let bad = STANDARD.encode(&raw);
        let err = decrypt(&bad, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_unsupported_version");
    }

    #[test]
    fn ciphertext_differs_across_invocations() {
        // 随机 salt + 随机 nonce 让两次 encrypt 同明文同密码产出不同 blob
        let a = encrypt("same", "pw").unwrap();
        let b = encrypt("same", "pw").unwrap();
        assert_ne!(a, b);
        // 两个 blob 都能 decrypt 回原文
        assert_eq!(decrypt(&a, "pw").unwrap(), "same");
        assert_eq!(decrypt(&b, "pw").unwrap(), "same");
    }

    #[test]
    fn version_byte_is_v2() {
        // wire format 钉死：第一个字节 = V2
        let blob = encrypt("anything", "pw").unwrap();
        let raw = STANDARD.decode(&blob).unwrap();
        assert_eq!(raw[0], super::V2);
        assert_eq!(super::V2, 0x02);
    }
}
