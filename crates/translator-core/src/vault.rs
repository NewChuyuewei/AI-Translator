//! Password-encrypted sensitive settings compatible with the legacy desktop.
//! File permissions and atomic persistence remain platform responsibilities.

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use zeroize::Zeroize;

const VAULT_VERSION: u8 = 1;
const ALGORITHM: &str = "AES-256-GCM/PBKDF2-HMAC-SHA256";
const SECRET_AAD: &[u8] = b"com.gemini.live.translator/secrets/v1";
const PBKDF2_ITERATIONS: u32 = 210_000;
const MIN_PBKDF2_ITERATIONS: u32 = 100_000;
const MAX_PBKDF2_ITERATIONS: u32 = 2_000_000;
pub const MAX_VAULT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretPayload {
    pub api_key: String,
    pub proxy_url: String,
}

impl Drop for SecretPayload {
    fn drop(&mut self) {
        self.api_key.zeroize();
        self.proxy_url.zeroize();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedSecrets {
    version: u8,
    algorithm: String,
    iterations: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

pub fn validate_vault_password(password: &str) -> Result<()> {
    if password.len() < 8 {
        return Err(anyhow!("加密密码至少需要 8 个字符"));
    }
    if password.len() > 1024 {
        return Err(anyhow!("加密密码过长"));
    }
    Ok(())
}

fn derive_key(password: &str, salt: &[u8], iterations: u32) -> Result<[u8; 32]> {
    let rounds = NonZeroU32::new(iterations).ok_or_else(|| anyhow!("加密配置迭代次数无效"))?;
    let mut key = [0_u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        rounds,
        salt,
        password.as_bytes(),
        &mut key,
    );
    Ok(key)
}

/// SWR-VAULT-001/002: emit the established authenticated format without
/// exposing plaintext in the serialized document.
pub fn encrypt_secrets(password: &str, payload: &SecretPayload) -> Result<Vec<u8>> {
    validate_vault_password(password)?;
    if payload.api_key.trim().is_empty() {
        return Err(anyhow!("保存加密配置前请填写 API Key"));
    }
    if payload.api_key.len() > MAX_VAULT_BYTES || payload.proxy_url.len() > MAX_VAULT_BYTES {
        return Err(anyhow!("敏感配置超过安全限制"));
    }

    let random = SystemRandom::new();
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 12];
    random
        .fill(&mut salt)
        .map_err(|_| anyhow!("无法生成加密随机数"))?;
    random
        .fill(&mut nonce)
        .map_err(|_| anyhow!("无法生成加密随机数"))?;

    let mut key_bytes = derive_key(password, &salt, PBKDF2_ITERATIONS)?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| anyhow!("无法初始化加密器"));
    key_bytes.zeroize();
    let key = aead::LessSafeKey::new(unbound?);
    let mut plaintext = serde_json::to_vec(payload).context("无法编码敏感配置")?;
    if plaintext.len() > MAX_VAULT_BYTES {
        plaintext.zeroize();
        return Err(anyhow!("敏感配置超过安全限制"));
    }
    if key
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(SECRET_AAD),
            &mut plaintext,
        )
        .is_err()
    {
        plaintext.zeroize();
        return Err(anyhow!("无法加密敏感配置"));
    }

    let encrypted = EncryptedSecrets {
        version: VAULT_VERSION,
        algorithm: ALGORITHM.into(),
        iterations: PBKDF2_ITERATIONS,
        salt: STANDARD.encode(salt),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(plaintext),
    };
    serde_json::to_vec_pretty(&encrypted).context("无法编码加密配置")
}

