use super::Provider;

pub struct CustomProvider;

impl Provider for CustomProvider {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn check_environment(&self) -> bool {
        println!("Custom provider: Environment checks are bypassed. Will rely on setup scripts.");
        true
    }

    fn default_toml(&self, project_name: &str) -> String {
        format!(r#"[project]
name = "{}"
provider = "custom"
enforce_password_on_project_import = false
description = "A custom project managed by Dam"

[setup]
# Untrusted commands on import will be written to a reviewable setup script.
commands = [
    "echo 'Run your setup commands here'"
]

[profiles.full]
description = "Full Admin Access"
include = ["**/*"]
exclude = [".git/**", ".dam/**"]

[profiles.contributor]
description = "Basic Contributor Access"
include = ["**/*"]
exclude = [".git/**", ".dam/**", "**/.env", "**/*.secret"]
"#, project_name)
    }
}