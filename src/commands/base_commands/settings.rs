use std::fs;
use std::io::{self, Write};
use std::path::Path;

const CONFIG_PATH: &str = ".dam/config.toml";

pub fn run(key: Option<String>, value: Option<String>, interactive: bool) {
    if !Path::new(CONFIG_PATH).exists() {
        println!("Error: No reservoir found. Run 'dam source' to initialize one first.");
        return;
    }

    let config_content = fs::read_to_string(CONFIG_PATH).unwrap_or_default();

    if interactive || (key.is_none() && value.is_none()) {
        if interactive {
            interactive_menu(config_content);
        } else {
            display_all_settings(&config_content);
        }
    } else if let Some(k) = key {
        if let Some(v) = value {
            update_single_setting(&config_content, &k, &v);
        } else {
            read_single_setting(&config_content, &k);
        }
    }
}

fn display_all_settings(content: &str) {
    println!("\n--- [DAM RESERVOIR ACTIVE SETTINGS] ---");
    println!("reservoir.name                     = \"{}\"", get_toml_val(content, "name").unwrap_or_default());
    println!("purities_overrides_impurities      = {}", get_toml_val(content, "purities_overrides_impurities").unwrap_or_else(|| "false".to_string()));
    println!("impurities_overrides_purities      = {}", get_toml_val(content, "impurities_overrides_purities").unwrap_or_else(|| "false".to_string()));
    println!("suppress_nested_warning            = {}", get_toml_val(content, "suppress_nested_warning").unwrap_or_else(|| "false".to_string()));
    println!("enforce_password_on_project_import = {}", get_toml_val(content, "enforce_password_on_project_import").unwrap_or_else(|| "false".to_string()));
    println!("github_repo                        = \"{}\"", get_toml_val(content, "github_repo").unwrap_or_default());
    println!("github_token_path                  = \"{}\"", get_toml_val(content, "github_token_path").unwrap_or_default());
    println!("\n* Tip: To edit a setting, run: ");
    println!("  dam settings <key> <value>  OR  dam settings --interactive\n");
}

pub fn get_toml_val(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) {
            if let Some(eq_idx) = trimmed.find('=') {
                let value_part = trimmed[eq_idx + 1..].trim();
                if value_part.starts_with('"') && value_part.ends_with('"') {
                    return Some(value_part[1..value_part.len() - 1].to_string());
                }
                return Some(value_part.to_string());
            }
        }
    }
    None
}