/// SWR-VAULT-003/004: bound input before parsing/decoding and collapse wrong
/// password, authentication failure, and invalid plaintext into one safe error.
pub fn decrypt_secrets(password: &str, bytes: &[u8]) -> Result<SecretPayload> {
    validate_vault_password(password)?;
    if bytes.len() > MAX_VAULT_BYTES {
        return Err(anyhow!("加密配置超过 1 MiB 安全限制"));
    }
    let encrypted: EncryptedSecrets =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("加密配置已损坏"))?;
    if encrypted.version != VAULT_VERSION
        || encrypted.algorithm != ALGORITHM
        || !(MIN_PBKDF2_ITERATIONS..=MAX_PBKDF2_ITERATIONS).contains(&encrypted.iterations)
    {
        return Err(anyhow!("不支持的加密配置格式"));
    }

    let salt_bytes = STANDARD
        .decode(&encrypted.salt)
        .map_err(|_| anyhow!("加密配置已损坏"))?;
    let salt: [u8; 16] = salt_bytes
        .try_into()
        .map_err(|_| anyhow!("加密配置已损坏"))?;
    let nonce_bytes = STANDARD
        .decode(&encrypted.nonce)
        .map_err(|_| anyhow!("加密配置已损坏"))?;
    let nonce: [u8; 12] = nonce_bytes
        .try_into()
        .map_err(|_| anyhow!("加密配置已损坏"))?;
    let mut ciphertext = STANDARD
        .decode(&encrypted.ciphertext)
        .map_err(|_| anyhow!("加密配置已损坏"))?;
    if ciphertext.len() > MAX_VAULT_BYTES + aead::AES_256_GCM.tag_len() {
        ciphertext.zeroize();
        return Err(anyhow!("加密配置已损坏"));
    }

    let mut key_bytes = derive_key(password, &salt, encrypted.iterations)?;
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &key_bytes)
        .map_err(|_| anyhow!("无法初始化解密器"));
    key_bytes.zeroize();
    let key = aead::LessSafeKey::new(unbound?);
    let plaintext_len = match key.open_in_place(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(SECRET_AAD),
        &mut ciphertext,
    ) {
        Ok(plaintext) => plaintext.len(),
        Err(_) => {
            ciphertext.zeroize();
            return Err(anyhow!("密码错误或加密配置已损坏"));
        }
    };
    let result = serde_json::from_slice(&ciphertext[..plaintext_len])
        .map_err(|_| anyhow!("密码错误或加密配置已损坏"));
    ciphertext.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> SecretPayload {
        SecretPayload {
            api_key: "PRIVATE-KEY".into(),
            proxy_url: "socks5://alice:password@127.0.0.1:1080".into(),
        }
    }

    #[test]
    fn swr_vault_001_round_trip_preserves_legacy_format_without_plaintext() {
        let payload = payload();
        let encrypted =
            encrypt_secrets("correct horse battery staple", &payload).expect("encrypt fixture");
        let text = String::from_utf8(encrypted.clone()).expect("vault JSON is UTF-8");
        assert!(text.contains(ALGORITHM));
        assert!(!text.contains("PRIVATE-KEY"));
        assert!(!text.contains("alice:password"));
        let decrypted =
            decrypt_secrets("correct horse battery staple", &encrypted).expect("decrypt fixture");
        assert_eq!(decrypted, payload);
    }

    #[test]
    fn swr_vault_002_random_nonce_prevents_repeat_ciphertext() {
        let first =
            encrypt_secrets("correct horse battery staple", &payload()).expect("first encryption");
        let second =
            encrypt_secrets("correct horse battery staple", &payload()).expect("second encryption");
        assert_ne!(first, second);
    }

    #[test]
    fn swr_vault_003_wrong_password_and_tampering_share_safe_error() {
        let encrypted =
            encrypt_secrets("correct horse battery staple", &payload()).expect("encrypt fixture");
        let wrong = decrypt_secrets("incorrect password", &encrypted)
            .expect_err("wrong password must fail")
            .to_string();
        let mut value: serde_json::Value =
            serde_json::from_slice(&encrypted).expect("parse fixture");
        value["ciphertext"] = serde_json::json!(STANDARD.encode([0_u8; 32]));
        let tampered = serde_json::to_vec(&value).expect("encode tampered fixture");
        let invalid = decrypt_secrets("correct horse battery staple", &tampered)
            .expect_err("tampering must fail")
            .to_string();
        assert_eq!(wrong, "密码错误或加密配置已损坏");
        assert_eq!(invalid, wrong);
        assert!(!wrong.contains("incorrect"));
    }

    #[test]
    fn swr_vault_004_bounds_password_input_and_format_parameters() {
        assert!(encrypt_secrets("short", &payload()).is_err());
        assert!(
            decrypt_secrets(
                "correct horse battery staple",
                &vec![b' '; MAX_VAULT_BYTES + 1]
            )
            .is_err()
        );
        let encrypted =
            encrypt_secrets("correct horse battery staple", &payload()).expect("encrypt fixture");
        let mut value: serde_json::Value =
            serde_json::from_slice(&encrypted).expect("parse fixture");
        value["iterations"] = serde_json::json!(99_999);
        let invalid = serde_json::to_vec(&value).expect("encode invalid fixture");
        assert!(decrypt_secrets("correct horse battery staple", &invalid).is_err());
    }
}
