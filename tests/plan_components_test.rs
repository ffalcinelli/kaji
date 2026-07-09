mod common;
use kaji::client::KeycloakClient;
use kaji::plan::components::{check_keys_drift, plan_components_or_keys};
use kaji::plan::{PlanContext, PlanOptions};
use kaji::utils::ui::DialoguerUi;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::fs;

#[tokio::test]
async fn test_plan_components_no_dir() {
    let client = KeycloakClient::new("http://localhost:8080".to_string());
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path();
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = DialoguerUi::new();

    let options = PlanOptions {
        changes_only: false,
        interactive: false,
    };

    let ctx = PlanContext {
        client: &client,
        workspace_dir,
        options,
        resolver,
        realm_name: "master",
        ui: &ui,
        profile: None,
    };

    // Should not fail if directory doesn't exist
    let res = plan_components_or_keys(&ctx, "non-existent").await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_check_keys_drift_fail() {
    // Client that will fail to connect
    let client = KeycloakClient::new("http://localhost:1".to_string());
    let options = PlanOptions {
        changes_only: true,
        interactive: false,
    };
    let res = check_keys_drift(&client, options, "master").await;
    // check_keys_drift ignores error if not available
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_plan_components_with_invalid_yaml() {
    let client = KeycloakClient::new("http://localhost:8080".to_string());
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path();
    let components_dir = workspace_dir.join("components");
    fs::create_dir_all(&components_dir).await.unwrap();
    fs::write(components_dir.join("bad.yaml"), "invalid: [ :")
        .await
        .unwrap();

    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = DialoguerUi::new();

    let options = PlanOptions {
        changes_only: false,
        interactive: false,
    };

    let ctx = PlanContext {
        client: &client,
        workspace_dir,
        options,
        resolver,
        realm_name: "master",
        ui: &ui,
        profile: None,
    };

    let res = plan_components_or_keys(&ctx, "components").await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_check_keys_drift_warning() {
    let mock_url = common::start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    let options = PlanOptions {
        changes_only: true,
        interactive: false,
    };

    // This should run and print a warning (we can't easily assert on stdout here without more effort,
    // but we can ensure it doesn't crash and hits the logic)
    let res = check_keys_drift(&client, options, "test-realm").await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_plan_components_no_identity() {
    let client = KeycloakClient::new("http://localhost:8080".to_string());
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path();
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = DialoguerUi::new();

    let options = PlanOptions {
        changes_only: false,
        interactive: false,
    };

    let ctx = PlanContext {
        client: &client,
        workspace_dir,
        options,
        resolver,
        realm_name: "master",
        ui: &ui,
        profile: None,
    };

    let components_dir = workspace_dir.join("components");
    fs::create_dir_all(&components_dir).await.unwrap();
    // Component with NO name and NO id (missing both)
    fs::write(components_dir.join("empty.yaml"), "providerId: ldap\n")
        .await
        .unwrap();

    let res = plan_components_or_keys(&ctx, "components").await;
    // It should fail to get identity
    assert!(res.is_err());
}
