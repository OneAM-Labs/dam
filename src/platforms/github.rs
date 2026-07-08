use super::SyncProvider;
use crate::commands::base_commands::settings::{get_toml_val, set_toml_val};
use crate::commands::seal::{FileEntry, Seal};
use base64::{Engine as _, engine::general_purpose};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
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
                    "Error: SSH authentication is configured, but the current GitHub Provider utilizes REST APIs which require a Personal Access Token. Native SSH Git Transport is pending implementation."
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
            .build()
            .unwrap();

        Self {
            owner,
            repo,
            client,
        }
    }

    fn resolve_auth() -> AuthMethod {
        let creds_path = ".dam/credentials.json";

        if let Ok(content) = fs::read_to_string(creds_path) {
            if let Ok(cred) = serde_json::from_str(&content) {
                return cred;
            }
        }

        let legacy_path = ".dam/credentials";
        if let Ok(token) = fs::read_to_string(legacy_path) {
            if !token.trim().is_empty() {
                return AuthMethod::ClassicToken {
                    token: token.trim().to_string(),
                };
            }
        }

        println!("\n🔑 Authentication Required for GitHub Provider");
        println!("Please select your authentication mechanism:");
        println!("  1. Fine-Grained Personal Access Token (Recommended)");
        println!("  2. Classic Personal Access Token");
        println!("  3. SSH Key (Note: Not currently supported via REST provider)");
        print!("Choice [1-3]: ");
        io::stdout().flush().unwrap();

        let mut choice = String::new();
        io::stdin().read_line(&mut choice).unwrap();

        let cred = match choice.trim() {
            "3" => {
                print!("Enter path to private SSH key (e.g. ~/.ssh/id_ed25519): ");
                io::stdout().flush().unwrap();
                let mut path = String::new();
                io::stdin().read_line(&mut path).unwrap();
                AuthMethod::SshKey {
                    key_path: path.trim().to_string(),
                    passphrase: None,
                }
            }
            _ => {
                let token_prompt = rpassword::prompt_password("Enter GitHub Token: ").unwrap();
                let t = token_prompt.trim().to_string();
                if choice.trim() == "1" {
                    AuthMethod::FineGrainedToken { token: t }
                } else {
                    AuthMethod::ClassicToken { token: t }
                }
            }
        };

        let _ = fs::write(creds_path, serde_json::to_string_pretty(&cred).unwrap());
        println!("✓ Credentials stored securely in {}", creds_path);
        cred
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
        // 1. Traverse local seals
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

        // 2. Fetch remote native Git commits
        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Ok((local_seals.len(), 0)); // No remote stream
        }

        let commits: Vec<serde_json::Value> = commits_resp.json()?;
        let mut remote_seal_ids = Vec::new();

        // 3. Extract seal mapping from commit messages
        for c in &commits {
            if let Some(msg) = c["commit"]["message"].as_str() {
                if let Some(start) = msg.find("DAM Sync [") {
                    if let Some(end) = msg[start + 10..].find(']') {
                        remote_seal_ids.push(msg[start + 10..start + 10 + end].to_string());
                        continue;
                    }
                }
            }
            // If it's a native Git commit, leave it empty as an unmapped commit
            remote_seal_ids.push(String::new());
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
        for sid in &remote_seal_ids {
            if !sid.is_empty() && local_seals.contains(sid) {
                break; // We hit the common ancestor
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

        // Find remote base commit to calculate what actually changed
        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}",
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
                    parent_shas.push(head_commit["sha"].as_str().unwrap().to_string());
                }
                for c in &commits {
                    if let Some(msg) = c["commit"]["message"].as_str() {
                        if let Some(start) = msg.find("DAM Sync [") {
                            if let Some(end) = msg[start + 10..].find(']') {
                                let sid = &msg[start + 10..start + 10 + end];
                                if Path::new(&format!(".dam/seals/{}.json", sid)).exists() {
                                    base_seal_id = Some(sid.to_string());
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

            // Check for empty repo and bootstrap if needed
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
                        "Failed to bootstrap empty repository: {}",
                        init_res.text()?
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(1500));
            } else if commits_res.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("Repository not found. Please ensure the target repository exists on GitHub and permissions are correct.".into());
            }
        }

        println!(
            "🚀 Pushing Stream '{}' natively to GitHub Git database...",
            stream
        );

        // Optimize: Only upload files that actually changed
        let mut base_seal_files = Vec::new();
        if let Some(bid) = base_seal_id {
            if let Ok(c) = fs::read_to_string(format!(".dam/seals/{}.json", bid)) {
                if let Ok(s) = serde_json::from_str::<Seal>(&c) {
                    base_seal_files = s.files;
                }
            }
        }

        let mut changed_files = Vec::new();
        let mut deleted_files = Vec::new();

        for hf in &latest_seal.files {
            if hf.path.starts_with(".dam/") {
                continue;
            } // Exclude internal DAM structures
            if let Some(bf) = base_seal_files.iter().find(|x| x.path == hf.path) {
                if bf.hash != hf.hash {
                    changed_files.push(hf.clone());
                }
            } else {
                changed_files.push(hf.clone());
            }
        }
        for bf in &base_seal_files {
            if bf.path.starts_with(".dam/") {
                continue;
            }
            if !latest_seal.files.iter().any(|x| x.path == bf.path) {
                deleted_files.push(bf.clone());
            }
        }

        let mut tree_items = Vec::new();

        for file_entry in changed_files {
            if file_entry.is_dir {
                continue;
            }
            let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
            if !obj_path.exists() {
                println!(
                    "⚠️  Warning: Object {} missing for file {}. Skipping.",
                    file_entry.hash, file_entry.path
                );
                continue;
            }

            let compressed_file = File::open(obj_path)?;
            let mut decoder = ZlibDecoder::new(compressed_file);
            let mut raw_data = Vec::new();
            decoder.read_to_end(&mut raw_data)?;

            let b64_content = general_purpose::STANDARD.encode(&raw_data);
            let blob_url = format!(
                "https://api.github.com/repos/{}/{}/git/blobs",
                self.owner, self.repo
            );
            let blob_body = serde_json::json!({"content": b64_content, "encoding": "base64"});
            let res = self.client.post(&blob_url).json(&blob_body).send()?;
            if !res.status().is_success() {
                return Err(format!(
                    "Failed to upload blob for {}: {}",
                    file_entry.path,
                    res.text()?
                )
                .into());
            }

            let sha = res.json::<serde_json::Value>()?["sha"]
                .as_str()
                .unwrap()
                .to_string();
            tree_items.push(serde_json::json!({
                "path": file_entry.path,
                "mode": "100644",
                "type": "blob",
                "sha": sha
            }));
        }

        for file_entry in deleted_files {
            if file_entry.is_dir {
                continue;
            }
            tree_items.push(serde_json::json!({
                "path": file_entry.path,
                "mode": "100644",
                "type": "blob",
                "sha": serde_json::Value::Null
            }));
        }

        // Post Tree creation
        let tree_url = format!(
            "https://api.github.com/repos/{}/{}/git/trees",
            self.owner, self.repo
        );
        let mut tree_body = serde_json::json!({ "tree": tree_items });
        if let Some(bg_sha) = base_git_sha {
            tree_body["base_tree"] = serde_json::json!(bg_sha);
        }

        let res = self.client.post(&tree_url).json(&tree_body).send()?;
        if !res.status().is_success() {
            return Err(format!("Failed to create remote Git tree: {}", res.text()?).into());
        }
        let tree_sha = res.json::<serde_json::Value>()?["sha"]
            .as_str()
            .unwrap()
            .to_string();

        // Construct Commit
        let commit_url = format!(
            "https://api.github.com/repos/{}/{}/git/commits",
            self.owner, self.repo
        );
        let commit_message = format!("DAM Sync [{}]: {}", latest_seal.id, latest_seal.message);

        let mut commit_body = serde_json::json!({
            "message": commit_message,
            "tree": tree_sha,
        });

        if !parent_shas.is_empty() {
            commit_body["parents"] = serde_json::json!(parent_shas);
        }

        let res = self.client.post(&commit_url).json(&commit_body).send()?;
        if !res.status().is_success() {
            return Err(format!("Failed to commit new tree to GitHub: {}", res.text()?).into());
        }
        let new_commit_sha = res.json::<serde_json::Value>()?["sha"]
            .as_str()
            .unwrap()
            .to_string();

        // Point remote ref to latest commit
        let ref_url = format!(
            "https://api.github.com/repos/{}/{}/git/refs/heads/{}",
            self.owner, self.repo, stream
        );
        let update_ref_body = serde_json::json!({
            "sha": new_commit_sha,
            "force": true
        });

        let patch_res = self.client.patch(&ref_url).json(&update_ref_body).send()?;
        if !patch_res.status().is_success() {
            let create_ref_url = format!(
                "https://api.github.com/repos/{}/{}/git/refs",
                self.owner, self.repo
            );
            let create_ref_body = serde_json::json!({
                "ref": format!("refs/heads/{}", stream),
                "sha": new_commit_sha
            });
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

        println!(
            "✓ Successfully pushed stream '{}' to remote repository natively!",
            stream
        );
        Ok(())
    }

    fn pull(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        println!(
            "📡 Translating Native Git Commits into local DAM Seals for '{}'...",
            stream
        );

        let commits_url = format!(
            "https://api.github.com/repos/{}/{}/commits?sha={}",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Err(format!(
                "Could not find remote stream '{}'. Nothing to pull.",
                stream
            )
            .into());
        }

        let commits: Vec<serde_json::Value> = commits_resp.json()?;
        let mut local_meta = crate::commands::stream::get_or_create_meta(stream);
        let mut current_seal_id = local_meta.latest_seal.clone();

        // Find which commits we are missing
        let mut missing_commits = Vec::new();
        for c in commits {
            let mut is_known = false;
            if let Some(msg) = c["commit"]["message"].as_str() {
                if let Some(start) = msg.find("DAM Sync [") {
                    if let Some(end) = msg[start + 10..].find(']') {
                        let sid = &msg[start + 10..start + 10 + end];
                        let local_path = format!(".dam/seals/{}.json", sid);
                        if Path::new(&local_path).exists() {
                            is_known = true;
                        }
                    }
                }
            }
            if is_known {
                break;
            }
            missing_commits.push(c);
        }

        if missing_commits.is_empty() {
            println!(
                "✅ Local stream '{}' is already up to date with remote.",
                stream
            );
            return Ok(());
        }

        missing_commits.reverse(); // Build seals chronologically
        println!(
            "⬇️  Translating {} missing Git commit(s)...",
            missing_commits.len()
        );
        fs::create_dir_all(".dam/seals")?;
        fs::create_dir_all(".dam/objects")?;

        let mut downloaded_objects_count = 0;

        for commit in missing_commits {
            let commit_sha = commit["sha"].as_str().unwrap();
            let commit_msg = commit["commit"]["message"].as_str().unwrap().to_string();
            let commit_time = commit["commit"]["author"]["date"]
                .as_str()
                .unwrap()
                .to_string();

            // Fetch commit details to pinpoint modified files
            let detail_url = format!(
                "https://api.github.com/repos/{}/{}/commits/{}",
                self.owner, self.repo, commit_sha
            );
            let detail: serde_json::Value = self.client.get(&detail_url).send()?.json()?;

            let mut new_files = Vec::new();
            if let Some(ref sid) = current_seal_id {
                let path = format!(".dam/seals/{}.json", sid);
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                        new_files = seal.files;
                    }
                }
            }

            if let Some(files_arr) = detail["files"].as_array() {
                for f in files_arr {
                    let path = f["filename"].as_str().unwrap().to_string();
                    if path.starts_with(".dam/") {
                        continue;
                    } // Exclude dam structures if present

                    let status = f["status"].as_str().unwrap();

                    if status == "removed" {
                        new_files.retain(|x| x.path != path);
                    } else {
                        if let Some(prev) = f.get("previous_filename").and_then(|v| v.as_str()) {
                            new_files.retain(|x| x.path != prev);
                        }

                        let blob_sha = f["sha"].as_str().unwrap();
                        print!("  ↓ Fetching object ({})... ", path);
                        io::stdout().flush()?;

                        let blob_url = format!(
                            "https://api.github.com/repos/{}/{}/git/blobs/{}",
                            self.owner, self.repo, blob_sha
                        );
                        let blob_res = self.client.get(&blob_url).send()?;
                        if !blob_res.status().is_success() {
                            println!("⚠️ Warning: Could not fetch blob {} for {}", blob_sha, path);
                            continue;
                        }

                        let blob_json: serde_json::Value = blob_res.json()?;
                        let b64 = blob_json["content"].as_str().unwrap().replace("\n", "");
                        let raw_data = general_purpose::STANDARD.decode(&b64)?;

                        let mut hasher = Sha256::new();
                        hasher.update(&raw_data);
                        let dam_hash = format!("{:x}", hasher.finalize());

                        let dest_obj = Path::new(".dam/objects").join(&dam_hash);
                        if !dest_obj.exists() {
                            let compressed_file = File::create(&dest_obj)?;
                            let mut encoder =
                                ZlibEncoder::new(compressed_file, Compression::default());
                            encoder.write_all(&raw_data)?;
                            encoder.finish()?;
                            downloaded_objects_count += 1;
                        }

                        new_files.retain(|x| x.path != path);
                        new_files.push(FileEntry {
                            path,
                            hash: dam_hash,
                            is_dir: false,
                        });
                        println!("Done");
                    }
                }
            }

            // Restore the original seal ID or construct a new fallback one representing the Git commit
            let new_seal_id = if let Some(start) = commit_msg.find("DAM Sync [") {
                if let Some(end) = commit_msg[start + 10..].find(']') {
                    commit_msg[start + 10..start + 10 + end].to_string()
                } else {
                    format!("seal_git_{}", &commit_sha[..8])
                }
            } else {
                format!("seal_git_{}", &commit_sha[..8])
            };

            let new_seal = Seal {
                id: new_seal_id.clone(),
                message: commit_msg,
                timestamp: commit_time,
                files: new_files,
                stream: stream.to_string(),
                parents: current_seal_id.clone().map(|s| vec![s]).unwrap_or_default(),
            };

            let path = format!(".dam/seals/{}.json", new_seal_id);
            fs::write(path, serde_json::to_string_pretty(&new_seal)?)?;
            current_seal_id = Some(new_seal_id);
        }

        if let Some(id) = current_seal_id {
            local_meta.latest_seal = Some(id);
            crate::commands::stream::save_meta(&local_meta);
        }

        println!(
            "✓ Successfully synced! Downloaded {} new objects for stream '{}'.",
            downloaded_objects_count, stream
        );
        Ok(())
    }
}
