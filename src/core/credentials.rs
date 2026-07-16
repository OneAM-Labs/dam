use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use pbkdf2::pbkdf2_hmac;
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Credential {
    pub alias: String,
    pub cred_type: String,
    pub secret: String,
    pub extra: Option<String>,
}

fn global_dir() -> PathBuf {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let path = PathBuf::from(home).join(".damconfig");
    fs::create_dir_all(&path).unwrap_or_default();
    path
}

fn index_path() -> PathBuf {
    global_dir().join("aliases.json")
}

pub fn load_aliases() -> Vec<String> {
    if let Ok(data) = fs::read_to_string(index_path()) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn add_alias(alias: &str) {
    let mut aliases = load_aliases();
    if !aliases.contains(&alias.to_string()) {
        aliases.push(alias.to_string());
        let _ = fs::write(index_path(), serde_json::to_string(&aliases).unwrap_or_default());
    }
}

fn remove_alias(alias: &str) {
    let mut aliases = load_aliases();
    aliases.retain(|a| a != alias);
    let _ = fs::write(index_path(), serde_json::to_string(&aliases).unwrap_or_default());
}

fn get_vault_password() -> String {
    if let Ok(pwd) = env::var("DAM_VAULT_PASSWORD") {
        return pwd;
    }
    rpassword::prompt_password("Enter Vault Master Password: ").unwrap_or_default()
}

pub fn encrypt_vault(data: &str, password: &str) -> Vec<u8> {
    let mut salt = [0u8; 16];
    thread_rng().fill_bytes(&mut salt);
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, 100_000, &mut key);

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let mut nonce_bytes = [0u8; 12];
    thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher.encrypt(nonce, data.as_bytes()).expect("encryption failure");

    let mut out = Vec::new();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    out
}

pub fn decrypt_vault(data: &[u8], password: &str) -> Result<String, String> {
    if data.len() < 28 {
        return Err("Invalid vault data".into());
    }
    let salt = &data[0..16];
    let nonce_bytes = &data[16..28];
    let ciphertext = &data[28..];

    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut key);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Invalid password or corrupted vault")?;
    String::from_utf8(plaintext).map_err(|_| "Invalid UTF-8 in vault".into())
}

fn load_vault(password: &str) -> Result<HashMap<String, Credential>, String> {
    let path = global_dir().join("vault.bin");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let json = decrypt_vault(&data, password)?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

fn save_vault(map: &HashMap<String, Credential>, password: &str) -> Result<(), String> {
    let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
    let encrypted = encrypt_vault(&json, password);
    fs::write(global_dir().join("vault.bin"), encrypted).map_err(|e| e.to_string())
}

pub fn save_credential(cred: Credential, use_vault: bool) -> Result<(), String> {
    if use_vault {
        let pwd = get_vault_password();
        let mut map = load_vault(&pwd).unwrap_or_default();
        map.insert(cred.alias.clone(), cred.clone());
        save_vault(&map, &pwd)?;
        add_alias(&cred.alias);
        Ok(())
    } else {
        let entry = keyring::Entry::new("dam_cli", &cred.alias).map_err(|e| e.to_string())?;
        let json = serde_json::to_string(&cred).map_err(|e| e.to_string())?;
        entry.set_password(&json).map_err(|e| e.to_string())?;
        add_alias(&cred.alias);
        Ok(())
    }
}

pub fn get_credential(alias: &str, use_vault: bool) -> Result<Credential, String> {
    if use_vault {
        let pwd = get_vault_password();
        let map = load_vault(&pwd)?;
        map.get(alias)
            .cloned()
            .ok_or_else(|| "Alias not found in vault".to_string())
    } else {
        let entry = keyring::Entry::new("dam_cli", alias).map_err(|e| e.to_string())?;
        let json = entry.get_password().map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }
}

pub fn delete_credential(alias: &str, use_vault: bool) -> Result<(), String> {
    if use_vault {
        let pwd = get_vault_password();
        let mut map = load_vault(&pwd)?;
        if map.remove(alias).is_some() {
            save_vault(&map, &pwd)?;
            remove_alias(alias);
        }
        Ok(())
    } else {
        let entry = keyring::Entry::new("dam_cli", alias).map_err(|e| e.to_string())?;
        entry.delete_password().map_err(|e| e.to_string())?;
        remove_alias(alias);
        Ok(())
    }
}