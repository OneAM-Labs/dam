pub mod github;

use std::error::Error;

/// Summary of a single open pull request on the remote platform.
pub struct PullRequestInfo {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head_ref: String,
}

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

    /// Lists open pull requests on the remote repository.
    fn list_pull_requests(&self) -> Result<Vec<PullRequestInfo>, Box<dyn Error>>;

    /// Checks out a pull request's head commit into a brand new local stream (named
    /// `pr-<number>`), leaving all existing streams untouched. Returns the new stream's name.
    fn checkout_pull_request(&self, number: u64) -> Result<String, Box<dyn Error>>;
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