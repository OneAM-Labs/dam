use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Reads the project name from config.toml, if set.
fn read_project_name() -> Option<String> {
    let config = fs::read_to_string(".dam/config.toml").ok()?;
    for line in config.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let name = rest.trim().trim_matches('"').to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// Prompts the user for a line of input, returning the trimmed string.
fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

pub fn run(archive: bool) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found.");
        return;
    }

    let project_name = read_project_name();

    if archive {
        run_archive(project_name);
    } else {
        run_destroy(project_name);
    }
}

fn run_destroy(project_name: Option<String>) {
    println!("=============================================================");
    println!("  DANGER: You are about to permanently destroy this reservoir.");
    println!("  This action CANNOT be undone.");
    println!("=============================================================");
    println!();

    // Friction 1: show what will be lost
    let seal_count = count_seals();
    println!("This will permanently delete:");
    println!("  - All {} seal(s) in history", seal_count);
    println!("  - All staged files");
    println!("  - All streams and configuration");
    println!();

    // Friction 2: project name confirmation
    let confirmation_word = match &project_name {
        Some(name) => {
            println!("Project name: \"{}\"", name);
            println!("Type the project name exactly to confirm destruction:");
            name.clone()
        }
        None => {
            println!("No project name is set in config.toml.");
            println!("Type DESTROY to confirm:");
            "DESTROY".to_string()
        }
    };

    let input = prompt("> ");
    if input != confirmation_word {
        println!("Confirmation did not match. Destruction aborted.");
        return;
    }

    // Friction 3: final "are you sure"
    println!();
    let yn = prompt("Are you absolutely sure? This is irreversible. (yes/N): ");
    if yn.to_lowercase() != "yes" {
        println!("Destruction aborted.");
        return;
    }

    fs::remove_dir_all(".dam").unwrap();
    println!("Reservoir destroyed.");
}

fn run_archive(project_name: Option<String>) {
    println!("=====================================================");
    println!("  You are about to archive this reservoir.");
    println!("  The .dam folder will be renamed to .dam_archive.");
    println!("=====================================================");
    println!();

    // Show what's being archived
    let seal_count = count_seals();
    println!("This will archive:");
    println!("  - {} seal(s)", seal_count);
    println!("  - All staged files and configuration");
    println!();

    // Friction: project name confirmation
    let confirmation_word = match &project_name {
        Some(name) => {
            println!("Project name: \"{}\"", name);
            println!("Type the project name exactly to confirm archiving:");
            name.clone()
        }
        None => {
            println!("No project name is set. Type ARCHIVE to confirm:");
            "ARCHIVE".to_string()
        }
    };

    let input = prompt("> ");
    if input != confirmation_word {
        println!("Confirmation did not match. Archive aborted.");
        return;
    }

    if Path::new(".dam_archive").exists() {
        println!();
        println!("Warning: .dam_archive already exists and will be overwritten.");
        let yn = prompt("Continue? (yes/N): ");
        if yn.to_lowercase() != "yes" {
            println!("Archive aborted.");
            return;
        }
        fs::remove_dir_all(".dam_archive").unwrap();
    }

    fs::rename(".dam", ".dam_archive").unwrap();
    println!("Reservoir archived to .dam_archive.");
}

fn count_seals() -> usize {
    fs::read_dir(".dam/seals")
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}