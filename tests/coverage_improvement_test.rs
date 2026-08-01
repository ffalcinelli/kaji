mod common;
use common::start_mock_server;
use kaji::apply;
use kaji::client::KeycloakClient;
use kaji::models::*;
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::MockUi;
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
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: false,
        review: // yes = false
        true,
        prune: false,
        ui: // review = true
        ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 2. Test planned_files exclusion
    let plan_file = workspace_dir.join(".kajiplan");
    // We want to apply only r1, so r2 should be skipped (hitting DA:68)
    let planned_files = vec![realm_dir.join("roles/r1.yaml")];
    fs::write(&plan_file, serde_json::to_string(&planned_files).unwrap()).unwrap();

    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: true,
        review: // yes = true
        false,
        prune: false,
        ui: // review = false
        ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 3. Test skipping non-yaml files (hitting DA:71)
    fs::write(realm_dir.join("roles/not-yaml.txt"), "some text").unwrap();
    // Also test overlay skip again (hitting DA:75)
    fs::write(realm_dir.join("roles/other.prod.yaml"), "name: other").unwrap();

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
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["some-realm".to_string()],
        yes: false,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .unwrap();

    // 2. Non-existent workspace
    let res = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: PathBuf::from("/non/existent/path"),
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

    // 3. No realms found
    let empty_dir = tempdir().unwrap();
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: empty_dir.path().to_path_buf(),
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

    let user_cred = CredentialRepresentation {
        id: None,
        type_: Some("password".to_string()),
        value: Some("super_secret_password".to_string()),
        temporary: Some(false),
        extra: HashMap::new(),
    };
    let user = UserRepresentation {
        id: Some("id".to_string()),
        username: Some("testuser".to_string()),
        enabled: Some(true),
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: Some(vec![user_cred]),
        extra: HashMap::new(),
    };
    let debug_user = format!("{:?}", user);
    assert!(debug_user.contains("********"));
    assert!(!debug_user.contains("super_secret_password"));
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
    let secrets_path = Arc::new(workspace_dir.join(".secrets"));
    let ui = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    let _ = apply::components::apply_components_or_keys(
        kaji::apply::ApplyContext {
            client: &client,
            workspace_dir: workspace_dir.join("test-realm"),
            secrets_path: secrets_path,
            resolver: resolver,
            planned_files: planned_files,
            realm_name: "test-realm",
            profile: None,
            review: false,
            ui: ui,
            yes: true,
            prune: false,
        },
        "components",
    )
    .await;
}

#[tokio::test]
async fn test_apply_secrets_file_loading_coverage() {
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
    fs::create_dir_all(&realm_dir).unwrap();

    fs::write(realm_dir.join("realm.yaml"), "realm: test-realm\n").unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![true]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let _ = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: Some("non_existent".to_string()),
    })
    .await;

    let profiles_dir = workspace_dir.join("profiles");
    fs::create_dir(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("no_secrets.yaml"),
        "server_url: http://dummy\nclient_id: foo\n",
    )
    .unwrap();

    let _ = apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test-realm".to_string()],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: Some("no_secrets".to_string()),
    })
    .await;
}

