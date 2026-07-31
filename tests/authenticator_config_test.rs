mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::AuthenticatorConfigRepresentation;
use kaji::{apply, inspect, plan, validate};
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
    if secrets_file_path.exists()
        && let Ok(lines) = fs::read_to_string(&secrets_file_path)
    {
        for line in lines.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let val = v.trim_matches('"').trim_matches('\'').to_string();
                vars.insert(k.to_string(), val);
            }
        }
    }
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(vars))
        as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = Arc::new(kaji::utils::ui::DialoguerUi::new());

    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: true,
        interactive: false,
        realms_to_plan: &["test-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .expect("Plan failed");

    // 3. Modify the existing config file (Update scenario)
    let config_file_path = config_path;
    let mut config_val: serde_json::Value =
        serde_yaml::from_str(&fs::read_to_string(&config_file_path).unwrap()).unwrap();
    if let Some(obj) = config_val.as_object_mut()
        && let Some(config_map) = obj.get_mut("config").and_then(|c| c.as_object_mut())
    {
        config_map.insert("loa-condition-level".to_string(), serde_json::json!("5"));
    }
    fs::write(
        &config_file_path,
        serde_yaml::to_string(&config_val).unwrap(),
    )
    .unwrap();

    // 4. Create a brand new config file (Create scenario)
    let new_config = AuthenticatorConfigRepresentation {
        id: None,
        alias: Some("new config".to_string()),
        config: Some({
            let mut map = std::collections::HashMap::new();
            map.insert("some-key".to_string(), serde_json::json!("some-value"));
            map
        }),
        extra: std::collections::HashMap::new(),
    };
    let new_config_path = config_dir.join("new-config.yaml");
    fs::write(
        &new_config_path,
        serde_yaml::to_string(&new_config).unwrap(),
    )
    .unwrap();

    // Update the local flow to reference the new config in the second execution
    let mut flow_val: serde_json::Value = serde_yaml::from_str(&flow_content).unwrap();
    if let Some(obj) = flow_val.as_object_mut()
        && let Some(execs) = obj
            .get_mut("authenticationExecutions")
            .and_then(|e| e.as_array_mut())
    {
        for exec in execs {
            if exec.get("authenticator").and_then(|a| a.as_str()) == Some("another-authenticator")
                && let Some(exec_obj) = exec.as_object_mut()
            {
                exec_obj.insert(
                    "authenticatorConfig".to_string(),
                    serde_json::json!("new config"),
                );
            }
        }
    }
    fs::write(&flow_path, serde_yaml::to_string(&flow_val).unwrap()).unwrap();

    // 5. Run validate (should pass offline validation)
    validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .expect("Validation failed");

    // 6. Run plan again (should detect the changes: 1 updated, 1 created)
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: true,
        interactive: false,
        realms_to_plan: &["test-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .expect("Plan after modifications failed");

    // 7. Run apply (should execute PUT and POST for config updates and creations)
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
    .expect("Apply modifications failed");
}

#[tokio::test]
async fn test_authenticator_config_validation_failures() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    fs::create_dir_all(&realm_dir).unwrap();

    // Create a realm.yaml so validate doesn't bail early
    fs::write(
        realm_dir.join("realm.yaml"),
        "realm: test-realm\nenabled: true\n",
    )
    .unwrap();

    let config_dir = realm_dir.join("authenticator-configs");
    fs::create_dir_all(&config_dir).unwrap();

    // Create a config missing the "alias" field
    fs::write(
        config_dir.join("invalid-config.yaml"),
        "config:\n  key: value\n",
    )
    .unwrap();

    // Validation should fail
    let res = validate::run(workspace_dir, &["test-realm".to_string()]).await;
    assert!(res.is_err());
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("alias is missing or empty"));
}
