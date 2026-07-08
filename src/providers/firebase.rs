use super::Provider;
use std::process::Command;

pub struct FirebaseProvider;

impl Provider for FirebaseProvider {
    fn name(&self) -> &'static str {
        "firebase"
    }

    fn check_environment(&self) -> bool {
        println!("Checking Firebase environment...");
        let output = Command::new("firebase").arg("--version").output();
        
        if let Ok(out) = output {
            println!("Firebase CLI version: {}", String::from_utf8_lossy(&out.stdout).trim());
            out.status.success()
        } else {
            false
        }
    }

    fn default_toml(&self, project_name: &str) -> String {
        format!(r#"[project]
name = "{}"
provider = "firebase"
enforce_password_on_project_import = false
description = "A Firebase project managed by Dam"

[setup]
commands = [
    "npm install",
    "firebase use --add"
]

[profiles.full]
description = "Full Admin Access"
include = ["**/*"]
exclude = [".git/**", "node_modules/**", ".firebase/**", ".dam/**"]

[profiles.contributor]
description = "Contributor Access (No env files or service accounts)"
include = ["**/*"]
exclude = [
    ".git/**", "node_modules/**", ".firebase/**", ".dam/**",
    "**/.env", "**/serviceAccountKey.json", "firebase-debug.log"
]
"#, project_name)
    }
}