mod common;
use anyhow::Result;
use kaji::args::{Cli, Commands};
use kaji::run_app;
use tempfile::tempdir;

#[tokio::test]
async fn test_run_app_validate() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Validate {
            workspace: Some(workspace),
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_inspect() -> Result<()> {
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Inspect {
            workspace: Some(workspace),
            yes: true,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_apply() -> Result<()> {
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let realm_dir = workspace.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();
    std::fs::write(realm_dir.join("realm.yaml"), "realm: test-realm\n").unwrap();

    let cli = Cli {
        command: Commands::Apply {
            workspace: Some(workspace),
            yes: true,
            review: false,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_plan() -> Result<()> {
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Plan {
            workspace: Some(workspace),
            changes_only: false,
            interactive: false,
        },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_clean() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Clean {
            workspace: Some(workspace),
            yes: true,
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_drift() -> Result<()> {
    // We need a mock server for drift because it calls init_client
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Drift {
            workspace: Some(workspace),
        },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_with_config_toml() -> Result<()> {
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    // Write a custom config toml file
    let config_path = dir.path().join("my_config.toml");
    let toml_content = format!(
        r#"
workspace = {:?}
server = "http://localhost:8080"
realms = ["master"]
client_id = "toml-client-id"
"#,
        workspace.to_string_lossy()
    );
    std::fs::write(&config_path, toml_content)?;

    // We pass None for workspace, server, and client_id,
    // and let them resolve from the configuration file.
    let cli = Cli {
        command: Commands::Clean {
            workspace: None,
            yes: true,
        },
        server: None,
        realms: vec![],
        user: None,
        password: None,
        client_id: None,
        client_secret: None,
        profile: None,
        vault_addr: None,
        vault_token: None,
        config: Some(config_path),
    };

    run_app(cli).await?;
    Ok(())
}
