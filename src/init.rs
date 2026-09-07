//! Initialization module to scaffold the kaji configuration file.

use crate::args::Config;
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Runs the initialization command to scaffold a configuration file.
///
/// # Errors
/// Returns an error if the output file already exists (and overwrite is declined),
/// or if serialization/writing fails.
pub async fn run(interactive: bool, output: Option<PathBuf>, ui: &dyn Ui) -> Result<()> {
    // 1. Determine output path
    let output_path = if let Some(path) = output {
        path
    } else if interactive {
        let input_path = ui.input(
            "Output configuration file path",
            Some("kaji.toml".to_string()),
            false,
        )?;
        PathBuf::from(input_path)
    } else {
        PathBuf::from("kaji.toml")
    };

    // 2. Overwrite check
    if output_path.exists() {
        if interactive {
            let overwrite = ui.confirm(
                &format!(
                    "File {:?} already exists. Do you want to overwrite it?",
                    output_path
                ),
                false,
            )?;
            if !overwrite {
                anyhow::bail!("Initialization aborted to prevent overwrite.");
            }
        } else {
            anyhow::bail!(
                "File {:?} already exists. Delete it or specify a different output path.",
                output_path
            );
        }
    }

    // 3. Prefill values from environment
    let env_server = std::env::var("KEYCLOAK_URL").ok();
    let env_realms = std::env::var("KEYCLOAK_REALMS").ok();
    let env_user = std::env::var("KEYCLOAK_USER").ok();
    let env_client_id = std::env::var("KEYCLOAK_CLIENT_ID").ok();
    let env_profile = std::env::var("KAJI_PROFILE").ok();
    let env_vault_addr = std::env::var("VAULT_ADDR").ok();
    let env_vault_token = std::env::var("VAULT_TOKEN").ok();
    let env_workspace = std::env::var("KAJI_WORKSPACE").ok();

    let mut config = Config::default();

    if interactive {
        ui.print_info("Scaffolding kaji configuration interactively.");

        let server_input = ui.input("Keycloak Server URL", env_server, true)?;
        config.server = if server_input.trim().is_empty() {
            None
        } else {
            Some(server_input.trim().to_string())
        };

        let realms_input = ui.input("Keycloak Realms (comma-separated)", env_realms, true)?;
        config.realms = if realms_input.trim().is_empty() {
            None
        } else {
            Some(
                realms_input
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            )
        };

        let user_input = ui.input("Keycloak Admin User", env_user, true)?;
        config.user = if user_input.trim().is_empty() {
            None
        } else {
            Some(user_input.trim().to_string())
        };

        let client_id_input = ui.input("Keycloak Client ID", env_client_id, true)?;
        config.client_id = if client_id_input.trim().is_empty() {
            None
        } else {
            Some(client_id_input.trim().to_string())
        };

        let profile_input = ui.input("Environment Profile Name", env_profile, true)?;
        config.profile = if profile_input.trim().is_empty() {
            None
        } else {
            Some(profile_input.trim().to_string())
        };

        let vault_addr_input = ui.input("HashiCorp Vault URL", env_vault_addr, true)?;
        config.vault_addr = if vault_addr_input.trim().is_empty() {
            None
        } else {
            Some(vault_addr_input.trim().to_string())
        };

        let vault_token_input = ui.input("HashiCorp Vault Token", env_vault_token, true)?;
        config.vault_token = if vault_token_input.trim().is_empty() {
            None
        } else {
            Some(vault_token_input.trim().to_string())
        };

        let workspace_input = ui.input("Workspace directory", env_workspace, true)?;
        config.workspace = if workspace_input.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(workspace_input.trim()))
        };
    } else {
        config.server = env_server;
        config.realms = env_realms.map(|r| {
            r.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });
        config.user = env_user;
        config.client_id = env_client_id;
        config.profile = env_profile;
        config.vault_addr = env_vault_addr;
        config.vault_token = env_vault_token;
        config.workspace = env_workspace.map(PathBuf::from);
    }

    // 4. Serialize to TOML and write securely
    let serialized =
        toml::to_string_pretty(&config).context("Failed to serialize configuration to TOML")?;
    crate::utils::write_secure(&output_path, &serialized)
        .await
        .context("Failed to write config file securely")?;

    ui.print_success(&format!(
        "Successfully scaffolded configuration file at {:?}",
        output_path
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::ui::MockUi;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static INIT_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn test_run_non_interactive_empty() {
        if std::env::var("RUN_TEST_INIT_NON_INTERACTIVE_EMPTY").is_ok() {
            let dir = tempdir().unwrap();
            let output_path = dir.path().join("kaji.toml");

            let ui = MockUi {
                inputs: Mutex::new(vec![]),
                confirms: Mutex::new(vec![]),
                selects: Mutex::new(vec![]),
                passwords: Mutex::new(vec![]),
            };

            run(false, Some(output_path.clone()), &ui).await.unwrap();

            assert!(output_path.exists());
            let content = tokio::fs::read_to_string(&output_path).await.unwrap();
            let config: Config = toml::from_str(&content).unwrap();
            assert!(config.server.is_none());
            assert!(config.realms.is_none());
            assert!(config.user.is_none());
            assert!(config.client_id.is_none());
            assert!(config.profile.is_none());
            assert!(config.vault_addr.is_none());
            assert!(config.vault_token.is_none());
            assert!(config.workspace.is_none());
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("init::tests::test_run_non_interactive_empty")
            .arg("--exact")
            .arg("--nocapture")
            .env("RUN_TEST_INIT_NON_INTERACTIVE_EMPTY", "1")
            .env_remove("KEYCLOAK_URL")
            .env_remove("KEYCLOAK_REALMS")
            .env_remove("KEYCLOAK_USER")
            .env_remove("KEYCLOAK_CLIENT_ID")
            .env_remove("KAJI_PROFILE")
            .env_remove("VAULT_ADDR")
            .env_remove("VAULT_TOKEN")
            .env_remove("KAJI_WORKSPACE")
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "Subprocess didn't run the test. Output: {}",
            stdout
        );
        assert!(output.status.success(), "Subprocess failed: {:?}", output);
    }

    #[tokio::test]
    async fn test_run_non_interactive_prefilled() {
        if std::env::var("RUN_TEST_INIT_NON_INTERACTIVE_PREFILLED").is_ok() {
            let dir = tempdir().unwrap();
            let output_path = dir.path().join("kaji.toml");

            let ui = MockUi {
                inputs: Mutex::new(vec![]),
                confirms: Mutex::new(vec![]),
                selects: Mutex::new(vec![]),
                passwords: Mutex::new(vec![]),
            };

            run(false, Some(output_path.clone()), &ui).await.unwrap();

            assert!(output_path.exists());
            let content = tokio::fs::read_to_string(&output_path).await.unwrap();
            let config: Config = toml::from_str(&content).unwrap();
            assert_eq!(config.server, Some("http://localhost:8080".to_string()));
            assert_eq!(
                config.realms,
                Some(vec!["master".to_string(), "dev".to_string()])
            );
            assert_eq!(config.user, Some("admin".to_string()));
            assert_eq!(config.client_id, Some("my-client".to_string()));
            assert_eq!(config.profile, Some("staging".to_string()));
            assert_eq!(config.vault_addr, Some("http://vault:8200".to_string()));
            assert_eq!(config.vault_token, Some("s.token123".to_string()));
            assert_eq!(config.workspace, Some(PathBuf::from("custom-ws")));
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("init::tests::test_run_non_interactive_prefilled")
            .arg("--exact")
            .arg("--nocapture")
            .env("RUN_TEST_INIT_NON_INTERACTIVE_PREFILLED", "1")
            .env("KEYCLOAK_URL", "http://localhost:8080")
            .env("KEYCLOAK_REALMS", "master,dev")
            .env("KEYCLOAK_USER", "admin")
            .env("KEYCLOAK_CLIENT_ID", "my-client")
            .env("KAJI_PROFILE", "staging")
            .env("VAULT_ADDR", "http://vault:8200")
            .env("VAULT_TOKEN", "s.token123")
            .env("KAJI_WORKSPACE", "custom-ws")
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "Subprocess didn't run the test. Output: {}",
            stdout
        );
        assert!(output.status.success(), "Subprocess failed: {:?}", output);
    }

    #[tokio::test]
    async fn test_run_non_interactive_exists_error() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("kaji.toml");
        tokio::fs::write(&output_path, "existing content")
            .await
            .unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        let result = run(false, Some(output_path.clone()), &ui).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already exists. Delete it or specify a different output path.")
        );
    }

    #[tokio::test]
    async fn test_run_interactive_happy_path() {
        if std::env::var("RUN_TEST_INIT_INTERACTIVE_HAPPY_PATH").is_ok() {
            let dir = tempdir().unwrap();
            let output_path = dir.path().join("kaji.toml");

            let ui = MockUi {
                inputs: Mutex::new(vec![
                    "http://keycloak.test".to_string(),
                    "test-realm".to_string(),
                    "test-user".to_string(),
                    "test-client".to_string(),
                    "test-profile".to_string(),
                    "http://vault.test".to_string(),
                    "vault-tok".to_string(),
                    "my-workspace-dir".to_string(),
                ]),
                confirms: Mutex::new(vec![]),
                selects: Mutex::new(vec![]),
                passwords: Mutex::new(vec![]),
            };

            run(true, Some(output_path.clone()), &ui).await.unwrap();

            assert!(output_path.exists());
            let content = tokio::fs::read_to_string(&output_path).await.unwrap();
            let config: Config = toml::from_str(&content).unwrap();
            assert_eq!(config.server, Some("http://keycloak.test".to_string()));
            assert_eq!(config.realms, Some(vec!["test-realm".to_string()]));
            assert_eq!(config.user, Some("test-user".to_string()));
            assert_eq!(config.client_id, Some("test-client".to_string()));
            assert_eq!(config.profile, Some("test-profile".to_string()));
            assert_eq!(config.vault_addr, Some("http://vault.test".to_string()));
            assert_eq!(config.vault_token, Some("vault-tok".to_string()));
            assert_eq!(config.workspace, Some(PathBuf::from("my-workspace-dir")));
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("init::tests::test_run_interactive_happy_path")
            .arg("--exact")
            .arg("--nocapture")
            .env("RUN_TEST_INIT_INTERACTIVE_HAPPY_PATH", "1")
            .env_remove("KEYCLOAK_URL")
            .env_remove("KEYCLOAK_REALMS")
            .env_remove("KEYCLOAK_USER")
            .env_remove("KEYCLOAK_CLIENT_ID")
            .env_remove("KAJI_PROFILE")
            .env_remove("VAULT_ADDR")
            .env_remove("VAULT_TOKEN")
            .env_remove("KAJI_WORKSPACE")
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "Subprocess didn't run the test. Output: {}",
            stdout
        );
        assert!(output.status.success(), "Subprocess failed: {:?}", output);
    }

    #[tokio::test]
    async fn test_run_interactive_happy_path_empty_inputs() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("kaji.toml");

        let ui = MockUi {
            inputs: Mutex::new(vec![
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
            ]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(true, Some(output_path.clone()), &ui).await.unwrap();

        assert!(output_path.exists());
        let content = tokio::fs::read_to_string(&output_path).await.unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert!(config.server.is_none());
        assert!(config.realms.is_none());
        assert!(config.user.is_none());
        assert!(config.client_id.is_none());
        assert!(config.profile.is_none());
        assert!(config.vault_addr.is_none());
        assert!(config.vault_token.is_none());
        assert!(config.workspace.is_none());
    }

    #[tokio::test]
    async fn test_run_interactive_overwrite_yes() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("kaji.toml");
        tokio::fs::write(&output_path, "old config content")
            .await
            .unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![
                "http://localhost".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
                "".to_string(),
            ]),
            confirms: Mutex::new(vec![true]), // overwrite = true
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(true, Some(output_path.clone()), &ui).await.unwrap();

        assert!(output_path.exists());
        let content = tokio::fs::read_to_string(&output_path).await.unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.server, Some("http://localhost".to_string()));
    }

    #[tokio::test]
    async fn test_run_interactive_overwrite_no() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("kaji.toml");
        tokio::fs::write(&output_path, "old config content")
            .await
            .unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![false]), // overwrite = false
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        let result = run(true, Some(output_path.clone()), &ui).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Initialization aborted to prevent overwrite.")
        );

        let content = tokio::fs::read_to_string(&output_path).await.unwrap();
        assert_eq!(content, "old config content");
    }

    #[tokio::test]
    async fn test_run_interactive_default_path() {
        let dir = tempdir().unwrap();
        // Since the function will prompt for path, let's run from the tempdir context or mock the UI path prompt
        let expected_path = dir.path().join("kaji.toml");

        let ui = MockUi {
            inputs: Mutex::new(vec![
                expected_path.to_string_lossy().to_string(), // Output file path
                "http://myhost".to_string(),                 // server
                "r1".to_string(),                            // realms
                "".to_string(),                              // user
                "".to_string(),                              // client_id
                "".to_string(),                              // profile
                "".to_string(),                              // vault_addr
                "".to_string(),                              // vault_token
                "".to_string(),                              // workspace
            ]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(true, None, &ui).await.unwrap();

        assert!(expected_path.exists());
        let content = tokio::fs::read_to_string(&expected_path).await.unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.server, Some("http://myhost".to_string()));
        assert_eq!(config.realms, Some(vec!["r1".to_string()]));
    }

    #[tokio::test]
    async fn test_run_non_interactive_default_path() {
        let _lock = INIT_TEST_MUTEX.lock().await;
        let dir = tempdir().unwrap();
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(false, None, &ui).await.unwrap();

        let expected_file = dir.path().join("kaji.toml");
        assert!(expected_file.exists());

        std::env::set_current_dir(original_cwd).unwrap();
    }

    #[tokio::test]
    async fn test_run_non_interactive_write_error() {
        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        let invalid_path = PathBuf::from("/invalid_dir_path_that_does_not_exist/kaji.toml");
        let result = run(false, Some(invalid_path), &ui).await;
        assert!(result.is_err());
    }
}
