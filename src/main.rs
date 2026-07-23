mod cli;
mod commands;
pub mod core;
pub mod providers;
pub mod platforms;

use clap::Parser;
use cli::{Cli, Commands};
use std::env;
use std::fs;
use std::io::{self, Write, IsTerminal};
use std::path::{Path, PathBuf};

const SUPPORTED_VERSIONS: &[&str] = &[env!("CARGO_PKG_VERSION"), "0.5.0", "0.5.1"];
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- TERMINAL COLOR SUPPORT FOR PATH WARNINGS --- //

#[cfg(windows)]
fn enable_windows_ansi_support() -> bool {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_OUTPUT_HANDLE,
    };

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == INVALID_HANDLE_VALUE || handle == 0 {
            return false;
        }

        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return true;
        }

        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true;
        }

        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_windows_ansi_support() -> bool {
    true
}

fn supports_color() -> bool {
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }

    if let Some(force) = env::var_os("CLICOLOR_FORCE") {
        if force != "0" {
            return true;
        }
    }

    if let Ok(term) = env::var("TERM") {
        if term == "dumb" {
            return false;
        }
    }

    io::stdout().is_terminal() && enable_windows_ansi_support()
}

struct Colors {
    bold: &'static str,
    reset: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    green: &'static str,
    dim: &'static str,
}

impl Colors {
    fn detect() -> Self {
        if supports_color() {
            Colors {
                bold: "\x1b[1m",
                reset: "\x1b[0m",
                yellow: "\x1b[1;33m",
                cyan: "\x1b[36m",
                green: "\x1b[32m",
                dim: "\x1b[2m",
            }
        } else {
            Colors {
                bold: "",
                reset: "",
                yellow: "",
                cyan: "",
                green: "",
                dim: "",
            }
        }
    }
}

fn get_global_marker_path() -> Option<PathBuf> {
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".dam_ignore_path_warning"))
}

fn check_and_warn_path() {
    // Skip check if user explicitly disabled it via env var
    if env::var_os("DAM_DISABLE_PATH_CHECK").is_some() {
        return;
    }

    // Skip if user permanently suppressed it via marker file
    if let Some(marker) = get_global_marker_path() {
        if marker.exists() {
            return;
        }
    }

    // Get the exact path of the currently executing binary
    let current_exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return, // Fail silently if we can't determine the execution path
    };

    let canonical_current = fs::canonicalize(&current_exe).unwrap_or_else(|_| current_exe.clone());
    let exe_name = current_exe.file_name().unwrap_or_default();
    
    // Scan the system PATH for this executable name
    let mut in_path = false;
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let candidate = dir.join(&exe_name);
            if candidate.exists() {
                // Ensure the file in PATH isn't a symlink or another file masking as DAM
                if let Ok(canonical_candidate) = fs::canonicalize(&candidate) {
                    if canonical_candidate == canonical_current {
                        in_path = true;
                        break;
                    }
                }
            }
        }
    }

    // If it's not in the PATH, generate an OS-specific warning with an interactive prompt
    if !in_path {
        if let Some(exe_dir) = current_exe.parent() {
            let dir_str = exe_dir.display();
            let c = Colors::detect();

            println!("{}⚠️  Warning: The 'dam' executable is not in your system PATH.{}", c.yellow, c.reset);
            println!("You are currently running it directly from: {}{}{}", c.cyan, dir_str, c.reset);
            println!("To run 'dam' from anywhere, please add its folder to your PATH.\n");
            
            #[cfg(target_os = "windows")]
            {
                println!("To add it on {}Windows{}, run this in PowerShell:", c.bold, c.reset);
                println!("  {}[Environment]::SetEnvironmentVariable(\"Path\", $env:Path + \";{}\", \"User\"){}\n", c.green, dir_str, c.reset);
            }
            
            #[cfg(not(target_os = "windows"))]
            {
                println!("To add it on {}macOS / Linux{}, run this in your terminal:", c.bold, c.reset);
                println!("  {}echo 'export PATH=\"$PATH:{}\"' >> ~/.zshrc{}", c.green, dir_str, c.reset);
                println!("  {}(Replace ~/.zshrc with ~/.bashrc if you use bash){}\n", c.dim, c.reset);
            }
            
            print!("Would you like to permanently hide this warning? (y/N): ");
            io::stdout().flush().unwrap();
            
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y") {
                if let Some(marker) = get_global_marker_path() {
                    let _ = fs::write(marker, "1");
                    println!("{}✅ Warning permanently suppressed.{}\n", c.green, c.reset);
                }
            } else {
                println!("{}(You can also hide this manually by setting DAM_DISABLE_PATH_CHECK=1){}\n", c.dim, c.reset);
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    
    check_and_warn_path();

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

    // Trigger automatic background update check unless running manual update command.
    if !matches!(cli.command, Commands::Update) {
        commands::update::auto_check();
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
        },
        Commands::Update => {
            commands::update::run();
        }
        Commands::Pr { command } => commands::pr::run(command),
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