use crate::models::ComponentRepresentation;
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

pub async fn rotate_keys_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;

    let rotated_count = rotate_keys_yaml(workspace_dir, &realm).await?;

    if rotated_count > 0 {
        ui.print_success(&format!(
            "Successfully generated {} rotated key component(s) for realm '{}'.",
            rotated_count, realm
        ));
    } else {
        ui.print_info(&format!(
            "No key providers found to rotate for realm '{}'.",
            realm
        ));
    }

    Ok(())
}

pub async fn rotate_keys_yaml(workspace_dir: &Path, realm: &str) -> Result<usize> {
    let keys_dir = workspace_dir.join(sanitize(realm)).join("components");

    let mut rotated_count = 0;
    let mut entries = match fs::read_dir(&keys_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(anyhow::Error::from(e).context("Failed to read components directory")),
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .context("Failed to read directory entry")?
    {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            let yaml_content = fs::read_to_string(&path)
                .await
                .context("Failed to read key YAML file")?;

            #[allow(clippy::collapsible_if)]
            if let Ok(component) = serde_yaml::from_str::<ComponentRepresentation>(&yaml_content) {
                if component.provider_type.as_deref() == Some("org.keycloak.keys.KeyProvider") {
                    let mut new_component = component.clone();
                    new_component.id = None;

                    let old_name = new_component
                        .name
                        .clone()
                        .unwrap_or_else(|| "key".to_string());
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .context("System clock is before UNIX EPOCH")?
                        .as_secs();
                    new_component.name = Some(format!("{}-rotated-{}", old_name, timestamp));

                    if let Some(config) = &mut new_component.config
                        && let Some(priority_vals) = config.get_mut("priority")
                        && let Some(arr) = priority_vals.as_array_mut()
                        && let Some(first) = arr.first_mut()
                        && let Some(p_str) = first.as_str()
                        && let Ok(p_num) = p_str.parse::<i64>()
                    {
                        *first = serde_json::Value::String((p_num + 10).to_string());
                    }

                    let new_filename = format!(
                        "{}.yaml",
                        new_component
                            .name
                            .as_deref()
                            .context("Missing component name after rotation")?
                    );
                    let new_file_path = keys_dir.join(new_filename);

                    let yaml = serde_yaml::to_string(&new_component)
                        .context("Failed to serialize rotated key to YAML")?;
                    crate::utils::write_secure(&new_file_path, &yaml)
                        .await
                        .context("Failed to write rotated key YAML file")?;

                    rotated_count += 1;
                }
            }
        }
    }

    Ok(rotated_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_rotate_keys_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        let keys_dir = workspace_dir.join("master").join("components");
        fs::create_dir_all(&keys_dir).await.unwrap();

        let original_component = ComponentRepresentation {
            id: None,
            name: Some("rsa-generated".to_string()),
            provider_id: Some("rsa-generated".to_string()),
            provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
            parent_id: Some("master".to_string()),
            sub_type: None,
            config: Some({
                let mut map = HashMap::new();
                map.insert("priority".to_string(), serde_json::json!(["100"]));
                map
            }),
            extra: HashMap::new(),
        };

        let original_yaml = serde_yaml::to_string(&original_component).unwrap();
        fs::write(keys_dir.join("rsa-generated.yaml"), original_yaml)
            .await
            .unwrap();

        let count = rotate_keys_yaml(workspace_dir, "master").await.unwrap();
        assert_eq!(count, 1);

        let mut entries = fs::read_dir(&keys_dir).await.unwrap();
        let mut found_rotated = false;

        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("rsa-generated-rotated-") {
                found_rotated = true;
                let content = fs::read_to_string(entry.path()).await.unwrap();
                let rotated: ComponentRepresentation = serde_yaml::from_str(&content).unwrap();

                let config = rotated.config.unwrap();
                let priority_array = config.get("priority").unwrap().as_array().unwrap();
                assert_eq!(priority_array[0].as_str().unwrap(), "110");
            }
        }

        assert!(found_rotated, "Did not find a rotated key component file");
    }

    #[tokio::test]
    async fn test_rotate_keys_yaml_no_dir() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let count = rotate_keys_yaml(workspace_dir, "master").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_rotate_keys_yaml_no_yaml_files() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let keys_dir = workspace_dir.join("master").join("components");
        fs::create_dir_all(&keys_dir).await.unwrap();
        fs::write(keys_dir.join("test.txt"), "not a yaml")
            .await
            .unwrap();

        let count = rotate_keys_yaml(workspace_dir, "master").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_rotate_keys_yaml_invalid_priority() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let keys_dir = workspace_dir.join("master").join("components");
        fs::create_dir_all(&keys_dir).await.unwrap();

        let component = ComponentRepresentation {
            id: None,
            name: Some("rsa".to_string()),
            provider_id: Some("rsa".to_string()),
            provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
            parent_id: Some("master".to_string()),
            sub_type: None,
            config: Some({
                let mut map = HashMap::new();
                map.insert("priority".to_string(), serde_json::json!(["invalid"]));
                map
            }),
            extra: HashMap::new(),
        };

        let yaml = serde_yaml::to_string(&component).unwrap();
        fs::write(keys_dir.join("rsa.yaml"), yaml).await.unwrap();

        let count = rotate_keys_yaml(workspace_dir, "master").await.unwrap();
        assert_eq!(count, 1); // It still rotates, but priority won't be updated

        let mut entries = fs::read_dir(&keys_dir).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("rsa-rotated-") {
                let content = fs::read_to_string(entry.path()).await.unwrap();
                let rotated: ComponentRepresentation = serde_yaml::from_str(&content).unwrap();
                let config = rotated.config.unwrap();
                let priority_array = config.get("priority").unwrap().as_array().unwrap();
                assert_eq!(priority_array[0].as_str().unwrap(), "invalid");
            }
        }
    }

    #[tokio::test]
    async fn test_rotate_keys_yaml_not_key_provider() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let keys_dir = workspace_dir.join("master").join("components");
        fs::create_dir_all(&keys_dir).await.unwrap();

        let component = ComponentRepresentation {
            id: None,
            name: Some("not-key".to_string()),
            provider_id: Some("not-key".to_string()),
            provider_type: Some("something.else".to_string()),
            parent_id: Some("master".to_string()),
            sub_type: None,
            config: None,
            extra: HashMap::new(),
        };

        let yaml = serde_yaml::to_string(&component).unwrap();
        fs::write(keys_dir.join("not-key.yaml"), yaml)
            .await
            .unwrap();

        let count = rotate_keys_yaml(workspace_dir, "master").await.unwrap();
        assert_eq!(count, 0);
    }
}
