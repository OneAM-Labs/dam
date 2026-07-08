use std::io::{self, Write};
use std::path::Path;
use std::fs;
use crate::platforms;

pub fn run(stream: Option<String>, action: Option<String>, platform_arg: Option<String>, force: bool) {
    let provider_name = platform_arg.unwrap_or_else(|| "github".to_string());
    println!("\n--- [DAM CLOUD SYNC: {}] ---", provider_name.to_uppercase());

    let provider = platforms::get_provider(&provider_name);

    let streams_to_sync = if let Some(s) = stream {
        let path = format!(".dam/streams/{}", s);
        if !Path::new(&path).exists() {
            println!("❌ Error: Stream '{}' does not exist. Aborting synchronization.", s);
            return;
        }
        vec![s]
    } else {
        let current = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();
        vec![current]
    };

    for s in streams_to_sync {
        if s == "main" && !force {
            println!("\n⚠️  You are about to synchronize the main Stream.");
            println!("This is generally discouraged because it is intended to remain your stable development Stream.");
            print!("Continue? (y/N): ");
            io::stdout().flush().unwrap();
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();
            if choice.trim().to_lowercase() != "y" {
                println!("Skipping main stream.");
                continue;
            }
        }

        println!("\n🔄 Synchronizing Stream: {}", s);
        
        if let Some(ref act) = action {
            match act.to_lowercase().as_str() {
                "push" => {
                    if let Err(e) = provider.push(&s) {
                        println!("❌ Push failed for {}: {}", s, e);
                    }
                }
                "pull" => {
                    if let Err(e) = provider.pull(&s) {
                        println!("❌ Pull failed for {}: {}", s, e);
                    }
                }
                _ => {
                    println!("Error: Unknown action '{}'.", act);
                }
            }
            continue;
        }

        // Interactive Diff
        match provider.check_diff(&s) {
            Ok((ahead, behind)) => {
                if ahead == 0 && behind == 0 {
                    println!("✅ Stream '{}' is completely up to date.", s);
                    continue;
                }

                println!("📊 State Diff for {}:", s);
                if ahead > 0 { println!("  ↑ {} local seal(s) ahead.", ahead); }
                if behind > 0 { println!("  ↓ {} remote change(s) missing.", behind); }

                if ahead > 0 && behind > 0 {
                    println!("⚠️  CONFLICT DETECTED in '{}'", s);
                    println!("  [1] Pull remote changes");
                    println!("  [2] Force Push local state");
                    print!("Choice [1/2/Skip]: ");
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    
                    if choice.trim() == "1" {
                        if let Err(e) = provider.pull(&s) { println!("❌ Pull failed: {}", e); }
                    } else if choice.trim() == "2" {
                        if let Err(e) = provider.push(&s) { println!("❌ Push failed: {}", e); }
                    }
                } else if ahead > 0 {
                    print!("Push local seals for '{}'? (Y/n): ", s);
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    if choice.trim().to_lowercase() != "n" {
                        if let Err(e) = provider.push(&s) { println!("❌ Push failed: {}", e); }
                    }
                } else if behind > 0 {
                    print!("Pull latest changes for '{}'? (Y/n): ", s);
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    if choice.trim().to_lowercase() != "n" {
                        if let Err(e) = provider.pull(&s) { println!("❌ Pull failed: {}", e); }
                    }
                }
            }
            Err(e) => {
                println!("❌ Failed to fetch diff for {}: {}", s, e);
            }
        }
    }
}