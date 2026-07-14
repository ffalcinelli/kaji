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

    let res = client.get_clients().await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("GET request failed"));
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
