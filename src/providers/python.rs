use super::Provider;
use std::process::Command;

pub struct PythonProvider;

impl Provider for PythonProvider {
    fn name(&self) -> &'static str {
        "python"
    }

    fn check_environment(&self) -> bool {
        println!("Checking Python environment...");
        // Attempts to check python3 first, falling back to python if unavailable.
        let output = Command::new("python3").arg("--version").output()
            .or_else(|_| Command::new("python").arg("--version").output());

        if let Ok(out) = output {
            println!("{}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or(""));
            out.status.success()
        } else {
            false
        }
    }

    fn default_toml(&self, project_name: &str) -> String {
        format!(r#"[project]
name = "{}"
provider = "python"
enforce_password_on_project_import = false
description = "A Python project managed by Dam"

[setup]
commands = [
    "python3 -m venv venv",
    "venv/bin/pip install -r requirements.txt"
]

[profiles.full]
description = "Full Admin Access"
include = ["**/*"]
exclude = [".git/**", "__pycache__/**", "venv/**", ".venv/**", ".dam/**"]

[profiles.contributor]
description = "Contributor Access (No env files or private keys)"
include = ["**/*"]
exclude = [
    ".git/**", "__pycache__/**", "venv/**", ".venv/**", ".dam/**",
    "**/.env", "**/*.pem", "**/*.key"
]
"#, project_name)
    }
}