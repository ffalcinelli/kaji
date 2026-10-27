use kaji::models::{
    AuthenticationFlowRepresentation, ClientRepresentation, ClientScopeRepresentation,
    ComponentRepresentation, GroupRepresentation, IdentityProviderRepresentation,
    RealmRepresentation, RequiredActionProviderRepresentation, RoleRepresentation,
    UserRepresentation,
};
use kaji::validate;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_validate() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_empty_role_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // Create roles directory
    let roles_dir = realm_dir.join("roles");
    fs::create_dir(&roles_dir).unwrap();

    // Create role with empty name
    let role = RoleRepresentation {
        id: None,
        name: "".to_string(),
        description: None,
        container_id: None,
        composite: false,
        client_role: false,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        roles_dir.join("role.yaml"),
        serde_yaml::to_string(&role).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Role name is empty"));
}

#[tokio::test]
async fn test_validate_duplicate_role_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // Create roles directory
    let roles_dir = realm_dir.join("roles");
    fs::create_dir(&roles_dir).unwrap();

    // Create first role
    let role1 = RoleRepresentation {
        id: None,
        name: "admin".to_string(),
        description: None,
        container_id: None,
        composite: false,
        client_role: false,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        roles_dir.join("role1.yaml"),
        serde_yaml::to_string(&role1).unwrap(),
    )
    .unwrap();

    // Create second role with same name
    let role2 = RoleRepresentation {
        id: None,
        name: "admin".to_string(),
        description: None,
        container_id: None,
        composite: false,
        client_role: false,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        roles_dir.join("role2.yaml"),
        serde_yaml::to_string(&role2).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Duplicate role name 'admin'"));
}

#[tokio::test]
async fn test_validate_missing_realm() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("realm.yaml not found"));
}

#[tokio::test]
async fn test_validate_empty_client_id() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // Create clients directory
    let clients_dir = realm_dir.join("clients");
    fs::create_dir(&clients_dir).unwrap();

    // Create client with empty client_id
    let client = ClientRepresentation {
        id: None,
        client_id: Some("".to_string()),
        secret: None,
        name: None,
        description: None,
        enabled: None,
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        clients_dir.join("client.yaml"),
        serde_yaml::to_string(&client).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Client ID is missing or empty"));
}

#[tokio::test]
async fn test_validate_empty_idp_alias() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let idps_dir = realm_dir.join("identity-providers");
    fs::create_dir(&idps_dir).unwrap();

    let idp = IdentityProviderRepresentation {
        internal_id: None,
        alias: Some("".to_string()),
        provider_id: Some("google".to_string()),
        enabled: None,
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
        idps_dir.join("idp.yaml"),
        serde_yaml::to_string(&idp).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err())
            .contains("Identity Provider alias is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_idp_provider_id() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let idps_dir = realm_dir.join("identity-providers");
    fs::create_dir(&idps_dir).unwrap();

    let idp = IdentityProviderRepresentation {
        internal_id: None,
        alias: Some("google".to_string()),
        provider_id: Some("".to_string()),
        enabled: None,
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
        idps_dir.join("idp.yaml"),
        serde_yaml::to_string(&idp).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err())
            .contains("Identity Provider providerId is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_client_scope_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let scopes_dir = realm_dir.join("client-scopes");
    fs::create_dir(&scopes_dir).unwrap();

    let scope = ClientScopeRepresentation {
        id: None,
        name: Some("".to_string()),
        description: None,
        protocol: None,
        attributes: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        scopes_dir.join("scope.yaml"),
        serde_yaml::to_string(&scope).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Client Scope name is missing or empty"));
}

#[tokio::test]
async fn test_validate_empty_group_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let groups_dir = realm_dir.join("groups");
    fs::create_dir(&groups_dir).unwrap();

    let group = GroupRepresentation {
        id: None,
        name: Some("".to_string()),
        path: None,
        sub_groups: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        groups_dir.join("group.yaml"),
        serde_yaml::to_string(&group).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Group name is missing or empty"));
}

#[tokio::test]
async fn test_validate_empty_username() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let users_dir = realm_dir.join("users");
    fs::create_dir(&users_dir).unwrap();

    let user = UserRepresentation {
        id: None,
        username: Some("".to_string()),
        enabled: None,
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        users_dir.join("user.yaml"),
        serde_yaml::to_string(&user).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("User username is missing or empty"));
}

#[tokio::test]
async fn test_validate_missing_username() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let users_dir = realm_dir.join("users");
    fs::create_dir(&users_dir).unwrap();

    let user = UserRepresentation {
        id: None,
        username: None,
        enabled: None,
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        users_dir.join("user.yaml"),
        serde_yaml::to_string(&user).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("User username is missing or empty"));
}

