use kaji::apply;
use kaji::client::KeycloakClient;
use kaji::models::{AuthenticationExecutionExportRepresentation, AuthenticationFlowRepresentation};
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::MockUi;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn test_serde_execution_alias_compatibility() {
    let json = serde_json::json!({
        "id": "exec-1",
        "displayName": "shared-mfa-flow",
        "authenticationFlow": true,
        "requirement": "REQUIRED",
        "priority": 10
    });

    let exec: AuthenticationExecutionExportRepresentation =
        serde_json::from_value(json).expect("Failed to deserialize execution");

    assert_eq!(exec.flow_alias.as_deref(), Some("shared-mfa-flow"));
    assert_eq!(exec.authenticator_flow, Some(true));
    assert_eq!(exec.requirement.as_deref(), Some("REQUIRED"));

    let flow = AuthenticationFlowRepresentation {
        id: Some("f1".to_string()),
        alias: Some("browser-flow".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(true),
        built_in: Some(false),
        authentication_executions: Some(vec![exec]),
        extra: HashMap::new(),
    };

    assert_eq!(flow.subflow_aliases(), vec!["shared-mfa-flow".to_string()]);
    assert!(!flow.is_subflow());

    let subflow = AuthenticationFlowRepresentation {
        id: Some("f2".to_string()),
        alias: Some("shared-mfa-flow".to_string()),
        description: None,
        provider_id: Some("basic-flow".to_string()),
        top_level: Some(false),
        built_in: Some(false),
        authentication_executions: None,
        extra: HashMap::new(),
    };
    assert!(subflow.is_subflow());
}

#[tokio::test]
async fn test_apply_auth_flow_409_conflict_auto_adoption() {
    let mut server = mockito::Server::new_async().await;
    let mock_url = server.url();
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock-token".to_string());

    // 1. Initial get_resources for AuthenticationFlowRepresentation returns empty (not in Keycloak yet)
    let _m_initial_get = server
        .mock("GET", "/admin/realms/test-realm/authentication/flows")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("[]")
        .create_async()
        .await;

    // 2. create_resource fails with 409 Conflict: "Flow sub-flow already exists"
    let _m_post_conflict = server
        .mock("POST", "/admin/realms/test-realm/authentication/flows")
        .with_status(409)
        .with_header("content-type", "application/json")
        .with_body(r#"{"errorMessage": "Flow sub-flow already exists"}"#)
        .create_async()
        .await;

    // 3. get_resources after cache invalidation discovers the auto-created flow with ID "remote-flow-id"
    let _m_subsequent_get = server
        .mock("GET", "/admin/realms/test-realm/authentication/flows")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {
                    "id": "remote-flow-id",
                    "alias": "sub-flow",
                    "providerId": "basic-flow",
                    "topLevel": false,
                    "builtIn": false
                }
            ]"#,
        )
        .create_async()
        .await;

    // 4. Update the adopted resource via PUT
    let _m_put = server
        .mock(
            "PUT",
            "/admin/realms/test-realm/authentication/flows/remote-flow-id",
        )
        .with_status(204)
        .create_async()
        .await;

    // 5. Enrichment fetch for the adopted flow
    let _m_enrich = server
        .mock(
            "GET",
            "/admin/realms/test-realm/authentication/flows/remote-flow-id",
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "id": "remote-flow-id",
                "alias": "sub-flow",
                "providerId": "basic-flow",
                "topLevel": false,
                "builtIn": false
            }"#,
        )
        .create_async()
        .await;

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    let flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir_all(&flows_dir).unwrap();

    let subflow_yaml = r#"
alias: sub-flow
providerId: basic-flow
topLevel: false
"#;
    fs::write(flows_dir.join("sub-flow.yaml"), subflow_yaml).unwrap();

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    let ctx = kaji::apply::ApplyContext {
        client: &client,
        workspace_dir: realm_dir.clone(),
        secrets_path: Arc::new(realm_dir.join(".secrets")),
        resolver,
        planned_files: Arc::new(None),
        realm_name: "test-realm",
        profile: None,
        review: false,
        ui,
        yes: true,
        prune: false,
        prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
    };

    // This must succeed by adopting the existing flow after 409 Conflict instead of failing!
    let result = apply::generic::apply_resources::<AuthenticationFlowRepresentation>(ctx).await;
    assert!(
        result.is_ok(),
        "Expected apply_resources to succeed with 409 auto-adoption, but got: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_apply_shared_flows_topological_staging() {
    let mut server = mockito::Server::new_async().await;
    let mock_url = server.url();
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock-token".to_string());

    // Initial get flows
    let _m_get_flows = server
        .mock("GET", "/admin/realms/test-realm/authentication/flows")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("[]")
        .create_async()
        .await;

    // POST calls for flows
    let _m_post_flows = server
        .mock("POST", "/admin/realms/test-realm/authentication/flows")
        .with_status(201)
        .expect_at_least(3)
        .create_async()
        .await;

    // Follow-up GET calls to resolve IDs
    let _m_fresh_flows = server
        .mock("GET", "/admin/realms/test-realm/authentication/flows")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"[
                {"id": "id-shared", "alias": "shared-mfa", "providerId": "basic-flow"},
                {"id": "id-browser", "alias": "browser-flow", "providerId": "basic-flow"},
                {"id": "id-direct", "alias": "direct-grant-flow", "providerId": "basic-flow"}
            ]"#,
        )
        .create_async()
        .await;

    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let realm_dir = workspace_dir.join("test-realm");
    let flows_dir = realm_dir.join("authentication-flows");
    fs::create_dir_all(&flows_dir).unwrap();

    // 1. Shared sub-flow (Tier 0)
    fs::write(
        flows_dir.join("shared-mfa.yaml"),
        r#"
alias: shared-mfa
providerId: basic-flow
topLevel: false
"#,
    )
    .unwrap();

    // 2. Parent flow 1 referencing shared-mfa (Tier 1)
    fs::write(
        flows_dir.join("browser-flow.yaml"),
        r#"
alias: browser-flow
providerId: basic-flow
topLevel: true
authenticationExecutions:
  - authenticatorFlow: true
    flowAlias: shared-mfa
    requirement: ALTERNATIVE
"#,
    )
    .unwrap();

    // 3. Parent flow 2 referencing shared-mfa (Tier 1)
    fs::write(
        flows_dir.join("direct-grant-flow.yaml"),
        r#"
alias: direct-grant-flow
providerId: basic-flow
topLevel: true
authenticationExecutions:
  - authenticatorFlow: true
    flowAlias: shared-mfa
    requirement: REQUIRED
"#,
    )
    .unwrap();

    let ui = Arc::new(MockUi {
        inputs: std::sync::Mutex::new(vec![]),
        confirms: std::sync::Mutex::new(vec![]),
        selects: std::sync::Mutex::new(vec![]),
        passwords: std::sync::Mutex::new(vec![]),
    });
    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

    let ctx = kaji::apply::ApplyContext {
        client: &client,
        workspace_dir: realm_dir.clone(),
        secrets_path: Arc::new(realm_dir.join(".secrets")),
        resolver,
        planned_files: Arc::new(None),
        realm_name: "test-realm",
        profile: None,
        review: false,
        ui,
        yes: true,
        prune: false,
        prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
    };

    let result = apply::generic::apply_resources::<AuthenticationFlowRepresentation>(ctx).await;
    assert!(
        result.is_ok(),
        "Expected apply_resources to succeed with shared flows, got: {:?}",
        result.err()
    );
}