#[tokio::test]
async fn test_apply_components_enrichment() {
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
    let components_dir = realm_dir.join("components");
    fs::create_dir_all(&components_dir).unwrap();

    let comp = ComponentRepresentation {
        id: Some("c1".to_string()),
        name: Some("component-1".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        parent_id: Some("test-realm".to_string()),
        sub_type: None,
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };
    fs::write(
        components_dir.join("comp-1.yaml"),
        serde_yaml::to_string(&comp).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let planned_files = Arc::new(None);
    let secrets_path = Arc::new(workspace_dir.join(".secrets"));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![true]),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    let _ = apply::components::apply_components_or_keys(
        kaji::apply::ApplyContext {
            client: &client,
            workspace_dir: realm_dir.clone(),
            secrets_path: secrets_path.clone(),
            resolver: resolver.clone(),
            planned_files: planned_files.clone(),
            realm_name: "test-realm",
            profile: None,
            review: false,
            ui: ui.clone(),
            yes: false,
            prune: false,
        },
        "components",
    )
    .await;

    let comp_no_id = ComponentRepresentation {
        id: None,
        name: Some("comp-2".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        parent_id: Some("test-realm".to_string()),
        sub_type: None,
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };
    fs::write(
        components_dir.join("comp-2.yaml"),
        serde_yaml::to_string(&comp_no_id).unwrap(),
    )
    .unwrap();

    let ui2 = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![true]),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    let _ = apply::components::apply_components_or_keys(
        kaji::apply::ApplyContext {
            client: &client,
            workspace_dir: realm_dir.clone(),
            secrets_path: secrets_path,
            resolver: resolver,
            planned_files: planned_files,
            realm_name: "test-realm",
            profile: None,
            review: false,
            ui: ui2,
            yes: false,
            prune: false,
        },
        "components",
    )
    .await;
}

#[test]
fn test_print_diff_and_interactive_prompt() {
    use kaji::models::RoleRepresentation;
    use kaji::plan::print_diff;

    let old_role = RoleRepresentation {
        id: Some("r1-id".to_string()),
        name: "r1".to_string(),
        description: Some("old desc".to_string()),
        container_id: None,
        composite: false,
        client_role: false,
        extra: HashMap::new(),
    };

    let new_role = RoleRepresentation {
        id: Some("r1-id".to_string()),
        name: "r1".to_string(),
        description: Some(
            "new desc which has longer text and more lines to test hunk printing".to_string(),
        ),
        container_id: None,
        composite: false,
        client_role: false,
        extra: HashMap::new(),
    };

    let res = print_diff(
        "role",
        Some(&old_role),
        &new_role,
        false,
        true,
        "role_prefix",
    )
    .unwrap();
    assert!(res);

    let res2 = print_diff(
        "role",
        Some(&old_role),
        &new_role,
        false,
        false,
        "role_prefix",
    )
    .unwrap();
    assert!(res2);

    let mut old_map = std::collections::BTreeMap::new();
    for i in 1..=15 {
        old_map.insert(format!("f{:02}", i), format!("v{}", i));
    }
    let mut new_map = old_map.clone();
    new_map.insert("f01".to_string(), "changed_v1".to_string());
    new_map.insert("f15".to_string(), "changed_v15".to_string());

    let res3 = print_diff("map", Some(&old_map), &new_map, false, false, "prefix").unwrap();
    assert!(res3);

    // Single line diff test
    let mut old_single = HashMap::new();
    old_single.insert("val".to_string(), "old".to_string());
    let mut new_single = HashMap::new();
    new_single.insert("val".to_string(), "new".to_string());

    let res4 = print_diff(
        "single",
        Some(&old_single),
        &new_single,
        false,
        false,
        "prefix",
    )
    .unwrap();
    assert!(res4);
}

#[tokio::test]
async fn test_plan_generic_error_paths() {
    let mock_url = start_mock_server().await;
    let client = KeycloakClient::new(mock_url);
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let roles_dir = workspace_dir.join("roles");
    fs::create_dir_all(&roles_dir).unwrap();

    // Create profiles directory and prod.yaml profile configuration file
    let profiles_dir = workspace_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(profiles_dir.join("prod.yaml"), "").unwrap();

    // Create an overlay file role.prod.yaml to hit overlay check line 52 in plan/generic.rs
    fs::write(roles_dir.join("role.prod.yaml"), "name: role-prod\n").unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        resolver: resolver.clone(),
        realm_name: "error-realm",
        ui: &*ui,
        profile: None,
    };

    let res = kaji::plan::generic::plan_resources::<RoleRepresentation>(&ctx).await;
    assert!(res.is_err());

    let ctx2 = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        resolver: resolver.clone(),
        realm_name: "test-realm",
        ui: &*ui,
        profile: None,
    };

    fs::write(roles_dir.join("invalid_role.yaml"), "name: { :").unwrap();
    let res2 = kaji::plan::generic::plan_resources::<RoleRepresentation>(&ctx2).await;
    assert!(res2.is_err());

    fs::write(
        roles_dir.join("invalid_deserialize.yaml"),
        "composite: [1, 2, 3]\n",
    )
    .unwrap();
    let res_deser = kaji::plan::generic::plan_resources::<RoleRepresentation>(&ctx2).await;
    assert!(res_deser.is_err());

    let clients_dir = workspace_dir.join("clients");
    fs::create_dir_all(&clients_dir).unwrap();
    fs::write(
        clients_dir.join("invalid_client.yaml"),
        "name: some-client-name\n",
    )
    .unwrap();

    let res3 = kaji::plan::generic::plan_resources::<ClientRepresentation>(&ctx2).await;
    assert!(res3.is_err());
}

#[tokio::test]
async fn test_plan_components_and_keys_coverage() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let keys_dir = workspace_dir.join("keys");
    fs::create_dir_all(&keys_dir).unwrap();

    let comp = ComponentRepresentation {
        id: None,
        name: Some("rsa-generated".to_string()),
        provider_id: Some("rsa-generated".to_string()),
        provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
        parent_id: Some("test-realm".to_string()),
        sub_type: None,
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };
    fs::write(
        keys_dir.join("rsa-generated.yaml"),
        serde_yaml::to_string(&comp).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        resolver: resolver.clone(),
        realm_name: "test-realm",
        ui: &*ui,
        profile: None,
    };

    let res = kaji::plan::components::plan_components_or_keys(&ctx, "keys").await;
    assert!(res.is_ok());

    // Write an invalid component file to trigger plan/components.rs:75 (deserialize failure)
    fs::write(keys_dir.join("invalid.yaml"), "providerId: [1, 2, 3]\n").unwrap();
    let res_err = kaji::plan::components::plan_components_or_keys(&ctx, "keys").await;
    assert!(res_err.is_err());
}

