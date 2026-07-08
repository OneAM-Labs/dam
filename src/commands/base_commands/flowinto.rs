use std::fs;
use std::io::{self, Write};
use std::path::Path;
use crate::commands::seal;

pub fn run(name: String) {
    let path = format!(".dam/streams/{}", name);

    if !Path::new(&path).exists() {
        println!("Stream does not exist. Use 'dam stream create {}' first.", name);
        return;
    }

    let staging_content = fs::read_to_string(".dam/staging.json").unwrap_or_else(|_| "[]".to_string());
    let staged_paths: Vec<String> = serde_json::from_str(&staging_content).unwrap_or_default();

    if !staged_paths.is_empty() {
        println!("\n⚠️  Unsaved continuity detected in your active workspace.");
        print!("Create temporary Continuity Snapshot (Seal) before flowing? (Y/n): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim().to_lowercase() != "n" {
            let active = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "unknown".to_string());
            let snapshot_msg = format!("Continuity auto-snapshot before flowing from {} to {}", active.trim(), name);
            seal::run(snapshot_msg,Vec::new());
        } else {
            println!("Leaving files unsealed. Flowing into stream...");
        }
    }

    fs::write(".dam/CURRENT", &name).unwrap();
    println!("🌊 Flow successfully moved to workspace: {}", name);
}