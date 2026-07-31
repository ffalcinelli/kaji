use std::sync::Arc;
mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::RealmRepresentation;
use kaji::{apply, clean, inspect, plan};
use std::fs;
use tempfile::tempdir;

use kaji::utils::ui::MockUi;

#[tokio::test]
async fn test_plan_edge_cases() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    // 1. Test run with non-existent directory
    let res = plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.join("non-existent"),
        changes_only: false,
        interactive: false,
        realms_to_plan: &[],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await;
    assert!(res.is_err());

    // 2. Test run with empty directory (no realms)
    let res = plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &[],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await;
    assert!(res.is_ok());

    // 3. Test with .secrets file
    fs::write(workspace_dir.join(".secrets"), "TEST_VAR=test_value").unwrap();

    let realm_dir = workspace_dir.join("new-realm");
    fs::create_dir(&realm_dir).unwrap();
    let realm = RealmRepresentation {
        realm: "new-realm".to_string(),
        enabled: Some(true),
        display_name: Some("New Realm".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // 4. Test auto-discovery of realms
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &[],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 5. Test plan_realm with 404 (realm doesn't exist on server)
    // "new-realm" should return 404 in my mock server
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &["new-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 6. Test with invalid YAML
    fs::write(realm_dir.join("invalid.yaml"), "invalid: [yaml").unwrap();
    let _res = plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &["new-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await;

    let roles_dir = realm_dir.join("roles");
    fs::create_dir_all(&roles_dir).unwrap();
    fs::write(roles_dir.join("invalid.yaml"), "invalid: [yaml").unwrap();
    let res = plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &["new-realm".to_string()],
        ui: ui.clone(),
        resolver,
        profile: None,
    })
    .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_apply_edge_cases() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    // 1. Test run with non-existent directory
    let res = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.join("non-existent"),
        realms_to_apply: &[],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await;
    assert!(res.is_err());

    // 2. Test run with empty directory (no realms)
    let res = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &[],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await;
    assert!(res.is_ok());

    // 3. Test with .secrets file
    fs::write(workspace_dir.join(".secrets"), "TEST_VAR=test_value").unwrap();

    let realm_dir = workspace_dir.join("test-realm");
    fs::create_dir(&realm_dir).unwrap();
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Test Realm".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // 4. Test auto-discovery of realms
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &[],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 5. Test with empty .kajiplan
    fs::write(workspace_dir.join(".kajiplan"), "[]").unwrap();
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 6. Test with invalid YAML in roles dir
    let roles_dir = realm_dir.join("roles");
    fs::create_dir_all(&roles_dir).unwrap();
    fs::write(roles_dir.join("invalid.yaml"), "invalid: [yaml").unwrap();
    let res = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: true,
        review: false,
        prune: false,
        ui: ui,
        resolver: resolver,
        profile: None,
    })
    .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_check_keys_drift() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

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

    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir,
        changes_only: true,
        interactive: false,
        realms_to_plan: &["test-realm".to_string()],
        ui,
        resolver,
        profile: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_inspect_edge_cases() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // 1. Test auto-discovery of realms
    inspect::run(&client, workspace_dir.clone(), &[], true)
        .await
        .unwrap();
    assert!(workspace_dir.join("test-realm").exists());

    // 2. Test with existing .secrets file (to cover appending)
    fs::write(workspace_dir.join(".secrets"), "OLD_VAR=old_value\n").unwrap();
    inspect::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
    )
    .await
    .unwrap();
    let secrets = fs::read_to_string(workspace_dir.join(".secrets")).unwrap();
    assert!(secrets.contains("OLD_VAR=old_value"));

    // 3. Test overwrite with same content (covers early return)
    inspect::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
    )
    .await
    .unwrap();

    // 4. Test failure cases - e.g. invalid server
    let bad_client = KeycloakClient::new("http://invalid".to_string());
    let res = inspect::run(
        &bad_client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
    )
    .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_clean_edge_cases() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let ui = MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    };

    // 1. Test run with non-existent directory
    let res = clean::run(workspace_dir.join("non-existent"), true, &[], &ui).await;
    assert!(res.is_ok());

    // 2. Test run with realms that don't exist
    let res = clean::run(
        workspace_dir.clone(),
        true,
        &["non-existent".to_string()],
        &ui,
    )
    .await;
    assert!(res.is_ok());

    // 3. Test cleaning a file instead of a directory
    fs::create_dir_all(&workspace_dir).unwrap();
    let file_path = workspace_dir.join("some_file.txt");
    fs::write(&file_path, "test").unwrap();
    clean::run(
        workspace_dir.clone(),
        true,
        &["some_file.txt".to_string()],
        &ui,
    )
    .await
    .unwrap();
    assert!(!file_path.exists());
}

#[tokio::test]
async fn test_validate_edge_cases() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // 1. Test run with non-existent directory
    let res = kaji::validate::run(workspace_dir.join("non-existent"), &[]).await;
    assert!(res.is_err());

    // 2. Test run with empty directory (no realms)
    fs::create_dir_all(&workspace_dir).unwrap();
    let res = kaji::validate::run(workspace_dir.clone(), &[]).await;
    assert!(res.is_ok());

    // 3. Test auto-discovery of realms for validation
    let realm_dir = workspace_dir.join("test-realm");
    fs::create_dir(&realm_dir).unwrap();
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Test Realm".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    kaji::validate::run(workspace_dir.clone(), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn test_inspect_edge_cases_2() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock".to_string());

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // 1. Test failure to get realms (500)
    let mut server = mockito::Server::new_async().await;
    let mut bad_client = KeycloakClient::new(server.url());
    bad_client.set_token("mock".to_string());
    let _m = server
        .mock("GET", "/admin/realms")
        .with_status(500)
        .create_async()
        .await;

    let res = inspect::run(&bad_client, workspace_dir.clone(), &[], true).await;
    assert!(res.is_err());
}
