use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use sha2::Sha256;
use std::error::Error;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

pub fn encrypt(data: &[u8], password: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill(&mut salt);

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, 100_000, &mut key);

    let cipher = Aes256Gcm::new(&key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, data).map_err(|_| "Encryption failed")?;

    let mut output = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

pub fn decrypt(encrypted_data: &[u8], password: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    if encrypted_data.len() < SALT_LEN + NONCE_LEN {
        return Err("Invalid data length".into());
    }

    let salt = &encrypted_data[0..SALT_LEN];
    let nonce_bytes = &encrypted_data[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &encrypted_data[SALT_LEN + NONCE_LEN..];

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut key);

    let cipher = Aes256Gcm::new(&key.into());
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed - incorrect password or corrupted data")?;

    Ok(plaintext)
}