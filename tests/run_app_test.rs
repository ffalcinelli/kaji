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
        command: Commands::Validate { workspace },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: None,
        profile: None,
        vault_addr: None,
        vault_token: None,
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
            workspace,
            yes: true,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
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
            workspace,
            yes: true,
            review: false,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
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
            workspace,
            changes_only: false,
            interactive: false,
        },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
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
            workspace,
            yes: true,
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: None,
        profile: None,
        vault_addr: None,
        vault_token: None,
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
        command: Commands::Drift { workspace },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: "admin-cli".to_string(),
        client_secret: Some("secret".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: None,
    };

    run_app(cli).await?;
    Ok(())
}
