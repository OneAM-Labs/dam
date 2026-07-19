use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UPDATE_URL: &str = "https://dam-pcp.web.app/latest.json";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- TERMINAL COLOR SUPPORT --- //
//
// Decides once per run whether it's safe to emit ANSI escape codes, so
// dumb terminals, piped output, and CI logs get plain text instead of
// garbage like "^[[1;33m" in their output.

/// On Windows, ANSI escapes are only rendered by the console if
/// `ENABLE_VIRTUAL_TERMINAL_PROCESSING` is turned on for the output
/// handle. Modern Windows Terminal turns this on itself, but plain
/// `cmd.exe` and older PowerShell hosts do not — this is the same gap
/// that Python's `rich` papers over via `colorama` on startup.
///
/// Returns `true` if the console already supports (or was successfully
/// switched into) VT processing. On non-Windows platforms this is
/// always `true`, since real terminals there already speak ANSI.
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
            // Not a real console (e.g. redirected to a file/pipe without
            // a backing conhost) — let the is_terminal() check downstream
            // be the deciding factor instead.
            return true;
        }

        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true; // Already on (Windows Terminal, ConEmu, etc).
        }

        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_windows_ansi_support() -> bool {
    true
}

/// Detects whether the current stdout can safely render ANSI escapes.
///
/// Honors the common conventions:
/// - `NO_COLOR` (any value) forces color off.
/// - `CLICOLOR_FORCE` (non-"0") forces color on, even when not a TTY
///   (useful for piping into something like `less -R`).
/// - `TERM=dumb` forces color off.
/// - Otherwise, only enable color when stdout is actually a terminal
///   AND (on Windows) VT processing could be enabled for it.
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

/// A small palette resolved once at print time. When color isn't
/// supported, every field is an empty string, so format strings using
/// these fields degrade to plain, readable text with no escape codes.
struct Colors {
    bold: &'static str,
    reset: &'static str,
    underline: &'static str,
    red: &'static str,
    green: &'static str,
    yellow: &'static str,
    blue: &'static str,
}

