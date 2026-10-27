//! Client roles (`clients/<clientId>/roles/<role>.yaml`) and role composites.
//!
//! Client roles belong to a client, so they live next to the client files in a directory named
//! after the (filename-sanitized) client ID. Role composites may reference client roles of any
//! client, so they are reconciled in a dedicated pass once every role exists.

use crate::apply::relations::reconcile_role_composites;
use crate::client::KeycloakResourceMapping;
use crate::models::{KeycloakResource, RoleRepresentation};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, log_line};
use crate::utils::yaml::{deserialize_value, list_yaml_files, load_yaml_with_overlay};
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs as async_fs;

/// A `clients/<dir>/roles` directory.
pub struct ClientRolesDir {
    /// Directory name (the sanitized client ID).
    pub dir_name: String,
    /// Path of the `roles` directory.
    pub roles_dir: PathBuf,
}

/// Lists the client role directories of a realm workspace.
///
/// # Errors
/// Returns an error if the `clients` directory cannot be read.
pub async fn client_role_dirs(realm_dir: &Path) -> Result<Vec<ClientRolesDir>> {
    let mut dirs = Vec::new();
    let entries = crate::utils::read_dir_entries(&realm_dir.join("clients")).await?;
    for entry in entries.unwrap_or_default() {
        let roles_dir = entry.path().join("roles");
        if crate::utils::is_dir_entry(&entry).await? && async_fs::metadata(&roles_dir).await.is_ok()
        {
            dirs.push(ClientRolesDir {
                dir_name: entry.file_name().to_string_lossy().to_string(),
                roles_dir,
            });
        }
    }
    dirs.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    Ok(dirs)
}

/// Lists the role files of a directory (overlays excluded).
pub async fn role_files(dir: &Path, profile: Option<&str>) -> Result<Vec<PathBuf>> {
    Ok(list_yaml_files(dir, profile).await?.unwrap_or_default())
}

/// Remote clients by sanitized client ID: `(client UUID, client ID)`.
pub async fn clients_by_dir_name(
    client: &crate::client::KeycloakClient,
) -> Result<HashMap<String, (String, String)>> {
    Ok(client
        .get_clients()
        .await
        .context("Failed to list clients (to match client role directories)")?
        .into_iter()
        .filter_map(|c| {
            let client_id = c.client_id?;
            Some((sanitize(&client_id), (c.id?, client_id)))
        })
        .collect())
}

async fn load_role(
    path: &Path,
    profile: Option<&str>,
    resolver: Arc<dyn crate::utils::secrets::SecretResolver>,
) -> Result<(serde_json::Value, serde_json::Value, RoleRepresentation)> {
    let mut val = load_yaml_with_overlay(path, profile).await?;
    let before_sub = val.clone();
    substitute_secrets(&mut val, resolver).await?;
    let rep: RoleRepresentation = deserialize_value(val.clone())?;
    Ok((before_sub, val, rep))
}

