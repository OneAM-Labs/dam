use std::fs;
use std::path::Path;

pub fn run(show_rules: bool, target: Option<String>) {
    if let Some(ref t) = target {
        if t.ends_with(".dam") {
            crate::core::project::inspect_project(t);
            return;
        }
    }

    // Legacy logic preserved
    if !Path::new(".dam").exists() {
        println!("No reservoir found. Run 'dam source' to begin.");
        return;
    }

    let current_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "unknown".to_string());
    println!("Current Flowing Stream: {}", current_stream.trim());

    let staging_content = fs::read_to_string(".dam/staging.json").unwrap_or_else(|_| "[]".to_string());
    let tracked: Vec<String> = serde_json::from_str(&staging_content).unwrap_or_default();

    println!("\nCollected items inside staging area:");
    if tracked.is_empty() {
        println!("  (Staging pool is dry. No files collected)");
    } else {
        for file in &tracked {
            println!("  [Staged] {}", file);
        }
    }

    if show_rules {
        println!("\n--- [RULE STRUCTURE: PURITIES & IMPURITIES] ---");
        let mut purities_count = 0;
        let mut impurities_count = 0;
        
        println!("Active Rule Files in Reservoir:");
        scan_rules(Path::new("."), &mut purities_count, &mut impurities_count);
        
        if purities_count == 0 && impurities_count == 0 {
            println!("  (No .purities or .impurities files found in this workspace)");
        }
        
        println!("\nSummary:");
        println!("  Total .purities files found:  {}", purities_count);
        println!("  Total .impurities files found: {}", impurities_count);
    }
}

fn scan_rules(dir: &Path, p_count: &mut usize, i_count: &mut usize) {
    if dir.components().any(|c| c.as_os_str() == ".dam" || c.as_os_str() == ".git") {
        return;
    }
    
    if let Ok(entries) = fs::read_dir(dir) {
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                if file_name == ".purities" {
                    *p_count += 1;
                    let display_path = path.to_string_lossy().replace('\\', "/");
                    let clean_path = display_path.strip_prefix("./").unwrap_or(&display_path);
                    println!("  [Purity]   {}", clean_path);
                } else if file_name == ".impurities" {
                    *i_count += 1;
                    let display_path = path.to_string_lossy().replace('\\', "/");
                    let clean_path = display_path.strip_prefix("./").unwrap_or(&display_path);
                    println!("  [Impurity] {}", clean_path);
                }
            } else if path.is_dir() {
                subdirs.push(path);
            }
        }
        
        for subdir in subdirs {
            scan_rules(&subdir, p_count, i_count);
        }
    }
}