use crate::models::GroupRepresentation;
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn create_group_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let name = ui.input("Group Name", None, false)?;

    create_group_yaml(workspace_dir, &realm, &name).await?;

    ui.print_success(&format!(
        "Successfully generated YAML for group '{}' in realm '{}'.",
        name, realm
    ));
    Ok(())
}

pub async fn create_group_yaml(workspace_dir: &Path, realm: &str, name: &str) -> Result<()> {
    let group = GroupRepresentation {
        id: None,
        name: Some(name.to_string()),
        path: None,
        sub_groups: None,
        extra: HashMap::new(),
    };

    let realm_dir = workspace_dir.join(sanitize(realm)).join("groups");
    fs::create_dir_all(&realm_dir)
        .await
        .context("Failed to create groups directory")?;

    let file_path = realm_dir.join(format!("{}.yaml", sanitize(name)));
    let yaml = serde_yaml::to_string(&group).context("Failed to serialize group to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write group YAML file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_group_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_group_yaml(workspace_dir, "master", "my-group")
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("groups")
            .join("my-group.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let group: GroupRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(group.name.as_deref(), Some("my-group"));
    }

    #[tokio::test]
    async fn test_create_group_yaml_path_traversal() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        // Attempt path traversal in realm and name
        create_group_yaml(workspace_dir, "../danger", "evil/../../cmd")
            .await
            .unwrap();

        // The sanitized realm should be "danger" (or similar depending on sanitize behavior)
        // and sanitized name should be "evil..cmd" or similar.
        // Important thing is it MUST stay under workspace_dir.

        let sanitized_realm = sanitize("../danger");
        let sanitized_name = sanitize("evil/../../cmd");

        let expected_path = workspace_dir
            .join(sanitized_realm)
            .join("groups")
            .join(format!("{}.yaml", sanitized_name));

        assert!(expected_path.exists());
        assert!(expected_path.starts_with(workspace_dir));
    }
}