impl Colors {
    fn detect() -> Self {
        if supports_color() {
            Colors {
                bold: "\x1b[1m",
                reset: "\x1b[0m",
                underline: "\x1b[4m",
                red: "\x1b[1;31m",
                green: "\x1b[1;32m",
                yellow: "\x1b[1;33m",
                blue: "\x1b[34m",
            }
        } else {
            Colors {
                bold: "",
                reset: "",
                underline: "",
                red: "",
                green: "",
                yellow: "",
                blue: "",
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
enum Urgency {
    Soft,
    Medium,
    Urgent,
}

impl Default for Urgency {
    fn default() -> Self {
        Urgency::Soft
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PlatformDownload {
    url: String,
    checksum: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Downloads {
    windows: Option<PlatformDownload>,
    linux: Option<PlatformDownload>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct UpdateManifest {
    version: String,
    urgency: Urgency,
    description: String,
    #[serde(default = "default_whats_new")]
    whats_new_url: String,
    downloads: Downloads,
}

#[derive(Serialize, Deserialize, Default)]
struct UpdateCache {
    last_checked: u64,
    last_notified: u64,
    manifest: Option<UpdateManifest>,
}

fn default_whats_new() -> String {
    "https://dam-pcp.web.app/docs#whats-new".to_string()
}

// --- CACHE MANAGEMENT --- //

fn get_cache_path() -> PathBuf {
    env::temp_dir().join("dam_cli_update_cache.json")
}

fn load_cache() -> UpdateCache {
    fs::read_to_string(get_cache_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &UpdateCache) {
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = fs::write(get_cache_path(), json);
    }
}

// --- PUBLIC API --- //

pub fn auto_check() {
    cleanup_old_exe();

    if !io::stdout().is_terminal() {
        return;
    }

    let mut cache = load_cache();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now.saturating_sub(cache.last_checked) >= 86400 {
        if let Some(manifest) = fetch_manifest(Duration::from_secs(5)) {
            cache.last_checked = now;
            cache.manifest = Some(manifest);
            save_cache(&cache);
        } else {
            cache.last_checked = now.saturating_sub(86400 - 3600);
            save_cache(&cache);
        }
    }

    if let Some(manifest) = &cache.manifest {
        if !is_newer_version(CURRENT_VERSION, &manifest.version) {
            return;
        }

        let time_since_notified = now.saturating_sub(cache.last_notified);
        let should_prompt = match manifest.urgency {
            Urgency::Urgent => true,
            Urgency::Medium => time_since_notified >= 86400,
            Urgency::Soft => time_since_notified >= 259200,
        };

        if should_prompt {
            cache.last_notified = now;
            save_cache(&cache);

            let c = Colors::detect();
            println!("\n🔔 {}Auto-Update Alert{}", c.yellow, c.reset);
            print_update_prompt(manifest);

            print!("\nWould you like to install the update now? (y/N): ");
            io::stdout().flush().unwrap();
            let mut input = String::new();

            if io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y") {
                execute_update(manifest);
                std::process::exit(0);
            } else {
                println!("Update deferred. Continuing execution...\n");
            }
        }
    }
}

pub fn run() {
    cleanup_old_exe();
    println!("🔄 Checking for updates...");

    let mut cache = load_cache();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let manifest = match fetch_manifest(Duration::from_secs(10)) {
        Some(m) => m,
        None => {
            println!("❌ Failed to connect to the update server.");
            return;
        }
    };

    // Update the cache from a manual run so that the background checks stay in sync
    cache.last_checked = now;
    cache.manifest = Some(manifest.clone());
    save_cache(&cache);

    if !is_newer_version(CURRENT_VERSION, &manifest.version) {
        println!("✅ DAM is up to date (v{}).", CURRENT_VERSION);
        return;
    }

    print_update_prompt(&manifest);

    print!("\nWould you like to install the update now? (y/N): ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Update skipped.");
        // Ensure auto-check respects this decision by updating the notification cooldown
        cache.last_notified = now;
        save_cache(&cache);
        return;
    }

    execute_update(&manifest);
}

// --- CORE UPDATE LOGIC --- //

/// Verifies if we have permissions to overwrite the current executable file.
fn has_write_permission() -> bool {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            // Attempt to create a temporary test file in the directory
            let test_file = parent.join(".dam_update_write_test");
            match File::create(&test_file) {
                Ok(_) => {
                    let _ = fs::remove_file(test_file);
                    return true;
                }
                Err(_) => return false,
            }
        }
    }
    false
}

fn execute_update(manifest: &UpdateManifest) {
    // 1. Check for elevated permissions before downloading anything
    if !has_write_permission() {
        println!("❌ Permission Denied! The CLI is installed in a system or protected directory.");
        #[cfg(unix)]
        println!("   Try running the update command with sudo: 'sudo dam update'");
        #[cfg(windows)]
        println!("   Try running your terminal as Administrator.");
        return;
    }

    let target_os = env::consts::OS;
    let payload = match target_os {
        "windows" => manifest.downloads.windows.as_ref(),
        "linux" => manifest.downloads.linux.as_ref(),
        _ => None,
    };

    if let Some(download_info) = payload {
        println!("📥 Downloading update payload...");
        let temp_dir = env::temp_dir();
        
        // Retain the exact file name and extension from the URL (e.g. dam-windows.zip)
        let file_name = download_info.url.split('/').last().unwrap_or("dam_update_payload.archive");
        let archive_path = temp_dir.join(file_name);

        if !download_file(&download_info.url, &archive_path) {
            return;
        }

        println!("🔒 Verifying secure checksum...");
        if !verify_checksum(&archive_path, &download_info.checksum) {
            println!(
                "❌ SECURITY ALERT: Checksum mismatch! The download may be corrupted or compromised."
            );
            let _ = fs::remove_file(&archive_path);
            return;
        }
        println!("✅ Checksum verified successfully.");

        println!("📦 Extracting update...");
        let extract_dir = temp_dir.join("dam_extract");

        let _ = fs::remove_dir_all(&extract_dir);
        let _ = fs::create_dir_all(&extract_dir);
        let bin_name = if target_os == "windows" {
            "dam.exe"
        } else {
            "dam"
        };

        if extract_payload(&archive_path, &extract_dir, bin_name) {
            // Because extract_payload succeeded, we are guaranteed to find the valid binary here
            if let Some(new_bin_path) = find_executable(&extract_dir, bin_name) {
                if apply_update(&new_bin_path) {
                    println!("\n✨ Successfully updated to v{}! ✨", manifest.version);
                    let _ = fs::remove_file(get_cache_path()); // Wipe cache to give the fresh binary a clean slate
                }
            }

            // Clean up ONLY on complete success
            let _ = fs::remove_file(&archive_path);
            let _ = fs::remove_dir_all(&extract_dir);
        } else {
            println!(
                "❌ Could not find a valid '{}' executable inside the payload format.",
                bin_name
            );
            println!(
                "💡 Debug Info: The downloaded payload and extracted files were left for inspection:"
            );
            println!("   Payload:   {}", archive_path.display());
            println!("   Extracted: {}", extract_dir.display());
            println!(
                "   Check if the executable is inside this folder or if the archive got corrupted."
            );
        }
    } else {
        println!(
            "⚠️  Auto-update is currently not supported for {}.",
            target_os
        );
        println!("Please download the latest version manually from: https://dam-pcp.web.app");
    }
}

fn apply_update(new_bin: &Path) -> bool {
    let current_exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            println!("❌ Could not locate running executable: {}", e);
            return false;
        }
    };

    let old_exe = current_exe.with_extension("old");

    if let Err(e) = fs::rename(&current_exe, &old_exe) {
        if e.kind() == io::ErrorKind::PermissionDenied {
            println!(
                "❌ Permission Denied! The CLI is installed in a system folder ({}).",
                current_exe.display()
            );
            #[cfg(unix)]
            println!("   Try running the update command with sudo: 'sudo dam update'");
            #[cfg(windows)]
            println!("   Try running your terminal as Administrator.");
        } else {
            println!("❌ Failed to prepare executable for update: {}", e);
        }
        return false;
    }

    if let Err(e) = fs::copy(new_bin, &current_exe) {
        println!("❌ Failed to write new executable: {}", e);
        let _ = fs::rename(&old_exe, &current_exe);
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut perms) = fs::metadata(&current_exe).map(|m| m.permissions()) {
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&current_exe, perms);
        }
    }

