use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use serde_json;
use crate::commands::seal;
use flate2::read::ZlibDecoder;

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