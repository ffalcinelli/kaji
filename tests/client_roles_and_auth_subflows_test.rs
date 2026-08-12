use anyhow::Result;
use kaji::apply;
use kaji::client::KeycloakClient;
use kaji::inspect;
use kaji::models::RoleRepresentation;
use kaji::utils::ui::MockUi;
use mockito::Server;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_client_roles_api_and_reconciliation() -> Result<()> {
    let mut server = Server::new_async().await;
    let url = server.url();

    // 0. Mock GET realm
    let _m_realm = server
        .mock("GET", "/admin/realms/test")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"realm": "test"}"#)
        .create_async()
        .await;

    // 1. Mock GET clients
    let _m_clients = server
        .mock("GET", "/admin/realms/test/clients")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {
                    "id": "client-uuid-123",
                    "clientId": "my-app"
                }
            ]"#,
        )
        .create_async()
        .await;

    // 2. Mock GET client roles
    let _m_client_roles = server
        .mock("GET", "/admin/realms/test/clients/client-uuid-123/roles")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {
                    "id": "role-uuid-456",
                    "name": "app-admin",
                    "clientRole": true,
                    "containerId": "client-uuid-123"
                }
            ]"#,
        )
        .create_async()
        .await;

    // 3. Mock GET realm roles & other resources
    let _m_realm_roles = server
        .mock("GET", "/admin/realms/test/roles")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[]"#)
        .create_async()
        .await;

    let _m_empty_scopes = server
        .mock("GET", "/admin/realms/test/client-scopes")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_idps = server
        .mock("GET", "/admin/realms/test/identity-provider/instances")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_groups = server
        .mock("GET", "/admin/realms/test/groups")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_users = server
        .mock("GET", "/admin/realms/test/users")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_flows = server
        .mock("GET", "/admin/realms/test/authentication/flows")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_actions = server
        .mock("GET", "/admin/realms/test/authentication/required-actions")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    let _m_empty_components = server
        .mock("GET", "/admin/realms/test/components")
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;

    // 4. Mock POST client role (create)
    let _m_create_role = server
        .mock("POST", "/admin/realms/test/clients/client-uuid-123/roles")
        .with_status(201)
        .create_async()
        .await;

    let mut client = KeycloakClient::new(url);
    client.set_target_realm("test".to_string());
    client.set_token("mock_token".to_string());

    // Test client methods
    let client_map = client.get_client_uuid_map().await?;
    assert_eq!(
        client_map.get("my-app"),
        Some(&"client-uuid-123".to_string())
    );

    let roles = client.get_client_roles("client-uuid-123").await?;
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].name, "app-admin");

    let new_role = RoleRepresentation {
        id: None,
        name: "app-user".to_string(),
        description: Some("App User".to_string()),
        container_id: None,
        composite: false,
        client_role: true,
        extra: std::collections::HashMap::new(),
    };

    client
        .create_client_role("client-uuid-123", &new_role)
        .await?;

    // Test inspect client roles
    let dir = tempdir()?;
    let workspace_dir = dir.path().to_path_buf();

    inspect::run(&client, workspace_dir.clone(), &["test".to_string()], true).await?;

    let exported_role = workspace_dir
        .join("test")
        .join("clients")
        .join("my-app")
        .join("roles")
        .join("app-admin.yaml");
    assert!(exported_role.exists());

    Ok(())
}

#[tokio::test]
async fn test_auth_subflows_and_executions_reconciliation() -> Result<()> {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Mock GET flows
    let _m_get_flows = server
        .mock("GET", "/admin/realms/test/authentication/flows")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {
                    "id": "flow-123",
                    "alias": "custom-browser",
                    "description": "Custom Browser Flow",
                    "providerId": "basic-flow",
                    "topLevel": true,
                    "builtIn": false
                }
            ]"#,
        )
        .create_async()
        .await;

    // Mock GET flow executions
    let _m_get_execs = server
        .mock(
            "GET",
            "/admin/realms/test/authentication/flows/custom-browser/executions",
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {
                    "id": "exec-111",
                    "authenticator": "auth-cookie",
                    "requirement": "ALTERNATIVE",
                    "priority": 10
                }
            ]"#,
        )
        .create_async()
        .await;

    // Mock POST add execution
    let _m_add_exec = server
        .mock(
            "POST",
            "/admin/realms/test/authentication/flows/custom-browser/executions/execution",
        )
        .with_status(201)
        .create_async()
        .await;

    // Mock POST add subflow
    let _m_add_subflow = server
        .mock(
            "POST",
            "/admin/realms/test/authentication/flows/custom-browser/executions/flow",
        )
        .with_status(201)
        .create_async()
        .await;

    // Mock PUT update flow
    let _m_put_flow = server
        .mock("PUT", "/admin/realms/test/authentication/flows/flow-123")
        .with_status(204)
        .create_async()
        .await;

    let mut client = KeycloakClient::new(url);
    client.set_target_realm("test".to_string());
    client.set_token("mock_token".to_string());

    let dir = tempdir()?;
    let workspace_dir = dir.path().join("test");
    let flows_dir = workspace_dir.join("authentication-flows");
    fs::create_dir_all(&flows_dir)?;

    let flow_yaml = r#"
alias: custom-browser
description: Custom Browser Flow Updated
providerId: basic-flow
topLevel: true
authenticationExecutions:
  - authenticator: auth-cookie
    requirement: REQUIRED
  - authenticator: auth-spnego
    requirement: ALTERNATIVE
  - authenticator: registration-page-form
    authenticatorFlow: true
    flowAlias: subflow-registration
    requirement: REQUIRED
"#;
    fs::write(flows_dir.join("custom-browser.yaml"), flow_yaml)?;

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(Vec::new()),
        confirms: std::sync::Mutex::new(Vec::new()),
        selects: std::sync::Mutex::new(Vec::new()),
        passwords: std::sync::Mutex::new(Vec::new()),
    });
    let resolver: Arc<dyn kaji::utils::secrets::SecretResolver> = Arc::new(
        kaji::utils::secrets::EnvResolver::new(std::collections::HashMap::new()),
    );

    // Apply flow
    apply::run(kaji::apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        realms_to_apply: &["test".to_string()],
        yes: true,
        review: false,
        prune: false,
        ui: ui.clone(),
        resolver: resolver.clone(),
        profile: None,
    })
    .await?;

    Ok(())
}
