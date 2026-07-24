use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use serde_json;
use crate::commands::seal;
use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};

fn hash_file(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

pub fn run(seal_id: String, preview: bool) {
    let seal_meta_path = format!(".dam/seals/{}.json", seal_id);
    if !Path::new(&seal_meta_path).exists() {
        println!("Error: Seal '{}' not found.", seal_id);
        return;
    }

    let meta_content = fs::read_to_string(&seal_meta_path).unwrap();
    let seal: seal::Seal = serde_json::from_str(&meta_content).unwrap();

    if preview {
        println!("\n--- Preview of Restoration [{}] ---", seal_id);
        println!("Description: {}", seal.message);
        for file in &seal.files {
            println!("  →  {}", file.path);
        }
        return;
    }

    // Safety Intercept: Verify if workspace tracking has items collected
    let staging_content = fs::read_to_string(".dam/staging.json").unwrap_or_else(|_| "[]".to_string());
    let staged_files: Vec<String> = serde_json::from_str(&staging_content).unwrap_or_default();

    if !staged_files.is_empty() {
        println!("\nWARNING: Unsaved changes detected in your staging area.");
        println!("Restoring may permanently overwrite adjustments made to those files.");
        print!("Would you like to auto-seal your current work before restoring? (Y/n): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim().to_lowercase();

        if input == "y" || input.is_empty() {
            print!("Enter safety seal message: ");
            io::stdout().flush().unwrap();
            let mut msg = String::new();
            io::stdin().read_line(&mut msg).unwrap();
            let msg = msg.trim();
            let fallback_msg = format!("Pre-apply snapshot for {}", seal_id);
            
            let final_msg = if msg.is_empty() { &fallback_msg } else { msg };
            seal::run(final_msg.to_string(),Vec::new());
        }
    }

    let mut changed_locally = Vec::new();
    for entry in &seal.files {
        if entry.is_dir {
            continue;
        }
        let workspace_target = Path::new(&entry.path);
        if workspace_target.exists() {
            if let Some(current_hash) = hash_file(workspace_target) {
                if current_hash != entry.hash {
                    changed_locally.push(entry.path.clone());
                }
            }
        }
    }

    if !changed_locally.is_empty() {
        println!(
            "\n⚠️  WARNING: The following files have local changes that don't match seal {}:",
            seal_id
        );
        for path in &changed_locally {
            println!("  - {}", path);
        }
        print!("Overwrite these files anyway? (y/N): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted. No files were changed.");
            return;
        }
    }

    println!("Restoring workspace files to matches from {}...", seal_id);

    for entry in &seal.files {
        let workspace_target = Path::new(&entry.path);

        if entry.is_dir {
            fs::create_dir_all(workspace_target).unwrap();
            println!("  ✓ Applied Directory: {}", entry.path);
        } else {
            if let Some(parent) = workspace_target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            
            let obj_path = Path::new(".dam/objects").join(&entry.hash);
            if obj_path.exists() {
                let compressed_file = File::open(obj_path).unwrap();
                let mut decoder = ZlibDecoder::new(compressed_file);
                let mut out_file = File::create(workspace_target).unwrap();
                io::copy(&mut decoder, &mut out_file).unwrap();
                println!("  ✓ Applied: {}", entry.path);
            } else {
                println!("  ! Warning: Object {} missing for file {}", entry.hash, entry.path);
            }
        }
    }
    println!("\nStream successfully brought back to state: {}", seal_id);
}