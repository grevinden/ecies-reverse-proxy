use aead::Aead;
use base64::Engine;
use chacha20poly1305::{aead, ChaCha20Poly1305, KeyInit, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Debug)]
pub enum Error {
    Base64(String),
    InvalidLength,
    InvalidKey,
    Decryption,
    Utf8,
    Hkdf,
    InvalidNonce,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Base64(e) => write!(f, "Base64 error: {}", e),
            Error::InvalidLength => write!(f, "Invalid encrypted data length"),
            Error::InvalidKey => write!(f, "Invalid key"),
            Error::Decryption => write!(f, "Decryption failed"),
            Error::Utf8 => write!(f, "Invalid UTF-8"),
            Error::Hkdf => write!(f, "HKDF error"),
            Error::InvalidNonce => write!(f, "Invalid nonce"),
        }
    }
}

impl std::error::Error for Error {}

pub fn decrypt(encrypted_b64: &str, secret_key: &[u8; 32]) -> Result<String, Error> {
    let encrypted_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encrypted_b64)
        .map_err(|e| Error::Base64(e.to_string()))?;
    if encrypted_bytes.len() < 32 + 12 + 16 {
        return Err(Error::InvalidLength);
    }
    let (ephemeral_public_bytes, rest) = encrypted_bytes.split_at(32);
    let (nonce_bytes, ciphertext) = rest.split_at(12);

    let ephemeral_public = PublicKey::from(
        <[u8; 32]>::try_from(ephemeral_public_bytes)
            .map_err(|_| Error::InvalidKey)?,
    );

    let receiver_secret = StaticSecret::from(*secret_key);
    let shared_secret = receiver_secret.diffie_hellman(&ephemeral_public);

    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut symmetric_key = [0u8; 32];
    hkdf.expand(b"ecies-chacha20-poly1305", &mut symmetric_key)
        .map_err(|_| Error::Hkdf)?;

    let cipher = ChaCha20Poly1305::new_from_slice(&symmetric_key)
        .map_err(|_| Error::InvalidKey)?;

    // Используем TryFrom вместо устаревшего from_slice
    let nonce = Nonce::try_from(nonce_bytes).map_err(|_| Error::InvalidNonce)?;

    let plaintext = cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| Error::Decryption)?;

    String::from_utf8(plaintext).map_err(|_| Error::Utf8)
}