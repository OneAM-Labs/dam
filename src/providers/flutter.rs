use super::Provider;
use std::process::Command;

pub struct FlutterProvider;

impl Provider for FlutterProvider {
    fn name(&self) -> &'static str {
        "flutter"
    }

    fn check_environment(&self) -> bool {
        println!("Checking Flutter environment...");
        let output = Command::new("flutter").arg("--version").output();
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
provider = "flutter"
enforce_password_on_project_import = false
description = "A Flutter project managed by Dam"

[setup]
commands = [
    "flutter pub get",
    "flutter clean"
]

[profiles.full]
description = "Full Admin Access"
include = ["**/*"]
exclude = [".git/**", "build/**", ".dart_tool/**", ".dam/**"]

[profiles.contributor]
description = "Contributor Access (No env files or key stores)"
include = ["**/*"]
exclude = [
    ".git/**", "build/**", ".dart_tool/**", ".dam/**",
    "**/*.env", "**/key.properties", "android/app/*.jks"
]
"#, project_name)
    }
}