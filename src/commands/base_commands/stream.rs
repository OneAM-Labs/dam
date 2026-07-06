use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::io::{self, Write};
use chrono::Utc;
use crate::cli::StreamCommands;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct StreamMeta {
    pub name: String,
    pub description: String,
    pub owner: String,
    pub priority: String,
    pub status: String,
    pub created_at: String,
    pub target: String,
    pub goals: Vec<String>,
    #[serde(default)]
    pub latest_seal: Option<String>,
}

pub fn get_or_create_meta(name: &str) -> StreamMeta {
    let path = format!(".dam/streams/{}", name);
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(meta) = serde_json::from_str(&content) {
            return meta;
        }
    }
    // Default config values
    StreamMeta {
        name: name.to_string(),
        description: "Standard workflow stream".to_string(),
        owner: std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string()),
        priority: if name == "main" { "High".to_string() } else { "Normal".to_string() },
        status: "Active".to_string(),
        created_at: Utc::now().to_rfc3339(),
        target: "main".to_string(),
        goals: vec![],
        latest_seal: None,
    }
}

pub fn save_meta(meta: &StreamMeta) {
    let path = format!(".dam/streams/{}", meta.name);
    fs::write(path, serde_json::to_string_pretty(meta).unwrap()).unwrap();
}

fn prompt_user(prompt: &str, default: &str) -> String {
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn run(command: Option<StreamCommands>) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found.");
        return;
    }

    match command {
        Some(StreamCommands::Create { name, description, owner, priority, target }) => {
            let path = format!(".dam/streams/{}", name);
            if Path::new(&path).exists() {
                println!("Stream '{}' already exists.", name);
                return;
            }

            println!("Creating new stream '{}'...", name);
            
            let default_priority = if name == "main" { "High" } else { "Normal" };
            
            // Interactively prompt for missing values
            let final_desc = description.unwrap_or_else(|| prompt_user("Description", "Standard workflow stream"));
            let final_owner = owner.unwrap_or_else(|| prompt_user("Owner", &std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())));
            let final_priority = priority.unwrap_or_else(|| prompt_user("Priority", default_priority));
            let final_target = target.unwrap_or_else(|| prompt_user("Target", "main"));

            let meta = StreamMeta {
                name: name.clone(),
                description: final_desc,
                owner: final_owner,
                priority: final_priority,
                status: "Active".to_string(),
                created_at: Utc::now().to_rfc3339(),
                target: final_target,
                goals: vec![],
                latest_seal: None,
            };
            
            save_meta(&meta);
            println!("✓ Created new rich stream: {}", name);
        }
        Some(StreamCommands::Inspect { name }) => {
            let target_name = name.unwrap_or_else(|| fs::read_to_string(".dam/CURRENT").unwrap_or_default().trim().to_string());
            if target_name.is_empty() {
                println!("No active stream context found.");
                return;
            }

            let meta = get_or_create_meta(&target_name);
            println!("\nCurrent Stream");
            println!("{}", meta.name);
            println!("──────────────");
            println!("Description : {}", meta.description);
            println!("Owner       : {}", meta.owner);
            println!("Priority    : {}", meta.priority);
            println!("Target      : {}", meta.target);
            println!("Status      : {}", meta.status);
            
            if let Some(latest) = &meta.latest_seal {
                println!("Latest Seal : {}", latest);
            } else {
                println!("Latest Seal : (None)");
            }

            if !meta.goals.is_empty() {
                println!("\nGoals");
                println!("─────");
                for (i, goal) in meta.goals.iter().enumerate() {
                    println!("{}. {}", i + 1, goal);
                }
            } else {
                println!("\nGoals: None configured. Use 'dam stream set-goal' to add one.");
            }
            println!();
        }
        Some(StreamCommands::SetGoal { text }) => {
            let current = fs::read_to_string(".dam/CURRENT").unwrap_or_default().trim().to_string();
            if current.is_empty() {
                println!("No active stream to set a goal for.");
                return;
            }
            let mut meta = get_or_create_meta(&current);
            meta.goals.push(text.clone());
            save_meta(&meta);
            println!("✓ Added goal to stream '{}': {}", current, text);
        }
        Some(StreamCommands::Delete { name }) => {
            let current = fs::read_to_string(".dam/CURRENT").unwrap_or_default().trim().to_string();
            if name == current {
                println!("Cannot delete the active stream. Switch to another stream first.");
                return;
            }
            let path = format!(".dam/streams/{}", name);
            if !Path::new(&path).exists() {
                println!("Stream '{}' does not exist.", name);
                return;
            }

            // Locate and report all seals committed specifically in this stream
            let seals_dir = Path::new(".dam/seals");
            let mut associated_seals = Vec::new();
            if seals_dir.exists() {
                for entry in fs::read_dir(seals_dir).unwrap().flatten() {
                    if entry.path().extension().map_or(false, |ext| ext == "json") {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            if let Ok(seal) = serde_json::from_str::<crate::commands::seal::Seal>(&content) {
                                if seal.stream == name {
                                    associated_seals.push(entry.path());
                                }
                            }
                        }
                    }
                }
            }
            
            if !associated_seals.is_empty() {
                print!("\n⚠️  Stream '{}' has {} associated seals.\nDeleting this stream will also permanently delete its history.\nAre you sure you want to proceed? (y/N): ", name, associated_seals.len());
                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();
                
                if input.trim().eq_ignore_ascii_case("y") {
                    let mut deleted_count = 0;
                    for seal_path in associated_seals {
                        if fs::remove_file(&seal_path).is_ok() {
                            deleted_count += 1;
                        }
                    }
                    println!("Deleted {} associated seals.", deleted_count);
                } else {
                    println!("Aborting stream deletion.");
                    return;
                }
            } else {
                print!("Are you sure you want to delete stream '{}'? (y/N): ", name);
                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborting.");
                    return;
                }
            }
            
            if fs::remove_file(&path).is_ok() {
                println!("✓ Deleted stream '{}'.", name);
            } else {
                println!("Failed to delete stream file.");
            }
        }
        None => {
            println!("Available Reservoir Streams:");
            let current = fs::read_to_string(".dam/CURRENT").unwrap_or_default();
            if let Ok(entries) = fs::read_dir(".dam/streams") {
                for entry in entries.flatten() {
                    let name = entry.file_name().into_string().unwrap_or_default();
                    let meta = get_or_create_meta(&name);
                    
                    if name == current.trim() {
                        println!("  * {} (active) - [{} Priority: {}]", name, meta.status, meta.priority);
                    } else {
                        println!("    {} - [{} Priority: {}]", name, meta.status, meta.priority);
                    }
                }
            }
        }
    }
}