#[tokio::test]
async fn test_apply_authenticator_configs_cache_hits() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("cache-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("cache-realm");

    let auth_configs_dir = realm_dir.join("authenticator-configs");
    fs::create_dir_all(&auth_configs_dir).unwrap();

    let config1 = AuthenticatorConfigRepresentation {
        alias: Some("config-0".to_string()),
        config: Some(HashMap::new()),
        id: None,
        extra: HashMap::new(),
    };
    fs::write(
        auth_configs_dir.join("config-0.yaml"),
        serde_yaml::to_string(&config1).unwrap(),
    )
    .unwrap();

    let config2 = AuthenticatorConfigRepresentation {
        alias: Some("config-1".to_string()),
        config: Some(HashMap::new()),
        id: None,
        extra: HashMap::new(),
    };
    fs::write(
        auth_configs_dir.join("config-1.yaml"),
        serde_yaml::to_string(&config2).unwrap(),
    )
    .unwrap();

    let local_flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir_all(&local_flows_dir).unwrap();

    let flow1 = AuthenticationFlowRepresentation {
        alias: Some("flow-1".to_string()),
        authentication_executions: Some(vec![AuthenticationExecutionExportRepresentation {
            authenticator: Some("review-profile".to_string()),
            authenticator_config: Some("config-1".to_string()),
            authenticator_flow: None,
            flow_alias: None,
            priority: None,
            requirement: None,
            user_setup_allowed: None,
            id: None,
            extra: HashMap::new(),
        }]),
        built_in: None,
        description: None,
        id: None,
        provider_id: None,
        top_level: None,
        extra: HashMap::new(),
    };
    fs::write(
        local_flows_dir.join("flow-1.yaml"),
        serde_yaml::to_string(&flow1).unwrap(),
    )
    .unwrap();

    let flow2 = AuthenticationFlowRepresentation {
        alias: Some("flow-2".to_string()),
        authentication_executions: Some(vec![AuthenticationExecutionExportRepresentation {
            authenticator: Some("another-authenticator".to_string()),
            authenticator_config: Some("config-0".to_string()),
            authenticator_flow: None,
            flow_alias: None,
            priority: None,
            requirement: None,
            user_setup_allowed: None,
            id: None,
            extra: HashMap::new(),
        }]),
        built_in: None,
        description: None,
        id: None,
        provider_id: None,
        top_level: None,
        extra: HashMap::new(),
    };
    fs::write(
        local_flows_dir.join("flow-2.yaml"),
        serde_yaml::to_string(&flow2).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let planned_files = Arc::new(None);
    let secrets_path = Arc::new(workspace_dir.join(".secrets"));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    let _ = apply::authenticator_config::apply_authenticator_configs(
        kaji::apply::ApplyContext {
            client: &client,
            workspace_dir: realm_dir.clone(),
            secrets_path: secrets_path,
            resolver: resolver,
            planned_files: planned_files,
            realm_name: "cache-realm",
            profile: None,
            review: false,
            ui: ui,
            yes: true,
            prune: false,
        }
    )
    .await;
}

#[tokio::test]
async fn test_plan_components_interactive() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let components_dir = workspace_dir.join("components");
    fs::create_dir_all(&components_dir).unwrap();

    let comp = ComponentRepresentation {
        id: Some("c1".to_string()),
        name: Some("component-1".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        parent_id: Some("different-realm".to_string()),
        sub_type: None,
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };
    fs::write(
        components_dir.join("component-1.yaml"),
        serde_yaml::to_string(&comp).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![0]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: true,
            verbose: false,
        },
        resolver: resolver.clone(),
        realm_name: "test-realm",
        ui: &*ui,
        profile: None,
    };

    let res = kaji::plan::components::plan_components_or_keys(&ctx, "components").await;
    assert!(res.is_ok());
    let (changed_paths, summary) = res.unwrap();
    assert_eq!(changed_paths.len(), 1);
    assert_eq!(summary.updated, 1);

    let keys_dir = workspace_dir.join("keys");
    fs::create_dir_all(&keys_dir).unwrap();

    let key_comp = ComponentRepresentation {
        id: Some("rsa-generated-id".to_string()),
        name: Some("rsa-generated".to_string()),
        provider_id: Some("rsa-generated".to_string()),
        provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
        parent_id: Some("different-realm".to_string()),
        sub_type: None,
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };
    fs::write(
        keys_dir.join("rsa-generated.yaml"),
        serde_yaml::to_string(&key_comp).unwrap(),
    )
    .unwrap();

    let ui2 = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![0]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx2 = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: true,
            verbose: false,
        },
        resolver: resolver.clone(),
        realm_name: "test-realm",
        ui: &*ui2,
        profile: None,
    };

    let res2 = kaji::plan::components::plan_components_or_keys(&ctx2, "keys").await;
    assert!(res2.is_ok());
}

