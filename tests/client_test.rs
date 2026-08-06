mod common;
use common::start_mock_server;
use kaji::client::KeycloakClient;
use kaji::models::ClientRepresentation;

#[tokio::test]
async fn test_login_password_grant() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    let result = client
        .login("admin-cli", None, Some("admin"), Some("admin"))
        .await;
    assert!(result.is_ok(), "Login failed: {:?}", result.err());
    assert_eq!(client.get_token().unwrap(), "mock_token");
}

#[tokio::test]
async fn test_login_client_credentials_grant() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    let result = client.login("admin-cli", Some("secret"), None, None).await;
    assert!(result.is_ok(), "Login failed: {:?}", result.err());
    assert_eq!(client.get_token().unwrap(), "mock_token");
}

#[tokio::test]
async fn test_login_fail() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    let result = client
        .login("admin-cli", None, Some("admin"), Some("wrong"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_login_parse_failure() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());

    // With username "bad_json", the mock server returns 200 OK but with invalid JSON structure
    // which should cause the JSON parsing into `TokenResponse` to fail.
    let result = client
        .login("admin-cli", None, Some("bad_json"), Some("admin"))
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse token response")
    );
}

#[tokio::test]
async fn test_get_realm() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let realm = client.get_realm().await.expect("Failed to get realm");
    assert_eq!(realm.realm, "test-realm");
    assert_eq!(realm.display_name, Some("Test Realm".to_string()));
}

#[tokio::test]
async fn test_get_clients() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let clients = client.get_clients().await.expect("Failed to get clients");
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0].client_id, Some("client-1".to_string()));
}

#[tokio::test]
async fn test_get_roles() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let roles = client.get_roles().await.expect("Failed to get roles");
    assert_eq!(roles.len(), 2);
    assert_eq!(roles[0].name, "role-1");
}

#[tokio::test]
async fn test_create_client() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let client_rep = ClientRepresentation {
        id: None,
        client_id: Some("new-client".to_string()),
        secret: None,
        name: None,
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

    let result = client.create_client(&client_rep).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_update_client() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let client_rep = ClientRepresentation {
        id: Some("1".to_string()),
        client_id: Some("client-1".to_string()),
        secret: None,
        name: Some("Updated Name".to_string()),
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

    let result = client.update_client("1", &client_rep).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_get_realms() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let realms = client.get_realms().await.expect("Failed to get realms");
    assert_eq!(realms.len(), 2);
    assert_eq!(realms[1].realm, "test-realm");
}

#[tokio::test]
async fn test_update_realm() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let realm = kaji::models::RealmRepresentation {
        realm: "test-realm".to_string(),
        enabled: Some(true),
        display_name: Some("New Name".to_string()),
        extra: std::collections::HashMap::new(),
    };

    let result = client.update_realm(&realm).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_roles() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let role = kaji::models::RoleRepresentation {
        id: None,
        name: "new-role".to_string(),
        description: None,
        composite: false,
        client_role: false,
        container_id: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_role(&role).await.is_ok());
    assert!(client.update_role("r1", &role).await.is_ok());
    assert!(client.delete_role("r1").await.is_ok());
}

#[tokio::test]
async fn test_create_identity_provider() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let idp = kaji::models::IdentityProviderRepresentation {
        alias: Some("new-google".to_string()),
        provider_id: Some("google".to_string()),
        enabled: Some(true),
        display_name: None,
        trust_email: None,
        store_token: None,
        add_read_token_role_on_create: None,
        authenticate_by_default: None,
        link_only: None,
        first_broker_login_flow_alias: None,
        post_broker_login_flow_alias: None,
        config: Some(std::collections::HashMap::new()),
        extra: std::collections::HashMap::new(),
        internal_id: None,
        update_profile_first_login_mode: None,
    };

    assert!(client.create_identity_provider(&idp).await.is_ok());
}

#[tokio::test]
async fn test_identity_providers() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let idps = client
        .get_identity_providers()
        .await
        .expect("Failed to get IDPs");
    assert_eq!(idps.len(), 1);

    let idp = kaji::models::IdentityProviderRepresentation {
        alias: Some("google".to_string()),
        provider_id: Some("google".to_string()),
        enabled: Some(true),
        display_name: None,
        trust_email: None,
        store_token: None,
        add_read_token_role_on_create: None,
        authenticate_by_default: None,
        link_only: None,
        first_broker_login_flow_alias: None,
        post_broker_login_flow_alias: None,
        config: Some(std::collections::HashMap::new()),
        extra: std::collections::HashMap::new(),
        internal_id: None,
        update_profile_first_login_mode: None,
    };

    assert!(client.create_identity_provider(&idp).await.is_ok());
    assert!(
        client
            .update_identity_provider("google", &idp)
            .await
            .is_ok()
    );
    assert!(client.delete_identity_provider("google").await.is_ok());
}

