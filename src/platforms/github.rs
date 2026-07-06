use super::CloudPlatform;
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

pub struct GitHubSync {
    owner: String,
    repo: String,
    client: reqwest::blocking::Client,
}

impl GitHubSync {
    pub fn new() -> Self {
        let (owner, repo) = Self::get_target_repo();
        let token = Self::resolve_token();

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

    /// Resolves the API key via a 4-tier waterfall resolution
    fn resolve_token() -> String {
        // Tier 1: Environment variable GITHUB_TOKEN
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            if !token.trim().is_empty() {
                return token.trim().to_string();
            }
        }

        // Tier 2: Check setting configuration (path to token file)
        let config_path = ".dam/config.toml";
        if let Ok(content) = fs::read_to_string(config_path) {
            if let Some(token_path) = get_toml_val(&content, "github_token_path") {
                if !token_path.trim().is_empty() {
                    if let Ok(token) = fs::read_to_string(token_path.trim()) {
                        if !token.trim().is_empty() {
                            return token.trim().to_string();
                        }
                    }
                }
            }
        }

        // Tier 3: Default secure credential file
        let default_creds_path = ".dam/credentials";
        if let Ok(token) = fs::read_to_string(default_creds_path) {
            if !token.trim().is_empty() {
                return token.trim().to_string();
            }
        }

        // Tier 4: Prompt user interactively and persist securely to .dam/credentials
        println!("\nℹ️  GitHub personal access token not found.");
        println!("Please create a token at https://github.com/settings/tokens with 'repo' scope.");
        let token_prompt = rpassword::prompt_password("GitHub Access Token: ").unwrap();
        let trimmed_token = token_prompt.trim().to_string();
        
        if trimmed_token.is_empty() {
            panic!("Error: GitHub Sync requires a non-empty personal access token.");
        }

        // Save token to .dam/credentials
        let _ = fs::write(default_creds_path, &trimmed_token);
        println!("✓ Token stored securely in .dam/credentials for subsequent transactions.");

        trimmed_token
    }

    /// Identifies owner/repo configurations, prompting interactively if not set.
    fn get_target_repo() -> (String, String) {
        let config_path = ".dam/config.toml";
        let content = fs::read_to_string(config_path).unwrap_or_default();

        if let Some(repo_val) = get_toml_val(&content, "github_repo") {
            let parts: Vec<&str> = repo_val.split('/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                return (parts[0].to_string(), parts[1].to_string());
            }
        }

        // Prompt
        print!("Enter target GitHub repository (format: owner/repo_name): ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let repo_str = input.trim();
        let parts: Vec<&str> = repo_str.split('/').collect();

        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            // Write the new setting back into config
            let updated_content = set_toml_val(&content, "github_repo", repo_str, true);
            let _ = fs::write(config_path, updated_content);
            println!("✓ Target repository configuration saved.");
            return (parts[0].to_string(), parts[1].to_string());
        }

        panic!("Error: Invalid repository format. Sync aborted.");
    }

    fn get_latest_local_seal(&self) -> Option<Seal> {
        let seals_dir = Path::new(".dam/seals");
        if !seals_dir.exists() {
            return None;
        }

        let mut seals: Vec<_> = fs::read_dir(seals_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "json"))
            .collect();

        seals.sort_by_key(|a| a.file_name());

        if let Some(latest) = seals.last() {
            let content = fs::read_to_string(latest.path()).unwrap();
            return serde_json::from_str(&content).ok();
        }
        None
    }
}

impl CloudPlatform for GitHubSync {
    fn check_diff(&self) -> Result<(usize, usize), Box<dyn Error>> {
        let main_sha;
        
        // 1. Fetch remote commit SHA for main or master
        let main_url = format!("https://api.github.com/repos/{}/{}/branches/main", self.owner, self.repo);
        let main_resp = self.client.get(&main_url).send()?;
        if main_resp.status().is_success() {
            main_sha = main_resp.json::<serde_json::Value>()?["commit"]["sha"].as_str().unwrap().to_string();
        } else {
            let master_url = format!("https://api.github.com/repos/{}/{}/branches/master", self.owner, self.repo);
            let master_resp = self.client.get(&master_url).send()?;
            if master_resp.status().is_success() {
                main_sha = master_resp.json::<serde_json::Value>()?["commit"]["sha"].as_str().unwrap().to_string();
            } else {
                // If neither branch exists, it's completely empty.
                // We count all local seals as 'ahead'.
                let local_count = fs::read_dir(".dam/seals").map(|d| d.count()).unwrap_or(0);
                return Ok((local_count, 0));
            }
        }

        // 2. Map remote tree to find all seal JSON files
        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1", self.owner, self.repo, main_sha);
        let tree_resp = self.client.get(&tree_url).send()?;
        if !tree_resp.status().is_success() { return Ok((1, 0)); }
        
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

        // 3. Map local seal JSON files
        let mut local_seals = Vec::new();
        let seals_dir = Path::new(".dam/seals");
        if seals_dir.exists() {
            for entry in fs::read_dir(seals_dir)?.flatten() {
                if entry.path().extension().map_or(false, |e| e == "json") {
                    local_seals.push(format!(".dam/seals/{}", entry.file_name().to_string_lossy()));
                }
            }
        }

        // 4. Calculate accurate state drift
        let ahead = local_seals.iter().filter(|l| !remote_seals.contains(l)).count();
        let behind = remote_seals.iter().filter(|r| !local_seals.contains(r)).count();

        Ok((ahead, behind))
    }

