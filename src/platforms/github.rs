use super::SyncProvider;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;
use crate::commands::seal::{Seal, FileEntry};
use crate::commands::base_commands::settings::{get_toml_val, set_toml_val};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use base64::{Engine as _, engine::general_purpose};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AuthMethod {
    FineGrainedToken { token: String },
    ClassicToken { token: String },
    SshKey { key_path: String, passphrase: Option<String> },
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
                panic!("Error: SSH authentication is configured, but the current GitHub Provider utilizes REST APIs which require a Personal Access Token. Native SSH Git Transport is pending implementation.");
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
            format!("DAM-CLI-Sync/{}", env!("CARGO_PKG_VERSION")).parse().unwrap(),
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
        
        // 1. Try to load structured credentials
        if let Ok(content) = fs::read_to_string(creds_path) {
            if let Ok(cred) = serde_json::from_str(&content) {
                return cred;
            }
        }
        
        // 2. Legacy fallback
        let legacy_path = ".dam/credentials";
        if let Ok(token) = fs::read_to_string(legacy_path) {
            if !token.trim().is_empty() {
                return AuthMethod::ClassicToken { token: token.trim().to_string() };
            }
        }

        // 3. Prompt interactively
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
                AuthMethod::SshKey { key_path: path.trim().to_string(), passphrase: None }
            },
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
            local_seals.push(format!(".dam/seals/{}.json", sid));
            if let Ok(content) = fs::read_to_string(format!(".dam/seals/{}.json", sid)) {
                if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                    current_seal_id = seal.parents.first().cloned();
                    continue;
                }
            }
            break;
        }

        let branch_url = format!("https://api.github.com/repos/{}/{}/branches/{}", self.owner, self.repo, stream);
        let branch_resp = self.client.get(&branch_url).send()?;
        
        if !branch_resp.status().is_success() {
            return Ok((local_seals.len(), 0));
        }

        let branch_sha = branch_resp.json::<serde_json::Value>()?["commit"]["sha"].as_str().unwrap().to_string();
        
        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1", self.owner, self.repo, branch_sha);
        let tree_resp = self.client.get(&tree_url).send()?;
        if !tree_resp.status().is_success() { return Ok((local_seals.len(), 0)); }
        
        let tree_data: serde_json::Value = tree_resp.json()?;
        let mut remote_seals = Vec::new();
        
        if let Some(tree_arr) = tree_data["tree"].as_array() {
            for item in tree_arr {
                if let Some(path) = item["path"].as_str() {
                    if path.starts_with(".dam/seals/") && path.ends_with(".json") {
                        remote_seals.push(path.to_string());
                    }
                }
            }
        }

        let ahead = local_seals.iter().filter(|l| !remote_seals.contains(l)).count();
        let behind = remote_seals.iter().filter(|r| !local_seals.contains(r)).count();

        Ok((ahead, behind))
    }

    fn push(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        let meta = crate::commands::stream::get_or_create_meta(stream);
        let latest_seal_id = meta.latest_seal.clone().ok_or(format!("No seals available in stream '{}'.", stream))?;
        
        let seal_path = format!(".dam/seals/{}.json", latest_seal_id);
        let content = fs::read_to_string(&seal_path)?;
        let latest_seal: Seal = serde_json::from_str(&content)?;

        // 0. Bootstrap check for Empty Repositories
        let commits_url = format!("https://api.github.com/repos/{}/{}/commits", self.owner, self.repo);
        let commits_res = self.client.get(&commits_url).send()?;
        
        if commits_res.status() == reqwest::StatusCode::CONFLICT {
            println!("💡 Empty repository detected. Bootstrapping initial structure to initialize Git DB...");
            let init_url = format!("https://api.github.com/repos/{}/{}/contents/.dam_keep", self.owner, self.repo);
            let init_body = serde_json::json!({
                "message": "Initialize DAM Reservoir",
                "content": general_purpose::STANDARD.encode("Initialized by DAM Cloud Sync.")
            });
            let init_res = self.client.put(&init_url).json(&init_body).send()?;
            if !init_res.status().is_success() {
                 return Err(format!("Failed to bootstrap empty repository: {}", init_res.text()?).into());
            }
            thread::sleep(Duration::from_millis(1500));
        } else if commits_res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("Repository not found. Please ensure the target repository exists on GitHub and permissions are correct.".into());
        }

        println!("🚀 Translating Stream '{}' into GitHub Git Database tree...", stream);
        let mut tree_items = Vec::new();

        // 1. Upload workspace files
        for file in &latest_seal.files {
            let file_entry: &FileEntry = file;
            if file_entry.is_dir { continue; }

            let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
            if !obj_path.exists() {
                println!("⚠️  Warning: Object {} missing for file {}. Skipping file.", file_entry.hash, file_entry.path);
                continue;
            }

            let compressed_file = File::open(obj_path)?;
            let mut decoder = ZlibDecoder::new(compressed_file);
            let mut raw_data = Vec::new();
            decoder.read_to_end(&mut raw_data)?;

            let b64_content = general_purpose::STANDARD.encode(&raw_data);
            let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs", self.owner, self.repo);
            let blob_body = serde_json::json!({
                "content": b64_content,
                "encoding": "base64"
            });

            let res = self.client.post(&blob_url).json(&blob_body).send()?;
            if !res.status().is_success() {
                return Err(format!("Failed to upload blob for {}: {}", file_entry.path, res.text()?).into());
            }

            let sha = res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();
            tree_items.push(serde_json::json!({
                "path": file_entry.path,
                "mode": "100644",
                "type": "blob",
                "sha": sha
            }));
        }

        // 2. Upload local DAM Seal metadata files for THIS stream
        let mut current_sid = Some(latest_seal_id.clone());
        while let Some(sid) = current_sid {
            let s_path = format!(".dam/seals/{}.json", sid);
            if let Ok(seal_data) = fs::read_to_string(&s_path) {
                let b64_seal = general_purpose::STANDARD.encode(seal_data.as_bytes());
                
                let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs", self.owner, self.repo);
                let seal_blob_body = serde_json::json!({"content": b64_seal, "encoding": "base64"});
        
                let seal_res = self.client.post(&blob_url).json(&seal_blob_body).send()?;
                if seal_res.status().is_success() {
                    let seal_sha = seal_res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();
                    tree_items.push(serde_json::json!({
                        "path": format!(".dam/seals/{}.json", sid),
                        "mode": "100644",
                        "type": "blob",
                        "sha": seal_sha
                    }));
                }
                
                if let Ok(s) = serde_json::from_str::<Seal>(&seal_data) {
                    current_sid = s.parents.first().cloned();
                } else { break; }
            } else { break; }
        }

        // 3. Upload stream meta
        let stream_meta_path = format!(".dam/streams/{}", stream);
        if let Ok(meta_data) = fs::read_to_string(&stream_meta_path) {
            let b64_meta = general_purpose::STANDARD.encode(meta_data.as_bytes());
            let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs", self.owner, self.repo);
            let meta_res = self.client.post(&blob_url).json(&serde_json::json!({"content": b64_meta, "encoding": "base64"})).send()?;
            if meta_res.status().is_success() {
                let meta_sha = meta_res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();
                tree_items.push(serde_json::json!({
                    "path": format!(".dam/streams/{}", stream),
                    "mode": "100644",
                    "type": "blob",
                    "sha": meta_sha
                }));
            }
        }

        // 4. Post Tree creation
        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees", self.owner, self.repo);
        let tree_body = serde_json::json!({ "tree": tree_items });

        let res = self.client.post(&tree_url).json(&tree_body).send()?;
        if !res.status().is_success() {
            return Err(format!("Failed to create remote Git tree: {}", res.text()?).into());
        }
        let tree_sha = res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();

        // 5. Get Parent SHA dynamically
        let mut parent_shas = Vec::new();
        let ref_url = format!("https://api.github.com/repos/{}/{}/git/refs/heads/{}", self.owner, self.repo, stream);
        if let Ok(ref_res) = self.client.get(&ref_url).send() {
            if ref_res.status().is_success() {
                if let Some(sha_str) = ref_res.json::<serde_json::Value>().ok().and_then(|v| v["object"]["sha"].as_str().map(String::from)) {
                    parent_shas.push(sha_str);
                }
            }
        }

        // 6. Construct Commit
        let commit_url = format!("https://api.github.com/repos/{}/{}/git/commits", self.owner, self.repo);
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
        let new_commit_sha = res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();

        // 7. Point remote ref to latest commit
        let update_ref_body = serde_json::json!({
            "sha": new_commit_sha,
            "force": true
        });

        let patch_res = self.client.patch(&ref_url).json(&update_ref_body).send()?;
        if !patch_res.status().is_success() {
            let create_ref_url = format!("https://api.github.com/repos/{}/{}/git/refs", self.owner, self.repo);
            let create_ref_body = serde_json::json!({
                "ref": format!("refs/heads/{}", stream),
                "sha": new_commit_sha
            });
            let create_res = self.client.post(&create_ref_url).json(&create_ref_body).send()?;
            if !create_res.status().is_success() {
                return Err(format!("Could not create stream reference on GitHub: {}", create_res.text()?).into());
            }
        }

        println!("✓ Successfully pushed stream '{}' to remote repository!", stream);
        Ok(())
    }

    fn pull(&self, stream: &str) -> Result<(), Box<dyn Error>> {
        println!("📡 Pulling latest Git Database tree structures for stream '{}'...", stream);
        
        let ref_url = format!("https://api.github.com/repos/{}/{}/git/refs/heads/{}", self.owner, self.repo, stream);
        let ref_res = self.client.get(&ref_url).send()?;
        
        if !ref_res.status().is_success() {
            return Err(format!("Could not find remote stream '{}'. Nothing to pull.", stream).into());
        }
        let branch_sha = ref_res.json::<serde_json::Value>()?["object"]["sha"].as_str().unwrap().to_string();

        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1", self.owner, self.repo, branch_sha);
        let tree_data: serde_json::Value = self.client.get(&tree_url).send()?.json()?;
        
        let mut remote_seals = Vec::new();
        let mut remote_files_map = HashMap::new(); 
        let mut remote_stream_meta = None;

        if let Some(tree_arr) = tree_data["tree"].as_array() {
            for item in tree_arr {
                if let (Some(path), Some(sha), Some(type_str)) = (item["path"].as_str(), item["sha"].as_str(), item["type"].as_str()) {
                    if type_str == "blob" {
                        remote_files_map.insert(path.to_string(), sha.to_string());
                        
                        if path.starts_with(".dam/seals/") && path.ends_with(".json") {
                            remote_seals.push((path.to_string(), sha.to_string()));
                        } else if path == format!(".dam/streams/{}", stream) {
                            remote_stream_meta = Some((path.to_string(), sha.to_string()));
                        }
                    }
                }
            }
        }

        let mut missing_seals = Vec::new();
        for (remote_path, remote_sha) in remote_seals {
            if !Path::new(&remote_path).exists() {
                missing_seals.push((remote_path, remote_sha));
            }
        }

        if missing_seals.is_empty() {
            println!("✅ Local stream '{}' is already up to date with remote.", stream);
            if let Some((path, sha)) = remote_stream_meta {
                 let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, sha);
                 if let Ok(blob_res) = self.client.get(&blob_url).send()?.json::<serde_json::Value>() {
                     if let Some(b64) = blob_res["content"].as_str() {
                         if let Ok(meta_data) = general_purpose::STANDARD.decode(b64.replace("\n", "")) {
                             let _ = fs::write(&path, meta_data);
                         }
                     }
                 }
            }
            return Ok(());
        }

        println!("⬇️  Downloading {} missing cloud seal(s)...", missing_seals.len());
        fs::create_dir_all(".dam/seals")?;
        fs::create_dir_all(".dam/objects")?;
        fs::create_dir_all(".dam/streams")?;

        let mut downloaded_objects_count = 0;
        let mut newest_timestamp = String::new();
        let mut head_seal_id = None;

        for (seal_path, seal_sha) in missing_seals {
            let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, seal_sha);
            let blob_res: serde_json::Value = self.client.get(&blob_url).send()?.json()?;
            let b64_content = blob_res["content"].as_str().unwrap().replace("\n", "");
            let seal_json_data = general_purpose::STANDARD.decode(&b64_content)?;
            
            let remote_seal: Seal = serde_json::from_slice(&seal_json_data)?;

            if remote_seal.timestamp > newest_timestamp {
                newest_timestamp = remote_seal.timestamp.clone();
                head_seal_id = Some(remote_seal.id.clone());
            }

            for file_entry in &remote_seal.files {
                if file_entry.is_dir { continue; }
                
                let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
                if obj_path.exists() { continue; } 

                if let Some(file_blob_sha) = remote_files_map.get(&file_entry.path) {
                    print!("  ↓ Fetching missing object {} ({})... ", &file_entry.hash[..7], file_entry.path);
                    io::stdout().flush()?;

                    let f_blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, file_blob_sha);
                    let f_blob_res: serde_json::Value = self.client.get(&f_blob_url).send()?.json()?;
                    let f_b64 = f_blob_res["content"].as_str().unwrap().replace("\n", "");
                    let raw_data = general_purpose::STANDARD.decode(&f_b64)?;

                    let compressed_file = File::create(&obj_path)?;
                    let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                    encoder.write_all(&raw_data)?;
                    encoder.finish()?;

                    println!("Done");
                    downloaded_objects_count += 1;
                }
            }
            fs::write(seal_path, seal_json_data)?;
        }

        if let Some((meta_path, meta_sha)) = remote_stream_meta {
             let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, meta_sha);
             if let Ok(blob_res) = self.client.get(&blob_url).send()?.json::<serde_json::Value>() {
                 if let Some(b64) = blob_res["content"].as_str() {
                     if let Ok(meta_data) = general_purpose::STANDARD.decode(b64.replace("\n", "")) {
                         let _ = fs::write(&meta_path, meta_data);
                     }
                 }
             }
        } else if let Some(head_id) = head_seal_id {
            let mut local_meta = crate::commands::stream::get_or_create_meta(stream);
            local_meta.latest_seal = Some(head_id);
            crate::commands::stream::save_meta(&local_meta);
        }

        println!("✓ Successfully synced! Downloaded {} new objects for stream '{}'.", downloaded_objects_count, stream);
        Ok(())
    }
}