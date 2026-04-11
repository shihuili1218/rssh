use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// ConfigCrypto — 加密配置备份
// Wire format: base64( salt[16] + hmac[32] + ciphertext )
// KDF: SHA256(password || salt), then 999× SHA256(prev || salt)
// Stream: SHA256(key || counter_le32) blocks, XOR plaintext
// Auth: HMAC-SHA256(key, ciphertext)
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "android"))]
fn derive_key(password: &str, salt: &[u8]) -> Vec<u8> {
    use sha2::{Sha256, Digest};
    let mut k = Sha256::digest([password.as_bytes(), salt].concat()).to_vec();
    for _ in 1..1000 {
        k = Sha256::digest([k.as_slice(), salt].concat()).to_vec();
    }
    k
}

#[cfg(not(target_os = "android"))]
fn keystream(key: &[u8], length: usize) -> Vec<u8> {
    use sha2::{Sha256, Digest};
    let mut out = Vec::with_capacity(length + 32);
    let mut ctr: u32 = 0;
    while out.len() < length {
        let block = Sha256::digest([
            key,
            &ctr.to_le_bytes(),
        ].concat());
        out.extend_from_slice(&block);
        ctr += 1;
    }
    out.truncate(length);
    out
}

#[cfg(not(target_os = "android"))]
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(not(target_os = "android"))]
pub fn encrypt(json: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let salt = {
        let mut buf = [0u8; 16];
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut s = seed;
        for b in buf.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *b = (s >> 33) as u8;
        }
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

#[cfg(not(target_os = "android"))]
pub fn decrypt(b64: &str, password: &str) -> AppResult<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let data = STANDARD.decode(b64)
        .map_err(|e| AppError::Config(format!("Base64 解码失败: {e}")))?;
    if data.len() < 49 {
        return Err(AppError::Config("Invalid encrypted config".into()));
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
        return Err(AppError::Config("密码错误或数据损坏".into()));
    }

    let ks = keystream(&key, cipher.len());
    let plain: Vec<u8> = cipher.iter().zip(ks.iter()).map(|(c, k)| c ^ k).collect();
    String::from_utf8(plain).map_err(|_| AppError::Config("解密数据非 UTF-8".into()))
}
