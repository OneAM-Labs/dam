use super::SyncProvider;
use crate::commands::base_commands::settings::{get_toml_val, set_toml_val};
use crate::commands::seal::{FileEntry, Seal};
use base64::{Engine as _, engine::general_purpose};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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
            .build()
            .unwrap();

        Self {
            owner,
            repo,
            client,
        }
    }

    fn extract_seal_id(msg: &str, fallback_sha: &str) -> String {
        // Check clean footer format: [dam:seal_xxxx]
        if let Some(start) = msg.find("[dam:") {
            if let Some(end) = msg[start + 5..].find(']') {
                return msg[start + 5..start + 5 + end].trim().to_string();
            }
        }
        // Backward compatibility for legacy prefix format: DAM Sync [seal_xxxx]
        if let Some(start) = msg.find("DAM Sync [") {
            if let Some(end) = msg[start + 10..].find(']') {
                return msg[start + 10..start + 10 + end].trim().to_string();
            }
        }
        format!("seal_git_{}", &fallback_sha[..8])
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
                print!("Enter path to private SSH key: ");
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
            "https://api.github.com/repos/{}/{}/commits?sha={}",
            self.owner, self.repo, stream
        );
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Ok((local_seals.len(), 0));
        }

        let commits: Vec<serde_json::Value> = commits_resp.json()?;
        let mut remote_seal_ids = Vec::new();

        for c in &commits {
            let msg = c["commit"]["message"].as_str().unwrap_or("");
            let sha = c["sha"].as_str().unwrap_or("");
            remote_seal_ids.push(Self::extract_seal_id(msg, sha));
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
                        "Failed to bootstrap empty repository: {}",
                        init_res.text()?
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

        let mut tree_items = Vec::new();
        for file_entry in changed_files {
            let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
            if !obj_path.exists() {
                println!(
                    "⚠️ Warning: Object {} missing for file {}. Skipping.",
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
            tree_items.push(serde_json::json!({ "path": file_entry.path, "mode": "100644", "type": "blob", "sha": sha }));
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
            let tree_url = format!(
                "https://api.github.com/repos/{}/{}/git/trees",
                self.owner, self.repo
            );
            let mut tree_body = serde_json::json!({ "tree": tree_items });
            if let Some(ref bg_sha) = base_git_sha {
                tree_body["base_tree"] = serde_json::json!(bg_sha);
            }

            let res = self.client.post(&tree_url).json(&tree_body).send()?;
            if !res.status().is_success() {
                return Err(format!("Failed to create remote Git tree: {}", res.text()?).into());
            }
            res.json::<serde_json::Value>()?["sha"]
                .as_str()
                .unwrap()
                .to_string()
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

        let res = self.client.post(&commit_url).json(&commit_body).send()?;
        if !res.status().is_success() {
            return Err(format!("Failed to commit new tree to GitHub: {}", res.text()?).into());
        }
        let new_commit_sha = res.json::<serde_json::Value>()?["sha"]
            .as_str()
            .unwrap()
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

        println!(
            "✓ Successfully pushed stream '{}' to remote repository natively!",
            stream
        );
        Ok(())
    }

    fn pull(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        println!("📡 Checking remote Git commits for stream '{}'...", stream);

        let commits_url = format!("https://api.github.com/repos/{}/{}/commits?sha={}", self.owner, self.repo, stream);
        let commits_resp = self.client.get(&commits_url).send()?;

        if !commits_resp.status().is_success() {
            return Err(format!("Could not find remote stream '{}'. Nothing to pull.", stream).into());
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
        let mut missing_commits = Vec::new();
        let mut remote_head_seal_id = None;

        for (i, c) in commits.iter().enumerate() {
            let msg = c["commit"]["message"].as_str().unwrap_or("");
            let sha = c["sha"].as_str().unwrap_or("");
            let expected_sid = Self::extract_seal_id(msg, sha);

            if i == 0 {
                remote_head_seal_id = Some(expected_sid.clone());
            }

            if local_seals.contains(&expected_sid) { break; }
            missing_commits.push(c.clone());
        }

        if missing_commits.is_empty() {
            println!("✅ Local stream '{}' is already up to date with remote.", stream);
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
        println!("⬇️  Downloading {} missing history commit(s)...", missing_commits.len());
        fs::create_dir_all(".dam/seals")?;
        fs::create_dir_all(".dam/objects")?;

        let mut current_remote_seal_id = None;

        for commit in missing_commits {
            let commit_sha = commit["sha"].as_str().unwrap();
            let raw_commit_msg = commit["commit"]["message"].as_str().unwrap().to_string();
            let commit_time = commit["commit"]["author"]["date"].as_str().unwrap().to_string();

            let clean_msg = if let Some(idx) = raw_commit_msg.find("\n\n[dam:") {
                raw_commit_msg[..idx].to_string()
            } else {
                raw_commit_msg.clone()
            };

            let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1", self.owner, self.repo, commit_sha);
            let tree_json: serde_json::Value = self.client.get(&tree_url).send()?.json()?;

            let mut new_files = Vec::new();

            if let Some(tree_arr) = tree_json["tree"].as_array() {
                for item in tree_arr {
                    if item["type"] != "blob" { continue; }
                    let path = item["path"].as_str().unwrap().to_string();
                    if path.starts_with(".dam/") { continue; }

                    let blob_sha = item["sha"].as_str().unwrap();
                    let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, blob_sha);
                    let blob_res = self.client.get(&blob_url).send()?;
                    if !blob_res.status().is_success() { continue; }

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

                    new_files.push(FileEntry { path, hash: dam_hash, is_dir: false });
                }
            }

            let new_seal_id = Self::extract_seal_id(&raw_commit_msg, commit_sha);

            // If we aren't diverged, link to our previous local seal. Otherwise, build remote chain.
            let parent_chain = match current_remote_seal_id.clone() {
                Some(id) => vec![id],
                None => if !is_diverged {
                    original_local_seal_id.clone().map(|s| vec![s]).unwrap_or_default()
                } else {
                    vec![]
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
            println!("\nWhich seal do you want to set as the active latest seal for stream '{}'?", stream);
            println!("  [1] Remote Latest : {} (Use GitHub's timeline)", remote_id);
            println!("  [2] Local Latest  : {} (Keep your local timeline)", local_id);
            print!("\nEnter your choice (1 or 2): ");
            io::stdout().flush()?;

            let mut choice = String::new();
            io::stdin().read_line(&mut choice)?;

            match choice.trim() {
                "1" => {
                    local_meta.latest_seal = Some(remote_id.clone());
                    println!("✓ Updated active stream pointer to Remote Latest ({}).", remote_id);
                    println!("  Note: Your previous local seal ({}) is still saved in .dam/seals/.", local_id);
                }
                "2" | _ => {
                    // Do nothing to local_meta.latest_seal, keep it pointing to local_id
                    println!("✓ Kept active stream pointer at Local Latest ({}).", local_id);
                    println!("  Note: The downloaded remote seal ({}) is saved in .dam/seals/ if you need it later.", remote_id);
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
}
