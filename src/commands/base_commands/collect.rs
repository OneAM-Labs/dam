use glob::Pattern;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[derive(Clone, Debug)]
struct Rule {
    pattern: String,
    base_dir: String,
}

impl Rule {
    fn matches(&self, path: &str) -> bool {
        if !self.base_dir.is_empty() && !path.starts_with(&self.base_dir) {
            return false;
        }

        let relative_path = if self.base_dir.is_empty() {
            path
        } else {
            &path[self.base_dir.len()..]
        };

        let pattern = if self.pattern.ends_with('/') {
            format!("{}**", self.pattern)
        } else {
            self.pattern.clone()
        };

        Pattern::new(&pattern)
            .map(|p| p.matches(relative_path))
            .unwrap_or(false)
    }
}

#[derive(Clone, Default)]
struct RuleSet {
    purities: Vec<Rule>,
    impurities: Vec<Rule>,
    dynamic_purities: bool,
    dynamic_impurities: bool,
}

pub fn run(
    path: String,
    override_purities: Option<String>,
    override_impurities: Option<String>,
    rule_priority: Option<String>,
) {
    let staging_path = ".dam/staging.json";
    let config_path = ".dam/config.toml";

    let mut tracked: Vec<String> = serde_json::from_str(
        &fs::read_to_string(staging_path).unwrap_or_else(|_| "[]".to_string()),
    )
    .unwrap_or_default();

    let target_path = Path::new(&path);
    if !target_path.exists() {
        println!("Error: Path '{}' does not exist.", path);
        return;
    }

    println!("\n--- [DAM COLLECTION RUN] ---");
    println!("Scanning target path: '{}'", path);

    let config_content = fs::read_to_string(config_path).unwrap_or_default();

    // 1. Resolve Rule Preference "Law" Priority
    let (priority_is_purities, priority_is_impurities, active_preference_msg) =
        if let Some(ref priority) = rule_priority {
            if priority.eq_ignore_ascii_case("purities") {
                (true, false, "CLI Override: Purities always win conflicts")
            } else if priority.eq_ignore_ascii_case("impurities") {
                (false, true, "CLI Override: Impurities always win conflicts")
            } else {
                println!(
                    "Warning: Unknown custom priority '{}'. Defaulting to reservoir config rules.",
                    priority
                );
                resolve_config_priority(&config_content)
            }
        } else {
            resolve_config_priority(&config_content)
        };
    println!("Conflict Resolution: {}", active_preference_msg);

    // 2. Prepare Rulesets based on bypass and custom path overrides
    let mut rules = RuleSet::default();

    match override_purities {
        Some(ref val) if val.is_empty() => {
            rules.dynamic_purities = false;
            println!("Purities Status: BYPASSED (Ignoring all .purities restrictions)");
        }
        Some(ref file_path) => {
            rules.dynamic_purities = false;
            let custom_path = Path::new(file_path);
            if custom_path.exists() {
                load_rules_from_file(custom_path, "", &mut rules.purities);
                println!(
                    "Purities Status: STATIC OVERRIDE (Using rules from '{}')",
                    file_path
                );
            } else {
                println!(
                    "Warning: Override purities file '{}' was not found! Proceeding with empty purities.",
                    file_path
                );
            }
        }
        None => {
            rules.dynamic_purities = true;
            println!("Purities Status: DYNAMIC (Scanning trees for local .purities)");
        }
    }

    match override_impurities {
        Some(ref val) if val.is_empty() => {
            rules.dynamic_impurities = false;
            println!("Impurities Status: BYPASSED (Ignoring all blocking rules)");
        }
        Some(ref file_path) => {
            rules.dynamic_impurities = false;
            let custom_path = Path::new(file_path);
            if custom_path.exists() {
                load_rules_from_file(custom_path, "", &mut rules.impurities);
                println!(
                    "Impurities Status: STATIC OVERRIDE (Using rules from '{}')",
                    file_path
                );
            } else {
                println!(
                    "Warning: Override impurities file '{}' was not found! Proceeding with empty impurities.",
                    file_path
                );
            }
        }
        None => {
            rules.dynamic_impurities = true;
            println!("Impurities Status: DYNAMIC (Scanning trees for local .impurities)");
        }
    }

    let mut items_to_collect = Vec::new();
    let mut conflicts = Vec::new();

    // Pre-load all ancestor rules before executing discovery so explicitly targeted directories
    // don't blind-skip the rules set by the root or parent directories.
    load_ancestor_rules(target_path, &mut rules);

    if target_path.is_file() {
        collect_single_file(target_path, &mut items_to_collect, &mut rules);
    } else {
        discover_items(
            target_path,
            &mut items_to_collect,
            &mut conflicts,
            &mut rules,
            true, // This is an explicit target, enable the override prompt
        );
    }

    // --- Conflict Resolution Phase ---
    if !conflicts.is_empty() {
        if priority_is_purities {
            println!(
                "Auto-resolved {} conflicts: Purities won (Files included seamlessly).",
                conflicts.len()
            );
            items_to_collect.extend(conflicts);
        } else if priority_is_impurities {
            println!(
                "Auto-resolved {} conflicts: Impurities won (Files silently excluded).",
                conflicts.len()
            );
            // We purposefully leave conflicts out of items_to_collect
        } else {
            // Both are false -> Warn and Prompt
            println!("\n[!] CONFLICT WARNING [!]");
            println!(
                "The following files explicitly match BOTH a .purities allowlist and an .impurities blocklist:"
            );
            for c in &conflicts {
                println!("  - {}", c);
            }

            print!(
                "\nBy default, these files will be EXCLUDED. Do you want to explicitly INCLUDE them anyway? (y/N): "
            );
            io::stdout().flush().unwrap();

            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();

            if input.trim().eq_ignore_ascii_case("y") {
                items_to_collect.extend(conflicts);
                println!("Files explicitly included for this collection run.");
            } else {
                println!("Files excluded (Impurities prevailed for this run).");
            }

            print!(
                "\nWould you like to save a default conflict resolution preference to avoid this warning in the future? \nType 'p' (Purities win), 'i' (Impurities win), or 'n' (Neither, keep warning me): "
            );
            io::stdout().flush().unwrap();

            let mut pref_input = String::new();
            io::stdin().read_line(&mut pref_input).unwrap();
            let pref = pref_input.trim().to_lowercase();

            if pref == "p" {
                set_config_preference(config_path, &config_content, true, false);
                println!("Preference saved: Purities will automatically win future conflicts.");
            } else if pref == "i" {
                set_config_preference(config_path, &config_content, false, true);
                println!(
                    "Preference saved: Impurities will automatically exclude future conflicts silently."
                );
            }
        }
    }

    if items_to_collect.is_empty() {
        println!("No valid files found to collect at '{}'.\n", path);
        return;
    }

    tracked.extend(items_to_collect);
    tracked.sort();
    tracked.dedup();

    fs::write(
        staging_path,
        serde_json::to_string_pretty(&tracked).unwrap(),
    )
    .unwrap();
    println!(
        "Successfully collected {} items into staging.\n",
        tracked.len()
    );
}