    fn push(&self) -> Result<(), Box<dyn Error>> {
        let latest_seal = self.get_latest_local_seal().ok_or("No local seals available to push. Please run 'dam seal' first.")?;
        
        // 0. Bootstrap check for Empty Repositories (Solves the 409 Conflict)
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
            // Allow GitHub a moment to establish backend tree indices
            thread::sleep(Duration::from_millis(1500));
        } else if commits_res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("Repository not found. Please ensure the target repository exists on GitHub and permissions are correct.".into());
        }

        println!("🚀 Translating Seal '{}' into GitHub Git Database tree...", latest_seal.id);
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

            // Read & decompress DAM Object before sending to GitHub (GitHub stores uncompressed)
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

        // 2. Upload ALL local DAM Seal metadata files to sync the timeline history
        let seals_dir = Path::new(".dam/seals");
        if seals_dir.exists() {
            for entry in fs::read_dir(seals_dir)?.flatten() {
                if entry.path().extension().map_or(false, |e| e == "json") {
                    let content = fs::read_to_string(&entry.path())?;
                    let b64_seal_metadata = general_purpose::STANDARD.encode(content.as_bytes());
                    
                    let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs", self.owner, self.repo);
                    let seal_blob_body = serde_json::json!({
                        "content": b64_seal_metadata,
                        "encoding": "base64"
                    });
            
                    let seal_res = self.client.post(&blob_url).json(&seal_blob_body).send()?;
                    if !seal_res.status().is_success() {
                        return Err(format!("Failed to upload seal metadata blob: {}", seal_res.text()?).into());
                    }
                    
                    let seal_sha = seal_res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();
                    let seal_filename = entry.path().file_name().unwrap().to_string_lossy().to_string();
                    
                    tree_items.push(serde_json::json!({
                        "path": format!(".dam/seals/{}", seal_filename),
                        "mode": "100644",
                        "type": "blob",
                        "sha": seal_sha
                    }));
                }
            }
        }

        // 3. Post Tree creation
        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees", self.owner, self.repo);
        let tree_body = serde_json::json!({ "tree": tree_items });

        let res = self.client.post(&tree_url).json(&tree_body).send()?;
        if !res.status().is_success() {
            return Err(format!("Failed to create remote Git tree: {}", res.text()?).into());
        }
        let tree_sha = res.json::<serde_json::Value>()?["sha"].as_str().unwrap().to_string();

        // 4. Get Parent SHA dynamically
        let mut parent_shas = Vec::new();
        for branch in &["main", "master"] {
            let ref_url = format!("https://api.github.com/repos/{}/{}/git/refs/heads/{}", self.owner, self.repo, branch);
            if let Ok(ref_res) = self.client.get(&ref_url).send() {
                if ref_res.status().is_success() {
                    if let Some(sha_str) = ref_res.json::<serde_json::Value>().ok().and_then(|v| v["object"]["sha"].as_str().map(String::from)) {
                        parent_shas.push(sha_str);
                        break;
                    }
                }
            }
        }

        // 5. Construct Commit
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

        // 6. Point remote ref to latest commit
        let ref_url = format!("https://api.github.com/repos/{}/{}/git/refs/heads/main", self.owner, self.repo);
        let update_ref_body = serde_json::json!({
            "sha": new_commit_sha,
            "force": true
        });

        let patch_res = self.client.patch(&ref_url).json(&update_ref_body).send()?;
        if !patch_res.status().is_success() {
            let create_ref_url = format!("https://api.github.com/repos/{}/{}/git/refs", self.owner, self.repo);
            let create_ref_body = serde_json::json!({
                "ref": "refs/heads/main",
                "sha": new_commit_sha
            });
            let create_res = self.client.post(&create_ref_url).json(&create_ref_body).send()?;
            if !create_res.status().is_success() {
                return Err(format!("Could not create main reference on GitHub: {}", create_res.text()?).into());
            }
        }

        println!("✓ Successfully pushed seal snapshot to remote repository on GitHub!");
        println!("✓ Remote Ref updated: refs/heads/main points to commit {}", &new_commit_sha[..7]);

        Ok(())
    }

    fn pull(&self) -> Result<(), Box<dyn Error>> {
        println!("📡 Pulling latest Git Database tree structures from {}/{}...", self.owner, self.repo);
        
        // 1. Get branch SHA
        let mut main_sha = String::new();
        for branch in &["main", "master"] {
            let ref_url = format!("https://api.github.com/repos/{}/{}/git/refs/heads/{}", self.owner, self.repo, branch);
            if let Ok(ref_res) = self.client.get(&ref_url).send() {
                if ref_res.status().is_success() {
                    main_sha = ref_res.json::<serde_json::Value>()?["object"]["sha"].as_str().unwrap().to_string();
                    break;
                }
            }
        }
        
        if main_sha.is_empty() {
            return Err("Could not find remote 'main' or 'master' branch. Nothing to pull.".into());
        }

        // 2. Traverse tree to identify missing seals & raw blob references
        let tree_url = format!("https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1", self.owner, self.repo, main_sha);
        let tree_data: serde_json::Value = self.client.get(&tree_url).send()?.json()?;
        
        let mut remote_seals = Vec::new();
        let mut remote_files_map = HashMap::new(); // Mappings for fast path -> sha lookups

        if let Some(tree_arr) = tree_data["tree"].as_array() {
            for item in tree_arr {
                if let (Some(path), Some(sha), Some(type_str)) = (item["path"].as_str(), item["sha"].as_str(), item["type"].as_str()) {
                    if type_str == "blob" {
                        remote_files_map.insert(path.to_string(), sha.to_string());
                        
                        if path.starts_with(".dam/seals/") && path.ends_with(".json") {
                            remote_seals.push((path.to_string(), sha.to_string()));
                        }
                    }
                }
            }
        }

        // 3. Compare with local seals
        let mut missing_seals = Vec::new();
        for (remote_path, remote_sha) in remote_seals {
            if !Path::new(&remote_path).exists() {
                missing_seals.push((remote_path, remote_sha));
            }
        }

        if missing_seals.is_empty() {
            println!("✅ Local workspace is already up to date with remote.");
            return Ok(());
        }

        println!("⬇️  Downloading {} missing cloud seal(s)...", missing_seals.len());
        fs::create_dir_all(".dam/seals")?;
        fs::create_dir_all(".dam/objects")?;

        let mut downloaded_objects_count = 0;

        for (seal_path, seal_sha) in missing_seals {
            let blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, seal_sha);
            let blob_res: serde_json::Value = self.client.get(&blob_url).send()?.json()?;
            let b64_content = blob_res["content"].as_str().unwrap().replace("\n", "");
            let seal_json_data = general_purpose::STANDARD.decode(&b64_content)?;
            
            let remote_seal: Seal = serde_json::from_slice(&seal_json_data)?;

            // 4. Download missing Objects tracked within the seal
            for file_entry in &remote_seal.files {
                if file_entry.is_dir { continue; }
                
                let obj_path = Path::new(".dam/objects").join(&file_entry.hash);
                if obj_path.exists() { continue; } // CAS benefit: skip if already downloaded!

                if let Some(file_blob_sha) = remote_files_map.get(&file_entry.path) {
                    print!("  ↓ Fetching missing object {} ({})... ", &file_entry.hash[..7], file_entry.path);
                    io::stdout().flush()?;

                    let f_blob_url = format!("https://api.github.com/repos/{}/{}/git/blobs/{}", self.owner, self.repo, file_blob_sha);
                    let f_blob_res: serde_json::Value = self.client.get(&f_blob_url).send()?.json()?;
                    let f_b64 = f_blob_res["content"].as_str().unwrap().replace("\n", "");
                    let raw_data = general_purpose::STANDARD.decode(&f_b64)?;

                    // Compress to align precisely with DAM's local object specifications
                    let compressed_file = File::create(&obj_path)?;
                    let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                    encoder.write_all(&raw_data)?;
                    encoder.finish()?;

                    println!("Done");
                    downloaded_objects_count += 1;
                } else {
                    println!("⚠️ Warning: File {} exists in seal but is missing from remote git tree.", file_entry.path);
                }
            }

            // Save the seal definition
            fs::write(seal_path, seal_json_data)?;
        }

        println!("✓ Successfully synced! Downloaded {} new objects into local reservoir.", downloaded_objects_count);
        println!("💡 To update your workspace to the latest state, run: dam apply [SEAL_ID]");

        Ok(())
    }
}