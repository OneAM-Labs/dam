use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use crate::providers;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn run(name: Option<String>, upgrade: bool) {
    if upgrade {
        if !Path::new(".dam").exists() {
            println!("Error: No reservoir found to upgrade. Run 'dam source' to initialize one.");
            return;
        }
        println!("--- [DAM RESERVOIR UPGRADE] ---");
        let mut config_content = fs::read_to_string(".dam/config.toml").unwrap_or_default();
        
        if !config_content.contains("version =") {
            config_content.push_str(&format!("\nversion = \"{}\"\n", CURRENT_VERSION));
        } else {
            let lines: Vec<String> = config_content.lines().map(|l| {
                if l.trim().starts_with("version =") { format!("version = \"{}\"", CURRENT_VERSION) } else { l.to_string() }
            }).collect();
            config_content = lines.join("\n") + "\n";
        }
        
        if !config_content.contains("enforce_password_on_project_import") {
            config_content.push_str("enforce_password_on_project_import = false\n");
        }
        
        fs::write(".dam/config.toml", config_content).unwrap();

        if !Path::new("dam.toml").exists() {
            println!("Generating missing dam.toml profile...");
            let provider = crate::providers::get_provider("custom");
            fs::write("dam.toml", provider.default_toml("upgraded-project")).unwrap();
        }
        println!("Upgrade complete! You are now fully migrated to DAM v{}.", CURRENT_VERSION);
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