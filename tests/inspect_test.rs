mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::inspect;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_inspect() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    inspect::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
    )
    .await
    .expect("Inspect failed");

    assert!(
        workspace_dir.join("test-realm").join("realm.yaml").exists(),
        "realm.yaml missing"
    );
    assert!(
        workspace_dir
            .join("test-realm")
            .join("clients/client-1.yaml")
            .exists(),
        "client-1.yaml missing"
    );
    assert!(
        workspace_dir
            .join("test-realm")
            .join("roles/role-1.yaml")
            .exists(),
        "role-1.yaml missing"
    );

    // Check content
    let realm_content =
        fs::read_to_string(workspace_dir.join("test-realm").join("realm.yaml")).unwrap();
    assert!(realm_content.contains("test-realm"));
}

#[tokio::test]
async fn test_inspect_auto_discovery() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("master".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // Passing empty list should discover all realms
    inspect::run(&client, workspace_dir.clone(), &[], true)
        .await
        .expect("Inspect failed");

    // The mock server returns "master" and "test-realm"
    assert!(
        workspace_dir.join("master").exists(),
        "master realm directory missing"
    );
    assert!(
        workspace_dir.join("test-realm").exists(),
        "test-realm realm directory missing"
    );
}

#[test]
fn test_inspect_aliases() {
    use clap::Parser;
    use kaji::args::{Cli, Commands};

    let args_sync = Cli::parse_from(["kaji", "sync"]);
    assert!(matches!(args_sync.command, Commands::Inspect { .. }));

    let args_pull = Cli::parse_from(["kaji", "pull"]);
    assert!(matches!(args_pull.command, Commands::Inspect { .. }));

    let args_export = Cli::parse_from(["kaji", "export"]);
    assert!(matches!(args_export.command, Commands::Inspect { .. }));
}
