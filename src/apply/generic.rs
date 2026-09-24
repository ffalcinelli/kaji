#![allow(clippy::collapsible_if)]
use crate::client::KeycloakClient;
use crate::models::{KeycloakResource, ResourceMeta};
use crate::utils::secrets::substitute_secrets;
pub use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE};
use crate::utils::ui::{Ui, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

async fn order_files_topologically<T>(
    files: Vec<std::path::PathBuf>,
    profile: Option<&str>,
) -> Vec<Vec<std::path::PathBuf>>
where
    T: KeycloakResource,
{
    if T::DIR_NAME != crate::models::AuthenticationFlowRepresentation::DIR_NAME || files.len() <= 1
    {
        return vec![files];
    }

    // Parse each flow file to find its declared alias and referenced sub-flows
    let mut file_info: Vec<(std::path::PathBuf, String, Vec<String>)> = Vec::new();
    for path in files {
        if let Ok(val) = load_yaml_with_overlay(&path, profile).await {
            if let Ok(flow) =
                serde_json::from_value::<crate::models::AuthenticationFlowRepresentation>(val)
            {
                let alias = flow.alias.clone().unwrap_or_default();
                let subflows = flow.subflow_aliases();
                file_info.push((path, alias, subflows));
                continue;
            }
        }
        file_info.push((path, String::new(), Vec::new()));
    }

    let mut remaining = file_info;
    let mut tiers = Vec::new();
    let mut satisfied_aliases: HashSet<String> = HashSet::new();

    while !remaining.is_empty() {
        let mut current_tier = Vec::new();
        let mut next_remaining = Vec::new();

        let pending_aliases: HashSet<String> = remaining
            .iter()
            .map(|(_, alias, _)| alias.clone())
            .filter(|a| !a.is_empty())
            .collect();

        for (path, alias, subflows) in remaining {
            // A flow can be applied if all its local subflow dependencies are already satisfied
            let dependencies_ready = subflows
                .iter()
                .all(|sub| !pending_aliases.contains(sub) || satisfied_aliases.contains(sub));

            if dependencies_ready {
                current_tier.push((path, alias));
            } else {
                next_remaining.push((path, alias, subflows));
            }
        }

        if current_tier.is_empty() {
            // Unresolvable cycle or mutual dependency: fall back to applying all remaining in one tier
            let fallback: Vec<std::path::PathBuf> =
                next_remaining.into_iter().map(|(p, _, _)| p).collect();
            tiers.push(fallback);
            break;
        }

        let mut tier_paths = Vec::new();
        for (path, alias) in current_tier {
            if !alias.is_empty() {
                satisfied_aliases.insert(alias);
            }
            tier_paths.push(path);
        }

        tiers.push(tier_paths);
        remaining = next_remaining;
    }

    tiers
}

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
        prompt_mutex,
    } = ctx;

    let dir_name = T::DIR_NAME;
    let resources_dir = workspace_dir.join(dir_name);
    let mut entries = match async_fs::read_dir(&resources_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

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
        if !is_yaml_file(&path) {
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
    let tiers = order_files_topologically::<T>(files, profile.as_deref()).await;

    for tier_files in tiers {
        let mut set = JoinSet::new();

        for path in tier_files {
            let client = client.clone();
            let existing_map = Arc::clone(&existing_map);
            let resolver = Arc::clone(&resolver);
            let realm_name = realm_name.to_string();
            let profile = profile.clone();
            let ui = Arc::clone(&ui);
            let pb = pb.clone();
            let secrets_path = Arc::clone(&secrets_path);
            let prompt_mutex = Arc::clone(&prompt_mutex);

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
                    let proceed = {
                        let _lock = prompt_mutex.lock().await;
                        ui.confirm(
                            &format!(
                                "Do you want to {} {} '{}'?",
                                action,
                                T::LABEL,
                                rep.get_name()
                            ),
                            true,
                        )?
                    };
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
                    let create_result = client.create_resource(&rep).await;
                    match create_result {
                        Ok(maybe_id) => {
                            pb.println(format!(
                                "  {} Created {} {}",
                                SUCCESS_CREATE,
                                T::LABEL,
                                rep.get_name()
                            ));

                            if let Some(id) = maybe_id {
                                final_id = Some(id);
                            } else {
                                // Fallback: Fetch resources to get the generated ID of the created resource
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
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            let is_conflict = err_str.contains("409")
                                || err_str.to_lowercase().contains("conflict")
                                || err_str.to_lowercase().contains("already exists");

                            let mut adopted = false;
                            if is_conflict {
                                client.invalidate_resource_cache::<T>();
                                if let Ok(fresh_resources) = client.get_resources::<T>().await {
                                    if let Some(fresh) = fresh_resources
                                        .into_iter()
                                        .find(|r| r.get_identity() == Some(identity.clone()))
                                    {
                                        if let Some(existing_id) = fresh.get_id() {
                                            let mut update_rep = rep.clone();
                                            update_rep.set_id(Some(existing_id.to_string()));
                                            if let Ok(()) = client
                                                .update_resource(existing_id, &update_rep)
                                                .await
                                            {
                                                pb.println(format!(
                                                    "  {} Reconciled existing {} {} (adopted after conflict)",
                                                    SUCCESS_UPDATE,
                                                    T::LABEL,
                                                    rep.get_name()
                                                ));
                                                final_id = Some(existing_id.to_string());
                                                adopted = true;
                                            }
                                        }
                                    }
                                }
                            }

                            if !adopted {
                                return Err(err).with_context(|| {
                                    format!(
                                        "Failed to create {} '{}' in realm '{}'",
                                        T::LABEL,
                                        rep.get_name(),
                                        realm_name
                                    )
                                });
                            }
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
                            Arc::clone(&prompt_mutex),
                        )
                        .await?;
                    }
                }

                pb.inc(1);
                Ok::<(), anyhow::Error>(())
            });
        }

        crate::utils::join_all_tasks(set, None).await?;
    }

    pb.finish_with_message(format!("Applied {}", T::LABEL));

    if prune {
        let mut declared = HashSet::new();
        match async_fs::read_dir(&resources_dir).await {
            Ok(mut entries) => {
                while let Some(entry) = entries.next_entry().await? {
                    let path = entry.path();
                    if !is_yaml_file(&path) {
                        continue;
                    }
                    if is_overlay_file(&path, profile.as_deref()) {
                        continue;
                    }
                    if let Ok(val) = load_yaml_with_overlay(&path, profile.as_deref()).await {
                        if let Ok(rep) = serde_json::from_value::<T>(val) {
                            if let Some(identity) = rep.get_identity() {
                                declared.insert(identity);
                            }
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
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
                        let _lock = prompt_mutex.lock().await;
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
    prompt_mutex: Arc<tokio::sync::Mutex<()>>,
) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Clone,
{
    let mut placeholders = Vec::new();
    let mut current_path = Vec::new();
    find_placeholders(local_val_before_sub, &mut current_path, &mut placeholders);

    let mut enriched_val = serde_json::to_value(enriched.clone())?;

    let mut new_secrets = std::collections::BTreeMap::new();
    let prefix = format!("realm_{}_{}", realm_name, T::SECRET_PREFIX);
    crate::utils::secrets::extract_secrets(&mut enriched_val, &prefix, &mut new_secrets);

    for (p, placeholder) in &placeholders {
        set_value_at_path(
            &mut enriched_val,
            p,
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
            let _lock = prompt_mutex.lock().await;
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
            let _lock = prompt_mutex.lock().await;
            append_secrets(secrets_path, &new_secrets).await?;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

fn find_placeholders(
    val: &serde_json::Value,
    current_path: &mut Vec<PathSegment>,
    placeholders: &mut Vec<(Vec<PathSegment>, String)>,
) {
    match val {
        serde_json::Value::String(s) => {
            if s.starts_with("${") && s.ends_with('}') {
                placeholders.push((current_path.clone(), s.clone()));
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                current_path.push(PathSegment::Key(k.clone()));
                find_placeholders(v, current_path, placeholders);
                current_path.pop();
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                current_path.push(PathSegment::Index(i));
                find_placeholders(v, current_path, placeholders);
                current_path.pop();
            }
        }
        _ => {}
    }
}

fn set_value_at_path(
    val: &mut serde_json::Value,
    segments: &[PathSegment],
    new_val: serde_json::Value,
) {
    if segments.is_empty() {
        return;
    }
    match &segments[0] {
        PathSegment::Key(k) => {
            if segments.len() == 1 {
                if let Some(obj) = val.as_object_mut() {
                    obj.insert(k.clone(), new_val);
                }
            } else if let Some(obj) = val.as_object_mut() {
                if let Some(next) = obj.get_mut(k) {
                    set_value_at_path(next, &segments[1..], new_val);
                }
            }
        }
        PathSegment::Index(idx) => {
            if segments.len() == 1 {
                if let Some(arr) = val.as_array_mut() {
                    if *idx < arr.len() {
                        arr[*idx] = new_val;
                    }
                }
            } else if let Some(arr) = val.as_array_mut() {
                if *idx < arr.len() {
                    set_value_at_path(&mut arr[*idx], &segments[1..], new_val);
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

    let mut content = match tokio::fs::read_to_string(secrets_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };

    let mut existing = std::collections::HashMap::new();
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            existing.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    let mut appended = false;
    for (k, v) in new_secrets {
        if !existing.contains_key(k) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            use std::fmt::Write;
            let _ = writeln!(&mut content, "{}={}", k, v);
            appended = true;
        }
    }

    if appended {
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
            Arc::new(tokio::sync::Mutex::new(())),
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

    #[tokio::test]
    async fn test_check_and_update_enrichment_with_slashed_keys() -> Result<()> {
        let temp = tempdir()?;
        let client_path = temp.path().join("client.yaml");
        let secrets_path = temp.path().join(".secrets");

        let local_yaml = serde_json::json!({
            "clientId": "test-client",
            "attributes": {
                "custom/url/endpoint": "${ENDPOINT_URL}",
                "user.attribute/department": "${DEPT_NAME}"
            }
        });
        fs::write(&client_path, serde_yaml::to_string(&local_yaml)?)?;

        let mut extra = HashMap::new();
        extra.insert(
            "attributes".to_string(),
            serde_json::json!({
                "custom/url/endpoint": "placeholder-to-overwrite",
                "user.attribute/department": "placeholder-to-overwrite",
                "other.field": "val"
            }),
        );

        let enriched_client = ClientRepresentation {
            id: Some("gen-id".to_string()),
            client_id: Some("test-client".to_string()),
            secret: None,
            name: None,
            description: None,
            enabled: Some(true),
            protocol: None,
            redirect_uris: None,
            web_origins: None,
            public_client: None,
            bearer_only: None,
            service_accounts_enabled: None,
            extra,
        };

        let ui = MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(vec![true]),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        };

        let client = KeycloakClient::new("http://dummy".to_string());
        check_and_update_enrichment(
            &client,
            &client_path,
            &local_yaml,
            &enriched_client,
            "test-realm",
            &secrets_path,
            &ui,
            false,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .await?;

        let content = fs::read_to_string(&client_path)?;
        let parsed: serde_json::Value = serde_yaml::from_str(&content)?;
        assert_eq!(
            parsed["attributes"]["custom/url/endpoint"].as_str(),
            Some("${ENDPOINT_URL}")
        );
        assert_eq!(
            parsed["attributes"]["user.attribute/department"].as_str(),
            Some("${DEPT_NAME}")
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
