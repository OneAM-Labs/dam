use super::{PullRequestInfo, SyncProvider};
use crate::commands::base_commands::settings::{get_toml_val, set_toml_val};
use crate::commands::seal::{FileEntry, Seal};
use crate::core::credentials::{Credential, get_credential, load_aliases, save_credential};
use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

const MAX_BLOB_BYTES: usize = 100 * 1024 * 1024; // GitHub hard limit
const WARN_BLOB_BYTES: usize = 50 * 1024 * 1024;
const MAX_ATTEMPTS: u32 = 6;
const TREE_CHUNK_START: usize = 400;
const TREE_CHUNK_MIN: usize = 25;
const BLOB_CACHE_PATH: &str = ".dam/push_blob_cache.json";

fn human_size(n: usize) -> String {
    let f = n as f64;
    if f >= 1_048_576.0 {
        format!("{:.1} MB", f / 1_048_576.0)
    } else if f >= 1024.0 {
        format!("{:.0} KB", f / 1024.0)
    } else {
        format!("{} B", n)
    }
}

pub enum AuthMethod {
    FineGrainedToken {
        token: String,
    },
    ClassicToken {
        token: String,
    },
    SshKey {
        key_path: String,
        passphrase: Option<String>,
    },
}

pub struct GitHubSync {
    owner: String,
    repo: String,
    client: reqwest::blocking::Client,
}