/// Applies client roles of every `clients/<clientId>/roles` directory, then (optionally)
/// prunes undeclared roles of those clients.
pub async fn apply_client_roles(ctx: crate::apply::ApplyContext<'_>) -> Result<()> {
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

    let dirs = client_role_dirs(&workspace_dir).await?;
    if dirs.is_empty() {
        return Ok(());
    }
    let clients = clients_by_dir_name(client).await?;

    for dir in dirs {
        let (client_internal_id, client_id) =
            clients
                .get(&dir.dir_name)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Hint: the directory name must be the (filename-sanitized) clientId of a client declared in clients/"
                    )
                })
                .with_context(|| {
                    format!(
                        "No client '{}' on the server for roles directory {}",
                        dir.dir_name,
                        dir.roles_dir.display()
                    )
                })?;
        let existing: HashMap<String, RoleRepresentation> = client
            .get_client_roles(&client_internal_id)
            .await
            .with_context(|| format!("Failed to list the roles of client '{}'", client_id))?
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();

        let files = role_files(&dir.roles_dir, profile.as_deref()).await?;
        let mut declared = HashSet::new();
        for path in files {
            // Only the name is needed to know which roles are declared: secrets are resolved
            // for the files being applied only.
            let raw = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            let name = raw
                .get("name")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("{} has no 'name'", path.display()))?;
            declared.insert(name.to_string());
            if planned_files
                .as_ref()
                .as_ref()
                .is_some_and(|p| !p.contains(&path))
            {
                continue;
            }
            async {
                let (before_sub, resolved, mut rep) =
                    load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
                let label = format!("role {} of client {}", rep.name, client_id);
                let current = existing.get(&rep.name);
                if review {
                    let action = if current.is_some() {
                        "update"
                    } else {
                        "create"
                    };
                    let proceed = {
                        let _lock = prompt_mutex.lock().await;
                        ui.confirm(&format!("Do you want to {} {}?", action, label), true)?
                    };
                    if !proceed {
                        return Ok(());
                    }
                }
                rep.client_role = true;
                rep.container_id = Some(client_internal_id.clone());
                let role_id = match current.and_then(|c| c.id.clone()) {
                    Some(id) => {
                        rep.id = Some(id.clone());
                        client
                            .update_resource(&id, &rep)
                            .await
                            .with_context(|| format!("Failed to update {}", label))?;
                        log_line(format!("  {} Updated {}", SUCCESS_UPDATE, label));
                        id
                    }
                    None => {
                        rep.id = None;
                        client
                            .create_client_role(&client_internal_id, &rep)
                            .await
                            .with_context(|| format!("Failed to create {}", label))?;
                        log_line(format!("  {} Created {}", SUCCESS_CREATE, label));
                        client
                            .get_client_role(&client_internal_id, &rep.name)
                            .await
                            .with_context(|| {
                                format!("Created {}, but looking up its ID failed", label)
                            })?
                            .id
                            .with_context(|| format!("Created {} has no ID", label))?
                    }
                };

                match client.get_resource::<RoleRepresentation>(&role_id).await {
                    Err(e) => log_line(crate::apply::generic::enrichment_skipped_warning(
                        "role", &label, &e,
                    )),
                    Ok(mut enriched) => {
                        rep.prepare_enriched(&mut enriched);
                        if before_sub.get("clientRole").is_none() {
                            enriched.client_role = false;
                        }
                        let client_realm_prefix = format!("{}_client_{}", realm_name, client_id);
                        crate::apply::generic::check_and_update_enrichment(
                            &path,
                            profile.as_deref(),
                            crate::apply::generic::LocalSource {
                                before_sub: &before_sub,
                                resolved: &resolved,
                            },
                            &enriched,
                            &client_realm_prefix,
                            &secrets_path,
                            &*ui,
                            yes,
                            Arc::clone(&prompt_mutex),
                        )
                        .await?;
                    }
                }
                Ok::<(), anyhow::Error>(())
            }
            .await
            .with_context(|| path.display().to_string())?;
        }

        if prune && !crate::apply::generic::is_protected_client(&client_id, realm_name) {
            for (name, role) in &existing {
                if declared.contains(name) {
                    continue;
                }
                let proceed = yes || {
                    let _lock = prompt_mutex.lock().await;
                    ui.confirm(
                        &format!("Prune/Delete role '{}' of client '{}'?", name, client_id),
                        false,
                    )?
                };
                if proceed {
                    let id = role.id.as_deref().with_context(|| {
                        format!("Role '{}' of client '{}' has no ID", name, client_id)
                    })?;
                    client
                        .delete_resource::<RoleRepresentation>(id)
                        .await
                        .with_context(|| {
                            format!("Failed to prune role '{}' of client '{}'", name, client_id)
                        })?;
                    log_line(format!(
                        "  Removed/Pruned role {} of client {}",
                        name, client_id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Reconciles the declared composites of every realm role and client role file.
pub async fn apply_role_composites(ctx: crate::apply::ApplyContext<'_>) -> Result<()> {
    let crate::apply::ApplyContext {
        client,
        workspace_dir,
        resolver,
        planned_files,
        profile,
        ..
    } = ctx;
    let is_planned = |path: &Path| {
        planned_files
            .as_ref()
            .as_ref()
            .is_none_or(|p| p.contains(path))
    };

    for path in role_files(
        &workspace_dir.join(RoleRepresentation::DIR_NAME),
        profile.as_deref(),
    )
    .await?
    {
        if !is_planned(&path) {
            continue;
        }
        async {
            let (_, _, rep) = load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
            if !rep.extra.contains_key("composites") {
                return Ok(());
            }
            let id = client
                .get_realm_role(&rep.name)
                .await
                .with_context(|| format!("Failed to look up realm role '{}'", rep.name))?
                .id
                .with_context(|| format!("Realm role '{}' has no ID", rep.name))?;
            let label = format!("realm role {}", rep.name);
            reconcile_role_composites(client, &rep, &id, &label).await
        }
        .await
        .with_context(|| path.display().to_string())?;
    }

    let dirs = client_role_dirs(&workspace_dir).await?;
    if dirs.is_empty() {
        return Ok(());
    }
    let clients = clients_by_dir_name(client).await?;
    for dir in dirs {
        let Some((client_internal_id, client_id)) = clients.get(&dir.dir_name) else {
            continue;
        };
        for path in role_files(&dir.roles_dir, profile.as_deref()).await? {
            if !is_planned(&path) {
                continue;
            }
            async {
                let (_, _, rep) =
                    load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
                if !rep.extra.contains_key("composites") {
                    return Ok(());
                }
                let id = client
                    .get_client_role(client_internal_id, &rep.name)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to look up role '{}' of client '{}'",
                            rep.name, client_id
                        )
                    })?
                    .id
                    .with_context(|| {
                        format!("Role '{}' of client '{}' has no ID", rep.name, client_id)
                    })?;
                let label = format!("role {} of client {}", rep.name, client_id);
                reconcile_role_composites(client, &rep, &id, &label).await
            }
            .await
            .with_context(|| path.display().to_string())?;
        }
    }
    Ok(())
}
