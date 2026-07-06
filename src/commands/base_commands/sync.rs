use std::io::{self, Write};
use crate::platforms;

pub fn run(action: Option<String>, platform_arg: Option<String>) {
    let provider_name = platform_arg.unwrap_or_else(|| "github".to_string());
    
    println!("\n--- [DAM CLOUD SYNC: {}] ---", provider_name.to_uppercase());

    // Initialize the target platform. (Triggers token and configuration waterfall gracefully)
    let platform = platforms::get_platform(&provider_name);

    if let Some(act) = action {
        match act.to_lowercase().as_str() {
            "push" => {
                if let Err(e) = platform.push() {
                    println!("❌ Push failed: {}", e);
                }
            }
            "pull" => {
                if let Err(e) = platform.pull() {
                    println!("❌ Pull failed: {}", e);
                }
            }
            _ => {
                println!("Error: Unknown action '{}'. Use 'push', 'pull', or leave blank for interactive.", act);
            }
        }
        return;
    }

    // --- Interactive Diff Handler ---
    match platform.check_diff() {
        Ok((ahead, behind)) => {
            if ahead == 0 && behind == 0 {
                println!("✅ Your local reservoir is completely up to date with the cloud.");
                return;
            }

            println!("📊 State Diff:");
            if ahead > 0 { println!("  ↑ {} local seal(s) ahead of remote.", ahead); }
            if behind > 0 { println!("  ↓ {} remote change(s) missing locally.", behind); }

            if ahead > 0 && behind > 0 {
                println!("\n⚠️  CONFLICT DETECTED: Both local and remote have advanced.");
                println!("  [1] Pull remote changes (merge into local workspace)");
                println!("  [2] Force Push local state (overwrite remote branch)");
                print!("\nChoice [1/2]: ");
                io::stdout().flush().unwrap();
                
                let mut choice = String::new();
                io::stdin().read_line(&mut choice).unwrap();
                
                if choice.trim() == "1" {
                    if let Err(e) = platform.pull() {
                        println!("❌ Pull failed: {}", e);
                    }
                } else if choice.trim() == "2" {
                    if let Err(e) = platform.push() {
                        println!("❌ Force Push failed: {}", e);
                    }
                } else {
                    println!("Aborting sync.");
                }
            } else if ahead > 0 {
                print!("\nDo you want to push your local seals to the cloud? (Y/n): ");
                io::stdout().flush().unwrap();
                let mut choice = String::new();
                io::stdin().read_line(&mut choice).unwrap();
                if choice.trim().to_lowercase() != "n" {
                    if let Err(e) = platform.push() {
                        println!("❌ Push failed: {}", e);
                    }
                }
            } else if behind > 0 {
                print!("\nDo you want to pull the latest changes? (Y/n): ");
                io::stdout().flush().unwrap();
                let mut choice = String::new();
                io::stdin().read_line(&mut choice).unwrap();
                if choice.trim().to_lowercase() != "n" {
                    if let Err(e) = platform.pull() {
                        println!("❌ Pull failed: {}", e);
                    }
                }
            }
        }
        Err(e) => println!("❌ Failed to fetch diff: {}", e),
    }
}