impl GitHubSync {
    pub fn new() -> Self {
        let (owner, repo) = Self::get_target_repo();
        let auth = Self::resolve_auth();

        let token = match auth {
            AuthMethod::FineGrainedToken { token } | AuthMethod::ClassicToken { token } => token,
            AuthMethod::SshKey { .. } => {
                panic!(
                    "Error: SSH authentication is configured, but the current GitHub Provider utilizes REST APIs which require a Personal Access Token."
                );
            }
        };

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token).parse().unwrap(),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            "application/vnd.github.v3+json".parse().unwrap(),
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            format!("DAM-CLI-Sync/{}", env!("CARGO_PKG_VERSION"))
                .parse()
                .unwrap(),
        );

        let client = reqwest::blocking::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap();

        Self {
            owner,
            repo,
            client,
        }
    }

    fn extract_dam_seal_id(msg: &str) -> Option<String> {
        if let Some(start) = msg.find("[dam:") {
            if let Some(end) = msg[start + 5..].find(']') {
                return Some(msg[start + 5..start + 5 + end].trim().trim_matches('"').to_string());
            }
        }
        if let Some(start) = msg.find("DAM Sync [") {
            if let Some(end) = msg[start + 10..].find(']') {
                return Some(msg[start + 10..start + 10 + end].trim().trim_matches('"').to_string());
            }
        }
        None
    }

    fn extract_seal_id(msg: &str, fallback_sha: &str) -> String {
        Self::extract_dam_seal_id(msg)
            .unwrap_or_else(|| format!("seal_git_{}", &fallback_sha[..8]))
    }

    fn blob_cache_key(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn read_blob_cache_file() -> HashMap<String, HashMap<String, String>> {
        fs::read_to_string(BLOB_CACHE_PATH)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// dam-hash -> GitHub blob sha for blobs already uploaded to THIS repo (resume after failure).
    fn load_blob_cache(&self) -> HashMap<String, String> {
        Self::read_blob_cache_file()
            .remove(&self.blob_cache_key())
            .unwrap_or_default()
    }

    fn save_blob_cache(&self, cache: &HashMap<String, String>) {
        let mut all = Self::read_blob_cache_file();
        all.insert(self.blob_cache_key(), cache.clone());
        if let Ok(s) = serde_json::to_string(&all) {
            let _ = fs::write(BLOB_CACHE_PATH, s);
        }
    }

    fn clear_blob_cache(&self) {
        let mut all = Self::read_blob_cache_file();
        all.remove(&self.blob_cache_key());
        if let Ok(s) = serde_json::to_string(&all) {
            let _ = fs::write(BLOB_CACHE_PATH, s);
        }
    }

    /// POST with retry/backoff. Retries timeouts, connection errors, 409/404 (fresh-repo
    /// replication lag), 429, 5xx and secondary rate limits (honouring Retry-After).
    fn post_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
        timeout: Duration,
        label: &str,
    ) -> Result<reqwest::blocking::Response, Box<dyn Error>> {
        use reqwest::StatusCode as S;
        let mut delay_ms: u64 = 1000;
        let mut last_err = String::new();

        for attempt in 1..=MAX_ATTEMPTS {
            let mut wait_ms = delay_ms;
            match self.client.post(url).json(body).timeout(timeout).send() {
                Ok(res) => {
                    let st = res.status();
                    if st.is_success() {
                        return Ok(res);
                    }
                    let retry_after = res
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok());
                    let remaining_zero = res
                        .headers()
                        .get("x-ratelimit-remaining")
                        .and_then(|v| v.to_str().ok())
                        .map(|v| v.trim() == "0")
                        .unwrap_or(false);
                    let detail = Self::format_api_response(res);
                    let lower = detail.to_lowercase();

                    let hard_rate_limit = st == S::FORBIDDEN && remaining_zero && retry_after.is_none();
                    let retryable = !hard_rate_limit
                        && (st == S::CONFLICT
                            || st == S::NOT_FOUND
                            || st == S::TOO_MANY_REQUESTS
                            || st.is_server_error()
                            || (st == S::FORBIDDEN
                                && (retry_after.is_some() || lower.contains("rate limit"))));
                    if !retryable {
                        return Err(format!("{} failed:\n{}", label, detail).into());
                    }
                    if let Some(s) = retry_after {
                        wait_ms = s.max(1) * 1000;
                    }
                    last_err = detail;
                }
                Err(e) => {
                    // timeouts / connection resets
                    last_err = e.to_string();
                }
            }

            if attempt < MAX_ATTEMPTS {
                println!(
                    "    ↻ {} failed (attempt {}/{}), retrying in {:.1}s...",
                    label, attempt, MAX_ATTEMPTS, wait_ms as f32 / 1000.0
                );
                thread::sleep(Duration::from_millis(wait_ms));
                delay_ms = (delay_ms * 2).min(15_000);
            }
        }
        Err(format!("{} failed after {} attempts:\n{}", label, MAX_ATTEMPTS, last_err).into())
    }

    /// Builds the remote tree in chunks: each request uses the previous request's tree as its
    /// base_tree, so GitHub never has to process thousands of entries in one call (which makes
    /// it answer 422 "request timed out ... input was too large"). Shrinks chunks on timeout.
    fn build_tree_incrementally(
        &self,
        items: &[serde_json::Value],
        base: Option<&str>,
    ) -> Result<String, Box<dyn Error>> {
        let tree_url = format!(
            "https://api.github.com/repos/{}/{}/git/trees",
            self.owner, self.repo
        );
        let total = items.len();
        let mut base_sha: Option<String> = base.map(String::from);
        let mut chunk_size = TREE_CHUNK_START;
        let mut pos = 0usize;
        println!("🌲 Building remote tree ({} entries)...", total);

        while pos < total {
            let end = (pos + chunk_size).min(total);
            let mut body = serde_json::json!({ "tree": items[pos..end].to_vec() });
            if let Some(ref b) = base_sha {
                body["base_tree"] = serde_json::json!(b);
            }

            match self.post_retry(&tree_url, &body, Duration::from_secs(180), "tree creation") {
                Ok(res) => {
                    let sha = res
                        .json::<serde_json::Value>()?["sha"]
                        .as_str()
                        .ok_or("GitHub returned no sha for the new tree")?
                        .to_string();
                    base_sha = Some(sha);
                    pos = end;
                    println!("  ✓ tree entries {}/{}", pos, total);
                }
                Err(e) => {
                    let msg = e.to_string();
                    let too_big = msg.contains("HTTP 422")
                        && (msg.contains("timed out") || msg.contains("too large"));
                    if too_big && chunk_size > TREE_CHUNK_MIN {
                        chunk_size = (chunk_size / 2).max(TREE_CHUNK_MIN);
                        println!(
                            "  ↳ GitHub timed out on that chunk; retrying with {} entries per request...",
                            chunk_size
                        );
                        continue;
                    }
                    if msg.contains("HTTP 422") && !too_big {
                        // Other 422s usually mean a cached blob no longer exists remotely.
                        self.clear_blob_cache();
                        println!("  ⚠️ Cleared the upload-resume cache; re-run the push to re-upload blobs.");
                    }
                    return Err(e);
                }
            }
        }
        base_sha.ok_or_else(|| "Tree build produced no result".into())
    }

    fn format_api_response(resp: reqwest::blocking::Response) -> String {
        let status = resp.status();
        let body = resp.text().unwrap_or_else(|_| "<unable to read response body>".to_string());
        let preview = body.lines().take(10).collect::<Vec<_>>().join("\n");
        let truncated = if preview.len() > 1000 {
            format!("{}...", &preview[..1000])
        } else {
            preview
        };
        format!("HTTP {}\n{}", status, truncated)
    }

    fn parse_cred(cred: Credential) -> AuthMethod {
        match cred.cred_type.as_str() {
            "SshKey" => AuthMethod::SshKey {
                key_path: cred.secret,
                passphrase: cred.extra,
            },
            "ClassicToken" => AuthMethod::ClassicToken { token: cred.secret },
            _ => AuthMethod::FineGrainedToken { token: cred.secret },
        }
    }

    fn resolve_auth() -> AuthMethod {
        let config_path = ".dam/config.toml";
        let content = fs::read_to_string(config_path).unwrap_or_default();

        if let Some(alias) = get_toml_val(&content, "github_cred_alias") {
            if let Ok(cred) =
                get_credential(&alias, false).or_else(|_| get_credential(&alias, true))
            {
                return Self::parse_cred(cred);
            }
        }

        println!("\n🔑 Authentication Required for GitHub Provider");
        let aliases = load_aliases();

        if !aliases.is_empty() {
            println!("Existing Credentials (Keychain/Vault):");
            for (i, alias) in aliases.iter().enumerate() {
                println!("  {}. {}", i + 1, alias);
            }
            println!("  {}. Create a NEW credential", aliases.len() + 1);
            print!("Choose [1-{}]: ", aliases.len() + 1);
            io::stdout().flush().unwrap();

            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();
            let choice_idx = choice.trim().parse::<usize>().unwrap_or(0);

            if choice_idx > 0 && choice_idx <= aliases.len() {
                let chosen_alias = &aliases[choice_idx - 1];
                let cred = get_credential(chosen_alias, false)
                    .or_else(|_| get_credential(chosen_alias, true))
                    .expect("Failed to load credential");

                let updated_content =
                    set_toml_val(&content, "github_cred_alias", chosen_alias, true);
                let _ = fs::write(config_path, updated_content);
                return Self::parse_cred(cred);
            }
        }

        println!("Creating new credential...");
        print!("Enter new alias name (e.g., github_token_main): ");
        io::stdout().flush().unwrap();
        let mut alias = String::new();
        io::stdin().read_line(&mut alias).unwrap();
        let alias = alias.trim().to_string();

        println!("Select Type:");
        println!("  1. Fine-Grained Token (Recommended)");
        println!("  2. Classic Token");
        println!("  3. SSH Key");
        print!("Choice [1-3]: ");
        io::stdout().flush().unwrap();
        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();

        let (cred_type, secret) = match choice.trim() {
            "3" => {
                print!("Enter path to private SSH key: ");
                io::stdout().flush().unwrap();
                let mut path = String::new();
                io::stdin().read_line(&mut path).unwrap();
                ("SshKey", path.trim().to_string())
            }
            _ => {
                let token = rpassword::prompt_password("Enter GitHub Token: ").unwrap();
                let ctype = if choice.trim() == "1" {
                    "FineGrainedToken"
                } else {
                    "ClassicToken"
                };
                (ctype, token.trim().to_string())
            }
        };

        let cred = Credential {
            alias: alias.clone(),
            cred_type: cred_type.to_string(),
            secret,
            extra: None,
        };

        if let Err(e) = save_credential(cred.clone(), false) {
            println!(
                "⚠️ OS Keychain failed ({}). Falling back to encrypted Vault...",
                e
            );
            save_credential(cred.clone(), true).expect("Vault save failed");
            println!("✓ Credential saved to Encrypted Vault.");
        } else {
            println!("✓ Credential saved to OS Keychain.");
        }

        let updated_content = set_toml_val(&content, "github_cred_alias", &alias, true);
        let _ = fs::write(config_path, updated_content);

        Self::parse_cred(cred)
    }

    fn get_target_repo() -> (String, String) {
        let config_path = ".dam/config.toml";
        let content = fs::read_to_string(config_path).unwrap_or_default();

        if let Some(repo_val) = get_toml_val(&content, "github_repo") {
            let parts: Vec<&str> = repo_val.split('/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                return (parts[0].to_string(), parts[1].to_string());
            }
        }

        print!("Enter target GitHub repository (format: owner/repo_name): ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let repo_str = input.trim();
        let parts: Vec<&str> = repo_str.split('/').collect();

        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            let updated_content = set_toml_val(&content, "github_repo", repo_str, true);
            let _ = fs::write(config_path, updated_content);
            println!("✓ Target repository configuration saved.");
            return (parts[0].to_string(), parts[1].to_string());
        }

        panic!("Error: Invalid repository format. Sync aborted.");
    }
}

