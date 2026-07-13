mod common;
use anyhow::Result;
use kaji::args::{Cli, Commands};
use kaji::{init_client, init_secrets, load_profile};
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_profile_loading() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path();
    let profiles_dir = workspace.join("profiles");
    fs::create_dir(&profiles_dir)?;

    let profile_content = r#"
server_url: "https://mock.prod.example.com"
client_id: "prod-cli"
client_secret: "prod-secret"
user: "prod-admin"
password: "prod-password"
secrets_file: ".secrets.prod"
"#;
    fs::write(profiles_dir.join("prod.yaml"), profile_content)?;

    // Test load_profile
    let profile = load_profile(workspace, "prod").await?;
    assert_eq!(profile.server_url, "https://mock.prod.example.com");
    assert_eq!(profile.client_id, Some("prod-cli".to_string()));
    assert_eq!(profile.user, Some("prod-admin".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_init_client_with_profile() -> Result<()> {
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path();
    let profiles_dir = workspace.join("profiles");
    fs::create_dir(&profiles_dir)?;

    let profile_content = format!(
        r#"
server_url: "{}"
client_id: "admin-cli"
client_secret: "secret"
"#,
        mock_url
    );
    fs::write(profiles_dir.join("test.yaml"), profile_content)?;

    let cli = Cli {
        command: Commands::Drift {
            workspace: Some(workspace.to_path_buf()),
            verbose: false,
        },
        server: None, // Required unless profile is present
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("ignored".to_string()),
        client_secret: None,
        profile: Some("test".to_string()),
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let profile = load_profile(workspace, "test").await?;
    let client = init_client(&cli, Some(&profile)).await?;
    assert_eq!(client.get_base_url(), mock_url);

    Ok(())
}

#[tokio::test]
async fn test_init_secrets_with_profile() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create profile-specific secrets file
    fs::write(
        workspace.join(".secrets.prod"),
        "KEYCLOAK_PROD_API_KEY=supersecret",
    )?;

    let profile = kaji::Profile {
        server_url: "http://localhost:8080".to_string(),
        client_id: None,
        client_secret: None,
        user: None,
        password: None,
        secrets_file: Some(".secrets.prod".to_string()),
        vault_addr: None,
        vault_token: None,
    };

    let cli = Cli {
        command: Commands::Validate {
            workspace: Some(workspace.to_path_buf()),
        },
        server: None,
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: Some("prod".to_string()),
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let resolver = init_secrets(&cli, workspace, Some(&profile)).await?;
    let resolved = resolver.resolve("KEYCLOAK_PROD_API_KEY").await?;
    assert_eq!(resolved, Some("supersecret".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_init_secrets_with_vault() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    let profile = kaji::Profile {
        server_url: "http://localhost:8080".to_string(),
        client_id: None,
        client_secret: None,
        user: None,
        password: None,
        secrets_file: None,
        vault_addr: Some("http://127.0.0.1:8200".to_string()),
        vault_token: Some("root-token".to_string()),
    };

    let cli = Cli {
        command: Commands::Validate {
            workspace: Some(workspace.to_path_buf()),
        },
        server: None,
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: Some("prod".to_string()),
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let resolver = init_secrets(&cli, workspace, Some(&profile)).await?;
    assert!(resolver.resolve("ANY_VAR").await.is_ok());

    Ok(())
}
