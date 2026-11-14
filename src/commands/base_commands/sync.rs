use crate::platforms;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::thread;
use std::time::Duration;

fn log_verbose(verbose: bool, message: &str) {
    if verbose {
        println!("{}", message);
    }
}

fn with_spinner<R, F: FnOnce() -> R>(verbose: bool, message: &str, f: F) -> R {
    if verbose {
        println!("{}", message);
        return f();
    }

    print!("{} ", message);
    io::stdout().flush().unwrap();

    let running = Arc::new(AtomicBool::new(true));
    let spinner_running = running.clone();
    let message_owned = message.to_string();
    let message_for_spinner = message_owned.clone();

    let handle = thread::spawn(move || {
        let frames = ['|', '/', '-', '\\'];
        let mut idx = 0;
        while spinner_running.load(Ordering::Relaxed) {
            print!("\r{} {}", message_for_spinner, frames[idx]);
            io::stdout().flush().unwrap();
            idx = (idx + 1) % frames.len();
            thread::sleep(Duration::from_millis(120));
        }
    });

    let result = f();
    running.store(false, Ordering::Relaxed);
    let _ = handle.join();
    print!("\r{} done\n", message_owned);
    io::stdout().flush().unwrap();
    result
}

/// Plain output with no spinner: push prints its own per-file progress, and a spinner's
/// `\r` redraw would overwrite it (and print "done" even when the push failed).
fn with_progress<R, F: FnOnce() -> R>(message: &str, f: F) -> R {
    println!("{}", message);
    f()
}

fn without_spinner<R, F: FnOnce() -> R>(verbose: bool, message: &str, f: F) -> R {
    if verbose {
        println!("{}", message);
        return f();
    }

    print!("{} ", message);
    io::stdout().flush().unwrap();
    let result = f();
    println!("done");
    io::stdout().flush().unwrap();
    result
}

pub fn run(
    stream: Option<String>,
    action: Option<String>,
    platform_arg: Option<String>,
    force: bool,
    verbose: bool,
    clone_url: Option<String>,
) {
    let provider_name = platform_arg.unwrap_or_else(|| "github".to_string());
    println!(
        "\n--- [DAM CLOUD SYNC: {}] ---",
        provider_name.to_uppercase()
    );

    let provider = platforms::get_provider(&provider_name);

    let streams_to_sync = if let Some(ref s) = stream {
        let path = format!(".dam/streams/{}", s);
        if !Path::new(&path).exists() {
            println!(
                "❌ Error: Stream '{}' does not exist. Aborting synchronization.",
                s
            );
            return;
        }
        vec![s.clone()]
    } else {
        let current = fs::read_to_string(".dam/CURRENT")
            .unwrap_or_else(|_| "main".to_string())
            .trim()
            .to_string();
        vec![current]
    };

    log_verbose(verbose, &format!("Sync target resolved: provider={}, force={}, stream={:?}, clone={:?}", provider_name, force, stream, clone_url));

    if let Some(clone_source) = clone_url {
        println!("🔄 Clone-based sync requested. Importing URL/Repo: {}", clone_source);
        crate::commands::import::run(clone_source, false, None);
        return;
    }

    for s in streams_to_sync {
        if s == "main" && !force {
            println!("\n⚠️  You are about to synchronize the main Stream.");
            println!(
                "This is generally discouraged because it is intended to remain your stable development Stream."
            );
            print!("Continue? (y/N): ");
            io::stdout().flush().unwrap();
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();
            if choice.trim().to_lowercase() != "y" {
                println!("Skipping main stream.");
                continue;
            }
        }

        println!("\n🔄 Synchronizing Stream: {}", s);

        if let Some(ref act) = action {
            match act.to_lowercase().as_str() {
                "push" => {
                    let result = with_progress(&format!("Pushing stream '{}'...", s), || provider.push(&s));
                    if let Err(e) = result {
                        println!("❌ Push failed for {}: {}", s, e);
                    }
                }
                "pull" => {
                    let result = without_spinner(verbose, &format!("Pulling stream '{}'...", s), || provider.pull(&s));
                    if let Err(e) = result {
                        println!("❌ Pull failed for {}: {}", s, e);
                    } else {
                        apply_workspace_if_current(&s, verbose);
                        verify_sync_status(&*provider, &s, verbose);
                    }
                }
                _ => {
                    println!("Error: Unknown action '{}'.", act);
                }
            }
            continue;
        }

        // Interactive Diff
        let diff_result = with_spinner(verbose, &format!("Checking remote diff for '{}'...", s), || provider.check_diff(&s));
        match diff_result {
            Ok((ahead, behind)) => {
                if ahead == 0 && behind == 0 {
                    println!("✅ Stream '{}' is completely up to date.", s);
                    continue;
                }

                println!("📊 State Diff for {}:", s);
                if ahead > 0 {
                    println!("  ↑ {} local seal(s) ahead.", ahead);
                }
                if behind > 0 {
                    println!("  ↓ {} remote change(s) missing.", behind);
                }

                if ahead > 0 && behind > 0 {
                    println!("⚠️  CONFLICT DETECTED in '{}'", s);
                    println!("  [1] Pull remote changes");
                    println!("  [2] Force Push local state");
                    print!("Choice [1/2/Skip]: ");
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();

                    if choice.trim() == "1" {
                        let result = without_spinner(verbose, &format!("Pulling stream '{}'...", s), || provider.pull(&s));
                        if let Err(e) = result {
                            println!("❌ Pull failed: {}", e);
                        } else {
                            apply_workspace_if_current(&s, verbose);
                            verify_sync_status(&*provider, &s, verbose);
                        }
                    } else if choice.trim() == "2" {
                        let result = with_progress(&format!("Pushing stream '{}'...", s), || provider.push(&s));
                        if let Err(e) = result {
                            println!("❌ Push failed: {}", e);
                        } else {
                            verify_sync_status(&*provider, &s, verbose);
                        }
                    }
                } else if ahead > 0 {
                    print!("Push local seals for '{}'? (Y/n): ", s);
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    if choice.trim().to_lowercase() != "n" {
                        let result = with_progress(&format!("Pushing stream '{}'...", s), || provider.push(&s));
                        if let Err(e) = result {
                            println!("❌ Push failed: {}", e);
                        } else {
                            verify_sync_status(&*provider, &s, verbose);
                        }
                    }
                } else if behind > 0 {
                    print!("Pull latest changes for '{}'? (Y/n): ", s);
                    io::stdout().flush().unwrap();
                    let mut choice = String::new();
                    io::stdin().read_line(&mut choice).unwrap();
                    if choice.trim().to_lowercase() != "n" {
                        let result = without_spinner(verbose, &format!("Pulling stream '{}'...", s), || provider.pull(&s));
                        if let Err(e) = result {
                            println!("❌ Pull failed: {}", e);
                        } else {
                            apply_workspace_if_current(&s, verbose);
                            verify_sync_status(&*provider, &s, verbose);
                        }
                    }
                }
            }
            Err(e) => {
                println!("❌ Failed to fetch diff for {}: {}", s, e);
            }
        }
    }
}

