use serde_json::json;

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// ConfigCrypto — 加密配置备份
// Wire format: base64( salt[16] + hmac[32] + ciphertext )
// KDF: SHA256(password || salt), then 999× SHA256(prev || salt)
// Stream: SHA256(key || counter_le32) blocks, XOR plaintext
// Auth: HMAC-SHA256(key, ciphertext)
// ---------------------------------------------------------------------------

fn derive_key(password: &str, salt: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut k = Sha256::digest([password.as_bytes(), salt].concat()).to_vec();
    for _ in 1..1000 {
        k = Sha256::digest([k.as_slice(), salt].concat()).to_vec();
    }
    k
}

fn keystream(key: &[u8], length: usize) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut out = Vec::with_capacity(length + 32);
    let mut ctr: u32 = 0;
    while out.len() < length {
        let block = Sha256::digest([key, &ctr.to_le_bytes()].concat());
        out.extend_from_slice(&block);
        ctr += 1;
    }
    out.truncate(length);
    out
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

pub fn encrypt(json: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let salt = {
        let mut buf = [0u8; 16];
        getrandom::getrandom(&mut buf).map_err(|e| AppError::other("crypto_rng_failed", json!({ "err": e.to_string() })))?;
        buf
    };

    let key = derive_key(password, &salt);
    let plain = json.as_bytes();
    let ks = keystream(&key, plain.len());
    let cipher: Vec<u8> = plain.iter().zip(ks.iter()).map(|(p, k)| p ^ k).collect();
    let mac = hmac_sha256(&key, &cipher);

    let mut out = Vec::with_capacity(16 + 32 + cipher.len());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&mac);
    out.extend_from_slice(&cipher);
    Ok(STANDARD.encode(&out))
}

pub fn decrypt(b64: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let data = STANDARD
        .decode(b64)
        .map_err(|e| AppError::config("crypto_base64_decode_failed", json!({ "err": e.to_string() })))?;
    // 16 salt + 32 mac + cipher(≥0)。空明文产出的合法 blob 是 48 字节；下限是 48。
    if data.len() < 48 {
        return Err(AppError::config("crypto_invalid_payload", json!({})));
    }

    let salt = &data[0..16];
    let stored_mac = &data[16..48];
    let cipher = &data[48..];
    let key = derive_key(password, salt);

    let expected = hmac_sha256(&key, cipher);
    let mut diff = 0u8;
    for (a, b) in stored_mac.iter().zip(expected.iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err(AppError::config("crypto_password_or_corrupted", json!({})));
    }

    let ks = keystream(&key, cipher.len());
    let plain: Vec<u8> = cipher.iter().zip(ks.iter()).map(|(c, k)| c ^ k).collect();
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
        // 多字节 UTF-8 + 长 payload，覆盖 keystream 多 block 路径
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
    fn wrong_password_rejected_by_mac() {
        let blob = encrypt("secret", "right").unwrap();
        let err = decrypt(&blob, "wrong").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn tampered_ciphertext_rejected_by_mac() {
        // 篡改 cipher 部分（offset >= 48），mac 校验必须先于解密失败
        let blob = encrypt("hello world", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = STANDARD.encode(&raw);
        let err = decrypt(&tampered, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_password_or_corrupted");
    }

    #[test]
    fn tampered_mac_rejected() {
        let blob = encrypt("hi", "pw").unwrap();
        let mut raw = STANDARD.decode(&blob).unwrap();
        // mac 在 [16, 48)
        raw[16] ^= 0x01;
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
        // 合法 blob 下限 = 48（salt 16 + mac 32 + cipher 0+）；< 48 必拒
        let raw = vec![0u8; 40];
        let blob = STANDARD.encode(&raw);
        let err = decrypt(&blob, "pw").unwrap_err();
        assert_eq!(err.code(), "crypto_invalid_payload");
    }

    #[test]
    fn ciphertext_differs_across_invocations() {
        // 随机 salt 让两次 encrypt 同一明文 + 同一密码产出不同 blob
        let a = encrypt("same", "pw").unwrap();
        let b = encrypt("same", "pw").unwrap();
        assert_ne!(a, b);
        // 两个 blob 都能 decrypt 回原文
        assert_eq!(decrypt(&a, "pw").unwrap(), "same");
        assert_eq!(decrypt(&b, "pw").unwrap(), "same");
    }
}
