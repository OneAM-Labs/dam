pub mod github;

use std::error::Error;

pub trait CloudPlatform {
    /// Connects to the remote and compares the remote state with the local stream.
    /// Returns a tuple: (commits_ahead, commits_behind)
    fn check_diff(&self) -> Result<(usize, usize), Box<dyn Error>>;

    /// Pushes the latest local seals to the platform.
    fn push(&self) -> Result<(), Box<dyn Error>>;

    /// Pulls the latest remote changes and maps them into local `.dam/objects`.
    fn pull(&self) -> Result<(), Box<dyn Error>>;
}

pub fn get_platform(name: &str) -> Box<dyn CloudPlatform> {
    match name.to_lowercase().as_str() {
        "github" => Box::new(github::GitHubSync::new()),
        _ => {
            println!("Warning: Platform '{}' not recognized. Defaulting to GitHub.", name);
            Box::new(github::GitHubSync::new())
        }
    }
}