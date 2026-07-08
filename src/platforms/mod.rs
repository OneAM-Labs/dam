pub mod github;

use std::error::Error;

/// SyncProvider replaces the older CloudPlatform to enforce a provider-agnostic 
/// interface for synchronizing DAM structures via varying underlying transports.
pub trait SyncProvider {
    /// Connects to the remote and compares the remote state with the specified local stream.
    /// Returns a tuple: (commits_ahead, commits_behind)
    fn check_diff(&self, stream: &str) -> Result<(usize, usize), Box<dyn Error>>;

    /// Pushes the latest local seals of the specific stream to the platform.
    fn push(&self, stream: &str) -> Result<(), Box<dyn Error>>;

    /// Pulls the latest remote changes of the specific stream and maps them into local `.dam/objects`.
    fn pull(&self, stream: &str) -> Result<(), Box<dyn Error>>;
}

pub fn get_provider(name: &str) -> Box<dyn SyncProvider> {
    match name.to_lowercase().as_str() {
        "github" => Box::new(github::GitHubSync::new()),
        _ => {
            println!("Warning: Provider '{}' not recognized. Defaulting to GitHub.", name);
            Box::new(github::GitHubSync::new())
        }
    }
}