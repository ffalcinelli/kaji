use crate::models::RoleRepresentation;
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn create_role_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let name = ui.input("Role Name", None, false)?;
    let description = ui.input("Description", None, true)?;
    let is_client_role = ui.confirm("Is this a client role?", false)?;

    let client_id = if is_client_role {
        Some(ui.input("Client ID", None, false)?)
    } else {
        None
    };

    let description_opt = if description.is_empty() {
        None
    } else {
        Some(description)
    };

    create_role_yaml(workspace_dir, &realm, &name, description_opt, client_id).await?;

    ui.print_success(&format!(
        "Successfully generated YAML for role '{}' in realm '{}'.",
        name, realm
    ));
    Ok(())
}

pub async fn create_role_yaml(
    workspace_dir: &Path,
    realm: &str,
    name: &str,
    description: Option<String>,
    client_id: Option<String>,
) -> Result<()> {
    let role = RoleRepresentation {
        id: None,
        name: name.to_string(),
        description,
        container_id: None,
        composite: false,
        client_role: client_id.is_some(),
        extra: HashMap::new(),
    };

    let realm_dir = workspace_dir.join(sanitize(realm));
    let roles_dir = if let Some(cid) = &client_id {
        realm_dir.join("clients").join(sanitize(cid)).join("roles")
    } else {
        realm_dir.join("roles")
    };

    fs::create_dir_all(&roles_dir)
        .await
        .context("Failed to create roles directory")?;

    let file_path = roles_dir.join(format!("{}.yaml", sanitize(name)));
    let yaml = serde_yaml::to_string(&role).context("Failed to serialize role to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write role YAML file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_role_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        // Realm role
        create_role_yaml(
            workspace_dir,
            "master",
            "admin",
            Some("desc".to_string()),
            None,
        )
        .await
        .unwrap();
        let realm_role_path = workspace_dir
            .join("master")
            .join("roles")
            .join("admin.yaml");
        assert!(realm_role_path.exists());
        let content = fs::read_to_string(&realm_role_path).await.unwrap();
        let role: RoleRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(role.name, "admin");
        assert_eq!(role.description, Some("desc".to_string()));
        assert!(!role.client_role);

        // Client role
        create_role_yaml(
            workspace_dir,
            "master",
            "editor",
            None,
            Some("my-client".to_string()),
        )
        .await
        .unwrap();
        let client_role_path = workspace_dir
            .join("master")
            .join("clients")
            .join("my-client")
            .join("roles")
            .join("editor.yaml");
        assert!(client_role_path.exists());
        let content = fs::read_to_string(&client_role_path).await.unwrap();
        let role: RoleRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(role.name, "editor");
        assert!(role.client_role);
        assert_eq!(role.description, None);
    }
}
