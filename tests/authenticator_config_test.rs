mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::{apply, inspect, plan};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_authenticator_config_inspect_plan_apply() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // 1. Run inspect
    inspect::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
    )
    .await
    .expect("Inspect failed");

    let realm_dir = workspace_dir.join("test-realm");
    assert!(realm_dir.exists());

    // Verify authenticator config was inspected and exported
    let config_dir = realm_dir.join("authenticator-configs");
    assert!(config_dir.exists());
    let mut exported_configs = fs::read_dir(&config_dir).unwrap();
    let config_entry = exported_configs.next().unwrap().unwrap();
    let config_path = config_entry.path();
    assert!(
        config_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("review profile config")
    );

    // Verify flow was exported with the config alias instead of UUID
    let flows_dir = realm_dir.join("authentication-flows");
    assert!(flows_dir.exists());
    let flow_path = flows_dir.join("flow-1.yaml");
    assert!(flow_path.exists());
    let flow_content = fs::read_to_string(&flow_path).unwrap();
    assert!(flow_content.contains("authenticatorConfig: review profile config"));

    // 2. Run plan (should show no changes)
    let secrets_file_path = workspace_dir.join(".secrets");
    let mut vars = std::collections::HashMap::new();
    if secrets_file_path.exists() {
        if let Ok(lines) = fs::read_to_string(&secrets_file_path) {
            for line in lines.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let val = v.trim_matches('"').trim_matches('\'').to_string();
                    vars.insert(k.to_string(), val);
                }
            }
        }
    }
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(vars))
        as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = Arc::new(kaji::utils::ui::DialoguerUi::new());

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
    .expect("Plan failed");

    // 3. Run apply
    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
        false,
        ui,
        resolver,
        None,
    )
    .await
    .expect("Apply failed");
}
