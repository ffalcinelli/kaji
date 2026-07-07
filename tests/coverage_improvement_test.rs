mod common;
use common::start_mock_server;
use kcd::apply;
use kcd::client::KeycloakClient;
use kcd::models::*;
use kcd::utils::secrets::{EnvResolver, SecretResolver};
use kcd::utils::ui::MockUi;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_coverage_gaps_apply_generic() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    fs::create_dir_all(realm_dir.join("roles")).unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![
            true,  // Accept sending everything because plan is empty
            false, // Reject applying r1
            true,  // Accept applying r2
        ]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    // Create some roles
    fs::write(realm_dir.join("roles/r1.yaml"), "name: r1\n").unwrap();
    fs::write(realm_dir.join("roles/r2.yaml"), "name: r2\n").unwrap();
    fs::write(realm_dir.join("roles/r1.prod.yaml"), "name: r1-prod\n").unwrap(); // Overlay

    // 1. Test review mode rejection and overlay skipping
    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        false, // yes = false
        true,  // review = true
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // 2. Test planned_files exclusion
    let plan_file = workspace_dir.join(".kcdplan");
    // We want to apply only r1, so r2 should be skipped (hitting DA:68)
    let planned_files = vec![realm_dir.join("roles/r1.yaml")];
    fs::write(&plan_file, serde_json::to_string(&planned_files).unwrap()).unwrap();

    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,  // yes = true
        false, // review = false
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // 3. Test skipping non-yaml files (hitting DA:71)
    fs::write(realm_dir.join("roles/not-yaml.txt"), "some text").unwrap();
    // Also test overlay skip again (hitting DA:75)
    fs::write(realm_dir.join("roles/other.prod.yaml"), "name: other").unwrap();

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
}

#[tokio::test]
async fn test_coverage_gaps_apply_mod_errors() {
    let mock_url = "http://invalid-url";
    let client = KeycloakClient::new(mock_url.to_string());
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![false]), // Reject sending everything
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    // 1. No planned changes, user says NO
    apply::run(
        &client,
        workspace_dir.clone(),
        &["some-realm".to_string()],
        false,
        false,
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();

    // 2. Non-existent workspace
    let res = apply::run(
        &client,
        PathBuf::from("/non/existent/path"),
        &[],
        true,
        false,
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await;
    assert!(res.is_err());

    // 3. No realms found
    let empty_dir = tempdir().unwrap();
    apply::run(
        &client,
        empty_dir.path().to_path_buf(),
        &[],
        true,
        false,
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .unwrap();
}

#[test]
fn test_models_debug_obfuscation() {
    let mut config = HashMap::new();
    config.insert("clientSecret".to_string(), "sensitive".to_string());
    config.insert("other".to_string(), "public".to_string());

    let idp = IdentityProviderRepresentation {
        internal_id: None,
        alias: Some("google".to_string()),
        provider_id: Some("google".to_string()),
        enabled: Some(true),
        update_profile_first_login_mode: None,
        trust_email: None,
        store_token: None,
        add_read_token_role_on_create: None,
        authenticate_by_default: None,
        link_only: None,
        first_broker_login_flow_alias: None,
        post_broker_login_flow_alias: None,
        display_name: None,
        config: Some(config),
        extra: HashMap::new(),
    };

    let debug_str = format!("{:?}", idp);
    assert!(debug_str.contains("********"));
    assert!(!debug_str.contains("public")); // Config is completely redacted now
    assert!(!debug_str.contains("sensitive"));

    let cred = CredentialRepresentation {
        id: Some("id".to_string()),
        type_: Some("password".to_string()),
        value: Some("mypassword".to_string()),
        temporary: Some(false),
        extra: HashMap::new(),
    };
    let debug_cred = format!("{:?}", cred);
    assert!(debug_cred.contains("********"));
    assert!(!debug_cred.contains("mypassword"));

    let mut comp_config = HashMap::new();
    comp_config.insert("secret".to_string(), serde_json::json!("sensitive"));
    let comp = ComponentRepresentation {
        id: Some("id".to_string()),
        name: Some("comp".to_string()),
        provider_id: Some("p".to_string()),
        provider_type: Some("t".to_string()),
        parent_id: None,
        sub_type: None,
        config: Some(comp_config),
        extra: HashMap::new(),
    };
    let debug_comp = format!("{:?}", comp);
    assert!(debug_comp.contains("********"));
    assert!(!debug_comp.contains("sensitive"));

    // Test obfuscate_config with None config
    let mut idp_no_config = idp.clone();
    idp_no_config.config = None;
    assert!(format!("{:?}", idp_no_config).contains("config: None"));
}

#[test]
fn test_models_extra_methods() {
    let group = GroupRepresentation {
        id: Some("id4".to_string()),
        name: Some("gname".to_string()),
        path: None,
        sub_groups: None,
        extra: HashMap::new(),
    };
    assert_eq!(group.get_filename(), "gname-id4");

    let comp = ComponentRepresentation {
        id: Some("id8".to_string()),
        name: Some("cname".to_string()),
        provider_id: Some("p3".to_string()),
        provider_type: Some("t1".to_string()),
        parent_id: None,
        sub_type: None,
        config: None,
        extra: HashMap::new(),
    };
    assert_eq!(comp.get_filename(), "cname-id8");

    assert_eq!(
        RoleRepresentation::object_path("role1"),
        "roles-by-id/role1"
    );

    // Test UserRepresentation identity with id only
    let user_id_only = UserRepresentation {
        id: Some("id5".to_string()),
        username: None,
        enabled: None,
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: HashMap::new(),
    };
    assert_eq!(user_id_only.get_identity(), Some("id5".to_string()));
    assert_eq!(user_id_only.get_name(), "unknown".to_string());

    // Test GroupRepresentation identity fallback
    let group_name_only = GroupRepresentation {
        id: None,
        name: Some("gname".to_string()),
        path: None,
        sub_groups: None,
        extra: HashMap::new(),
    };
    assert_eq!(group_name_only.get_identity(), Some("gname".to_string()));
}

#[tokio::test]
async fn test_apply_components_gaps() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let components_dir = workspace_dir.join("test-realm/components");
    fs::create_dir_all(&components_dir).unwrap();

    // Test with both id and name missing (triggers DA:53 in models.rs macro?)
    // Actually, ComponentRepresentation get_identity uses id.or_else(|| name).
    fs::write(components_dir.join("invalid.yaml"), "providerId: some-p\n").unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let planned_files = Arc::new(None);

    let _ = apply::components::apply_components_or_keys(
        &client,
        &workspace_dir.join("test-realm"),
        "components",
        resolver,
        planned_files,
        "test-realm",
        None,
    )
    .await;
}
