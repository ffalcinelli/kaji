#![allow(clippy::collapsible_if)]
use crate::client::KeycloakClient;
use crate::models::{KeycloakResource, ResourceMeta};
use crate::utils::secrets::substitute_secrets;
pub use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE};
use crate::utils::ui::{Ui, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

#[allow(clippy::too_many_arguments)]
pub async fn apply_resources<T>(ctx: crate::apply::ApplyContext<'_>) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + crate::client::KeycloakResourceMapping
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Send
        + Sync
        + Clone
        + 'static,
{
    let crate::apply::ApplyContext {
        client,
        workspace_dir,
        secrets_path,
        resolver,
        planned_files,
        realm_name,
        profile,
        review,
        ui,
        yes,
        prune,
    } = ctx;

    let dir_name = T::DIR_NAME;
    let resources_dir = workspace_dir.join(dir_name);
    if !async_fs::try_exists(&resources_dir).await? {
        return Ok(());
    }

    let existing_resources = client
        .get_resources::<T>()
        .await
        .with_context(|| format!("Failed to get {} for realm '{}'", T::LABEL, realm_name))?;

    let existing_map: HashMap<String, String> = existing_resources
        .iter()
        .filter_map(|r| {
            let identity = r.get_identity();
            let id = r.get_id();
            match (identity, id) {
                (Some(identity), Some(id)) => Some((identity, id.to_string())),
                _ => None,
            }
        })
        .collect();
    let existing_map = Arc::new(existing_map);

    let mut entries = async_fs::read_dir(&resources_dir).await?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if planned_files
            .as_ref()
            .as_ref()
            .is_some_and(|plan| !plan.contains(&path))
        {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        // Skip overlay files themselves
        if is_overlay_file(&path, profile.as_deref()) {
            continue;
        }
        files.push(path);
    }

    if files.is_empty() {
        return Ok(());
    }

    let pb = create_progress_bar(files.len() as u64, &format!("Applying {}", T::LABEL));
    let mut set = JoinSet::new();

    for path in files {
        let client = client.clone();
        let existing_map = Arc::clone(&existing_map);
        let resolver = Arc::clone(&resolver);
        let realm_name = realm_name.to_string();
        let profile = profile.clone();
        let ui = Arc::clone(&ui);
        let pb = pb.clone();
        let secrets_path = Arc::clone(&secrets_path);

        set.spawn(async move {
            let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            let local_val_before_sub = val.clone();
            substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
            let mut rep: T = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

            let identity = rep.get_identity().with_context(|| {
                format!("Failed to get identity for {} in {:?}", T::LABEL, path)
            })?;

            let id_opt = existing_map.get(&identity);

            if review {
                let action = if id_opt.is_some() { "update" } else { "create" };
                let proceed = ui.confirm(
                    &format!(
                        "Do you want to {} {} '{}'?",
                        action,
                        T::LABEL,
                        rep.get_name()
                    ),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    return Ok::<(), anyhow::Error>(());
                }
            }

            let mut final_id = None;
            if let Some(id) = id_opt {
                rep.set_id(Some(id.clone()));
                client.update_resource(id, &rep).await.with_context(|| {
                    format!(
                        "Failed to update {} '{}' in realm '{}'",
                        T::LABEL,
                        rep.get_name(),
                        realm_name
                    )
                })?;
                pb.println(format!(
                    "  {} Updated {} {}",
                    SUCCESS_UPDATE,
                    T::LABEL,
                    rep.get_name()
                ));
                final_id = Some(id.clone());
            } else {
                rep.set_id(None);
                client.create_resource(&rep).await.with_context(|| {
                    format!(
                        "Failed to create {} '{}' in realm '{}'",
                        T::LABEL,
                        rep.get_name(),
                        realm_name
                    )
                })?;
                pb.println(format!(
                    "  {} Created {} {}",
                    SUCCESS_CREATE,
                    T::LABEL,
                    rep.get_name()
                ));

                // Fetch resources to get the generated ID of the created resource
                let fresh_resources = client.get_resources::<T>().await?;
                if let Some(fresh) = fresh_resources
                    .into_iter()
                    .find(|r| r.get_identity() == Some(identity.clone()))
                {
                    if let Some(id) = fresh.get_id() {
                        final_id = Some(id.to_string());
                    }
                }
            }

            if let Some(id) = final_id {
                if let Ok(enriched) = client.get_resource::<T>(&id).await {
                    check_and_update_enrichment(
                        &client,
                        &path,
                        &local_val_before_sub,
                        &enriched,
                        &realm_name,
                        &secrets_path,
                        &*ui,
                        yes,
                    )
                    .await?;
                }
            }

            pb.inc(1);
            Ok::<(), anyhow::Error>(())
        });
    }

    crate::utils::join_all_tasks(set, None).await?;
    pb.finish_with_message(format!("Applied {}", T::LABEL));

    if prune {
        let mut declared = HashSet::new();
        if async_fs::try_exists(&resources_dir).await? {
            let mut entries = async_fs::read_dir(&resources_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "yaml") {
                    continue;
                }
                if is_overlay_file(&path, profile.as_deref()) {
                    continue;
                }
                if let Ok(content) = async_fs::read_to_string(&path).await {
                    if let Ok(val) = serde_yaml::from_str::<serde_json::Value>(&content) {
                        if let Ok(rep) = serde_json::from_value::<T>(val) {
                            if let Some(identity) = rep.get_identity() {
                                declared.insert(identity);
                            }
                        }
                    }
                }
            }
        }

        for remote in &existing_resources {
            if let (Some(identity), Some(id)) = (remote.get_identity(), remote.get_id()) {
                if !declared.contains(&identity) {
                    if is_protected_resource::<T>(&identity, realm_name) {
                        continue;
                    }

                    let proceed = if yes {
                        true
                    } else {
                        ui.confirm(
                            &format!("Prune/Delete remote {} '{}'?", T::LABEL, remote.get_name()),
                            false,
                        )?
                    };

                    if proceed {
                        client.delete_resource::<T>(id).await.with_context(|| {
                            format!("Failed to prune {} '{}'", T::LABEL, remote.get_name())
                        })?;
                        eprintln!("  Removed/Pruned {} {}", T::LABEL, remote.get_name());
                    }
                }
            }
        }
    }

    Ok(())
}

fn is_protected_resource<T>(identity: &str, realm_name: &str) -> bool
where
    T: KeycloakResource,
{
    let path = T::API_PATH;
    if path == "clients" {
        let protected = [
            "admin-cli",
            "security-admin-console",
            "account",
            "account-console",
            "broker",
            "realm-management",
        ];
        protected.contains(&identity)
    } else if path == "roles" {
        let default_role = format!("default-roles-{}", realm_name);
        let protected = ["offline_access", "uma_authorization", &default_role];
        protected.contains(&identity)
    } else if path == "client-scopes" {
        let protected = [
            "profile",
            "email",
            "address",
            "phone",
            "offline_access",
            "roles",
            "web-origins",
            "microprofile-jwt",
        ];
        protected.contains(&identity)
    } else if path == "authentication/flows" {
        let protected = [
            "browser",
            "direct grant",
            "registration",
            "registration form",
            "reset credentials",
            "clients",
            "first broker login",
            "saml ecp",
            "docker auth",
            "http challenge",
        ];
        protected.contains(&identity)
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn check_and_update_enrichment<T>(
    _client: &KeycloakClient,
    path: &std::path::Path,
    local_val_before_sub: &serde_json::Value,
    enriched: &T,
    realm_name: &str,
    secrets_path: &std::path::Path,
    ui: &dyn Ui,
    yes: bool,
) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Clone,
{
    let mut placeholders = std::collections::HashMap::new();
    let mut path_buf = String::with_capacity(128);
    find_placeholders(local_val_before_sub, &mut path_buf, &mut placeholders);

    let mut enriched_val = serde_json::to_value(enriched.clone())?;

    let mut new_secrets = std::collections::BTreeMap::new();
    let prefix = format!("realm_{}_{}", realm_name, T::SECRET_PREFIX);
    crate::utils::secrets::extract_secrets(&mut enriched_val, &prefix, &mut new_secrets);

    for (path_str, placeholder) in &placeholders {
        set_value_at_path(
            &mut enriched_val,
            path_str,
            serde_json::Value::String(placeholder.clone()),
        );
    }

    let mut sorted_local_val = local_val_before_sub.clone();
    crate::utils::recursive_sort(&mut sorted_local_val);
    let local_yaml = serde_yaml::to_string(&sorted_local_val)?;

    crate::utils::recursive_sort(&mut enriched_val);
    let enriched_yaml = serde_yaml::to_string(&enriched_val)?;

    if local_yaml != enriched_yaml {
        let proceed = if yes {
            true
        } else {
            ui.confirm(
                &format!(
                    "Keycloak enriched the representation of {} '{}'. Update the local file?",
                    T::LABEL,
                    enriched.get_name()
                ),
                true,
            )?
        };

        if proceed {
            crate::utils::write_secure(path, &enriched_yaml).await?;
            append_secrets(secrets_path, &new_secrets).await?;
        }
    }

    Ok(())
}

fn find_placeholders(
    val: &serde_json::Value,
    path: &mut String,
    placeholders: &mut std::collections::HashMap<String, String>,
) {
    match val {
        serde_json::Value::String(s) => {
            if s.starts_with("${") && s.ends_with('}') {
                placeholders.insert(path.clone(), s.clone());
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let original_len = path.len();
                path.push('/');
                path.push_str(k);
                find_placeholders(v, path, placeholders);
                path.truncate(original_len);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let original_len = path.len();
                use std::fmt::Write;
                write!(path, "/{}", i).expect("Failed to append array index to JSON path buffer");
                find_placeholders(v, path, placeholders);
                path.truncate(original_len);
            }
        }
        _ => {}
    }
}

fn set_value_at_path(val: &mut serde_json::Value, path: &str, new_val: serde_json::Value) {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    set_value_at_path_rec(val, &segments, new_val);
}

fn set_value_at_path_rec(
    val: &mut serde_json::Value,
    segments: &[&str],
    new_val: serde_json::Value,
) {
    if segments.is_empty() {
        return;
    }
    let seg = segments[0];
    if segments.len() == 1 {
        if let Some(obj) = val.as_object_mut() {
            obj.insert(seg.to_string(), new_val);
        } else if let Some(arr) = val.as_array_mut() {
            if let Ok(idx) = seg.parse::<usize>() {
                if idx < arr.len() {
                    arr[idx] = new_val;
                }
            }
        }
    } else {
        if let Some(obj) = val.as_object_mut() {
            if let Some(next) = obj.get_mut(seg) {
                set_value_at_path_rec(next, &segments[1..], new_val);
            }
        } else if let Some(arr) = val.as_array_mut() {
            if let Ok(idx) = seg.parse::<usize>() {
                if idx < arr.len() {
                    set_value_at_path_rec(&mut arr[idx], &segments[1..], new_val);
                }
            }
        }
    }
}

pub async fn append_secrets(
    secrets_path: &std::path::Path,
    new_secrets: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if new_secrets.is_empty() {
        return Ok(());
    }
    let mut existing = std::collections::HashMap::new();
    if tokio::fs::try_exists(secrets_path).await.unwrap_or(false) {
        if let Ok(content) = tokio::fs::read_to_string(secrets_path).await {
            for line in content.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    existing.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }

    let mut to_append = String::new();
    for (k, v) in new_secrets {
        if !existing.contains_key(k) {
            to_append.push_str(&format!("{}={}\n", k, v));
        }
    }

    if !to_append.is_empty() {
        let mut content = if tokio::fs::try_exists(secrets_path).await.unwrap_or(false) {
            tokio::fs::read_to_string(secrets_path)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&to_append);
        crate::utils::write_secure(secrets_path, &content).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ClientRepresentation;
    use crate::utils::ui::MockUi;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_check_and_update_enrichment_unit() -> Result<()> {
        let temp = tempdir()?;
        let client_path = temp.path().join("client.yaml");
        let secrets_path = temp.path().join(".secrets");

        // Write pre-existing secrets
        fs::write(&secrets_path, "EXISTING_KEY=old_val\n")?;

        // 1. Write a local YAML value containing secret placeholders inside object and array
        let local_yaml = serde_json::json!({
            "id": null,
            "clientId": "test-client",
            "name": "Initial Name",
            "secret": "${CLIENT_SECRET}",
            "redirectUris": [
                "http://localhost",
                "${REDIRECT_URI_PLACEHOLDER}"
            ]
        });
        fs::write(&client_path, serde_yaml::to_string(&local_yaml)?)?;

        // 2. Prepare an enriched representation returned from keycloak.
        // It has a secret value "my-new-secret" (which will be extracted),
        // and some new client fields.
        let enriched_client = ClientRepresentation {
            id: Some("generated-id-123".to_string()),
            client_id: Some("test-client".to_string()),
            secret: None,
            name: Some("Enriched Name from Keycloak".to_string()),
            description: None,
            enabled: Some(true),
            protocol: None,
            redirect_uris: Some(vec![
                "http://localhost".to_string(),
                "enriched-redirect-uri".to_string(),
            ]),
            web_origins: None,
            public_client: None,
            bearer_only: None,
            service_accounts_enabled: None,
            extra: [("secret".to_string(), serde_json::json!("my-new-secret"))]
                .into_iter()
                .collect(),
        };

        // UI confirms update
        let ui = MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(vec![true]),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        };

        let client = KeycloakClient::new("http://dummy".to_string());

        // Call check_and_update_enrichment with yes = false, confirm = true
        check_and_update_enrichment(
            &client,
            &client_path,
            &local_yaml,
            &enriched_client,
            "test-realm",
            &secrets_path,
            &ui,
            false,
        )
        .await?;

        // 3. Verify that:
        // A. The local file was updated with enriched fields
        let content = fs::read_to_string(&client_path)?;
        let parsed: serde_json::Value = serde_yaml::from_str(&content)?;

        // - ID is updated
        assert_eq!(
            parsed.get("id").and_then(|v| v.as_str()),
            Some("generated-id-123")
        );
        // - Name is updated
        assert_eq!(
            parsed.get("name").and_then(|v| v.as_str()),
            Some("Enriched Name from Keycloak")
        );
        // - Placeholders are preserved!
        assert_eq!(
            parsed.get("secret").and_then(|v| v.as_str()),
            Some("${CLIENT_SECRET}")
        );
        let redirect_uris = parsed
            .get("redirectUris")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(
            redirect_uris[0].as_str(),
            Some("${REDIRECT_URI_PLACEHOLDER}")
        );

        // B. New secret was appended to .secrets, while preserving the existing one!
        let secrets_content = fs::read_to_string(&secrets_path)?;
        assert!(secrets_content.contains("EXISTING_KEY=old_val"));
        assert!(
            secrets_content
                .contains("KEYCLOAK_REALM_TEST_REALM_CLIENT_TEST_CLIENT_SECRET=my-new-secret")
        );

        Ok(())
    }

    #[test]
    fn test_is_protected_resource_branches() {
        use crate::models::{
            AuthenticationFlowRepresentation, ClientRepresentation, ClientScopeRepresentation,
            GroupRepresentation, RoleRepresentation,
        };

        // clients
        assert!(is_protected_resource::<ClientRepresentation>(
            "admin-cli",
            "myrealm"
        ));
        assert!(is_protected_resource::<ClientRepresentation>(
            "security-admin-console",
            "myrealm"
        ));
        assert!(is_protected_resource::<ClientRepresentation>(
            "account", "myrealm"
        ));
        assert!(!is_protected_resource::<ClientRepresentation>(
            "my-custom-client",
            "myrealm"
        ));

        // roles
        assert!(is_protected_resource::<RoleRepresentation>(
            "offline_access",
            "myrealm"
        ));
        assert!(is_protected_resource::<RoleRepresentation>(
            "default-roles-myrealm",
            "myrealm"
        ));
        assert!(!is_protected_resource::<RoleRepresentation>(
            "my-custom-role",
            "myrealm"
        ));

        // client-scopes
        assert!(is_protected_resource::<ClientScopeRepresentation>(
            "profile", "myrealm"
        ));
        assert!(is_protected_resource::<ClientScopeRepresentation>(
            "roles", "myrealm"
        ));
        assert!(!is_protected_resource::<ClientScopeRepresentation>(
            "my-custom-scope",
            "myrealm"
        ));

        // authentication flows
        assert!(is_protected_resource::<AuthenticationFlowRepresentation>(
            "browser", "myrealm"
        ));
        assert!(is_protected_resource::<AuthenticationFlowRepresentation>(
            "direct grant",
            "myrealm"
        ));
        assert!(!is_protected_resource::<AuthenticationFlowRepresentation>(
            "my-custom-flow",
            "myrealm"
        ));

        // other (e.g. groups)
        assert!(!is_protected_resource::<GroupRepresentation>(
            "my-custom-group",
            "myrealm"
        ));
    }

    #[tokio::test]
    async fn test_append_secrets_empty() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        let new_secrets = std::collections::BTreeMap::new();

        append_secrets(&secrets_path, &new_secrets).await?;

        assert!(!secrets_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_new_file() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "val1".to_string());
        new_secrets.insert("KEY2".to_string(), "val2".to_string());

        append_secrets(&secrets_path, &new_secrets).await?;

        assert!(secrets_path.exists());
        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("KEY1=val1\n"));
        assert!(content.contains("KEY2=val2\n"));
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_existing_file() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        fs::write(&secrets_path, "EXISTING_KEY=old_val\nKEY1=old_val1\n")?;

        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "new_val1".to_string()); // Should be ignored since KEY1 exists
        new_secrets.insert("KEY2".to_string(), "val2".to_string()); // Should be added

        append_secrets(&secrets_path, &new_secrets).await?;

        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("EXISTING_KEY=old_val\n"));
        assert!(content.contains("KEY1=old_val1\n"));
        assert!(!content.contains("KEY1=new_val1"));
        assert!(content.contains("KEY2=val2\n"));
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_missing_newline() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        fs::write(&secrets_path, "EXISTING_KEY=old_val")?; // No trailing newline

        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "val1".to_string());

        append_secrets(&secrets_path, &new_secrets).await?;

        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("EXISTING_KEY=old_val\n"));
        assert!(content.contains("KEY1=val1\n"));
        Ok(())
    }
}
