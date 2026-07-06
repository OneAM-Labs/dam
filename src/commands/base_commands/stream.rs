use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::io::{self, Write};
use chrono::Utc;
use crate::cli::StreamCommands;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct StreamMeta {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub owner: String,
    pub priority: String,
    pub status: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
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
        description: Some("Standard workflow stream".to_string()),
        owner: std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string()),
        priority: if name == "main" { "High".to_string() } else { "Normal".to_string() },
        status: "Active".to_string(),
        created_at: Utc::now().to_rfc3339(),
        target: Some("main".to_string()),
        goals: vec![],
        notes: None,
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
            
            let final_desc = description.unwrap_or_else(|| prompt_user("Description", "Standard workflow stream"));
            let final_owner = owner.unwrap_or_else(|| prompt_user("Owner", &std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())));
            let final_priority = priority.unwrap_or_else(|| prompt_user("Priority", default_priority));
            let final_target = target.unwrap_or_else(|| prompt_user("Target", "main"));

            let meta = StreamMeta {
                name: name.clone(),
                description: Some(final_desc),
                owner: final_owner,
                priority: final_priority,
                status: "Active".to_string(),
                created_at: Utc::now().to_rfc3339(),
                target: Some(final_target),
                goals: vec![],
                notes: None,
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

            let path = format!(".dam/streams/{}", target_name);
            if !Path::new(&path).exists() {
                println!("❌ Error: Stream '{}' does not exist. Please create it first using 'dam stream create {}'.", target_name, target_name);
                return;
            }

            let meta = get_or_create_meta(&target_name);
            println!("\nCurrent Stream");
            println!("{}", meta.name);
            println!("──────────────");
            println!("Description : {}", meta.description.as_deref().unwrap_or("(None)"));
            println!("Owner       : {}", meta.owner);
            println!("Priority    : {}", meta.priority);
            println!("Target      : {}", meta.target.as_deref().unwrap_or("(None)"));
            println!("Status      : {}", meta.status);
            println!("Notes       : {}", meta.notes.as_deref().unwrap_or("(None)"));
            
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
                println!("\nGoals: None configured.");
            }
            println!();
        }
        Some(StreamCommands::Goal { name, clear, text }) => {
            let path = format!(".dam/streams/{}", name);
            if !Path::new(&path).exists() {
                println!("❌ Error: Stream '{}' does not exist. Please create the stream first using 'dam stream create {}'.", name, name);
                return;
            }
            let mut meta = get_or_create_meta(&name);
            if clear {
                meta.goals.clear();
                println!("✓ Cleared goals for stream '{}'", name);
            } else if let Some(t) = text {
                meta.goals.push(t.clone());
                println!("✓ Added goal to stream '{}': {}", name, t);
            }
            save_meta(&meta);
        }
        Some(StreamCommands::Notes { name, clear, text }) => {
            let path = format!(".dam/streams/{}", name);
            if !Path::new(&path).exists() {
                println!("❌ Error: Stream '{}' does not exist. Please create the stream first using 'dam stream create {}'.", name, name);
                return;
            }
            let mut meta = get_or_create_meta(&name);
            if clear {
                meta.notes = None;
                println!("✓ Cleared notes for stream '{}'", name);
            } else if let Some(t) = text {
                meta.notes = Some(t.clone());
                println!("✓ Updated notes for stream '{}'", name);
            }
            save_meta(&meta);
        }
        Some(StreamCommands::Description { name, clear, text }) => {
            let path = format!(".dam/streams/{}", name);
            if !Path::new(&path).exists() {
                println!("❌ Error: Stream '{}' does not exist. Please create the stream first using 'dam stream create {}'.", name, name);
                return;
            }
            let mut meta = get_or_create_meta(&name);
            if clear {
                meta.description = None;
                println!("✓ Cleared description for stream '{}'", name);
            } else if let Some(t) = text {
                meta.description = Some(t.clone());
                println!("✓ Updated description for stream '{}'", name);
            }
            save_meta(&meta);
        }
        Some(StreamCommands::Target { name, clear, text }) => {
            let path = format!(".dam/streams/{}", name);
            if !Path::new(&path).exists() {
                println!("❌ Error: Stream '{}' does not exist. Please create the stream first using 'dam stream create {}'.", name, name);
                return;
            }
            let mut meta = get_or_create_meta(&name);
            if clear {
                meta.target = None;
                println!("✓ Cleared target for stream '{}'", name);
            } else if let Some(t) = text {
                meta.target = Some(t.clone());
                println!("✓ Updated target for stream '{}'", name);
            }
            save_meta(&meta);
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