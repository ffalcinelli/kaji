mod common;
use common::start_mock_server;
use kaji::apply;
use kaji::client::KeycloakClient;
use kaji::models::{ClientRepresentation, RealmRepresentation, RoleRepresentation};
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_apply() {
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
    std::fs::create_dir_all(&realm_dir).unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    // Create realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Updated Realm".to_string()),
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // Create roles
    let roles_dir = realm_dir.join("roles");
    fs::create_dir(&roles_dir).unwrap();
    let role = RoleRepresentation {
        id: None,
        name: "new-role".to_string(),
        description: Some("New Role".to_string()),
        container_id: None,
        composite: false,
        client_role: false,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        roles_dir.join("new-role.yaml"),
        serde_yaml::to_string(&role).unwrap(),
    )
    .unwrap();

    let existing_role = RoleRepresentation {
        id: None,
        name: "role-1".to_string(), // Matches mock server response
        description: Some("Updated Role 1".to_string()),
        container_id: None,
        composite: false,
        client_role: false,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        roles_dir.join("role-1.yaml"),
        serde_yaml::to_string(&existing_role).unwrap(),
    )
    .unwrap();

    // Create clients
    let clients_dir = realm_dir.join("clients");
    fs::create_dir(&clients_dir).unwrap();
    let client_rep = ClientRepresentation {
        id: None,
        client_id: Some("new-client".to_string()),
        secret: None,
        name: Some("New Client".to_string()),
        description: None,
        enabled: Some(true),
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        clients_dir.join("new-client.yaml"),
        serde_yaml::to_string(&client_rep).unwrap(),
    )
    .unwrap();

    let existing_client = ClientRepresentation {
        id: None,
        client_id: Some("client-1".to_string()),
        secret: None,
        name: Some("Updated Client 1".to_string()),
        description: None,
        enabled: Some(true),
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        clients_dir.join("client-1.yaml"),
        serde_yaml::to_string(&existing_client).unwrap(),
    )
    .unwrap();

    // Identity Providers
    let idps_dir = realm_dir.join("identity-providers");
    fs::create_dir(&idps_dir).unwrap();
    let idp = kaji::models::IdentityProviderRepresentation {
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
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        idps_dir.join("google.yaml"),
        serde_yaml::to_string(&idp).unwrap(),
    )
    .unwrap();

    let new_idp = kaji::models::IdentityProviderRepresentation {
        internal_id: None,
        alias: Some("new-idp".to_string()),
        provider_id: Some("oidc".to_string()),
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
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        idps_dir.join("new-idp.yaml"),
        serde_yaml::to_string(&new_idp).unwrap(),
    )
    .unwrap();

    // Client Scopes
    let scopes_dir = realm_dir.join("client-scopes");
    fs::create_dir(&scopes_dir).unwrap();
    let scope = kaji::models::ClientScopeRepresentation {
        id: None,
        name: Some("scope-1".to_string()),
        description: None,
        protocol: Some("openid-connect".to_string()),
        attributes: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        scopes_dir.join("scope-1.yaml"),
        serde_yaml::to_string(&scope).unwrap(),
    )
    .unwrap();

    let new_scope = kaji::models::ClientScopeRepresentation {
        id: None,
        name: Some("new-scope".to_string()),
        description: None,
        protocol: Some("openid-connect".to_string()),
        attributes: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        scopes_dir.join("new-scope.yaml"),
        serde_yaml::to_string(&new_scope).unwrap(),
    )
    .unwrap();

    // Groups
    let groups_dir = realm_dir.join("groups");
    fs::create_dir(&groups_dir).unwrap();
    let group = kaji::models::GroupRepresentation {
        id: None,
        name: Some("group-1".to_string()),
        path: Some("/group-1".to_string()),
        sub_groups: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        groups_dir.join("group-1.yaml"),
        serde_yaml::to_string(&group).unwrap(),
    )
    .unwrap();

    let new_group = kaji::models::GroupRepresentation {
        id: None,
        name: Some("new-group".to_string()),
        path: Some("/new-group".to_string()),
        sub_groups: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        groups_dir.join("new-group.yaml"),
        serde_yaml::to_string(&new_group).unwrap(),
    )
    .unwrap();

    // Users
    let users_dir = realm_dir.join("users");
    fs::create_dir(&users_dir).unwrap();
    let user = kaji::models::UserRepresentation {
        id: None,
        username: Some("user-1".to_string()),
        enabled: Some(true),
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        users_dir.join("user-1.yaml"),
        serde_yaml::to_string(&user).unwrap(),
    )
    .unwrap();

    let new_user = kaji::models::UserRepresentation {
        id: None,
        username: Some("new-user".to_string()),
        enabled: Some(true),
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        users_dir.join("new-user.yaml"),
        serde_yaml::to_string(&new_user).unwrap(),
    )
    .unwrap();

    // Authentication Flows
    let flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir(&flows_dir).unwrap();
    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("flow-1".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        flows_dir.join("flow-1.yaml"),
        serde_yaml::to_string(&flow).unwrap(),
    )
    .unwrap();

    let new_flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("new-flow".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        flows_dir.join("new-flow.yaml"),
        serde_yaml::to_string(&new_flow).unwrap(),
    )
    .unwrap();

    // Required Actions
    let actions_dir = realm_dir.join("required-actions");
    fs::create_dir(&actions_dir).unwrap();
    let action = kaji::models::RequiredActionProviderRepresentation {
        alias: Some("action-1".to_string()),
        name: Some("Action 1".to_string()),
        provider_id: Some("action-provider".to_string()),
        enabled: Some(true),
        default_action: Some(false),
        priority: Some(10),
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        actions_dir.join("action-1.yaml"),
        serde_yaml::to_string(&action).unwrap(),
    )
    .unwrap();

    let new_action = kaji::models::RequiredActionProviderRepresentation {
        alias: Some("new-action".to_string()),
        name: Some("New Action".to_string()),
        provider_id: Some("new-action-provider".to_string()),
        enabled: Some(true),
        default_action: Some(false),
        priority: Some(11),
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        actions_dir.join("new-action.yaml"),
        serde_yaml::to_string(&new_action).unwrap(),
    )
    .unwrap();

    // Components
    let components_dir = realm_dir.join("components");
    fs::create_dir(&components_dir).unwrap();
    let component = kaji::models::ComponentRepresentation {
        id: None,
        name: Some("component-1".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        sub_type: None,
        parent_id: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        components_dir.join("component-1.yaml"),
        serde_yaml::to_string(&component).unwrap(),
    )
    .unwrap();

    let new_component = kaji::models::ComponentRepresentation {
        id: None,
        name: Some("new-component".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        sub_type: None,
        parent_id: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        components_dir.join("new-component.yaml"),
        serde_yaml::to_string(&new_component).unwrap(),
    )
    .unwrap();

    // Keys (stored as components in 'keys' directory)
    let keys_dir = realm_dir.join("keys");
    fs::create_dir(&keys_dir).unwrap();
    let key_component = kaji::models::ComponentRepresentation {
        id: None,
        name: Some("rsa-generated".to_string()),
        provider_id: Some("rsa-generated".to_string()),
        provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
        sub_type: None,
        parent_id: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        keys_dir.join("rsa-generated.yaml"),
        serde_yaml::to_string(&key_component).unwrap(),
    )
    .unwrap();

    let ui = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

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
    .expect("Apply failed");

    let plan_file = workspace_dir.join(".kajiplan");
    assert!(
        !plan_file.exists(),
        ".kajiplan should not exist after apply"
    );

    // Load secrets for subsequent runs
    let mut secrets_map = HashMap::new();
    let secrets_file = workspace_dir.join(".secrets");
    if secrets_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&secrets_file) {
            for line in content.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    secrets_map.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }
    let resolver_with_secrets: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(secrets_map));

    // Test with .kajiplan
    let planned_files = vec![realm_dir.join("realm.yaml")];
    fs::write(&plan_file, serde_json::to_string(&planned_files).unwrap()).unwrap();

    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
        false,
        ui.clone(),
        resolver_with_secrets.clone(),
        None,
    )
    .await
    .expect("Apply with plan failed");

    assert!(
        !plan_file.exists(),
        ".kajiplan should be deleted after apply with plan"
    );

    // Test with empty plan
    fs::write(&plan_file, "[]").unwrap();
    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true,
        false,
        ui.clone(),
        resolver_with_secrets.clone(),
        None,
    )
    .await
    .expect("Apply with empty plan failed");

    assert!(
        !plan_file.exists(),
        ".kajiplan should be deleted after apply with empty plan"
    );

    // 4. Test review mode (interactive)
    ui.confirms.lock().unwrap().push(false); // Reject first change
    ui.confirms.lock().unwrap().push(true); // Accept second change
    // We need to know how many resources are being applied to fill the confirms queue correctly.
    // Actually, let's just test a single resource type to be safe.

    let single_resource_dir = workspace_dir.join("review-realm");
    fs::create_dir_all(single_resource_dir.join("roles")).unwrap();
    fs::write(
        single_resource_dir.join("realm.yaml"),
        "realm: review-realm\n",
    )
    .unwrap();
    fs::write(single_resource_dir.join("roles/r1.yaml"), "name: r1\n").unwrap();

    let mut review_client = client.clone();
    review_client.set_target_realm("review-realm".to_string());

    // Clear confirms and add one 'true' for the initial prompt and one 'false' for the resource review
    {
        let mut confirms = ui.confirms.lock().unwrap();
        confirms.clear();
        confirms.push(true); // Yes, send everything anyway
        confirms.push(false); // No, don't apply this specific role
    }

    apply::run(
        &review_client,
        workspace_dir.clone(),
        &["review-realm".to_string()],
        false, // yes = false
        true,  // review = true
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await
    .expect("Apply with review failed");

    // Assert that the confirms queue was fully consumed
    assert!(
        ui.confirms.lock().unwrap().is_empty(),
        "All confirms should be consumed during review apply"
    );
}

