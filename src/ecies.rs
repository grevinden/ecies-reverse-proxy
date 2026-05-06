use base64::Engine;
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

pub fn decrypt(encrypted_b64: &str, secret_key: &[u8; 32]) -> Result<String, String> {
    let encrypted_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encrypted_b64)
        .map_err(|_| "Invalid base64".to_string())?;
    if encrypted_bytes.len() < 32 + 12 + 16 {
        return Err("Too short".to_string());
    }
    let (ephemeral_public_bytes, rest) = encrypted_bytes.split_at(32);
    let (nonce_bytes, ciphertext) = rest.split_at(12);

    let ephemeral_public = PublicKey::from(
        <[u8; 32]>::try_from(ephemeral_public_bytes)
            .map_err(|_| "Invalid ephemeral key")?,
    );
    let receiver_secret = StaticSecret::from(*secret_key);
    let shared_secret = receiver_secret.diffie_hellman(&ephemeral_public);

    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut symmetric_key = [0u8; 32];
    hkdf.expand(b"ecies-chacha20-poly1305", &mut symmetric_key)
        .map_err(|_| "HKDF error")?;

    let cipher = ChaCha20Poly1305::new_from_slice(&symmetric_key)
        .map_err(|_| "Invalid key")?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;

    String::from_utf8(plaintext).map_err(|e| format!("Invalid UTF-8: {}", e))
}