    true
}

fn cleanup_old_exe() {
    if let Ok(current_exe) = env::current_exe() {
        let old_exe = current_exe.with_extension("old");
        if old_exe.exists() {
            let _ = fs::remove_file(old_exe);
        }
    }
}

// --- HELPER FUNCTIONS --- //

fn print_update_prompt(manifest: &UpdateManifest) {
    let c = Colors::detect();

    let urgency_color = match manifest.urgency {
        Urgency::Urgent => c.red,
        Urgency::Medium => c.yellow,
        Urgency::Soft => c.green,
    };

    println!("  Current Version : {}v{}{}", c.red, CURRENT_VERSION, c.reset);
    println!("  Latest Version  : {}v{}{}", c.green, manifest.version, c.reset);
    println!(
        "  Priority        : {}{:?}{}",
        urgency_color, manifest.urgency, c.reset
    );
    println!(
        "\n  {}What's New:{}\n  {}",
        c.bold, c.reset, manifest.description
    );
    println!(
        "\n  {}Read the full changelog at:{}\n  {}{}{}",
        c.underline, c.reset, c.blue, manifest.whats_new_url, c.reset
    );
}

fn fetch_manifest(timeout: Duration) -> Option<UpdateManifest> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .ok()?;

    let resp = client.get(UPDATE_URL).send().ok()?;
    if resp.status().is_success() {
        resp.json::<UpdateManifest>().ok()
    } else {
        None
    }
}

