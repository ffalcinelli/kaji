mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::{ClientRepresentation, RealmRepresentation, RoleRepresentation};
use kaji::plan;
use kaji::utils::ui::DialoguerUi;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_plan() {
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

    // Create realm.yaml - matches mock
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("Test Realm".to_string()), // Matches mock
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
        description: Some("Role 1".to_string()), // Matches mock
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
        name: Some("Client 1".to_string()), // Matches mock
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

    let ui = Arc::new(DialoguerUi::new());
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;

    // Run plan
    plan::run(kaji::plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: // changes_only
        false,
        realms_to_plan: // interactive
        &["test-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .expect("Plan failed");

    let plan_file = workspace_dir.join(".kajiplan");
    assert!(plan_file.exists(), "Plan file .kajiplan should exist");
    let plan_content = fs::read_to_string(&plan_file).unwrap();
    let planned_paths: Vec<std::path::PathBuf> = serde_json::from_str(&plan_content).unwrap();

    let planned_names: Vec<String> = planned_paths
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();

    assert!(planned_names.contains(&"new-role.yaml".to_string()));
    assert!(planned_names.contains(&"new-client.yaml".to_string()));
    assert!(planned_names.contains(&"new-idp.yaml".to_string()));
    assert!(planned_names.contains(&"new-scope.yaml".to_string()));
    assert!(planned_names.contains(&"new-group.yaml".to_string()));
    assert!(planned_names.contains(&"new-user.yaml".to_string()));
    assert!(planned_names.contains(&"new-flow.yaml".to_string()));
    assert!(planned_names.contains(&"new-action.yaml".to_string()));
    assert!(planned_names.contains(&"new-component.yaml".to_string()));

    assert!(!planned_names.contains(&"role-1.yaml".to_string()));
    assert!(!planned_names.contains(&"user-1.yaml".to_string()));
    assert!(!planned_names.contains(&"group-1.yaml".to_string()));

    // Test with changes_only=true
    plan::run(kaji::plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: true,
        interactive: // changes_only
        false,
        realms_to_plan: // interactive
        &["test-realm".to_string()],
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await
    .expect("Plan with changes_only failed");

    // Test with non-existent realm
    plan::run(kaji::plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &["non-existent".to_string()],
        ui: ui,
        resolver: resolver,
        profile: None,
    })
    .await
    .expect("Plan for non-existent realm failed");
}
