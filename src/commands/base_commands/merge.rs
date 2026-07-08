use std::fs;
use std::path::Path;
use std::io::{self, Read, Write};
use serde::{Deserialize, Serialize};
use flate2::read::ZlibDecoder;
use crate::commands::seal::{Seal, FileEntry, run as run_seal};
use crate::commands::stream::{get_or_create_meta};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PendingMerge {
    pub source_stream: String,
    pub target_stream: String,
    pub base_seal: Option<String>,
    pub source_seal: String,
    pub target_seal: String,
    pub files: Vec<FileEntry>,
    pub conflicts: Vec<String>,
}

pub fn run(source: Option<String>, apply: bool) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found.");
        return;
    }

    if apply {
        apply_pending_merge();
        return;
    }

    let source_stream_name = match source {
        Some(s) => s,
        None => {
            println!("Error: Must specify a source stream name to merge (e.g. `dam merge <stream-name>`) or use `--apply`.");
            return;
        }
    };

    let active_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();
    if active_stream == source_stream_name {
        println!("Cannot merge stream '{}' into itself.", active_stream);
        return;
    }

    let active_meta = get_or_create_meta(&active_stream);
    let source_meta = get_or_create_meta(&source_stream_name);

    let active_head = match &active_meta.latest_seal {
        Some(s) => s.clone(),
        None => {
            println!("Active stream '{}' has no historical seals yet. Commit something first.", active_stream);
            return;
        }
    };

    let source_head = match &source_meta.latest_seal {
        Some(s) => s.clone(),
        None => {
            println!("Source stream '{}' has no seals to merge.", source_stream_name);
            return;
        }
    };

    println!("Calculating smart merge: {} ➔ {}", source_stream_name, active_stream);

    // Identify Common Ancestor (Base Seal) by climbing the DAG back
    let base_seal_id = find_common_ancestor(&active_head, &source_head);
    if let Some(ref base) = base_seal_id {
        println!("Found common ancestor base seal: {}", base);
    } else {
        println!("No common ancestor found (merging from independent root streams).");
    }

    // Load Seals
    let base_seal = base_seal_id.as_ref().and_then(|id| load_seal(id));
    let active_seal = load_seal(&active_head).unwrap();
    let source_seal = load_seal(&source_head).unwrap();

    let mut resolved_files: Vec<FileEntry> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();

    // Smart 3-way conflict resolution map
    // We map every unique path from all three inputs.
    let mut all_paths = std::collections::HashSet::new();
    if let Some(ref base) = base_seal {
        for f in &base.files { all_paths.insert(f.path.clone()); }
    }
    for f in &active_seal.files { all_paths.insert(f.path.clone()); }
    for f in &source_seal.files { all_paths.insert(f.path.clone()); }

    for path in all_paths {
        let base_f = base_seal.as_ref().and_then(|b| b.files.iter().find(|f| f.path == path));
        let active_f = active_seal.files.iter().find(|f| f.path == path);
        let source_f = source_seal.files.iter().find(|f| f.path == path);

        match (base_f, active_f, source_f) {
            // Case 1: Existed in base, unchanged in active, changed/removed in source
            (Some(b), Some(a), Some(s)) if a.hash == b.hash && s.hash != b.hash => {
                resolved_files.push(s.clone()); // Source changes win
            }
            (Some(b), Some(a), None) if a.hash == b.hash => {
                // Deleted in source, untouched in active -> remains deleted (do not push to resolved_files)
            }

            // Case 2: Existed in base, changed/removed in active, unchanged in source
            (Some(b), Some(a), Some(s)) if s.hash == b.hash && a.hash != b.hash => {
                resolved_files.push(a.clone()); // Active changes win
            }
            (Some(b), None, Some(s)) if s.hash == b.hash => {
                // Deleted in active, untouched in source -> remains deleted
            }

            // Case 3: Existed in base, modified differently in both active and source
            (Some(b), Some(a), Some(s)) if a.hash != b.hash && s.hash != b.hash => {
                if a.hash == s.hash {
                    resolved_files.push(a.clone()); // Both did the same modification
                } else {
                    // Conflict! Differing modifications
                    conflicts.push(path.clone());
                    resolved_files.push(a.clone()); // Place active file as default
                }
            }

            // Case 4: Existed in base, deleted in both
            (Some(_), None, None) => {}

            // Case 5: New file in both streams (not in base)
            (None, Some(a), Some(s)) => {
                if a.hash == s.hash {
                    resolved_files.push(a.clone());
                } else {
                    conflicts.push(path.clone());
                    resolved_files.push(a.clone()); // Place active file as default
                }
            }

            // Case 6: New file only in source
            (None, None, Some(s)) => {
                resolved_files.push(s.clone());
            }

            // Case 7: New file only in active
            (None, Some(a), None) => {
                resolved_files.push(a.clone());
            }

            // Fallback safety
            _ => {
                if let Some(a) = active_f {
                    resolved_files.push(a.clone());
                }
            }
        }
    }

    let pending = PendingMerge {
        source_stream: source_stream_name.clone(),
        target_stream: active_stream,
        base_seal: base_seal_id,
        source_seal: source_head,
        target_seal: active_head,
        files: resolved_files,
        conflicts: conflicts.clone(),
    };

    // Store the calculated candidates to draft file
    fs::write(".dam/pending_merge.json", serde_json::to_string_pretty(&pending).unwrap()).unwrap();

    println!("\n⚖️  Merge Plan Formulated!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Source Stream : {}", pending.source_stream);
    println!("Files Resolved: {}", pending.files.len());
    println!("Conflicts Found: {}", conflicts.len());

    if !conflicts.is_empty() {
        println!("\n⚠️  CONFLICTION DETECTED in the following paths:");
        for c in &conflicts {
            println!("  ! {}", c);
        }
        println!("\nMerge created with conflicts marked. Workspace active files will default to your stream versions.");
    }

    println!("\nTo apply and commit this merge, run:\n  dam merge --apply");
}

