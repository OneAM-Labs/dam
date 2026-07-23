use crate::cli::PrCommands;
use crate::commands::seal;
use crate::platforms::{self, SyncProvider};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub fn run(command: Option<PrCommands>) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found. Run 'dam source' first.");
        return;
    }

    let provider = platforms::get_provider("github");

    match command {
        Some(PrCommands::List) => list_prs(provider.as_ref()),
        Some(PrCommands::Checkout { number }) => checkout_pr(provider.as_ref(), number),
        None => {
            list_prs(provider.as_ref());
            print!("\nEnter a PR number to check out (or press Enter to cancel): ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let trimmed = input.trim();

            if trimmed.is_empty() {
                println!("Cancelled.");
                return;
            }

            match trimmed.parse::<u64>() {
                Ok(number) => checkout_pr(provider.as_ref(), number),
                Err(_) => println!("Invalid PR number: '{}'.", trimmed),
            }
        }
    }
}

fn list_prs(provider: &dyn SyncProvider) {
    println!("\n--- Open Pull Requests ---");
    match provider.list_pull_requests() {
        Ok(prs) if prs.is_empty() => println!("No open pull requests found."),
        Ok(prs) => {
            for pr in &prs {
                println!(
                    "  #{:<5} {}  (by {}, branch '{}')",
                    pr.number, pr.title, pr.author, pr.head_ref
                );
            }
        }
        Err(e) => println!("❌ Failed to list pull requests: {}", e),
    }
}

fn checkout_pr(provider: &dyn SyncProvider, number: u64) {
    let staging_content =
        fs::read_to_string(".dam/staging.json").unwrap_or_else(|_| "[]".to_string());
    let staged_files: Vec<String> = serde_json::from_str(&staging_content).unwrap_or_default();

    if !staged_files.is_empty() {
        println!("\n⚠️  Unsaved changes detected in your active workspace.");
        print!(
            "Seal your current work before checking out PR #{}? (Y/n): ",
            number
        );
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        if input.trim().to_lowercase() != "n" {
            let msg = format!("Safety seal before checking out PR #{}", number);
            seal::run(msg, Vec::new());
        }
    }

    println!("\nChecking out PR #{}...", number);
    match provider.checkout_pull_request(number) {
        Ok(stream_name) => {
            let previous_stream =
                fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string());
            fs::write(".dam/CURRENT", &stream_name).unwrap();

            let meta = crate::commands::stream::get_or_create_meta(&stream_name);
            if let Some(seal_id) = meta.latest_seal {
                crate::commands::apply::run(seal_id, false);
            }

            println!("✓ PR #{} checked out into stream '{}'.", number, stream_name);
            println!(
                "  Run `dam flowinto {}` to go back to your previous stream.",
                previous_stream.trim()
            );
        }
        Err(e) => println!("❌ Failed to check out PR #{}: {}", number, e),
    }
}
