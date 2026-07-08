use std::fs;
use std::path::Path;

/// All the things `drain` knows how to clear.
/// Add a new variant here and handle it in `run` to extend the command.
pub enum DrainTarget {
    /// Wipe the staging area back to an empty array.
    Collection,
    // Future targets can go here, e.g.:
    // Cache,
    // Conflicts,
}

impl DrainTarget {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "collection" => Some(Self::Collection),
            _ => None,
        }
    }
}

pub fn run(target: String) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found.");
        return;
    }

    match DrainTarget::from_str(&target) {
        Some(DrainTarget::Collection) => drain_collection(),
        None => {
            println!("Unknown drain target: '{}'", target);
            println!();
            println!("Available targets:");
            println!("  collection   Clear the staging area");
        }
    }
}

fn drain_collection() {
    let staging_path = ".dam/staging.json";

    let tracked: Vec<String> = serde_json::from_str(
        &fs::read_to_string(staging_path).unwrap_or_else(|_| "[]".to_string()),
    )
    .unwrap_or_default();

    if tracked.is_empty() {
        println!("Staging area is already empty.");
        return;
    }

    println!("{} item(s) currently staged:", tracked.len());
    for item in &tracked {
        println!("  - {}", item);
    }

    fs::write(staging_path, "[]").unwrap();
    println!("\nCollection drained. Staging area is now empty.");
}