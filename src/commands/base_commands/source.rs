use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use crate::providers;
use crate::core::credentials::{save_credential, Credential};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

// Helper to parse semver strings (e.g. "0.4.1") into comparable tuples (0, 4, 1)
fn parse_version(v: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = v.trim_matches('"').split('.').collect();
    let major = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

pub fn run(name: Option<String>, upgrade: bool) {
    if upgrade {
        if !Path::new(".dam").exists() {
            println!("Error: No reservoir found to upgrade. Run 'dam source' to initialize one.");
            return;
        }
        println!("--- [DAM RESERVOIR UPGRADE] ---");
        let mut config_content = fs::read_to_string(".dam/config.toml").unwrap_or_default();
        
        // 1. Identify current project version
        let mut current_version_str = "0.1.0".to_string(); // Fallback for very old reservoirs
        for line in config_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("version =") {
                if let Some(v) = trimmed.split('=').nth(1) {
                    current_version_str = v.trim().trim_matches('"').to_string();
                }
            }
        }
        
        let project_version = parse_version(&current_version_str);
        let target_version = parse_version(CURRENT_VERSION);
        
        println!("Current project version: v{}", current_version_str);
        
        if project_version >= target_version {
            println!("Reservoir is already up-to-date with DAM v{}.", CURRENT_VERSION);
            return;
        }
        
        println!("Upgrading to DAM v{}...", CURRENT_VERSION);

        // --- SEQUENTIAL MIGRATION PIPELINE ---

        // MIGRATION: Pre-v0.4.0 Structural Updates
        if project_version < (0, 4, 0) {
            println!("-> Applying v0.4.0 structural updates...");
            if !config_content.contains("enforce_password_on_project_import") {
                config_content.push_str("enforce_password_on_project_import = false\n");
            }
            if !Path::new("dam.toml").exists() {
                println!("   Generating missing dam.toml profile...");
                let provider = crate::providers::get_provider("custom");
                fs::write("dam.toml", provider.default_toml("upgraded-project")).unwrap();
            }
        }

        // MIGRATION: Pre-v0.5.0 Credential System Migration
        if project_version < (0, 5, 0) {
            println!("-> Applying v0.5.0 credential system updates...");
            let legacy_json = Path::new(".dam/credentials.json");
            let legacy_raw = Path::new(".dam/credentials");

            if legacy_json.exists() || legacy_raw.exists() {
                println!("\n   [Legacy Credentials Detected]");
                println!("   DAM v0.5.0 introduces a secure, centralized Credential Manager.");
                print!("   Would you like to migrate your old project credentials now? (Y/n): ");
                io::stdout().flush().unwrap();
                let mut mig_choice = String::new();
                io::stdin().read_line(&mut mig_choice).unwrap();

                if mig_choice.trim().to_lowercase() != "n" {
                    print!("   Enter an alias for this credential [Default: github_legacy]: ");
                    io::stdout().flush().unwrap();
                    let mut alias = String::new();
                    io::stdin().read_line(&mut alias).unwrap();
                    let alias = if alias.trim().is_empty() { "github_legacy".to_string() } else { alias.trim().to_string() };

                    print!("   Store in local Encrypted Vault instead of OS Keychain? (y/N): ");
                    io::stdout().flush().unwrap();
                    let mut vault_choice = String::new();
                    io::stdin().read_line(&mut vault_choice).unwrap();
                    let use_vault = vault_choice.trim().eq_ignore_ascii_case("y");

                    let mut cred_type = "ClassicToken".to_string();
                    let mut secret = String::new();
                    let mut extra = None;

                    if legacy_json.exists() {
                        if let Ok(content) = fs::read_to_string(legacy_json) {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                                if let Some(t) = json.get("type").and_then(|v| v.as_str()) {
                                    cred_type = t.to_string();
                                }
                                if let Some(t) = json.get("token").and_then(|v| v.as_str()) {
                                    secret = t.to_string();
                                } else if let Some(p) = json.get("key_path").and_then(|v| v.as_str()) {
                                    secret = p.to_string();
                                    if let Some(pass) = json.get("passphrase").and_then(|v| v.as_str()) {
                                        extra = Some(pass.to_string());
                                    }
                                }
                            }
                        }
                    } else if legacy_raw.exists() {
                        if let Ok(content) = fs::read_to_string(legacy_raw) {
                            secret = content.trim().to_string();
                        }
                    }

                    if !secret.is_empty() {
                        let cred = Credential { alias: alias.clone(), cred_type, secret, extra };
                        match save_credential(cred, use_vault) {
                            Ok(_) => {
                                println!("   ✓ Credential successfully migrated to {}.", if use_vault { "Encrypted Vault" } else { "OS Keychain" });
                                if !config_content.contains("github_cred_alias") {
                                    config_content.push_str(&format!("github_cred_alias = \"{}\"\n", alias));
                                }
                                let _ = fs::remove_file(legacy_json);
                                let _ = fs::remove_file(legacy_raw);
                                println!("   ✓ Insecure legacy credential files have been permanently removed.");
                            },
                            Err(e) => {
                                println!("   ❌ Failed to migrate credential: {}", e);
                                println!("   ⚠️  Legacy files have been preserved. You can try manually importing them using 'dam creds create'.");
                            }
                        }
                    } else {
                        println!("   ⚠️ Could not read contents of legacy credentials. Skipping migration.");
                    }
                } else {
                    println!("   ⚠️ Skipping migration. Please securely delete legacy credentials manually once you update to the new system.");
                }
            }
        }
        
        // Example for future migrations:
        // if project_version < (0, 5, 0) { ... apply 0.5.0 updates ... }

        // --- END MIGRATIONS ---
        
        // 2. Finalize version label in config
        if !config_content.contains("version =") {
            config_content.push_str(&format!("\nversion = \"{}\"\n", CURRENT_VERSION));
        } else {
            let lines: Vec<String> = config_content.lines().map(|l| {
                if l.trim().starts_with("version =") { format!("version = \"{}\"", CURRENT_VERSION) } else { l.to_string() }
            }).collect();
            config_content = lines.join("\n") + "\n";
        }
        
        fs::write(".dam/config.toml", config_content).unwrap();

        println!("Upgrade complete! Your reservoir has been safely updated to DAM v{}.", CURRENT_VERSION);
        return;
    }

    if Path::new(".dam").exists() {
        println!("Reservoir already exists here.");
        return;
    }

    let mut search_dir = env::current_dir().unwrap();
    let mut parent_exists = false;

    while search_dir.pop() {
        if search_dir.join(".dam").exists() {
            parent_exists = true;
            break;
        }
    }

    if parent_exists {
        println!("Warning: A parent directory already contains a .dam reservoir.");
        print!("Are you sure you want to create a nested, separate reservoir here? (y/N): ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Source initialization aborted.");
            return;
        }
    }

    println!("\n--- [DAM RESERVOIR INTERACTIVE SETUP] ---");

    let default_project_name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unnamed-reservoir".to_string());

    let project_name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            print!("1. Project name [Default: '{}']: ", default_project_name);
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            let trimmed = input.trim();
            if trimmed.is_empty() { default_project_name } else { trimmed.to_string() }
        }
    };

    println!("2. Law of Priority for Conflicts:");
    println!("   [p] Purities automatically win");
    println!("   [i] Impurities silently win");
    println!("   [n] Neither (prompt me every time)");
    print!("   Choose [p/i/n] [Default: n]: ");
    io::stdout().flush().unwrap();
    let mut priority_input = String::new();
    io::stdin().read_line(&mut priority_input).unwrap();
    let pref = priority_input.trim().to_lowercase();
    let (p_override, i_override) = if pref == "p" { (true, false) } else if pref == "i" { (false, true) } else { (false, false) };

    print!("3. Suppress parent nested directory warnings? (y/N) [Default: N]: ");
    io::stdout().flush().unwrap();
    let mut warning_input = String::new();
    io::stdin().read_line(&mut warning_input).unwrap();
    let suppress_warning = warning_input.trim().eq_ignore_ascii_case("y");

    print!("4. Enforce password protection on project exports? (y/N) [Default: N]: ");
    io::stdout().flush().unwrap();
    let mut pwd_input = String::new();
    io::stdin().read_line(&mut pwd_input).unwrap();
    let enforce_pwd = pwd_input.trim().eq_ignore_ascii_case("y");

    // NEW PROJECT FEATURE SETUP
    println!("\n--- [PROJECT EXPORT/IMPORT SETUP] ---");
    print!("5. Initialize Advanced Project Profiles (dam.toml)? (Y/n): ");
    io::stdout().flush().unwrap();
    let mut toml_input = String::new();
    io::stdin().read_line(&mut toml_input).unwrap();
    
    if !toml_input.trim().eq_ignore_ascii_case("n") {
        print!("   Which provider matches your project? (flutter/firebase/python/custom) [Default: custom]: ");
        io::stdout().flush().unwrap();
        let mut prov_input = String::new();
        io::stdin().read_line(&mut prov_input).unwrap();
        let prov_str = prov_input.trim();
        let prov_str = if prov_str.is_empty() { "custom" } else { prov_str };
        
        let provider = providers::get_provider(prov_str);
        let toml_content = provider.default_toml(&project_name);
        fs::write("dam.toml", toml_content).unwrap();
        println!("   Generated dam.toml with '{}' profiles.", prov_str);
    }

    fs::create_dir(".dam").unwrap();
    fs::create_dir(".dam/seals").unwrap();
    fs::create_dir(".dam/streams").unwrap();
    fs::create_dir(".dam/objects").unwrap();
    fs::write(".dam/CURRENT", "main").unwrap();

    let config_toml = format!(
        r#"[reservoir]
version = "{}"
type = "native"
name = "{}"
suppress_nested_warning = {}
purities_overrides_impurities = {}
impurities_overrides_purities = {}
enforce_password_on_project_import = {}
"#,
        CURRENT_VERSION, project_name, suppress_warning, p_override, i_override, enforce_pwd
    );

    fs::write(".dam/config.toml", config_toml).unwrap();
    fs::write(".dam/staging.json", "[]").unwrap();
    fs::write(".dam/streams/main", "").unwrap();

    println!("\n[Success] Reservoir initialized for project \"{}\".", project_name);
}