fn apply_pending_merge() {
    let pending_path = Path::new(".dam/pending_merge.json");
    if !pending_path.exists() {
        println!("No pending merge found to apply. Calculate a merge first: `dam merge <stream-name>`");
        return;
    }

    let content = fs::read_to_string(pending_path).unwrap();
    let pending: PendingMerge = serde_json::from_str(&content).unwrap();

    println!("\nPending Merge Evaluation");
    println!("=========================");
    println!("Targeting current workspace flow from: {}", pending.source_stream);
    println!("Number of tracks to merge            : {} file entries", pending.files.len());

    if !pending.conflicts.is_empty() {
        println!("⚠️  WARNING: {} structural conflicts were detected in this merge candidate.", pending.conflicts.len());
        println!("Applying this will overwrite conflicts with the target stream's default versions.");
    }

    print!("\nApply and seal this merge into history? (y/N): ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Aborted.");
        return;
    }

    // Restore workspace files from the objects pool matching the merge resolution
    for entry in &pending.files {
        if entry.is_dir {
            fs::create_dir_all(&entry.path).ok();
            continue;
        }

        let obj_path = Path::new(".dam/objects").join(&entry.hash);
        if obj_path.exists() {
            // Decompress Zlib object and write out to path
            let mut file = fs::File::open(obj_path).unwrap();
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).unwrap();

            let mut decoder = ZlibDecoder::new(&buffer[..]);
            let mut decompressed = Vec::new();
            if decoder.read_to_end(&mut decompressed).is_ok() {
                if let Some(parent) = Path::new(&entry.path).parent() {
                    fs::create_dir_all(parent).ok();
                }
                fs::write(&entry.path, decompressed).unwrap();
            }
        }
    }

    // To preserve staging state while generating a sealed entry, we can put resolved paths temporarily into staging
    let paths_json: Vec<String> = pending.files.iter().map(|f| f.path.clone()).collect();
    fs::write(".dam/staging.json", serde_json::to_string(&paths_json).unwrap()).unwrap();

    // Generate merged seal. The parent list must be multiparent [TargetHead, SourceHead]
    let msg = format!("Merge stream '{}' into '{}'", pending.source_stream, pending.target_stream);
    let new_seal_id = run_seal(msg, vec![pending.source_seal.clone()]);

    // Clean up candidate file
    fs::remove_file(pending_path).ok();

    println!("🌊 Successfully merged and committed as Seal {}!", new_seal_id);
}

fn load_seal(id: &str) -> Option<Seal> {
    let path = format!(".dam/seals/{}.json", id);
    if let Ok(content) = fs::read_to_string(path) {
        serde_json::from_str::<Seal>(&content).ok()
    } else {
        None
    }
}

// Climb backward tracking parent lines using standard breath-first search to find intersection
fn find_common_ancestor(a: &str, b: &str) -> Option<String> {
    let mut a_lineage = std::collections::HashSet::new();
    let mut queue = vec![a.to_string()];

    while let Some(current) = queue.pop() {
        if a_lineage.insert(current.clone()) {
            if let Some(seal) = load_seal(&current) {
                for parent in &seal.parents {
                    queue.push(parent.clone());
                }
            }
        }
    }

    let mut b_queue = vec![b.to_string()];
    while let Some(current) = b_queue.pop() {
        if a_lineage.contains(&current) {
            return Some(current);
        }
        if let Some(seal) = load_seal(&current) {
            for parent in &seal.parents {
                b_queue.push(parent.clone());
            }
        }
    }

    None
}