fn load_ancestor_rules(target_path: &Path, rules: &mut RuleSet) {
    let path_str = target_path.to_string_lossy();
    // If the target is the root itself, discover_items will handle loading it.
    let is_root = path_str == "." || path_str == "./" || path_str.is_empty();

    if !is_root {
        // 1. Always load root rules first
        if rules.dynamic_purities {
            load_rules_from_file(Path::new(".purities"), "", &mut rules.purities);
        }
        if rules.dynamic_impurities {
            load_rules_from_file(Path::new(".impurities"), "", &mut rules.impurities);
        }

        // 2. Progressively load rules down the ancestor chain
        let mut accum_path = std::path::PathBuf::new();
        if let Some(parent) = target_path.parent() {
            for component in parent.components() {
                let comp_str = component.as_os_str().to_string_lossy();
                if comp_str == "." {
                    continue;
                }

                accum_path.push(component);
                
                let mut dir_prefix = accum_path.to_string_lossy().replace('\\', "/");
                if !dir_prefix.is_empty() && !dir_prefix.ends_with('/') {
                    dir_prefix.push('/');
                }

                if rules.dynamic_purities {
                    load_rules_from_file(&accum_path.join(".purities"), &dir_prefix, &mut rules.purities);
                }
                if rules.dynamic_impurities {
                    load_rules_from_file(&accum_path.join(".impurities"), &dir_prefix, &mut rules.impurities);
                }
            }
        }
    }
}

fn set_config_preference(config_path: &str, content: &str, p_win: bool, i_win: bool) {
    let mut lines: Vec<String> = content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.starts_with("purities_overrides_impurities")
                && !trimmed.starts_with("impurities_overrides_purities")
        })
        .map(|s| s.to_string())
        .collect();

    lines.push(format!("purities_overrides_impurities = {}", p_win));
    lines.push(format!("impurities_overrides_purities = {}", i_win));

    fs::write(config_path, lines.join("\n") + "\n").unwrap();
}

fn resolve_config_priority(config_content: &str) -> (bool, bool, &'static str) {
    let p_win = config_content.contains("purities_overrides_impurities = true");
    let i_win = config_content.contains("impurities_overrides_purities = true");

    if p_win {
        (true, false, "Reservoir Config: Purities automatically win")
    } else if i_win {
        (
            false,
            true,
            "Reservoir Config: Impurities silently win (Files excluded)",
        )
    } else {
        (false, false, "Unset (Will warn on conflict)")
    }
}

fn load_rules_from_file(path: &Path, base_dir: &str, target_vec: &mut Vec<Rule>) {
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                target_vec.push(Rule {
                    pattern: trimmed.replace('\\', "/"),
                    base_dir: base_dir.to_string(),
                });
            }
        }
    }
}

