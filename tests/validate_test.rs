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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Role name is empty")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Duplicate role name: admin")
    );
}

#[tokio::test]
async fn test_validate_missing_realm() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();

    let result = validate::run(workspace_dir.clone(), &["test-realm".to_string()]).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("realm.yaml not found")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Client ID is missing or empty")
    );
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
        result
            .unwrap_err()
            .to_string()
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
        result
            .unwrap_err()
            .to_string()
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Client Scope name is missing or empty")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Group name is missing or empty")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("User username is missing or empty")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("User username is missing or empty")
    );
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
        result
            .unwrap_err()
            .to_string()
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
        result
            .unwrap_err()
            .to_string()
            .contains("Required Action alias is missing or empty")
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
        result
            .unwrap_err()
            .to_string()
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Component name is empty")
    );
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
        result
            .unwrap_err()
            .to_string()
            .contains("Component providerId is missing or empty")
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Realm name is empty in realm.yaml")
    );
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
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("User username is missing or empty")
    );
}
