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
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
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
    let mut entries = match async_fs::read_dir(realm_dir.join("clients")).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(dirs),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let roles_dir = entry.path().join("roles");
        if entry.file_type().await?.is_dir() && async_fs::metadata(&roles_dir).await.is_ok() {
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
    let mut files = Vec::new();
    let mut entries = match async_fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if is_yaml_file(&path) && !is_overlay_file(&path, profile) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Remote clients by sanitized client ID: `(client UUID, client ID)`.
pub async fn clients_by_dir_name(
    client: &crate::client::KeycloakClient,
) -> Result<HashMap<String, (String, String)>> {
    Ok(client
        .get_clients()
        .await?
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
    let rep: RoleRepresentation = serde_json::from_value(val.clone())
        .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;
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
        let (client_uuid, client_id) = clients.get(&dir.dir_name).cloned().with_context(|| {
            format!(
                "Client '{}' of roles directory {:?} not found in realm '{}'",
                dir.dir_name, dir.roles_dir, realm_name
            )
        })?;
        let existing: HashMap<String, RoleRepresentation> = client
            .get_client_roles(&client_uuid)
            .await?
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();

        let files = role_files(&dir.roles_dir, profile.as_deref()).await?;
        let mut declared = HashSet::new();
        for path in files {
            let (before_sub, resolved, mut rep) =
                load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
            declared.insert(rep.name.clone());
            if planned_files
                .as_ref()
                .as_ref()
                .is_some_and(|p| !p.contains(&path))
            {
                continue;
            }
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
                    continue;
                }
            }
            rep.client_role = true;
            rep.container_id = Some(client_uuid.clone());
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
                        .create_client_role(&client_uuid, &rep)
                        .await
                        .with_context(|| format!("Failed to create {}", label))?;
                    log_line(format!("  {} Created {}", SUCCESS_CREATE, label));
                    client
                        .get_client_role(&client_uuid, &rep.name)
                        .await?
                        .id
                        .with_context(|| format!("Created {} has no ID", label))?
                }
            };

            if let Ok(mut enriched) = client.get_resource::<RoleRepresentation>(&role_id).await {
                rep.prepare_enriched(&mut enriched);
                crate::apply::generic::check_and_update_enrichment(
                    crate::apply::generic::LocalSource {
                        path: &path,
                        profile: profile.as_deref(),
                        before_sub: &before_sub,
                        resolved: &resolved,
                    },
                    &enriched,
                    realm_name,
                    &secrets_path,
                    &*ui,
                    yes,
                    Arc::clone(&prompt_mutex),
                )
                .await?;
            }
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
                    let id = role.id.as_deref().context("Remote role has no ID")?;
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
        let (_, _, rep) = load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
        if !rep.extra.contains_key("composites") {
            continue;
        }
        let id = client
            .get_realm_role(&rep.name)
            .await?
            .id
            .with_context(|| format!("Realm role '{}' has no ID", rep.name))?;
        let label = format!("realm role {}", rep.name);
        reconcile_role_composites(client, &rep, &id, &label).await?;
    }

    let dirs = client_role_dirs(&workspace_dir).await?;
    if dirs.is_empty() {
        return Ok(());
    }
    let clients = clients_by_dir_name(client).await?;
    for dir in dirs {
        let Some((client_uuid, client_id)) = clients.get(&dir.dir_name) else {
            continue;
        };
        for path in role_files(&dir.roles_dir, profile.as_deref()).await? {
            if !is_planned(&path) {
                continue;
            }
            let (_, _, rep) = load_role(&path, profile.as_deref(), Arc::clone(&resolver)).await?;
            if !rep.extra.contains_key("composites") {
                continue;
            }
            let id = client
                .get_client_role(client_uuid, &rep.name)
                .await?
                .id
                .with_context(|| {
                    format!("Role '{}' of client '{}' has no ID", rep.name, client_id)
                })?;
            let label = format!("role {} of client {}", rep.name, client_id);
            reconcile_role_composites(client, &rep, &id, &label).await?;
        }
    }
    Ok(())
}