impl SyncProvider for GitHubSync {
    fn check_diff(&self, stream: &str) -> Result<(usize, usize), Box<dyn Error>> {
        let mut local_seals = Vec::new();
        let meta = crate::commands::stream::get_or_create_meta(stream);
        let mut current_seal_id = meta.latest_seal;

        while let Some(sid) = current_seal_id {
            local_seals.push(sid.clone());
            if let Ok(content) = fs::read_to_string(format!(".dam/seals/{}.json", sid)) {
                if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                    current_seal_id = seal.parents.first().cloned();
                    continue;
                }
            }
            break;
        }

        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}&per_page=100",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Err(format!(
                "GitHub API error while checking commits for '{}':\n{}",
                stream,
                Self::format_api_response(commits_resp)
            )
            .into());
        }

        let commits: Vec<serde_json::Value> = commits_resp.json()?;
        let mut remote_seal_ids = Vec::new();

        // Build a remote seal id list using deterministic mapping for non-DAM commits.
        for c in &commits {
            let msg = c["commit"]["message"].as_str().unwrap_or("");
            let sha = c["sha"].as_str().unwrap_or("");
            let seal_id = Self::extract_dam_seal_id(msg)
                .map(|s| s.trim().trim_matches('"').to_string())
                .unwrap_or_else(|| format!("seal_git_{}", &sha[..sha.len().min(8)]));

            remote_seal_ids.push(seal_id);
        }

        // Optional debug dump when troubleshooting mismatched ahead/behind counts
        if std::env::var_os("DAM_DEBUG_SYNC").is_some() {
            println!("--- DAM DEBUG: Remote seal ids (latest first, showing up to 50) ---");
            for (i, id) in remote_seal_ids.iter().take(50).enumerate() {
                println!("  {}: {}", i + 1, id);
            }
            println!("--- DAM DEBUG: Local seals (latest first) ---");
            for (i, id) in local_seals.iter().take(50).enumerate() {
                println!("  {}: {}", i + 1, id);
            }
            println!("--- END DEBUG ---");
        }

        let mut ahead = 0;
        for ls in &local_seals {
            if !remote_seal_ids.contains(ls) {
                ahead += 1;
            } else {
                break;
            }
        }

        let mut behind = 0;
        // Build a set of seal files present on disk so we don't count already-downloaded
        // seals as 'missing' just because the active meta chain doesn't include them.
        let mut seals_on_disk = std::collections::HashSet::new();
        if let Ok(entries) = fs::read_dir(".dam/seals") {
            for e in entries.flatten() {
                if let Some(name) = e.file_name().to_str() {
                    // strip optional extension if present
                    let id = name.trim().trim_end_matches(".json").to_string();
                    seals_on_disk.insert(id);
                }
            }
        }

        for sid in &remote_seal_ids {
            if sid.is_empty() {
                continue;
            }
            if local_seals.contains(sid) || seals_on_disk.contains(sid) {
                break;
            }
            behind += 1;
        }

        Ok((ahead, behind))
    }

    fn push(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        let meta = crate::commands::stream::get_or_create_meta(stream);
        let latest_seal_id = meta
            .latest_seal
            .clone()
            .ok_or(format!("No seals available in stream '{}'.", stream))?;

        let seal_path = format!(".dam/seals/{}.json", latest_seal_id);
        let content = fs::read_to_string(&seal_path)?;
        let latest_seal: Seal = serde_json::from_str(&content)?;

        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}&per_page=100",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send();

        let mut base_git_sha = None;
        let mut base_seal_id = None;
        let mut parent_shas = Vec::new();

        if let Ok(resp) = commits_resp {
            if resp.status().is_success() {
                let commits: Vec<serde_json::Value> = resp.json()?;
                if let Some(head_commit) = commits.first() {
                    if let Some(s) = head_commit["sha"].as_str() {
                        parent_shas.push(s.to_string());
                    }
                }
                for c in &commits {
                    let msg = c["commit"]["message"].as_str().unwrap_or("");
                    let sha = c["sha"].as_str().unwrap_or("");
                    let sid = Self::extract_seal_id(msg, sha);
                    if Path::new(&format!(".dam/seals/{}.json", sid)).exists() {
                        base_seal_id = Some(sid);
                        if let Some(tree_sha) = c
                            .get("commit")
                            .and_then(|cm| cm.get("tree"))
                            .and_then(|t| t.get("sha"))
                            .and_then(|s| s.as_str())
                        {
                            base_git_sha = Some(tree_sha.to_string());
                        }
                        break;
                    }
                }
            }
        }

        if parent_shas.is_empty() {
            let base_stream = meta.target.as_deref().unwrap_or("main");
            println!(
                "  ↳ Brand new remote stream detected. Stitching history to base stream '{}'...",
                base_stream
            );
            let base_ref_url = format!(
                "https://api.github.com/repos/{}/{}/git/refs/heads/{}",
                self.owner, self.repo, base_stream
            );
            if let Ok(base_res) = self.client.get(&base_ref_url).send() {
                if base_res.status().is_success() {
                    if let Some(sha_str) = base_res
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|v| v["object"]["sha"].as_str().map(String::from))
                    {
                        parent_shas.push(sha_str.clone());
                        let commit_url = format!(
                            "https://api.github.com/repos/{}/{}/git/commits/{}",
                            self.owner, self.repo, sha_str
                        );
                        if let Ok(c_res) = self.client.get(&commit_url).send() {
                            if let Ok(c_val) = c_res.json::<serde_json::Value>() {
                                if let Some(tree_sha) = c_val
                                    .get("tree")
                                    .and_then(|t| t.get("sha"))
                                    .and_then(|s| s.as_str())
                                {
                                    base_git_sha = Some(tree_sha.to_string());
                                }
                            }
                        }
                    }
                }
            }

            let commits_res = self
                .client
                .get(format!(
                    "https://api.github.com/repos/{}/{}/commits",
                    self.owner, self.repo
                ))
                .send()?;
            if commits_res.status() == reqwest::StatusCode::CONFLICT {
                println!(
                    "💡 Empty repository detected. Bootstrapping initial structure to initialize Git DB..."
                );
                let init_url = format!(
                    "https://api.github.com/repos/{}/{}/contents/.dam_keep",
                    self.owner, self.repo
                );
                let init_body = serde_json::json!({
                    "message": "Initialize DAM Reservoir",
                    "content": general_purpose::STANDARD.encode("Initialized by DAM Cloud Sync.")
                });
                let init_res = self.client.put(&init_url).json(&init_body).send()?;
                if !init_res.status().is_success() {
                    return Err(format!(
                        "Failed to bootstrap empty repository:\n{}",
                        Self::format_api_response(init_res)
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(1500));
            } else if commits_res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("Repository not found. Please ensure target repository exists and permissions are correct.".into());
            }
        }

        println!(
            "🚀 Pushing Stream '{}' natively to GitHub Git database...",
            stream
        );

        let mut base_seal_files = Vec::new();
        if let Some(ref bid) = base_seal_id {
            if let Ok(c) = fs::read_to_string(format!(".dam/seals/{}.json", bid)) {
                if let Ok(s) = serde_json::from_str::<Seal>(&c) {
                    base_seal_files = s.files;
                }
            }
        }
        // Fetch actual remote paths from GitHub to guarantee deleted files are wiped even during Force Push
        let mut remote_paths = HashSet::new();
        if let Some(ref bg_sha) = base_git_sha {
            let tree_url = format!(
                "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
                self.owner, self.repo, bg_sha
            );
            if let Ok(res) = self.client.get(&tree_url).send() {
                if let Ok(tree_json) = res.json::<serde_json::Value>() {
                    if let Some(tree_arr) = tree_json["tree"].as_array() {
                        for item in tree_arr {
                            if item["type"] == "blob" {
                                if let Some(p) = item["path"].as_str() {
                                    remote_paths.insert(p.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut changed_files = Vec::new();
        let mut active_paths = HashSet::new();

        for hf in &latest_seal.files {
            if hf.path.starts_with(".dam/") || hf.is_dir {
                continue;
            }
            active_paths.insert(hf.path.clone());
            if let Some(bf) = base_seal_files.iter().find(|x| x.path == hf.path) {
                if bf.hash != hf.hash {
                    changed_files.push(hf.clone());
                }
            } else {
                changed_files.push(hf.clone());
            }
        }

        let total = changed_files.len();
        let mut blob_cache = self.load_blob_cache();
        let blob_url = format!(
            "https://api.github.com/repos/{}/{}/git/blobs",
            self.owner, self.repo
        );
        let mut tree_items = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let (mut uploaded, mut reused, mut bytes_sent) = (0usize, 0usize, 0u64);
        let started = Instant::now();
        println!("📦 {} file(s) to upload.", total);

        for (i, file_entry) in changed_files.iter().enumerate() {
            let n = i + 1;
            let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
            if !obj_path.exists() {
                println!(
                    "  [{}/{}] ⚠️ Object {} missing for {}. Skipping.",
                    n, total, file_entry.hash, file_entry.path
                );
                skipped.push(file_entry.path.clone());
                continue;
            }

            // Resume / dedupe: identical content (or a previous interrupted push) is never re-sent.
            if let Some(sha) = blob_cache.get(&file_entry.hash) {
                reused += 1;
                println!("  [{}/{}] ↺ {} (already uploaded)", n, total, file_entry.path);
                tree_items.push(serde_json::json!({ "path": file_entry.path, "mode": "100644", "type": "blob", "sha": sha }));
                continue;
            }

            let compressed_file = File::open(&obj_path)?;
            let mut decoder = ZlibDecoder::new(compressed_file);
            let mut raw_data = Vec::new();
            decoder.read_to_end(&mut raw_data)?;
            let raw_len = raw_data.len();

            if raw_len > MAX_BLOB_BYTES {
                println!(
                    "  [{}/{}] ⛔ {} is {} - over GitHub's 100 MB limit. NOT uploaded.",
                    n, total, file_entry.path, human_size(raw_len)
                );
                skipped.push(file_entry.path.clone());
                continue;
            }
            if raw_len > WARN_BLOB_BYTES {
                println!("  ⚠️ {} is large ({}); this may take a while.", file_entry.path, human_size(raw_len));
            }

            println!("  [{}/{}] ⬆ {} ({})", n, total, file_entry.path, human_size(raw_len));
            let b64_content = general_purpose::STANDARD.encode(&raw_data);
            drop(raw_data);
            let blob_body = serde_json::json!({ "content": b64_content, "encoding": "base64" });
            drop(b64_content);

            // Allow more time for bigger files: 60s base + 15s per MB, capped at 15 min.
            let timeout = Duration::from_secs((60 + (raw_len as u64 / 1_048_576) * 15).min(900));
            let res = match self.post_retry(
                &blob_url,
                &blob_body,
                timeout,
                &format!("blob upload for {}", file_entry.path),
            ) {
                Ok(r) => r,
                Err(e) => {
                    self.save_blob_cache(&blob_cache); // keep progress for the next attempt
                    return Err(e);
                }
            };
            let sha = res
                .json::<serde_json::Value>()?["sha"]
                .as_str()
                .ok_or("GitHub returned no sha for the uploaded blob")?
                .to_string();

            uploaded += 1;
            bytes_sent += raw_len as u64;
            blob_cache.insert(file_entry.hash.clone(), sha.clone());
            if uploaded % 20 == 0 {
                self.save_blob_cache(&blob_cache);
            }
            tree_items.push(serde_json::json!({ "path": file_entry.path, "mode": "100644", "type": "blob", "sha": sha }));
        }
        self.save_blob_cache(&blob_cache);
        println!(
            "✓ Blobs done: {} uploaded ({}), {} reused, {} skipped in {:.1}s.",
            uploaded,
            human_size(bytes_sent as usize),
            reused,
            skipped.len(),
            started.elapsed().as_secs_f32()
        );
        if !skipped.is_empty() {
            println!("⚠️ These files were NOT included in the push:");
            for p in &skipped {
                println!("    - {}", p);
            }
        }
        // Explicitly delete files from remote tree that no longer exist in our active seal
        for remote_p in remote_paths {
            if !active_paths.contains(&remote_p) && !remote_p.starts_with(".dam/") {
                tree_items.push(serde_json::json!({ "path": remote_p, "mode": "100644", "type": "blob", "sha": serde_json::Value::Null }));
            }
        }

        let tree_sha = if tree_items.is_empty() {
            base_git_sha
                .clone()
                .ok_or("No file changes detected and no base tree available.")?
        } else {
            self.build_tree_incrementally(&tree_items, base_git_sha.as_deref())?
        };

        let commit_url = format!(
            "https://api.github.com/repos/{}/{}/git/commits",
            self.owner, self.repo
        );
        // Clean commit titles: store tracking metadata in the description footer!
        let commit_message = if latest_seal.message.contains("[dam:") {
            latest_seal.message.clone()
        } else {
            format!("{}\n\n[dam:{}]", latest_seal.message, latest_seal.id)
        };

        let mut commit_body = serde_json::json!({ "message": commit_message, "tree": tree_sha });
        if !parent_shas.is_empty() {
            commit_body["parents"] = serde_json::json!(parent_shas);
        }

        let res = self.post_retry(
            &commit_url,
            &commit_body,
            Duration::from_secs(120),
            "commit creation",
        )?;
        let new_commit_sha = res.json::<serde_json::Value>()?["sha"]
            .as_str()
            .ok_or("GitHub returned no sha for the new commit")?
            .to_string();

        let ref_url = format!(
            "https://api.github.com/repos/{}/{}/git/refs/heads/{}",
            self.owner, self.repo, stream
        );
        let update_ref_body = serde_json::json!({ "sha": new_commit_sha, "force": true });
        let patch_res = self.client.patch(&ref_url).json(&update_ref_body).send()?;

        if !patch_res.status().is_success() {
            let create_ref_url = format!(
                "https://api.github.com/repos/{}/{}/git/refs",
                self.owner, self.repo
            );
            let create_ref_body = serde_json::json!({ "ref": format!("refs/heads/{}", stream), "sha": new_commit_sha });
            let create_res = self
                .client
                .post(&create_ref_url)
                .json(&create_ref_body)
                .send()?;
            if !create_res.status().is_success() {
                return Err(format!(
                    "Could not create stream reference on GitHub: {}",
                    create_res.text()?
                )
                .into());
            }
        }

        self.clear_blob_cache();

        println!(
            "✓ Successfully pushed stream '{}' to remote repository natively!",
            stream
        );
        Ok(())
    }

    fn sync_release(
        &self,
        release_name: &str,
        release_version: &str,
        seal_id: &str,
        stream: &str,
        description: Option<&String>,
        tags: &[String],
    ) -> Result<String, Box<dyn Error>> {
        let body_description = if let Some(desc) = description {
            if tags.is_empty() {
                format!("{}\n\nDAM release from seal {}", desc.trim(), seal_id)
            } else {
                format!("{}\n\nTags: {}\n\nDAM release from seal {}", desc.trim(), tags.join(", "), seal_id)
            }
        } else if tags.is_empty() {
            format!("DAM release from seal {}", seal_id)
        } else {
            format!("Tags: {}\n\nDAM release from seal {}", tags.join(", "), seal_id)
        };

        let safe_tag = release_version
            .replace('/', "_")
            .replace(' ', "_")
            .replace('\n', "_");
        let tag_url = format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            self.owner,
            self.repo,
            safe_tag
        );

        let mut release_id = None;
        if let Ok(resp) = self.client.get(&tag_url).send() {
            if resp.status().is_success() {
                if let Ok(existing) = resp.json::<serde_json::Value>() {
                    if let Some(id) = existing["id"].as_u64() {
                        release_id = Some(id);
                    }
                }
            }
        }

        let release_body = serde_json::json!({
            "tag_name": safe_tag,
            "name": release_name,
            "body": body_description,
            "draft": false,
            "prerelease": false,
            "target_commitish": stream,
        });

        let result = if let Some(id) = release_id {
            let update_url = format!(
                "https://api.github.com/repos/{}/{}/releases/{}",
                self.owner, self.repo, id
            );
            let resp = self.client.patch(&update_url).json(&release_body).send()?;
            if !resp.status().is_success() {
                return Err(format!(
                    "Failed to update GitHub release '{}': {}",
                    release_name,
                    Self::format_api_response(resp)
                )
                .into());
            }
            Ok(format!("Updated GitHub release '{}'", release_name))
        } else {
            let create_url = format!(
                "https://api.github.com/repos/{}/{}/releases",
                self.owner, self.repo
            );
            let resp = self.client.post(&create_url).json(&release_body).send()?;
            if !resp.status().is_success() {
                return Err(format!(
                    "Failed to create GitHub release '{}': {}",
                    release_name,
                    Self::format_api_response(resp)
                )
                .into());
            }
            Ok(format!("Created GitHub release '{}'", release_name))
        };

        result
    }

    fn pull(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        println!("📡 Checking remote Git commits for stream '{}'...", stream);

        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}&per_page=100",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Err(format!(
                "Could not find remote stream '{}'.\n{}",
                stream,
                Self::format_api_response(commits_resp)
            )
            .into());
        }

        let commits: Vec<serde_json::Value> = commits_resp.json()?;
        let mut local_meta = crate::commands::stream::get_or_create_meta(stream);
        let original_local_seal_id = local_meta.latest_seal.clone();
        // 1. Gather our local history chain
        let mut local_seals = Vec::new();
        let mut temp_sid = original_local_seal_id.clone();
        while let Some(sid) = temp_sid {
            local_seals.push(sid.clone());
            if let Ok(content) = fs::read_to_string(format!(".dam/seals/{}.json", sid)) {
                if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                    temp_sid = seal.parents.first().cloned();
                    continue;
                }
            }
            break;
        }
        // 2. See what commits GitHub has that we don't
        let mut missing_commits: Vec<(serde_json::Value, String)> = Vec::new();
        let mut remote_head_seal_id = None;

        for c in &commits {
            let msg = c["commit"]["message"].as_str().unwrap_or("");
            let sha = c["sha"].as_str().unwrap_or("");
            let expected_sid = Self::extract_dam_seal_id(msg)
                .unwrap_or_else(|| format!("seal_git_{}", &sha[..sha.len().min(8)]));

            if remote_head_seal_id.is_none() {
                remote_head_seal_id = Some(expected_sid.clone());
            }

            // If we already have this seal in our chain or on disk, stop collecting missing commits.
            if local_seals.contains(&expected_sid) || Path::new(&format!(".dam/seals/{}.json", expected_sid)).exists() {
                break;
            }

            missing_commits.push((c.clone(), expected_sid));
        }

        if missing_commits.is_empty() {
            println!(
                "✅ Local stream '{}' is already up to date with remote.",
                stream
            );
            return Ok(());
        }
        // 3. Check for divergence (We have local seals that remote doesn't know about)
        let is_diverged = if let Some(ref remote_head) = remote_head_seal_id {
            !local_seals.contains(remote_head) && original_local_seal_id.is_some()
        } else {
            false
        };
        // 4. Download missing history objects and seals safely to disk first
        missing_commits.reverse();
        println!(
            "⬇️  Downloading {} missing DAM-formatted history commit(s)...",
            missing_commits.len()
        );
        fs::create_dir_all(".dam/seals")?;
        fs::create_dir_all(".dam/objects")?;

        let mut current_remote_seal_id = None;
        let mut tree_cache: HashMap<String, serde_json::Value> = HashMap::new();

        for (commit, new_seal_id) in missing_commits {
            let commit_sha = commit["sha"].as_str().unwrap();
            let raw_commit_msg = commit["commit"]["message"].as_str().unwrap().to_string();
            let commit_time = commit["commit"]["author"]["date"]
                .as_str()
                .unwrap()
                .to_string();

            let clean_msg = if let Some(idx) = raw_commit_msg.find("\n\n[dam:") {
                raw_commit_msg[..idx].to_string()
            } else {
                raw_commit_msg.clone()
            };

            let commit_obj_url = format!(
                "https://api.github.com/repos/{}/{}/git/commits/{}",
                self.owner, self.repo, commit_sha
            );
            println!("   ⬇️  Fetching tree metadata for commit {}...", commit_sha);
            let commit_obj_resp = self.client.get(&commit_obj_url).send()?;
            if !commit_obj_resp.status().is_success() {
                return Err(format!(
                    "Failed to download commit object {}:\n{}",
                    commit_sha,
                    Self::format_api_response(commit_obj_resp)
                )
                .into());
            }
            let commit_obj: serde_json::Value = commit_obj_resp.json()?;
            let tree_sha = commit_obj["tree"]["sha"]
                .as_str()
                .ok_or("Invalid commit object returned from GitHub")?;

            let tree_url = format!(
                "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
                self.owner, self.repo, tree_sha
            );
            println!("   ⬇️  Downloading tree {} for commit {}...", tree_sha, commit_sha);
            let tree_json = if let Some(cached) = tree_cache.get(tree_sha) {
                cached.clone()
            } else {
                let tree_resp = self.client.get(&tree_url).send()?;
                if !tree_resp.status().is_success() {
                    return Err(format!(
                        "Failed to download tree for commit {}:\n{}",
                        commit_sha,
                        Self::format_api_response(tree_resp)
                    )
                    .into());
                }
                let json: serde_json::Value = tree_resp.json()?;
                tree_cache.insert(tree_sha.to_string(), json.clone());
                json
            };

            let mut new_files = Vec::new();

            if let Some(tree_arr) = tree_json["tree"].as_array() {
                for item in tree_arr {
                    if item["type"] != "blob" {
                        continue;
                    }
                    let path = item["path"].as_str().unwrap().to_string();
                    if path.starts_with(".dam/") {
                        continue;
                    }

                    let blob_sha = item["sha"].as_str().unwrap();
                    let blob_url = format!(
                        "https://api.github.com/repos/{}/{}/git/blobs/{}",
                        self.owner, self.repo, blob_sha
                    );
                    let blob_res = self.client.get(&blob_url).send()?;
                    if !blob_res.status().is_success() {
                        continue;
                    }

                    let blob_json: serde_json::Value = blob_res.json()?;
                    let b64 = blob_json["content"].as_str().unwrap().replace('\n', "");
                    let raw_data = general_purpose::STANDARD.decode(&b64)?;

                    let mut hasher = Sha256::new();
                    hasher.update(&raw_data);
                    let dam_hash = format!("{:x}", hasher.finalize());

                    let dest_obj = Path::new(".dam/objects").join(&dam_hash);
                    if !dest_obj.exists() {
                        let compressed_file = File::create(&dest_obj)?;
                        let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                        encoder.write_all(&raw_data)?;
                        encoder.finish()?;
                    }

                    new_files.push(FileEntry {
                        path,
                        hash: dam_hash,
                        is_dir: false,
                    });
                }
            }

            // If we aren't diverged, link to our previous local seal. Otherwise, build remote chain.
            let parent_chain = match current_remote_seal_id.clone() {
                Some(id) => vec![id],
                None => {
                    if !is_diverged {
                        original_local_seal_id
                            .clone()
                            .map(|s| vec![s])
                            .unwrap_or_default()
                    } else {
                        vec![]
                    }
                }
            };

            let new_seal = Seal {
                id: new_seal_id.clone(),
                message: clean_msg,
                timestamp: commit_time,
                files: new_files,
                stream: stream.to_string(),
                parents: parent_chain,
            };

            let path = format!(".dam/seals/{}.json", new_seal_id);
            fs::write(path, serde_json::to_string_pretty(&new_seal)?)?;
            current_remote_seal_id = Some(new_seal_id);
        }
        // 5. INTERACTIVE DIVERGENCE RESOLUTION
        if is_diverged {
            let local_id = original_local_seal_id.as_ref().unwrap();
            let remote_id = current_remote_seal_id.as_ref().unwrap();

            println!("\n⚠️  DIVERGENT TIMELINES DETECTED!");
            println!("--------------------------------------------------");
            println!("Both your local machine and GitHub have new, un-shared seals.");
            println!("All remote seals and files have been safely downloaded to your reservoir.");
            println!(
                "\nWhich seal do you want to set as the active latest seal for stream '{}'?",
                stream
            );
            println!(
                "  [1] Remote Latest : {} (Use GitHub's timeline)",
                remote_id
            );
            println!(
                "  [2] Local Latest  : {} (Keep your local timeline)",
                local_id
            );
            print!("\nEnter your choice (1 or 2): ");
            io::stdout().flush()?;

            let mut choice = String::new();
            io::stdin().read_line(&mut choice)?;

            match choice.trim() {
                "1" => {
                    local_meta.latest_seal = Some(remote_id.clone());
                    println!(
                        "✓ Updated active stream pointer to Remote Latest ({}).",
                        remote_id
                    );
                    println!(
                        "  Note: Your previous local seal ({}) is still saved in .dam/seals/.",
                        local_id
                    );
                }
                "2" | _ => {
                     // Do nothing to local_meta.latest_seal, keep it pointing to local_id
                    println!(
                        "✓ Kept active stream pointer at Local Latest ({}).",
                        local_id
                    );
                    println!(
                        "  Note: The downloaded remote seal ({}) is saved in .dam/seals/ if you need it later.",
                        remote_id
                    );
                }
            }
        } else {
            // Standard clean fast-forward update
            if let Some(id) = current_remote_seal_id {
                local_meta.latest_seal = Some(id);
            }
        }

        crate::commands::stream::save_meta(&local_meta);
        println!("✓ Sync operation finished for stream '{}'.", stream);
        Ok(())
    }

    fn list_pull_requests(&self) -> Result<Vec<PullRequestInfo>, Box<dyn Error>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls?state=open&per_page=100",
            self.owner, self.repo
        );
        let resp = self.client.get(&url).send()?;
        if !resp.status().is_success() {
            return Err(format!(
                "Failed to list pull requests:\n{}",
                Self::format_api_response(resp)
            )
            .into());
        }

        let prs: Vec<serde_json::Value> = resp.json()?;
        Ok(prs
            .iter()
            .map(|p| PullRequestInfo {
                number: p["number"].as_u64().unwrap_or(0),
                title: p["title"].as_str().unwrap_or("").to_string(),
                author: p["user"]["login"].as_str().unwrap_or("unknown").to_string(),
                head_ref: p["head"]["ref"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    fn checkout_pull_request(&self, number: u64) -> Result<String, Box<dyn Error>> {
        let pr_url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}",
            self.owner, self.repo, number
        );
        let resp = self.client.get(&pr_url).send()?;
        if !resp.status().is_success() {
            return Err(format!(
                "Pull request #{} not found or inaccessible:\n{}",
                number,
                Self::format_api_response(resp)
            )
            .into());
        }

        let pr: serde_json::Value = resp.json()?;
        let head_sha = pr["head"]["sha"]
            .as_str()
            .ok_or("Pull request has no head commit")?
            .to_string();
        let head_ref = pr["head"]["ref"].as_str().unwrap_or("unknown").to_string();
        let title = pr["title"].as_str().unwrap_or("").to_string();

        println!(
            "📥 Fetching PR #{} ('{}', branch '{}')...",
            number, title, head_ref
        );

        let tree_url = format!(
            "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
            self.owner, self.repo, head_sha
        );
        let tree_json: serde_json::Value = self.client.get(&tree_url).send()?.json()?;

        fs::create_dir_all(".dam/objects")?;
        let mut files = Vec::new();

        if let Some(tree_arr) = tree_json["tree"].as_array() {
            for item in tree_arr {
                if item["type"] != "blob" {
                    continue;
                }
                let path = item["path"].as_str().unwrap_or("").to_string();
                if path.starts_with(".dam/") {
                    continue;
                }

                let blob_sha = item["sha"].as_str().unwrap_or("");
                let blob_url = format!(
                    "https://api.github.com/repos/{}/{}/git/blobs/{}",
                    self.owner, self.repo, blob_sha
                );
                let blob_res = self.client.get(&blob_url).send()?;
                if !blob_res.status().is_success() {
                    continue;
                }

                let blob_json: serde_json::Value = blob_res.json()?;
                let b64 = blob_json["content"].as_str().unwrap_or("").replace('\n', "");
                let raw_data = general_purpose::STANDARD.decode(&b64)?;

                let mut hasher = Sha256::new();
                hasher.update(&raw_data);
                let dam_hash = format!("{:x}", hasher.finalize());

                let dest_obj = Path::new(".dam/objects").join(&dam_hash);
                if !dest_obj.exists() {
                    let compressed_file = File::create(&dest_obj)?;
                    let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                    encoder.write_all(&raw_data)?;
                    encoder.finish()?;
                }

                files.push(FileEntry {
                    path,
                    hash: dam_hash,
                    is_dir: false,
                });
            }
        }

        let stream_name = format!("pr-{}", number);
        let seal_id = format!("seal_pr{}", &head_sha[..head_sha.len().min(8)]);

        let seal = Seal {
            id: seal_id.clone(),
            message: format!("Checked out PR #{}: {}", number, title),
            timestamp: Utc::now().to_rfc3339(),
            files,
            stream: stream_name.clone(),
            parents: vec![],
        };
        fs::write(
            format!(".dam/seals/{}.json", seal_id),
            serde_json::to_string_pretty(&seal)?,
        )?;

        let mut meta = crate::commands::stream::get_or_create_meta(&stream_name);
        meta.latest_seal = Some(seal_id);
        meta.description = Some(format!("Checked out from PR #{} ({})", number, head_ref));
        crate::commands::stream::save_meta(&meta);

        Ok(stream_name)
    }
}