fn download_file(url: &str, dest: &Path) -> bool {
    let mut response = match reqwest::blocking::get(url) {
        Ok(r) if r.status().is_success() => r,
        _ => return false,
    };

    let mut file = match File::create(dest) {
        Ok(f) => f,
        Err(_) => return false,
    };

    io::copy(&mut response, &mut file).is_ok()
}

fn verify_checksum(path: &Path, expected_hash: &str) -> bool {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut hasher = Sha256::new();
    if io::copy(&mut file, &mut hasher).is_err() {
        return false;
    }
    let hash = hasher.finalize();
    let calculated_hash = format!("{:x}", hash);

    calculated_hash
        .trim()
        .eq_ignore_ascii_case(expected_hash.trim())
}

/// Checks the Magic Bytes of a file to mathematically prove it is an executable program.
fn is_valid_executable(path: &Path) -> bool {
    if let Ok(mut file) = File::open(path) {
        let mut magic = [0u8; 4];
        if file.read_exact(&mut magic).is_ok() {
            let is_elf = magic == [0x7f, b'E', b'L', b'F']; // Linux binary
            let is_mz = magic[0] == b'M' && magic[1] == b'Z'; // Windows binary
            let is_macho = magic == [0xFE, 0xED, 0xFA, 0xCE]
                || magic == [0xFE, 0xED, 0xFA, 0xCF]
                || magic == [0xCF, 0xFA, 0xED, 0xFE]
                || magic == [0xCE, 0xFA, 0xED, 0xFE]; // Mac

            return is_elf || is_mz || is_macho;
        }
    }
    false
}

/// Advanced payload extractor: Will ONLY extract based on the validated extension type to avoid file corruption 
fn extract_payload(archive_path: &Path, dest_dir: &Path, bin_name: &str) -> bool {
    let file_name = archive_path.to_string_lossy().to_lowercase();

    if file_name.ends_with(".zip") {
        if let Ok(file) = File::open(archive_path) {
            if let Ok(mut archive) = zip::ZipArchive::new(file) {
                let _ = archive.extract(dest_dir);
                for i in 0..archive.len() {
                    if let Ok(mut zip_file) = archive.by_index(i) {
                        let outpath = match zip_file.enclosed_name() {
                            Some(path) => dest_dir.join(path),
                            None => continue,
                        };
                        if zip_file.name().ends_with('/') {
                            let _ = fs::create_dir_all(&outpath);
                        } else {
                            if let Some(p) = outpath.parent() {
                                let _ = fs::create_dir_all(p);
                            }
                            if let Ok(mut outfile) = File::create(&outpath) {
                                let _ = io::copy(&mut zip_file, &mut outfile);
                            }
                        }
                    }
                }
                return find_executable(dest_dir, bin_name).is_some();
            }
        }
    } else if is_valid_executable(archive_path) {
        // Uncompressed direct binary fallback (unchanged)
        if let Ok(mut rewind_file) = File::open(archive_path) {
            let out_path = dest_dir.join(bin_name);
            if let Ok(mut out_file) = File::create(&out_path) {
                if io::copy(&mut rewind_file, &mut out_file).is_ok() {
                    return find_executable(dest_dir, bin_name).is_some();
                }
            }
        }
    }

    false
}

fn is_newer_version(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let c_parts = parse(current);
    let l_parts = parse(latest);

    for (c, l) in c_parts.iter().zip(l_parts.iter()) {
        if l > c {
            return true;
        }
        if c > l {
            return false;
        }
    }
    l_parts.len() > c_parts.len()
}

fn find_executable(dir: &Path, target_name: &str) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_executable(&path, target_name) {
                    return Some(found);
                }
            } else if path.is_file() {
                if let Some(file_name) = path.file_name() {
                    if file_name.to_string_lossy() == target_name {
                        if is_valid_executable(&path) {
                            return Some(path);
                        } else {
                            println!(
                                "⚠️ Debug: Found a file named '{}' at {}, but it failed the Magic Bytes check (not a recognized binary).",
                                target_name,
                                path.display()
                            );
                        }
                    }
                }
            }
        }
    }
    None
}