#[tokio::test]
async fn test_client_scopes() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let scopes = client
        .get_client_scopes()
        .await
        .expect("Failed to get scopes");
    assert_eq!(scopes.len(), 1);

    let scope = kaji::models::ClientScopeRepresentation {
        id: None,
        name: Some("new-scope".to_string()),
        description: None,
        protocol: None,
        attributes: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_client_scope(&scope).await.is_ok());
    assert!(client.update_client_scope("s1", &scope).await.is_ok());
    assert!(client.delete_client_scope("s1").await.is_ok());
}

#[tokio::test]
async fn test_groups() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let groups = client.get_groups().await.expect("Failed to get groups");
    assert_eq!(groups.len(), 1);

    let group = kaji::models::GroupRepresentation {
        id: None,
        name: Some("new-group".to_string()),
        path: None,
        sub_groups: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_group(&group).await.is_ok());
    assert!(client.update_group("g1", &group).await.is_ok());
    assert!(client.delete_group("g1").await.is_ok());
}

#[tokio::test]
async fn test_users() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let users = client.get_users().await.expect("Failed to get users");
    assert_eq!(users.len(), 1);

    let user = kaji::models::UserRepresentation {
        id: None,
        username: Some("new-user".to_string()),
        enabled: Some(true),
        email: None,
        first_name: None,
        last_name: None,
        email_verified: None,
        credentials: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_user(&user).await.is_ok());
    assert!(client.update_user("u1", &user).await.is_ok());
    assert!(client.delete_user("u1").await.is_ok());
}

#[tokio::test]
async fn test_authentication_flows() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let flows = client
        .get_authentication_flows()
        .await
        .expect("Failed to get flows");
    assert_eq!(flows.len(), 1);

    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("new-flow".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_authentication_flow(&flow).await.is_ok());
    assert!(client.update_authentication_flow("f1", &flow).await.is_ok());
    assert!(client.delete_authentication_flow("f1").await.is_ok());
}

#[tokio::test]
async fn test_create_authentication_flow() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("new-flow".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_authentication_flow(&flow).await.is_ok());
}

#[tokio::test]
async fn test_required_actions() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let actions = client
        .get_required_actions()
        .await
        .expect("Failed to get actions");
    assert_eq!(actions.len(), 1);

    let action = kaji::models::RequiredActionProviderRepresentation {
        alias: Some("action-1".to_string()),
        name: Some("Action 1".to_string()),
        provider_id: Some("action-provider".to_string()),
        enabled: Some(true),
        default_action: Some(false),
        priority: Some(10),
        config: Some(std::collections::HashMap::new()),
        extra: std::collections::HashMap::new(),
    };

    assert!(
        client
            .update_required_action("action-1", &action)
            .await
            .is_ok()
    );
    assert!(client.register_required_action(&action).await.is_ok());
    assert!(client.delete_required_action("action-1").await.is_ok());
}

#[tokio::test]
async fn test_components() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let components = client
        .get_components()
        .await
        .expect("Failed to get components");
    assert_eq!(components.len(), 3);

    let component = kaji::models::ComponentRepresentation {
        id: None,
        name: Some("new-component".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        parent_id: None,
        sub_type: None,
        config: Some(std::collections::HashMap::new()),
        extra: std::collections::HashMap::new(),
    };

    assert!(client.create_component(&component).await.is_ok());
    assert!(client.update_component("c1", &component).await.is_ok());
    assert!(client.delete_component("c1").await.is_ok());
}

#[tokio::test]
async fn test_get_keys() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let keys = client.get_keys().await.expect("Failed to get keys");
    let keys_list = keys.keys.expect("Failed to get keys list");
    assert_eq!(keys_list.len(), 1);
    assert_eq!(keys_list[0].kid, Some("key-1".to_string()));
}

#[tokio::test]
async fn test_unauthenticated_error() {
    let mock_url = start_mock_server().await;
    let client = KeycloakClient::new(mock_url);
    let result = client.get_realms().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Not authenticated")
    );
}

#[tokio::test]
async fn test_delete_client() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let result = client.delete_client("1").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_login_no_credentials() {
    let mut client = KeycloakClient::new("http://localhost".to_string());
    let result = client.login("admin-cli", None, None, None).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Either username/password or client_secret must be provided")
    );
}

