use std::sync::Arc;
mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::RealmRepresentation;
use kaji::{apply, plan};
use std::fs;
use tempfile::tempdir;

use kaji::utils::ui::MockUi;

#[tokio::test]
async fn test_ultimate_flow() {
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

    // 1. Initial plan - should have changes (creation)
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("New Realm".to_string()), // Different from mock
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
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

    // Verify .kajiplan exists
    assert!(workspace_dir.join(".kajiplan").exists());

    // 2. Apply
    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
        false,
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // Verify .kajiplan is gone
    assert!(!workspace_dir.join(".kajiplan").exists());

    // 3. Plan again - should have no changes (matches)
    // Wait, my mock server doesn't actually store state,
    // so it will still show changes unless I match the mock exactly.
    let realm_matches = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Test Realm".to_string()), // Matches mock
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm_matches).unwrap(),
    )
    .unwrap();

    plan::run(
        &client,
        workspace_dir.clone(),
        true, // changes_only
        false,
        &["test-realm".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // Verify .kajiplan does not exist
    assert!(!workspace_dir.join(".kajiplan").exists());

    // 4. Interactive plan - say 'no' to changes
    let realm_mismatch = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Mismatch".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm_mismatch).unwrap(),
    )
    .unwrap();

    ui.confirms.lock().unwrap().push(false); // Say 'no' to including change in plan

    plan::run(
        &client,
        workspace_dir.clone(),
        false,
        true, // interactive
        &["test-realm".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // Verify .kajiplan does not exist (rejected)
    assert!(!workspace_dir.join(".kajiplan").exists());
}