#[tokio::test]
async fn test_apply_aborted_by_user() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock_token".to_string());

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Write empty .kajiplan
    let plan_file = workspace_dir.join(".kajiplan");
    std::fs::write(&plan_file, "[]").unwrap();

    let ui = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![false]), // User aborts when prompted to apply everything
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    let res = apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        false, // yes = false
        false, // review = false
        ui.clone(),
        resolver,
        None,
    )
    .await;

    assert!(res.is_ok());
    // Since it aborted, the plan_file should NOT be deleted
    assert!(plan_file.exists());
}

#[tokio::test]
async fn test_apply_component_no_id_no_name() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock_token".to_string());

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(realm_dir.join("components")).unwrap();

    // Component without name and without id (only providerId)
    std::fs::write(
        realm_dir.join("components").join("empty.yaml"),
        "providerId: ldap\nproviderType: org.keycloak.storage.UserStorageProvider\n",
    )
    .unwrap();

    let ui = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    let res = apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true, // yes = true
        false,
        ui.clone(),
        resolver,
        None,
    )
    .await;

    assert!(res.is_ok());
}

#[tokio::test]
async fn test_apply_authenticator_configs_review() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock_token".to_string());

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let configs_dir = workspace_dir.join("authenticator-configs");
    std::fs::create_dir_all(&configs_dir).unwrap();

    // Create a local config file representing an update to an existing config
    let config_update = kaji::models::AuthenticatorConfigRepresentation {
        id: Some("config-1".to_string()),
        alias: Some("review profile config".to_string()),
        config: Some(HashMap::from([(
            "loa-condition-level".to_string(),
            serde_json::json!("5"),
        )])),
        extra: HashMap::new(),
    };
    std::fs::write(
        configs_dir.join("config-update.yaml"),
        serde_yaml::to_string(&config_update).unwrap(),
    )
    .unwrap();

    // Create a local config file representing a new config
    let config_new = kaji::models::AuthenticatorConfigRepresentation {
        id: None,
        alias: Some("new config".to_string()),
        config: Some(HashMap::from([(
            "param".to_string(),
            serde_json::json!("val"),
        )])),
        extra: HashMap::new(),
    };
    std::fs::write(
        configs_dir.join("config-new.yaml"),
        serde_yaml::to_string(&config_new).unwrap(),
    )
    .unwrap();

    // We also need local authentication flow execution referencing "new config"
    let flows_dir = workspace_dir.join("authentication-flows");
    std::fs::create_dir_all(&flows_dir).unwrap();
    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: Some("f1".to_string()),
        alias: Some("flow-1".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: Some(vec![
            kaji::models::AuthenticationExecutionExportRepresentation {
                id: Some("exec-1".to_string()),
                authenticator: Some("review-profile".to_string()),
                authenticator_config: Some("new config".to_string()),
                requirement: Some("REQUIRED".to_string()),
                priority: Some(1),
                user_setup_allowed: None,
                authenticator_flow: None,
                flow_alias: None,
                extra: HashMap::new(),
            },
        ]),
        extra: HashMap::new(),
    };
    std::fs::write(
        flows_dir.join("flow-1.yaml"),
        serde_yaml::to_string(&flow).unwrap(),
    )
    .unwrap();

    // 1. Run review mode where we reject both update and create
    let ui_reject = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![false, false]), // Reject first, reject second
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    let secrets_path = Arc::new(workspace_dir.join(".secrets"));
    let res = apply::authenticator_config::apply_authenticator_configs(
        &client,
        &workspace_dir,
        secrets_path.clone(),
        resolver.clone(),
        Arc::new(None),
        "test-realm",
        None,
        true, // review = true
        ui_reject.clone(),
        true, // yes = true
    )
    .await;
    assert!(res.is_ok());

    // 2. Run review mode where we accept both update and create
    let ui_accept = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![true, true]), // Accept first, accept second
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });
    let res2 = apply::authenticator_config::apply_authenticator_configs(
        &client,
        &workspace_dir,
        secrets_path.clone(),
        resolver.clone(),
        Arc::new(None),
        "test-realm",
        None,
        true, // review = true
        ui_accept.clone(),
        true, // yes = true
    )
    .await;
    assert!(res2.is_ok());
}