#[tokio::test]
async fn test_register_required_action_no_name() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let action = kaji::models::RequiredActionProviderRepresentation {
        alias: Some("action-2".to_string()),
        name: None,
        provider_id: Some("action-provider-2".to_string()),
        enabled: Some(true),
        default_action: Some(false),
        priority: Some(10),
        config: None,
        extra: std::collections::HashMap::new(),
    };

    assert!(client.register_required_action(&action).await.is_ok());
}

#[tokio::test]
async fn test_register_required_action_no_provider_id() {
    let client = KeycloakClient::new("http://localhost".to_string());
    let action = kaji::models::RequiredActionProviderRepresentation {
        alias: Some("action-2".to_string()),
        name: None,
        provider_id: None,
        enabled: Some(true),
        default_action: Some(false),
        priority: Some(10),
        config: None,
        extra: std::collections::HashMap::new(),
    };

    let result = client.register_required_action(&action).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Provider ID required for registration")
    );
}

#[tokio::test]
async fn test_post_send_failure() {
    let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
    client.set_token("mock_token".to_string());
    let client_rep = ClientRepresentation {
        id: None,
        client_id: Some("test".to_string()),
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
    let result = client.create_client(&client_rep).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to send POST request")
    );
}

#[tokio::test]
async fn test_delete_send_failure() {
    let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
    client.set_token("mock_token".to_string());
    let result = client.delete_client("test").await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to send DELETE request")
    );
}

#[tokio::test]
async fn test_api_error() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("non-existent".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let result = client.get_realm().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("GET request failed")
    );
}

#[tokio::test]
async fn test_map_flow_executions() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("flow-1".to_string()),
        description: None,
        provider_id: None,
        top_level: None,
        built_in: None,
        authentication_executions: Some(vec![
            kaji::models::AuthenticationExecutionExportRepresentation {
                id: None,
                authenticator: None,
                authenticator_config: Some("config-1".to_string()),
                requirement: None,
                priority: None,
                authenticator_flow: None,
                flow_alias: None,
                user_setup_allowed: None,
                extra: std::collections::HashMap::new(),
            },
        ]),
        extra: std::collections::HashMap::new(),
    };

    let mapped_flow = client.map_flow_executions(flow).await;
    let executions = mapped_flow.authentication_executions.unwrap();
    assert_eq!(executions.len(), 1);
    assert_eq!(
        executions[0].authenticator_config,
        Some("review profile config".to_string())
    );
}

#[tokio::test]
async fn test_unmap_flow_executions() {
    let mock_url = start_mock_server().await;
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client
        .login("admin-cli", Some("secret"), None, None)
        .await
        .expect("Login failed");

    let flow = kaji::models::AuthenticationFlowRepresentation {
        id: None,
        alias: Some("flow-1".to_string()),
        description: None,
        provider_id: None,
        top_level: None,
        built_in: None,
        authentication_executions: Some(vec![
            kaji::models::AuthenticationExecutionExportRepresentation {
                id: None,
                authenticator: None,
                authenticator_config: Some("review profile config".to_string()),
                requirement: None,
                priority: None,
                authenticator_flow: None,
                flow_alias: None,
                user_setup_allowed: None,
                extra: std::collections::HashMap::new(),
            },
            kaji::models::AuthenticationExecutionExportRepresentation {
                id: None,
                authenticator: None,
                authenticator_config: Some("non-uuid-unknown".to_string()),
                requirement: None,
                priority: None,
                authenticator_flow: None,
                flow_alias: None,
                user_setup_allowed: None,
                extra: std::collections::HashMap::new(),
            },
            kaji::models::AuthenticationExecutionExportRepresentation {
                id: None,
                authenticator: None,
                authenticator_config: Some("123e4567-e89b-12d3-a456-426614174000".to_string()),
                requirement: None,
                priority: None,
                authenticator_flow: None,
                flow_alias: None,
                user_setup_allowed: None,
                extra: std::collections::HashMap::new(),
            },
        ]),
        extra: std::collections::HashMap::new(),
    };

    let unmapped_flow = client.unmap_flow_executions(flow).await;
    let executions = unmapped_flow.authentication_executions.unwrap();
    assert_eq!(executions.len(), 3);
    assert_eq!(
        executions[0].authenticator_config,
        Some("config-1".to_string())
    );
    assert_eq!(
        executions[1].authenticator_config,
        None
    );
    assert_eq!(
        executions[2].authenticator_config,
        Some("123e4567-e89b-12d3-a456-426614174000".to_string())
    );
}
