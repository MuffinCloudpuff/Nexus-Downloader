use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose};

const PSK: &[u8; 32] = b"GD3_LOCAL_SECURE_KEY_2026_YOUTUB"; // 32 bytes

pub fn decrypt_cookies(encrypted_base64: &str, iv_base64: &str) -> Option<String> {
    let encrypted_bytes = general_purpose::STANDARD.decode(encrypted_base64).ok()?;
    let iv_bytes = general_purpose::STANDARD.decode(iv_base64).ok()?;

    if iv_bytes.len() != 12 {
        tracing::error!("Invalid IV length: {}", iv_bytes.len());
        return None;
    }

    let cipher = Aes256Gcm::new(PSK.into());
    let nonce = Nonce::from_slice(&iv_bytes);

    match cipher.decrypt(nonce, encrypted_bytes.as_ref()) {
        Ok(decrypted_bytes) => String::from_utf8(decrypted_bytes).ok(),
        Err(e) => {
            tracing::error!("Failed to decrypt cookies: {:?}", e);
            None
        }
    }
}

pub fn encrypt_cookies(cookie_text: &str) -> Option<(String, String)> {
    let mut iv_bytes = [0_u8; 12];
    getrandom::fill(&mut iv_bytes).ok()?;

    let cipher = Aes256Gcm::new(PSK.into());
    let nonce = Nonce::from_slice(&iv_bytes);
    let encrypted = match cipher.encrypt(nonce, cookie_text.as_bytes()) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!("Failed to encrypt cookies: {:?}", error);
            return None;
        }
    };

    Some((
        general_purpose::STANDARD.encode(encrypted),
        general_purpose::STANDARD.encode(iv_bytes),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_crypto_roundtrip() {
        let (encrypted, iv) = encrypt_cookies("SESSDATA=secret").unwrap();
        assert_ne!(encrypted, "SESSDATA=secret");
        assert_eq!(
            decrypt_cookies(&encrypted, &iv),
            Some("SESSDATA=secret".to_string())
        );
    }
}