#[tokio::test]
async fn test_apply_enrichment() {
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
    std::fs::create_dir_all(&realm_dir).unwrap();

    let resolver: Arc<dyn SecretResolver> =
        Arc::new(EnvResolver::new(std::collections::HashMap::new()));

    // Create a clients directory and a client.yaml
    let clients_dir = realm_dir.join("clients");
    std::fs::create_dir(&clients_dir).unwrap();

    let client_file = clients_dir.join("client-1.yaml");
    let client_rep = ClientRepresentation {
        id: None,
        client_id: Some("client-1".to_string()),
        secret: None,
        name: Some("Initial Name".to_string()),
        description: None,
        enabled: Some(true),
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    std::fs::write(&client_file, serde_yaml::to_string(&client_rep).unwrap()).unwrap();

    // We expect Keycloak to return "Enriched Client 1" as the name, and id "1" (since get_single_client_handler mocks this).
    // Let's test with yes=true (auto-accept).
    let ui_yes = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        true, // yes = true (auto-accept enrichment)
        false,
        ui_yes,
        resolver.clone(),
        None,
    )
    .await
    .expect("Apply enrichment yes failed");

    // Read client-1.yaml back and verify it was updated with the enriched fields!
    let content = std::fs::read_to_string(&client_file).unwrap();
    let updated_client: ClientRepresentation = serde_yaml::from_str(&content).unwrap();
    assert_eq!(updated_client.id, Some("1".to_string()));
    assert_eq!(updated_client.name, Some("Enriched Client 1".to_string()));

    // Also verify that the newly generated secret was written to .secrets!
    let secrets_content = std::fs::read_to_string(workspace_dir.join(".secrets")).unwrap();
    assert!(
        secrets_content
            .contains("KEYCLOAK_REALM_TEST_REALM_CLIENT_CLIENT_1_SECRET=enriched-client-secret")
    );

    // Now test with yes=false, confirm=false (user rejects enrichment)
    let client_file_2 = clients_dir.join("client-2.yaml");
    let client_rep_2 = ClientRepresentation {
        id: None,
        client_id: Some("client-2".to_string()),
        secret: None,
        name: Some("Initial Name 2".to_string()),
        description: None,
        enabled: Some(true),
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    std::fs::write(
        &client_file_2,
        serde_yaml::to_string(&client_rep_2).unwrap(),
    )
    .unwrap();

    let ui_no = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![false]), // Reject enrichment prompt
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    apply::run(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        false, // yes = false (prompt)
        false,
        ui_no,
        resolver.clone(),
        None,
    )
    .await
    .expect("Apply enrichment no failed");

    // Verify client-2.yaml was NOT updated
    let content_2 = std::fs::read_to_string(&client_file_2).unwrap();
    let updated_client_2: ClientRepresentation = serde_yaml::from_str(&content_2).unwrap();
    assert_eq!(updated_client_2.id, None);
    assert_eq!(updated_client_2.name, Some("Initial Name 2".to_string()));
}

#[tokio::test]
async fn test_apply_pruning() {
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
    std::fs::create_dir_all(&realm_dir).unwrap();

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    // Create realm.yaml
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

    // Create clients directory but only put client-1.yaml in it (client-2 exists on the mock server and should be pruned)
    let clients_dir = realm_dir.join("clients");
    fs::create_dir(&clients_dir).unwrap();
    let client_rep = ClientRepresentation {
        id: Some("1".to_string()),
        client_id: Some("client-1".to_string()),
        secret: None,
        name: Some("Client 1".to_string()),
        description: None,
        enabled: Some(true),
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        clients_dir.join("client-1.yaml"),
        serde_yaml::to_string(&client_rep).unwrap(),
    )
    .unwrap();

    let ui = Arc::new(kaji::utils::ui::MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(vec![true, true, true]), // Send anyway, Enrichment client-1, Pruning client-2
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });

    apply::run_ext(
        &client,
        workspace_dir.clone(),
        &["test-realm".to_string()],
        false, // yes = false (prompt)
        false, // review = false
        true,  // prune = true
        ui,
        resolver.clone(),
        None,
    )
    .await
    .expect("Apply pruning failed");
}
