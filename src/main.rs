mod cli;
mod commands;
pub mod core;
pub mod providers;
pub mod platforms;

use clap::Parser;
use cli::{Cli, Commands};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const SUPPORTED_VERSIONS: &[&str] = &[env!("CARGO_PKG_VERSION"), "0.5.0"];
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let cli = Cli::parse();
    let original_cwd = env::current_dir().unwrap();

    // The 'source' command handles its own initialization logic safely in the current dir.
    if let Commands::Source { name, upgrade } = cli.command {
        commands::source::run(name, upgrade);
        return;
    }

    // For all other commands, resolve the active reservoir root and change working directory.
    let root = resolve_and_cd_reservoir(&original_cwd);

    // Version Check 
    if let Some(_) = &root {
        let config_path = Path::new(".dam/config.toml");
        if config_path.exists() {
            let content = fs::read_to_string(config_path).unwrap_or_default();
            let mut version = "0.1.0".to_string(); // Default if missing
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("version =") || trimmed.starts_with("version=") {
                    let parts: Vec<&str> = trimmed.split('=').collect();
                    if parts.len() == 2 {
                        version = parts[1].trim().trim_matches('"').to_string();
                    }
                }
            }
            if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
                println!("⚠️  Warning: Your reservoir is on version '{}', which is not supported by this CLI ({}).", version, CURRENT_VERSION);
                println!("⚠️  Please run `dam source --upgrade` to migrate your project structure to the latest version.\n");
            }
        }
    }

    match cli.command {
        Commands::Source { .. } => unreachable!(),
        Commands::Inspect { rules, target } => commands::inspect::run(rules, target),
        Commands::Drain { target } => commands::drain::run(target),
        Commands::Destroy { archive } => commands::destroy::run(archive),
        Commands::Collect {
            path,
            override_purities,
            override_impurities,
            rule_priority,
        } => {
            let adjusted_path = if let Some(r) = &root {
                adjust_path(&original_cwd, r, &path)
            } else {
                path
            };

            let adjusted_purities = override_purities.map(|p| {
                if p.is_empty() {
                    p
                } else if let Some(r) = &root {
                    adjust_path(&original_cwd, r, &p)
                } else {
                    p
                }
            });

            let adjusted_impurities = override_impurities.map(|p| {
                if p.is_empty() {
                    p
                } else if let Some(r) = &root {
                    adjust_path(&original_cwd, r, &p)
                } else {
                    p
                }
            });

            commands::collect::run(
                adjusted_path,
                adjusted_purities,
                adjusted_impurities,
                rule_priority,
            );
        }
        Commands::Seal { message, list } => {
            if let Some(n) = list {
                commands::seal::list_seals(n);
            } else if let Some(msg) = message {
                commands::seal::run(msg, Vec::new());
            } else {
                println!(
                    "Error: You must provide a message to create a seal, or use --list <N> to view history."
                );
            }
        }
        Commands::Timeline { graph } => commands::timeline::run(graph),
        Commands::Stream { command } => commands::stream::run(command),
        Commands::Flowinto { name } => commands::flowinto::run(name),
        Commands::Apply { seal_id, preview } => commands::apply::run(seal_id, preview),
        Commands::Merge { source, apply } => commands::merge::run(source, apply),
        Commands::Export { target } => match target {
            cli::ExportTarget::Seal { seal_id, zip } => {
                commands::export::run(seal_id, zip, false, None);
            }
            cli::ExportTarget::Project { project_name, profile } => {
                commands::export::run(project_name, false, true, profile);
            }
        },
        Commands::Import { file } => {
            let adjusted_path = if let Some(r) = &root {
                adjust_path(&original_cwd, r, &file)
            } else {
                file
            };
            commands::import::run(adjusted_path);
        }
        Commands::Settings {
            key,
            value,
            interactive,
        } => {
            commands::settings::run(key, value, interactive);
        },
        Commands::Creds { command } => commands::creds::run(command),
        Commands::Sync { stream, action, platform, force } => {
            commands::sync::run(stream, action, platform, force);
        }
    }
}

fn adjust_path(cwd: &Path, root: &Path, user_path: &str) -> String {
    let abs_path = cwd.join(user_path);
    match abs_path.strip_prefix(root) {
        Ok(rel) => {
            let rel_str = rel.to_string_lossy().into_owned();
            if rel_str.is_empty() {
                ".".to_string()
            } else {
                rel_str
            }
        }
        Err(_) => user_path.to_string(),
    }
}

fn resolve_and_cd_reservoir(original_cwd: &Path) -> Option<PathBuf> {
    let mut search_dir = original_cwd.to_path_buf();
    let mut active_dam = None;
    let mut parent_dam = None;

    loop {
        if search_dir.join(".dam").exists() {
            if active_dam.is_none() {
                active_dam = Some(search_dir.clone());
            } else {
                parent_dam = Some(search_dir.clone());
                break;
            }
        }
        if !search_dir.pop() {
            break;
        }
    }

    let active_dir = active_dam?;

    if let Some(parent) = parent_dam {
        let config_path = active_dir.join(".dam/config.toml");
        let config_content = fs::read_to_string(&config_path).unwrap_or_default();

        if !config_content.contains("suppress_nested_warning = true") {
            println!(
                "Warning: A parent reservoir exists at '{}'.",
                parent.display()
            );
            print!("Would you like to use the parent reservoir instead? (y/N): ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            if input.trim().eq_ignore_ascii_case("y") {
                env::set_current_dir(&parent).unwrap();
                return Some(parent);
            } else {
                print!(
                    "Would you like to permanently disable this warning for this child reservoir? (y/N): "
                );
                io::stdout().flush().unwrap();
                let mut disable_input = String::new();
                io::stdin().read_line(&mut disable_input).unwrap();

                if disable_input.trim().eq_ignore_ascii_case("y") {
                    if let Ok(mut file) = fs::OpenOptions::new().append(true).open(&config_path) {
                        writeln!(file, "suppress_nested_warning = true").unwrap();
                        println!("Warning disabled and saved to config.");
                    }
                }
            }
        }
    }

    env::set_current_dir(&active_dir).unwrap();
    Some(active_dir)
}