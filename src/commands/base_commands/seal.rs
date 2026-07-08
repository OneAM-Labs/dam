use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use sha2::{Sha256, Digest};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use crate::commands::stream::{get_or_create_meta, save_meta};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub path: String,
    pub hash: String,
    pub is_dir: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Seal {
    pub id: String,
    pub message: String,
    pub timestamp: String,
    pub files: Vec<FileEntry>,
    #[serde(default = "default_stream")]
    pub stream: String,
    #[serde(default)]
    pub parents: Vec<String>,
}

fn default_stream() -> String {
    "main".to_string()
}

pub fn run(message: String, mut extra_parents: Vec<String>) -> String {
    if !Path::new(".dam").exists() {
        println!("No reservoir found. Run 'dam source' first.");
        return String::new();
    }

    let staging_content = fs::read_to_string(".dam/staging.json").unwrap_or_else(|_| "[]".to_string());
    let staged_paths: Vec<String> = serde_json::from_str(&staging_content).unwrap_or_default();

    if staged_paths.is_empty() && extra_parents.is_empty() {
        println!("Staging area is clear. Nothing to collect into a seal.");
        return String::new();
    }

    let timestamp = Utc::now().to_rfc3339();
    
    // Read active stream context and gather parents
    let current_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();
    let mut stream_meta = get_or_create_meta(&current_stream);
    
    let mut parents = Vec::new();
    if let Some(ref latest) = stream_meta.latest_seal {
        parents.push(latest.clone());
    }
    parents.append(&mut extra_parents);

    // Generate robust non-collision short hash for the seal ID
    let mut id_hasher = Sha256::new();
    id_hasher.update(message.as_bytes());
    id_hasher.update(timestamp.as_bytes());
    for p in &parents {
        id_hasher.update(p.as_bytes());
    }
    for p in &staged_paths {
        id_hasher.update(p.as_bytes());
    }
    let id_hash_hex = format!("{:x}", id_hasher.finalize());
    let id = format!("seal_{}", &id_hash_hex[..8]);

    let mut files_meta = Vec::new();

    for file_path in &staged_paths {
        let src_path = Path::new(file_path);
        let is_dir = file_path.ends_with('/') || src_path.is_dir();

        let hash = if is_dir {
            "dir".to_string() 
        } else if src_path.is_file() {
            let mut file = File::open(src_path).unwrap();
            let mut hasher = Sha256::new();
            io::copy(&mut file, &mut hasher).unwrap();
            let hash_hex = format!("{:x}", hasher.finalize());

            let dest_obj = Path::new(".dam/objects").join(&hash_hex);
            if !dest_obj.exists() {
                let mut source_file = File::open(src_path).unwrap();
                let mut buffer = Vec::new();
                source_file.read_to_end(&mut buffer).unwrap();

                let compressed_file = File::create(dest_obj).unwrap();
                let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                encoder.write_all(&buffer).unwrap();
                encoder.finish().unwrap();
            }
            hash_hex
        } else {
            continue;
        };

        files_meta.push(FileEntry {
            path: file_path.clone(),
            hash,
            is_dir,
        });
    }

    let seal = Seal {
        id: id.clone(),
        message,
        timestamp,
        files: files_meta,
        stream: current_stream,
        parents,
    };

    let path = format!(".dam/seals/{}.json", id);
    fs::write(path, serde_json::to_string_pretty(&seal).unwrap()).unwrap();
    fs::write(".dam/staging.json", "[]").unwrap(); 

    // Update stream HEAD
    stream_meta.latest_seal = Some(id.clone());
    save_meta(&stream_meta);

    println!("Created {} safely.", id);
    id
}

pub fn list_seals(n: usize) {
    let seals_dir = Path::new(".dam/seals");
    if !seals_dir.exists() {
        println!("No reservoir found or no seals exist yet.");
        return;
    }

    let current_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();
    let stream_meta = get_or_create_meta(&current_stream);
    
    let mut current_seal_id = stream_meta.latest_seal;
    let mut count = 0;

    if current_seal_id.is_none() {
        println!("No seals found in this stream.");
        return;
    }

    println!("\n--- Last {} Seals in '{}' (Historical Chain) ---", n, current_stream);
    
    while let Some(seal_id) = current_seal_id {
        if count >= n { break; }
        
        let path = format!(".dam/seals/{}.json", seal_id);
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                let time = seal.timestamp.replace("T", " ").split('+').next().unwrap_or(&seal.timestamp).to_string();
                let parent_info = if seal.parents.is_empty() {
                    "root".to_string()
                } else {
                    seal.parents.join(", ")
                };
                println!("  {} | {} | parents: [{}] | \"{}\"", seal.id, time, parent_info, seal.message);
                
                // Traverse backwards through the primary parent
                current_seal_id = seal.parents.get(0).cloned();
                count += 1;
            } else { break; }
        } else { break; }
    }
    println!();
}