use anyhow::Result;
use kaji::client::KeycloakClient;
use kaji::{apply, inspect, plan};
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

struct DockerComposeGuard;

impl DockerComposeGuard {
    fn new() -> Self {
        println!("Starting Keycloak with docker compose...");
        let status = Command::new("docker")
            .args(["compose", "up", "-d", "--wait"])
            .status()
            .expect("Failed to execute docker compose up");

        if !status.success() {
            panic!("docker compose up failed with status: {}", status);
        }
        Self
    }
}

impl Drop for DockerComposeGuard {
    fn drop(&mut self) {
        println!("Tearing down Keycloak...");
        let _ = Command::new("docker")
            .args(["compose", "down", "-v"])
            .status();
    }
}

async fn wait_for_keycloak() -> Result<KeycloakClient> {
    let mut client = KeycloakClient::new("http://localhost:8080".to_string());

    let mut attempts = 0;
    loop {
        match client
            .login("admin-cli", None, Some("admin"), Some("admin"))
            .await
        {
            Ok(_) => {
                println!("Successfully connected to Keycloak.");
                return Ok(client);
            }
            Err(e) => {
                attempts += 1;
                if attempts > 30 {
                    anyhow::bail!("Failed to connect to Keycloak after 30 attempts: {}", e);
                }
                println!("Waiting for Keycloak... ({}/30)", attempts);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

#[tokio::test]
async fn test_real_keycloak_integration() -> Result<()> {
    // 1. Bring up Keycloak
    let _guard = DockerComposeGuard::new();

    // 2. Wait for it to be ready and get a client
    let mut client = wait_for_keycloak().await?;
    client.set_target_realm("master".to_string());

    let dir = tempdir()?;
    let workspace_dir = dir.path().to_path_buf();

    // 3. Inspect the current (empty/default) state
    println!("Inspecting initial state...");
    inspect::run(
        &client,
        workspace_dir.clone(),
        &["master".to_string()],
        true,
    )
    .await?;

    let realm_file = workspace_dir.join("master").join("realm.yaml");
    assert!(realm_file.exists(), "realm.yaml should exist after inspect");

    // 4. Modify something (e.g. add a client)
    let clients_dir = workspace_dir.join("master").join("clients");
    fs::create_dir_all(&clients_dir)?;

    let new_client_yaml = r#"
clientId: integration-test-client
name: Integration Test Client
enabled: true
publicClient: true
standardFlowEnabled: true
"#;
    fs::write(
        clients_dir.join("integration-test-client.yaml"),
        new_client_yaml,
    )?;

    use kaji::utils::ui::DialoguerUi;

    // 5. Plan - Should see changes
    println!("Planning changes...");
    // Just ensuring plan runs without error
    let resolver = Arc::new(kaji::utils::secrets::EnvResolver::new(
        std::collections::HashMap::new(),
    )) as Arc<dyn kaji::utils::secrets::SecretResolver>;
    let ui = Arc::new(DialoguerUi::new());
    plan::run(
        &client,
        workspace_dir.clone(),
        true,
        false,
        &["master".to_string()],
        ui.clone(),
        resolver.clone(),
        None,
    )
    .await?;

    // 6. Apply the changes
    println!("Applying changes...");
    apply::run(
        &client,
        workspace_dir.clone(),
        &["master".to_string()],
        true,
        false,
        ui,
        resolver,
        None,
    )
    .await?;

    // 7. Verify the client was created by inspecting to a new dir
    let inspect_dir2 = dir.path().join("inspect2");
    println!("Inspecting applied state...");
    inspect::run(&client, inspect_dir2.clone(), &["master".to_string()], true).await?;

    // We might have a file named something like `integration-test-client.yaml` or whatever `sanitize` outputs
    // The id is not known, but the client_id is known. So there should be a file for it.
    let applied_clients_dir = inspect_dir2.join("master").join("clients");

    // Find the file containing `integration-test-client`
    let mut found = false;
    for entry in fs::read_dir(applied_clients_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let content = fs::read_to_string(&path)?;
            if content.contains("integration-test-client") {
                found = true;
                break;
            }
        }
    }

    assert!(
        found,
        "The newly created client was not found in the subsequent inspect."
    );

    println!("Integration test completed successfully!");
    Ok(())
}
