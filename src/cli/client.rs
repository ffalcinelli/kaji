use crate::models::{ClientRepresentation, ClientScopeRepresentation};
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn create_client_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let client_id = ui.input("Client ID", None, false)?;
    let is_public = ui.confirm("Is this a public client? (No for confidential)", true)?;

    create_client_yaml(workspace_dir, &realm, &client_id, is_public).await?;

    ui.print_success(&format!(
        "Successfully generated YAML for client '{}' in realm '{}'.",
        client_id, realm
    ));
    Ok(())
}

pub async fn create_client_yaml(
    workspace_dir: &Path,
    realm: &str,
    client_id: &str,
    is_public: bool,
) -> Result<()> {
    let client = ClientRepresentation {
        id: None,
        client_id: Some(client_id.to_string()),
        name: None,
        description: None,
        enabled: Some(true),
        protocol: Some("openid-connect".to_string()),
        redirect_uris: Some(vec!["/*".to_string()]),
        web_origins: Some(vec!["+".to_string()]),
        public_client: Some(is_public),
        bearer_only: Some(false),
        service_accounts_enabled: Some(!is_public),
        extra: HashMap::new(),
    };

    let realm_dir = workspace_dir.join(sanitize(realm)).join("clients");
    fs::create_dir_all(&realm_dir)
        .await
        .context("Failed to create clients directory")?;

    let file_path = realm_dir.join(format!("{}.yaml", sanitize(client_id)));
    let yaml = serde_yaml::to_string(&client).context("Failed to serialize client to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write client YAML file")?;

    Ok(())
}

pub async fn create_client_scope_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let name = ui.input("Scope Name", None, false)?;
    let protocol = ui.input("Protocol", Some("openid-connect".to_string()), false)?;

    create_client_scope_yaml(workspace_dir, &realm, &name, &protocol).await?;

    ui.print_success(&format!(
        "Successfully generated YAML for client scope '{}' in realm '{}'.",
        name, realm
    ));
    Ok(())
}

pub async fn create_client_scope_yaml(
    workspace_dir: &Path,
    realm: &str,
    name: &str,
    protocol: &str,
) -> Result<()> {
    let scope = ClientScopeRepresentation {
        id: None,
        name: Some(name.to_string()),
        description: None,
        protocol: Some(protocol.to_string()),
        attributes: Some(HashMap::new()),
        extra: HashMap::new(),
    };

    let scopes_dir = workspace_dir.join(sanitize(realm)).join("client-scopes");
    fs::create_dir_all(&scopes_dir)
        .await
        .context("Failed to create client-scopes directory")?;

    let file_path = scopes_dir.join(format!("{}.yaml", sanitize(name)));
    let yaml = serde_yaml::to_string(&scope).context("Failed to serialize client scope to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write client scope YAML file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::ui::MockUi;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_client_interactive() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        let mock_ui = MockUi {
            inputs: std::sync::Mutex::new(vec!["master".to_string(), "my-client".to_string()]),
            confirms: std::sync::Mutex::new(vec![true]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        };

        create_client_interactive(workspace_dir, &mock_ui)
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("clients")
            .join("my-client.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let client: ClientRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(client.client_id.as_deref(), Some("my-client"));
        assert_eq!(client.public_client, Some(true));
    }

    #[tokio::test]
    async fn test_create_client_scope_interactive() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        let mock_ui = MockUi {
            inputs: std::sync::Mutex::new(vec![
                "master".to_string(),
                "my-scope".to_string(),
                "openid-connect".to_string(),
            ]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        };

        create_client_scope_interactive(workspace_dir, &mock_ui)
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("client-scopes")
            .join("my-scope.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let scope: ClientScopeRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(scope.name.as_deref(), Some("my-scope"));
        assert_eq!(scope.protocol.as_deref(), Some("openid-connect"));
    }

    #[tokio::test]
    async fn test_create_client_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_client_yaml(workspace_dir, "master", "testclient", true)
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("clients")
            .join("testclient.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let client: ClientRepresentation = serde_yaml::from_str(&content).unwrap();

        assert_eq!(client.client_id.as_deref(), Some("testclient"));
        assert_eq!(client.public_client, Some(true));
        assert_eq!(client.service_accounts_enabled, Some(false));

        // Confidential client
        create_client_yaml(workspace_dir, "master", "confidential", false)
            .await
            .unwrap();
        let file_path2 = workspace_dir
            .join("master")
            .join("clients")
            .join("confidential.yaml");
        let content2 = fs::read_to_string(&file_path2).await.unwrap();
        let client2: ClientRepresentation = serde_yaml::from_str(&content2).unwrap();
        assert_eq!(client2.public_client, Some(false));
        assert_eq!(client2.service_accounts_enabled, Some(true));
    }

    #[tokio::test]
    async fn test_create_client_scope_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_client_scope_yaml(workspace_dir, "master", "my-scope", "openid-connect")
            .await
            .unwrap();
        let file_path = workspace_dir
            .join("master")
            .join("client-scopes")
            .join("my-scope.yaml");
        assert!(file_path.exists());
        let content = fs::read_to_string(&file_path).await.unwrap();
        let scope: ClientScopeRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(scope.name.as_deref(), Some("my-scope"));
        assert_eq!(scope.protocol.as_deref(), Some("openid-connect"));
    }

    #[tokio::test]
    async fn test_create_client_scope_yaml_sanitization() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_client_scope_yaml(
            workspace_dir,
            "master",
            "../malicious/path",
            "openid-connect",
        )
        .await
        .unwrap();

        let sanitized_name = sanitize("../malicious/path");
        let file_path = workspace_dir
            .join("master")
            .join("client-scopes")
            .join(format!("{}.yaml", sanitized_name));

        assert!(file_path.exists());
        let content = fs::read_to_string(&file_path).await.unwrap();
        let scope: ClientScopeRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(scope.name.as_deref(), Some("../malicious/path"));
        assert_eq!(scope.protocol.as_deref(), Some("openid-connect"));
    }
}