#[tokio::test]
async fn test_plan_generic_interactive() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let roles_dir = workspace_dir.join("roles");
    fs::create_dir_all(&roles_dir).unwrap();

    let role = RoleRepresentation {
        id: Some("r1".to_string()),
        name: "role-1".to_string(),
        description: Some("changed-description".to_string()),
        container_id: None,
        composite: false,
        client_role: false,
        extra: HashMap::new(),
    };
    fs::write(
        roles_dir.join("role-1.yaml"),
        serde_yaml::to_string(&role).unwrap(),
    )
    .unwrap();

    let role2 = RoleRepresentation {
        id: None,
        name: "r2".to_string(),
        description: Some("new role description".to_string()),
        container_id: None,
        composite: false,
        client_role: false,
        extra: HashMap::new(),
    };
    fs::write(
        roles_dir.join("r2.yaml"),
        serde_yaml::to_string(&role2).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![0, 0]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: true,
            verbose: false,
        },
        resolver,
        realm_name: "test-realm",
        ui: &*ui,
        profile: None,
    };

    let res = kaji::plan::generic::plan_resources::<RoleRepresentation>(&ctx).await;
    assert!(res.is_ok());
    let (changed_paths, summary) = res.unwrap();
    assert_eq!(changed_paths.len(), 2);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.created, 1);
}

#[tokio::test]
async fn test_plan_generic_missing_identity() {
    use kaji::models::ClientScopeRepresentation;
    let mock_url = start_mock_server().await;
    let client = KeycloakClient::new(mock_url);
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let client_scopes_dir = workspace_dir.join("client-scopes");
    fs::create_dir_all(&client_scopes_dir).unwrap();

    // Client scope with name None to trigger plan/generic.rs:71-78 (get_identity is None)
    let scope = ClientScopeRepresentation {
        id: None,
        name: None, // Missing name and ID!
        description: None,
        protocol: None,
        attributes: None,
        extra: HashMap::new(),
    };
    fs::write(
        client_scopes_dir.join("invalid.yaml"),
        serde_yaml::to_string(&scope).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });

    let ctx = kaji::plan::PlanContext {
        client: &client,
        workspace_dir: &workspace_dir,
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        resolver,
        realm_name: "test-realm",
        ui: &*ui,
        profile: None,
    };

    let res = kaji::plan::generic::plan_resources::<ClientScopeRepresentation>(&ctx).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_apply_authenticator_configs_missing_execution() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("cache-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("cache-realm");

    let auth_configs_dir = realm_dir.join("authenticator-configs");
    fs::create_dir_all(&auth_configs_dir).unwrap();

    let config1 = AuthenticatorConfigRepresentation {
        alias: Some("config-0".to_string()),
        config: Some(HashMap::new()),
        id: None,
        extra: HashMap::new(),
    };
    fs::write(
        auth_configs_dir.join("config-0.yaml"),
        serde_yaml::to_string(&config1).unwrap(),
    )
    .unwrap();

    let local_flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir_all(&local_flows_dir).unwrap();

    // Authenticator "non-existent" is not in flow-2 executions list on mock server
    let flow2 = AuthenticationFlowRepresentation {
        alias: Some("flow-2".to_string()),
        authentication_executions: Some(vec![AuthenticationExecutionExportRepresentation {
            authenticator: Some("non-existent".to_string()), // Triggers missing execution error!
            authenticator_config: Some("config-0".to_string()),
            authenticator_flow: None,
            flow_alias: None,
            priority: None,
            requirement: None,
            user_setup_allowed: None,
            id: None,
            extra: HashMap::new(),
        }]),
        built_in: None,
        description: None,
        id: None,
        provider_id: None,
        top_level: None,
        extra: HashMap::new(),
    };
    fs::write(
        local_flows_dir.join("flow-2.yaml"),
        serde_yaml::to_string(&flow2).unwrap(),
    )
    .unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let planned_files = Arc::new(None);
    let secrets_path = Arc::new(workspace_dir.join(".secrets"));
    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    let res = apply::authenticator_config::apply_authenticator_configs(
        kaji::apply::ApplyContext {
            client: &client,
            workspace_dir: realm_dir.clone(),
            secrets_path: secrets_path,
            resolver: resolver,
            planned_files: planned_files,
            realm_name: "cache-realm",
            profile: None,
            review: false,
            ui: ui,
            yes: true,
            prune: false,
        }
    )
    .await;
    assert!(res.is_err());
}