#[tokio::test]
async fn test_validate_empty_auth_flow_alias() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir(&flows_dir).unwrap();

    let flow = AuthenticationFlowRepresentation {
        id: None,
        alias: Some("".to_string()),
        description: None,
        provider_id: None,
        top_level: None,
        built_in: None,
        authentication_executions: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        flows_dir.join("flow.yaml"),
        serde_yaml::to_string(&flow).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err())
            .contains("Authentication Flow alias is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_required_action_alias() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let actions_dir = realm_dir.join("required-actions");
    fs::create_dir(&actions_dir).unwrap();

    let action = RequiredActionProviderRepresentation {
        alias: Some("".to_string()),
        name: None,
        provider_id: Some("provider".to_string()),
        enabled: None,
        default_action: None,
        priority: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        actions_dir.join("action.yaml"),
        serde_yaml::to_string(&action).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err()).contains("Required Action alias is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_required_action_provider_id() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let actions_dir = realm_dir.join("required-actions");
    fs::create_dir(&actions_dir).unwrap();

    let action = RequiredActionProviderRepresentation {
        alias: Some("alias".to_string()),
        name: None,
        provider_id: Some("".to_string()),
        enabled: None,
        default_action: None,
        priority: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        actions_dir.join("action.yaml"),
        serde_yaml::to_string(&action).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err())
            .contains("Required Action providerId is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_component_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let components_dir = realm_dir.join("components");
    fs::create_dir(&components_dir).unwrap();

    let component = ComponentRepresentation {
        id: None,
        name: Some("".to_string()),
        provider_id: Some("provider".to_string()),
        provider_type: None,
        parent_id: None,
        sub_type: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        components_dir.join("component.yaml"),
        serde_yaml::to_string(&component).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Component name is empty"));
}

#[tokio::test]
async fn test_validate_missing_component_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let components_dir = realm_dir.join("components");
    fs::create_dir(&components_dir).unwrap();

    let component = ComponentRepresentation {
        id: None,
        name: None, // Missing name
        provider_id: Some("provider".to_string()),
        provider_type: None,
        parent_id: None,
        sub_type: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        components_dir.join("component.yaml"),
        serde_yaml::to_string(&component).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(
        result.is_ok(),
        "Validation should succeed for missing component name. Error: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_validate_empty_component_provider_id() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create valid realm.yaml
    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let components_dir = realm_dir.join("components");
    fs::create_dir(&components_dir).unwrap();

    let component = ComponentRepresentation {
        id: None,
        name: Some("name".to_string()),
        provider_id: Some("".to_string()),
        provider_type: None,
        parent_id: None,
        sub_type: None,
        config: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        components_dir.join("component.yaml"),
        serde_yaml::to_string(&component).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        format!("{:#}", result.unwrap_err()).contains("Component providerId is missing or empty")
    );
}

#[tokio::test]
async fn test_validate_empty_realm_name() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    // Create realm.yaml with empty name
    let realm = RealmRepresentation {
        realm: "".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("Realm name is empty"));
}

#[tokio::test]
async fn test_validate_empty_username_edge_case() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    let users_dir = realm_dir.join("users");
    fs::create_dir(&users_dir).unwrap();

    let user = UserRepresentation {
        id: None,
        username: Some("".to_string()),
        enabled: None,
        first_name: None,
        last_name: None,
        email: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        users_dir.join("user.yaml"),
        serde_yaml::to_string(&user).unwrap(),
    )
    .unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(format!("{:#}", result.unwrap_err()).contains("User username is missing or empty"));
}

#[tokio::test]
async fn test_validate_nonexistent_workspace() {
    let dir = tempdir().unwrap();
    let nonexistent = dir.path().join("does-not-exist");

    let res = validate::run(nonexistent.clone(), &[]).await;
    assert!(res.is_err());
    let err_str = format!("{:#}", res.unwrap_err());
    assert!(err_str.contains("Hint: Create the workspace directory first or use `kaji init`."));
    assert!(err_str.contains("Input directory"));

    let res_profile = validate::run_with_profile(nonexistent, &[], Some("prod")).await;
    assert!(res_profile.is_err());
    let err_str = format!("{:#}", res_profile.unwrap_err());
    assert!(err_str.contains("Hint: Create the workspace directory first or use `kaji init`."));
}

#[tokio::test]
async fn test_validate_missing_components_dir() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let realm = RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: None,
        extra: std::collections::HashMap::new(),
    };
    fs::write(
        realm_dir.join("realm.yaml"),
        serde_yaml::to_string(&realm).unwrap(),
    )
    .unwrap();

    // No components directory exists; validation should succeed cleanly
    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_validate_client_roles_empty_and_duplicate() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    let client_roles_dir = realm_dir.join("clients").join("my-client").join("roles");
    std::fs::create_dir_all(&client_roles_dir).unwrap();

    fs::write(realm_dir.join("realm.yaml"), "realm: test-realm\n").unwrap();

    // 1. Empty role name inside clients/<clientId>/roles/
    fs::write(client_roles_dir.join("bad.yaml"), "name: \"\"\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(
        format!("{:#}", err).contains("Role name is empty"),
        "Expected empty role name error, got: {err}"
    );

    // 2. Duplicate role names inside clients/<clientId>/roles/
    fs::write(client_roles_dir.join("bad.yaml"), "name: reader\n").unwrap();
    fs::write(client_roles_dir.join("dup.yaml"), "name: reader\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(
        format!("{:#}", err).contains("Duplicate client role name 'reader'"),
        "Expected duplicate client role error, got: {err}"
    );

    // 3. Valid distinct client roles
    fs::write(client_roles_dir.join("dup.yaml"), "name: writer\n").unwrap();
    assert!(
        validate::run(workspace_dir, &["test-realm".to_string()])
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_validate_duplicate_resource_identities_and_subgroups() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();
    // Also verify realm.yml fallback
    fs::write(realm_dir.join("realm.yml"), "realm: test-realm\n").unwrap();

    // Duplicate clientId
    let clients_dir = realm_dir.join("clients");
    fs::create_dir_all(&clients_dir).unwrap();
    fs::write(clients_dir.join("c1.yaml"), "clientId: app\n").unwrap();
    fs::write(clients_dir.join("c2.yaml"), "clientId: app\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate clientId 'app'"));
    fs::remove_file(clients_dir.join("c2.yaml")).unwrap();

    // Duplicate IDP alias
    let idps_dir = realm_dir.join("identity-providers");
    fs::create_dir_all(&idps_dir).unwrap();
    fs::write(
        idps_dir.join("i1.yaml"),
        "alias: google\nproviderId: google\n",
    )
    .unwrap();
    fs::write(
        idps_dir.join("i2.yaml"),
        "alias: google\nproviderId: oidc\n",
    )
    .unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate Identity Provider alias 'google'"));
    fs::remove_file(idps_dir.join("i2.yaml")).unwrap();

    // Duplicate Client Scope name
    let scopes_dir = realm_dir.join("client-scopes");
    fs::create_dir_all(&scopes_dir).unwrap();
    fs::write(scopes_dir.join("s1.yaml"), "name: custom-scope\n").unwrap();
    fs::write(scopes_dir.join("s2.yaml"), "name: custom-scope\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate Client Scope name 'custom-scope'"));
    fs::remove_file(scopes_dir.join("s2.yaml")).unwrap();

    // Duplicate Group and invalid/duplicate subGroups
    let groups_dir = realm_dir.join("groups");
    fs::create_dir_all(&groups_dir).unwrap();
    fs::write(
        groups_dir.join("g1.yaml"),
        "name: eng\nsubGroups:\n  - name: \"\"\n",
    )
    .unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Group name is missing or empty"));

    fs::write(
        groups_dir.join("g1.yaml"),
        "name: eng\nsubGroups:\n  - name: team\n  - name: team\n",
    )
    .unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate subGroup name 'team' under group '/eng'"));

    fs::write(groups_dir.join("g1.yaml"), "name: eng\n").unwrap();
    fs::write(groups_dir.join("g2.yaml"), "name: eng\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate Group 'eng'"));
    fs::remove_file(groups_dir.join("g2.yaml")).unwrap();

    // Duplicate User username
    let users_dir = realm_dir.join("users");
    fs::create_dir_all(&users_dir).unwrap();
    fs::write(users_dir.join("u1.yaml"), "username: alice\n").unwrap();
    fs::write(users_dir.join("u2.yaml"), "username: alice\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate User username 'alice'"));
    fs::remove_file(users_dir.join("u2.yaml")).unwrap();

    // Duplicate Required Action alias
    let actions_dir = realm_dir.join("required-actions");
    fs::create_dir_all(&actions_dir).unwrap();
    fs::write(
        actions_dir.join("a1.yaml"),
        "alias: terms\nproviderId: terms\n",
    )
    .unwrap();
    fs::write(
        actions_dir.join("a2.yaml"),
        "alias: terms\nproviderId: terms\n",
    )
    .unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate Required Action alias 'terms'"));
    fs::remove_file(actions_dir.join("a2.yaml")).unwrap();

    // Duplicate Authenticator Config alias
    let configs_dir = realm_dir.join("authenticator-configs");
    fs::create_dir_all(&configs_dir).unwrap();
    fs::write(configs_dir.join("ac1.yaml"), "alias: otp-cfg\n").unwrap();
    fs::write(configs_dir.join("ac2.yaml"), "alias: otp-cfg\n").unwrap();
    let err = validate::run(workspace_dir.clone(), &["test-realm".to_string()])
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Duplicate Authenticator Config alias 'otp-cfg'"));
    fs::remove_file(configs_dir.join("ac2.yaml")).unwrap();

    // Once duplicates are removed, validation succeeds
    assert!(
        validate::run(workspace_dir, &["test-realm".to_string()])
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_validate_reports_every_problem_with_its_files() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    for sub in ["clients", "users", "authentication-flows"] {
        std::fs::create_dir_all(realm_dir.join(sub)).unwrap();
    }
    fs::write(realm_dir.join("realm.yaml"), "realm: test-realm\n").unwrap();
    fs::write(realm_dir.join("clients/a.yaml"), "clientId: app\n").unwrap();
    fs::write(realm_dir.join("clients/b.yaml"), "clientId: app\n").unwrap();
    fs::write(realm_dir.join("users/u.yaml"), "username: \"\"\n").unwrap();
    fs::write(
        realm_dir.join("authentication-flows/f.yaml"),
        "alias: my-flow\nauthenticationExecutions:\n- authenticator: auth-cookie\n  requirement: MAYBE\n",
    )
    .unwrap();

    let err = validate::run(workspace_dir, &["test-realm".to_string()])
        .await
        .unwrap_err();
    let msg = format!("{:#}", err);
    assert!(msg.contains("Realm 'test-realm' is invalid"), "{msg}");
    assert!(msg.contains("3 problems found"), "{msg}");
    assert!(
        msg.contains("b.yaml: Duplicate clientId 'app' (also declared in")
            && msg.contains("a.yaml)"),
        "{msg}"
    );
    assert!(
        msg.contains("u.yaml: User username is missing or empty"),
        "{msg}"
    );
    assert!(
        msg.contains(
            "f.yaml: Invalid requirement 'MAYBE' in execution #1 (auth-cookie) of flow 'my-flow'"
        ),
        "{msg}"
    );
}