pub fn set_toml_val(content: &str, key: &str, new_value: &str, is_string: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    let formatted = if is_string {
        format!("\"{}\"", new_value)
    } else {
        new_value.to_string()
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) {
            if trimmed.contains('=') {
                let indent_len = line.len() - line.trim_start().len();
                let indentation = &line[0..indent_len];
                lines.push(format!("{}{} = {}", indentation, key, formatted));
                found = true;
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if !found {
        lines.push(format!("{} = {}", key, formatted));
    }

    lines.join("\n") + "\n"
}

fn update_single_setting(content: &str, key: &str, value: &str) {
    let normalized_key = key.trim().to_lowercase().replace("reservoir.", "");
    
    let (clean_key, is_string) = match normalized_key.as_str() {
        "name" => ("name", true),
        "purities_overrides_impurities" => ("purities_overrides_impurities", false),
        "impurities_overrides_purities" => ("impurities_overrides_purities", false),
        "suppress_nested_warning" | "suppress" => ("suppress_nested_warning", false),
        "enforce_password_on_project_import" | "enforce_password" => ("enforce_password_on_project_import", false),
        "github_repo" | "repo" => ("github_repo", true),
        "github_token_path" | "token_path" => ("github_token_path", true),
        _ => {
            println!("Error: Unsupported setting key '{}'.", key);
            return;
        }
    };

    if !is_string && value != "true" && value != "false" {
        println!("Error: Value must be 'true' or 'false'. Received '{}'.", value);
        return;
    }

    let mut updated_config = set_toml_val(content, clean_key, value, is_string);
    
    // Automatically disable opposite conflict resolution paths if one is toggled on
    if value == "true" {
        if clean_key == "purities_overrides_impurities" {
            updated_config = set_toml_val(&updated_config, "impurities_overrides_purities", "false", false);
        } else if clean_key == "impurities_overrides_purities" {
            updated_config = set_toml_val(&updated_config, "purities_overrides_impurities", "false", false);
        }
    }

    fs::write(CONFIG_PATH, updated_config).unwrap();
    println!("Updated setting: {} = {}", clean_key, value);
}

fn read_single_setting(content: &str, key: &str) {
    let normalized_key = key.trim().to_lowercase().replace("reservoir.", "");
    let clean_key = match normalized_key.as_str() {
        "name" => "name",
        "purities_overrides_impurities" => "purities_overrides_impurities",
        "impurities_overrides_purities" => "impurities_overrides_purities",
        "suppress_nested_warning" | "suppress" => "suppress_nested_warning",
        "enforce_password_on_project_import" | "enforce_password" => "enforce_password_on_project_import",
        "github_repo" | "repo" => "github_repo",
        "github_token_path" | "token_path" => "github_token_path",
        _ => {
            println!("Error: Unsupported setting key '{}'.", key);
            return;
        }
    };

    if let Some(val) = get_toml_val(content, clean_key) {
        println!("{}", val);
    } else {
        println!("Setting '{}' is not configured.", clean_key);
    }
}

fn interactive_menu(mut content: String) {
    loop {
        let current_name = get_toml_val(&content, "name").unwrap_or_else(|| "unnamed".to_string());
        let p_overrides = get_toml_val(&content, "purities_overrides_impurities").unwrap_or_else(|| "false".to_string());
        let i_overrides = get_toml_val(&content, "impurities_overrides_purities").unwrap_or_else(|| "false".to_string());
        let current_nested = get_toml_val(&content, "suppress_nested_warning").unwrap_or_else(|| "false".to_string());
        let current_pwd = get_toml_val(&content, "enforce_password_on_project_import").unwrap_or_else(|| "false".to_string());
        let current_repo = get_toml_val(&content, "github_repo").unwrap_or_else(|| "".to_string());
        let current_token_path = get_toml_val(&content, "github_token_path").unwrap_or_else(|| "".to_string());

        println!("\n=======================================================");
        println!("             DAM CONFIGURATION DASHBOARD               ");
        println!("=======================================================");
        println!(" 1. Reservoir Name                   : {}", current_name);
        println!(" 2. Conflict: Purities Win           : {}", p_overrides);
        println!(" 3. Conflict: Impurities Win         : {}", i_overrides);
        println!(" 4. Suppress Nesting Warnings        : {}", current_nested);
        println!(" 5. Enforce Password Encrypt         : {}", current_pwd);
        println!(" 6. GitHub Repository Target         : {}", if current_repo.is_empty() { "[Not Configured]" } else { &current_repo });
        println!(" 7. GitHub Custom Token Path         : {}", if current_token_path.is_empty() { "[None - Checks Default Locations]" } else { &current_token_path });
        println!(" 8. Save and Exit");
        println!("=======================================================");
        print!("Choose option (1-8): ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();

        match choice.trim() {
            "1" => {
                print!("Enter new project/reservoir name: ");
                io::stdout().flush().unwrap();
                let mut next_name = String::new();
                io::stdin().read_line(&mut next_name).unwrap();
                let trimmed = next_name.trim();
                if !trimmed.is_empty() {
                    content = set_toml_val(&content, "name", trimmed, true);
                }
            }
            "2" => {
                let next_val = if p_overrides == "true" { "false" } else { "true" };
                content = set_toml_val(&content, "purities_overrides_impurities", next_val, false);
                if next_val == "true" {
                    content = set_toml_val(&content, "impurities_overrides_purities", "false", false);
                }
                println!("Rule changed: 'Purities Win' is now {}", next_val);
            }
            "3" => {
                let next_val = if i_overrides == "true" { "false" } else { "true" };
                content = set_toml_val(&content, "impurities_overrides_purities", next_val, false);
                if next_val == "true" {
                    content = set_toml_val(&content, "purities_overrides_impurities", "false", false);
                }
                println!("Rule changed: 'Impurities Win' is now {}", next_val);
            }
            "4" => {
                let next_val = if current_nested == "true" { "false" } else { "true" };
                content = set_toml_val(&content, "suppress_nested_warning", next_val, false);
                println!("Rule changed: 'Suppress Nesting Warnings' is now {}", next_val);
            }
            "5" => {
                let next_val = if current_pwd == "true" { "false" } else { "true" };
                content = set_toml_val(&content, "enforce_password_on_project_import", next_val, false);
                println!("Rule changed: 'Enforce Password Encrypt' is now {}", next_val);
            }
            "6" => {
                print!("Enter GitHub Target Repository (e.g., owner/repo_name): ");
                io::stdout().flush().unwrap();
                let mut next_repo = String::new();
                io::stdin().read_line(&mut next_repo).unwrap();
                let trimmed = next_repo.trim();
                if !trimmed.is_empty() {
                    content = set_toml_val(&content, "github_repo", trimmed, true);
                    println!("GitHub Target Repository changed to: {}", trimmed);
                }
            }
            "7" => {
                print!("Enter Path to Token File (or press Enter to reset to defaults): ");
                io::stdout().flush().unwrap();
                let mut next_path = String::new();
                io::stdin().read_line(&mut next_path).unwrap();
                let trimmed = next_path.trim();
                content = set_toml_val(&content, "github_token_path", trimmed, true);
                if trimmed.is_empty() {
                    println!("GitHub custom token path cleared.");
                } else {
                    println!("GitHub custom token path set to: {}", trimmed);
                }
            }
            "8" | "" => {
                fs::write(CONFIG_PATH, &content).unwrap();
                println!("Changes successfully committed to config.toml!");
                break;
            }
            _ => {
                println!("Invalid selection. Please try again.");
            }
        }
    }
}