fn collect_single_file(path: &Path, item_list: &mut Vec<String>, rules: &mut RuleSet) {
    let mut clean_path = path.to_string_lossy().replace('\\', "/");
    if clean_path.starts_with("./") {
        clean_path = clean_path[2..].to_string();
    }

    // load_ancestor_rules now handles gathering the relevant rules beforehand,
    // so we no longer need to check parent directory contents here.

    let has_purities_rules = !rules.purities.is_empty();
    let is_pure = rules.purities.iter().any(|r| r.matches(&clean_path));
    let is_impure = rules.impurities.iter().any(|r| r.matches(&clean_path));

    let passes = if has_purities_rules {
        is_pure && !is_impure
    } else {
        !is_impure
    };

    if passes {
        item_list.push(clean_path);
        return;
    }

    println!("\n[!] RULE WARNING [!]");
    if has_purities_rules && !is_pure {
        println!(
            "'{}' does not match any .purities allowlist rule.",
            clean_path
        );
    }
    if is_impure {
        println!("'{}' matches an .impurities blocklist rule.", clean_path);
    }

    print!("Do you want to explicitly include it anyway? (y/N): ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    if input.trim().eq_ignore_ascii_case("y") {
        item_list.push(clean_path);
        println!("File explicitly included for this collection run.");
    } else {
        println!("File excluded.");
    }
}

fn discover_items(
    path: &Path,
    item_list: &mut Vec<String>,
    conflicts: &mut Vec<String>,
    current_rules: &mut RuleSet,
    is_explicit_target: bool,
) {
    if path.components().any(|c| c.as_os_str() == ".dam") {
        return;
    }

    let mut path_str = path.to_string_lossy().replace('\\', "/");
    if path_str.starts_with("./") {
        path_str = path_str[2..].to_string();
    }
    if path_str == "." {
        path_str = String::new();
    }

    if path.is_dir() {
        let dir_prefix = if path_str.is_empty() {
            String::new()
        } else {
            format!("{}/", path_str)
        };

        let is_dir_impure = current_rules
            .impurities
            .iter()
            .any(|r| r.matches(&dir_prefix));
        let has_purities_rules = !current_rules.purities.is_empty();

        let mut should_block = false;
        let mut block_reason = "";

        if is_dir_impure && !has_purities_rules {
            should_block = true;
            block_reason = "matches an .impurities blocklist rule";
        } else if has_purities_rules {
            let dir_allowed = current_rules
                .purities
                .iter()
                .any(|r| r.matches(&dir_prefix) || r.pattern.starts_with(&dir_prefix));
            if !dir_allowed {
                should_block = true;
                block_reason = "does not match any .purities allowlist rule";
            }
        }

        let mut override_granted = false;

        // Override logic only kicks in for explicitly targeted folders breaking rules
        if should_block {
            if is_explicit_target && !path_str.is_empty() {
                println!("\n[!] DIRECTORY RULE WARNING [!]");
                println!("The targeted directory '{}' {}.", path_str, block_reason);
                print!("Do you want to explicitly include it and scan its contents anyway? (y/N): ");
                io::stdout().flush().unwrap();

                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();

                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Directory excluded.");
                    return;
                }
                println!("Directory explicitly included for this collection run.");
                override_granted = true;
            } else {
                return; // Silently ignore standard recursive subdirectories
            }
        }

        // Clean slate the inherited rules *only* if an explicit override was granted,
        // otherwise maintain strict global rules for the files inside.
        let mut local_rules = if override_granted {
            RuleSet {
                purities: Vec::new(),
                impurities: Vec::new(),
                dynamic_purities: current_rules.dynamic_purities,
                dynamic_impurities: current_rules.dynamic_impurities,
            }
        } else {
            current_rules.clone()
        };

        if local_rules.dynamic_purities {
            load_rules_from_file(
                &path.join(".purities"),
                &dir_prefix,
                &mut local_rules.purities,
            );
        }
        if local_rules.dynamic_impurities {
            load_rules_from_file(
                &path.join(".impurities"),
                &dir_prefix,
                &mut local_rules.impurities,
            );
        }

        if let Ok(entries) = fs::read_dir(path) {
            let mut entries = entries.flatten().peekable();

            if entries.peek().is_none() && !path_str.is_empty() {
                evaluate_and_push(&dir_prefix, item_list, conflicts, &local_rules);
            } else {
                for entry in entries {
                    // Pass false to ensure nested subdirectories remain silent upon failure
                    discover_items(&entry.path(), item_list, conflicts, &mut local_rules, false);
                }
            }
        }
    } else if path.is_file() {
        evaluate_and_push(&path_str, item_list, conflicts, current_rules);
    }
}

fn evaluate_and_push(
    clean_path: &str,
    item_list: &mut Vec<String>,
    conflicts: &mut Vec<String>,
    rules: &RuleSet,
) {
    let has_purities_rules = !rules.purities.is_empty();
    let is_pure = rules.purities.iter().any(|r| r.matches(clean_path));
    let is_impure = rules.impurities.iter().any(|r| r.matches(clean_path));

    if has_purities_rules {
        if is_pure {
            if is_impure {
                conflicts.push(clean_path.to_string());
            } else {
                item_list.push(clean_path.to_string());
            }
        }
    } else {
        if !is_impure {
            item_list.push(clean_path.to_string());
        }
    }
}
