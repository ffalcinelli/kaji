use std::sync::Arc;
mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::RealmRepresentation;
use kaji::plan;
use std::fs;
use tempfile::tempdir;

use kaji::utils::ui::MockUi;

#[tokio::test]
async fn test_plan_extended_scenarios() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    fs::create_dir(&realm_dir).unwrap();

    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    // scenario: mismatching realm name in realm.yaml
    let realm = RealmRepresentation {
        realm: "different-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Test Realm".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // This should work because plan::run uses directory names as realm names,
    // and realm::plan_realm just compares local realm.yaml with remote realm.
    plan::run(
        &client,
        workspace_dir.clone(),
        false,
        false,
        &["test-realm".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // scenario: .kajiplan exists but is empty
    fs::write(workspace_dir.join(".kajiplan"), "[]").unwrap();
    plan::run(
        &client,
        workspace_dir.clone(),
        true,
        false,
        &["test-realm".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // scenario: .kajiplan exists with non-existent files
    fs::write(
        workspace_dir.join(".kajiplan"),
        "[\"test-realm/non-existent.yaml\"]",
    )
    .unwrap();
    plan::run(
        &client,
        workspace_dir.clone(),
        false,
        false,
        &["test-realm".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // scenario: run for a specific realm that doesn't have a directory
    let res = plan::run(
        &client,
        workspace_dir.clone(),
        false,
        false,
        &["no-dir-realm".to_string()],
        ui.clone(),
        resolver,
        None,
    )
    .await;
    assert!(res.is_ok());
}
