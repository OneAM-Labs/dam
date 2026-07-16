use crate::cli::CredsCommands;
use crate::core::credentials::{delete_credential, load_aliases, save_credential, Credential};
use std::io::{self, Write};

pub fn run(command: CredsCommands) {
    match command {
        CredsCommands::Create { alias, vault } => {
            let alias_name = if let Some(a) = alias {
                a
            } else {
                print!("Enter alias name (e.g., github_personal): ");
                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin().read_line(&mut input).unwrap();
                input.trim().to_string()
            };

            println!("Select Credential Type:");
            println!("  1. Fine-Grained Token (GitHub)");
            println!("  2. Classic Token (GitHub)");
            println!("  3. SSH Key");
            println!("  4. Generic Secret");
            print!("Choice [1-4]: ");
            io::stdout().flush().unwrap();
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();

            let (cred_type, secret) = match choice.trim() {
                "1" => ("FineGrainedToken", rpassword::prompt_password("Enter Fine-Grained Token: ").unwrap()),
                "2" => ("ClassicToken", rpassword::prompt_password("Enter Classic Token: ").unwrap()),
                "3" => {
                    print!("Enter path to private SSH key: ");
                    io::stdout().flush().unwrap();
                    let mut path = String::new();
                    io::stdin().read_line(&mut path).unwrap();
                    ("SshKey", path.trim().to_string())
                }
                _ => ("Generic", rpassword::prompt_password("Enter Secret: ").unwrap()),
            };

            let cred = Credential {
                alias: alias_name.clone(),
                cred_type: cred_type.to_string(),
                secret: secret.trim().to_string(),
                extra: None,
            };

            match save_credential(cred, vault) {
                Ok(_) => {
                    if vault {
                        println!("✓ Credential '{}' saved securely in local Encrypted Vault.", alias_name);
                    } else {
                        println!("✓ Credential '{}' saved securely in OS Keychain.", alias_name);
                    }
                }
                Err(e) => println!("❌ Failed to save credential: {}", e),
            }
        }
        CredsCommands::List { vault: _ } => {
            let aliases = load_aliases();
            if aliases.is_empty() {
                println!("No credentials found.");
            } else {
                println!("Registered Credential Aliases:");
                for a in aliases {
                    println!("  - {}", a);
                }
            }
        }
        CredsCommands::Delete { alias, vault } => {
            match delete_credential(&alias, vault) {
                Ok(_) => println!("✓ Credential '{}' deleted successfully.", alias),
                Err(e) => println!("❌ Failed to delete credential: {}", e),
            }
        }
    }
}