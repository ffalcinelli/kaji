mod common;
use kaji::client::KeycloakClient;

#[tokio::test]
async fn test_client_error_handling() {
    let mut server = mockito::Server::new_async().await;
    let mock_url = server.url();
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock-token".to_string());

    // 1. Test 500 Internal Server Error for get_realm
    let _m = server
        .mock("GET", "/admin/realms/test-realm")
        .with_status(500)
        .create_async()
        .await;

    let res = client.get_realm().await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("500"));

    // 2. Test Network error (invalid URL)
    let bad_client = KeycloakClient::new("http://invalid.url.that.does.not.exist".to_string());
    let res = bad_client.get_realm().await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_client_get_all_error() {
    let mut server = mockito::Server::new_async().await;
    let mock_url = server.url();
    let mut client = KeycloakClient::new(mock_url);
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock-token".to_string());

    let _m = server
        .mock("GET", "/admin/realms/test-realm/clients")
        .with_status(500)
        .create_async()
        .await;

    let err = client.get_clients().await.unwrap_err();
    assert_eq!(
        kaji::error::http_status(&err),
        Some(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
    );
    let msg = err.to_string();
    assert!(
        msg.starts_with("GET http") && msg.contains("/admin/realms/test-realm/clients failed: 500"),
        "{msg}"
    );
}

#[tokio::test]
async fn test_client_timeout_error() {
    let mut server = mockito::Server::new_async().await;
    let mock_url = server.url();
    let mut client =
        KeycloakClient::new(mock_url).with_timeout(std::time::Duration::from_millis(50));
    client.set_target_realm("test-realm".to_string());
    client.set_token("mock-token".to_string());

    let _m = server
        .mock("GET", "/admin/realms/test-realm")
        .with_status(200)
        .with_chunked_body(|w| {
            std::thread::sleep(std::time::Duration::from_millis(200));
            w.write_all(b"{\"id\":\"test\"}")
        })
        .create_async()
        .await;

    let res = client.get_realm().await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    let debug_msg = format!("{:?}", err).to_lowercase();
    assert!(
        debug_msg.contains("timeout")
            || debug_msg.contains("timed out")
            || debug_msg.contains("failed to send get request"),
        "Error debug message was: {}",
        debug_msg
    );
}

/// Mocks a realm whose top-level flow `browser-custom` declares `executions`, and whose
/// executions endpoint fails like Keycloak does when a provider jar is missing.
async fn missing_provider_server(executions: serde_json::Value) -> mockito::ServerGuard {
    let mut server = mockito::Server::new_async().await;
    let flows = serde_json::json!([
        {"id": "f1", "alias": "browser-custom", "topLevel": true, "builtIn": false,
         "providerId": "basic-flow", "authenticationExecutions": executions},
        {"id": "f2", "alias": "browser", "topLevel": true, "builtIn": true,
         "providerId": "basic-flow", "authenticationExecutions": []}
    ]);
    server
        .mock("GET", "/admin/realms/r/authentication/flows")
        .with_status(200)
        .with_body(flows.to_string())
        .create_async()
        .await;
    server
        .mock(
            "GET",
            "/admin/realms/r/authentication/flows/browser-custom/executions",
        )
        .with_status(404)
        .with_body(r#"{"error":"Could not find authenticator provider"}"#)
        .create_async()
        .await;
    server
        .mock(
            "GET",
            "/admin/realms/r/authentication/flows/browser/executions",
        )
        .with_status(200)
        .with_body("[]")
        .create_async()
        .await;
    for (kind, ids) in [
        ("authenticator-providers", r#"[{"id":"auth-cookie"}]"#),
        ("client-authenticator-providers", "[]"),
        ("form-providers", r#"[{"id":"registration-page-form"}]"#),
        ("form-action-providers", "[]"),
    ] {
        server
            .mock(
                "GET",
                format!("/admin/realms/r/authentication/{}", kind).as_str(),
            )
            .with_status(200)
            .with_body(ids)
            .create_async()
            .await;
    }
    server
}

fn flows_client(server: &mockito::ServerGuard) -> KeycloakClient {
    let mut client = KeycloakClient::new(server.url());
    client.set_target_realm("r".to_string());
    client.set_token("mock-token".to_string());
    client
}

#[tokio::test]
async fn test_missing_authenticator_provider_is_named() {
    let server = missing_provider_server(serde_json::json!([
        {"authenticator": "auth-cookie", "requirement": "ALTERNATIVE", "priority": 10},
        {"authenticator": "my-otp-authenticator", "requirement": "REQUIRED", "priority": 20}
    ]))
    .await;
    let client = flows_client(&server);

    let err = client
        .get_resources::<kaji::models::AuthenticationFlowRepresentation>()
        .await
        .unwrap_err();
    assert!(kaji::client::is_not_found(&err));
    let rendered = kaji::utils::ui::format_error(&err);
    assert!(
        rendered.contains(
            "Authentication flow 'browser-custom' uses authenticator providers not installed on the server: my-otp-authenticator"
        ),
        "{rendered}"
    );
    assert!(
        rendered.contains(
            "/admin/realms/r/authentication/flows/browser-custom/executions failed: 404 Not Found - Could not find authenticator provider"
        ),
        "{rendered}"
    );
    assert!(
        rendered.ends_with(
            "Hint: install the provider jar on the Keycloak server, or remove these executions from the flow"
        ),
        "{rendered}"
    );
}

#[tokio::test]
async fn test_missing_provider_in_sub_flow_lists_sub_flows() {
    let server = missing_provider_server(serde_json::json!([
        {"authenticator": "auth-cookie", "requirement": "ALTERNATIVE", "priority": 10},
        {"flowAlias": "otp-forms", "authenticatorFlow": true, "requirement": "ALTERNATIVE",
         "priority": 20}
    ]))
    .await;
    let client = flows_client(&server);

    let err = client
        .get_resources::<kaji::models::AuthenticationFlowRepresentation>()
        .await
        .unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains(
            "Failed to list the executions of authentication flow 'browser-custom': an execution in it or in its sub-flows (otp-forms) may use an authenticator provider that is not installed on the server"
        ),
        "{msg}"
    );
}