fn verify_sync_status(provider: &dyn platforms::SyncProvider, stream: &str, verbose: bool) {
    println!("\n🔍 Verifying DAM sync state for '{}'...", stream);
    // Retry verification a few times to account for remote eventual consistency after pushes.
    let mut attempt = 0;
    let max_attempts = 5;
    loop {
        attempt += 1;
        match with_spinner(verbose, &format!("Verifying sync status for '{}' (attempt {}/{})...", stream, attempt, max_attempts), || provider.check_diff(stream)) {
            Ok((ahead, behind)) => {
                if ahead == 0 && behind == 0 {
                    println!("✅ Verification succeeded: '{}' is synchronized.", stream);
                    return;
                } else {
                    println!("⚠️ Verification detected remaining differences for '{}':", stream);
                    if ahead > 0 {
                        println!("  ↑ {} local seal(s) still ahead.", ahead);
                    }
                    if behind > 0 {
                        println!("  ↓ {} remote DAM-formatted change(s) still missing.", behind);
                    }
                    if attempt >= max_attempts {
                        println!("  Suggestion: re-run 'dam sync --action pull' or inspect the remote repository history.");
                        return;
                    }
                    // Wait briefly before retrying to allow GitHub eventual consistency
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    continue;
                }
            }
            Err(e) => {
                println!("❌ Verification failed for '{}': {}", stream, e);
                return;
            }
        }
    }
}

/// Helper function to automatically update workspace files on disk after pulling active stream
fn apply_workspace_if_current(stream: &str, verbose: bool) {
    let current_stream = fs::read_to_string(".dam/CURRENT")
        .unwrap_or_else(|_| "main".to_string())
        .trim()
        .to_string();
    if stream == &current_stream {
        let meta = crate::commands::stream::get_or_create_meta(stream);
        if let Some(ref latest) = meta.latest_seal {
            println!("📂 Pulled seal '{}' is available.", latest);
            print!("Apply pulled seal to workspace files now? (Y/n): ");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).unwrap();
            if input.trim().is_empty() || input.trim().eq_ignore_ascii_case("y") {
                println!(
                    "📂 Applying pulled seal '{}' to active workspace files...",
                    latest
                );
                crate::commands::apply::run(latest.clone(), false);
            } else {
                println!("Skipped applying pulled seal '{}'.", latest);
            }
            // After attempting to pull & optionally apply, re-check remote diff to detect divergence
            let provider = crate::platforms::get_provider(&std::env::var("DAM_SYNC_PROVIDER").unwrap_or_else(|_| "github".to_string()));
            let check_result = with_spinner(verbose, &format!("Re-checking remote diff for '{}'...", stream), || provider.check_diff(stream));
            if let Ok((_ahead, behind)) = check_result {
                if behind > 0 {
                    println!("\n⚠️  Detected that remote still has changes ({} commits behind).", behind);
                    println!("This can happen if the remote was merged outside of DAM's commit format.");
                    println!("Clearing local latest seal for stream '{}' to allow rebuild/resync.", stream);
                    let mut meta = crate::commands::stream::get_or_create_meta(stream);
                    meta.latest_seal = None;
                    crate::commands::stream::save_meta(&meta);
                    println!("Suggestion: Consider using a standard merge commit message that preserves DAM metadata, or recreate the stream history as needed.");
                }
            }
        }
    }
}