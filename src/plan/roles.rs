#![allow(clippy::collapsible_if)]

use crate::models::{KeycloakResource, ResourceMeta, RoleRepresentation};
use crate::plan::{PlanContext, PlanSummary, print_diff};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::SPARKLE;
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

pub async fn plan_roles(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();

    // 1. Discover all role files (both realm roles and client roles)
    let mut role_files = Vec::new();

    // Realm roles dir: <workspace_dir>/roles
    let realm_roles_dir = ctx.workspace_dir.join("roles");
    if async_fs::try_exists(&realm_roles_dir).await? {
        let mut entries = async_fs::read_dir(&realm_roles_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|ext| ext == "yaml")
                && !is_overlay_file(&path, ctx.profile.as_deref())
            {
                role_files.push(path);
            }
        }
    }

    // Client roles dirs: <workspace_dir>/clients/<client_id>/roles
    let clients_dir = ctx.workspace_dir.join("clients");
    if async_fs::try_exists(&clients_dir).await? {
        let mut client_entries = async_fs::read_dir(&clients_dir).await?;
        while let Some(client_entry) = client_entries.next_entry().await? {
            let client_path = client_entry.path();
            if client_path.is_dir() {
                let client_roles_dir = client_path.join("roles");
                if async_fs::try_exists(&client_roles_dir).await? {
                    let mut role_entries = async_fs::read_dir(&client_roles_dir).await?;
                    while let Some(role_entry) = role_entries.next_entry().await? {
                        let path = role_entry.path();
                        if path.is_file()
                            && path.extension().is_some_and(|ext| ext == "yaml")
                            && !is_overlay_file(&path, ctx.profile.as_deref())
                        {
                            role_files.push(path);
                        }
                    }
                }
            }
        }
    }

    if role_files.is_empty() {
        return Ok((changed_files, summary));
    }

    // 2. Fetch remote realm roles
    let remote_realm_roles = ctx
        .client
        .get_roles()
        .await
        .with_context(|| format!("Failed to get roles for realm '{}'", ctx.realm_name))?;
    let realm_roles_map: HashMap<String, RoleRepresentation> = remote_realm_roles
        .into_iter()
        .map(|r| (r.name.clone(), r))
        .collect();

    // 3. Fetch remote client roles for all clients
    let mut client_roles_map: HashMap<(String, String), RoleRepresentation> = HashMap::new();
    if let Ok(clients) = ctx.client.get_clients().await {
        for cl in clients {
            if let (Some(client_id), Some(client_uuid)) = (cl.client_id, cl.id) {
                let c_roles = ctx
                    .client
                    .get_client_roles(&client_uuid)
                    .await
                    .unwrap_or_default();
                for r in c_roles {
                    client_roles_map.insert((client_id.clone(), r.name.clone()), r);
                }
            }
        }
    }

    let realm_roles_map = Arc::new(realm_roles_map);
    let client_roles_map = Arc::new(client_roles_map);

    let mut set = tokio::task::JoinSet::new();

    for path in role_files {
        let resolver = Arc::clone(&ctx.resolver);
        let realm_roles_map = Arc::clone(&realm_roles_map);
        let client_roles_map = Arc::clone(&client_roles_map);
        let profile = ctx.profile.clone();

        set.spawn(async move {
            let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            substitute_secrets(&mut val, resolver).await?;
            let local: RoleRepresentation = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize role YAML file {:?}", path))?;

            // Check if this is a client role
            let is_client_role =
                local.client_role || path.components().any(|c| c.as_os_str() == "clients");

            let target_client_id = if is_client_role {
                let mut client_id = None;
                let components: Vec<_> = path.components().collect();
                for i in 0..components.len() {
                    if components[i].as_os_str() == "clients" && i + 1 < components.len() {
                        client_id =
                            Some(components[i + 1].as_os_str().to_string_lossy().to_string());
                        break;
                    }
                }
                client_id.or_else(|| local.container_id.clone())
            } else {
                None
            };

            let remote = if let Some(ref cid) = target_client_id {
                client_roles_map
                    .get(&(cid.clone(), local.name.clone()))
                    .cloned()
            } else {
                realm_roles_map.get(&local.name).cloned()
            };

            Ok::<
                (
                    RoleRepresentation,
                    PathBuf,
                    Option<RoleRepresentation>,
                    Option<String>,
                ),
                anyhow::Error,
            >((local, path, remote, target_client_id))
        });
    }

    for res in crate::utils::join_all_tasks(set, None).await? {
        let (local, path, remote, target_client_id) = res;
        let is_update = remote.is_some();
        let mut remote_clone = None;

        let label = if let Some(ref cid) = target_client_id {
            format!("client role ({})", cid)
        } else {
            "role".to_string()
        };

        let changed = if let Some(r) = remote {
            let mut rc = r.clone();
            if !local.has_id() {
                rc.clear_metadata();
            }
            let diff_name = format!("{} {}", label, local.get_name());
            let ch = print_diff(
                &diff_name,
                Some(&rc),
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                RoleRepresentation::SECRET_PREFIX,
            )?;
            remote_clone = Some(rc);
            ch
        } else {
            eprintln!("\n{} Will create {} {}", SPARKLE, label, local.get_name());
            print_diff(
                &format!("{} {}", label, local.get_name()),
                None::<&RoleRepresentation>,
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                RoleRepresentation::SECRET_PREFIX,
            )?
        };

        if changed {
            let mut include = true;
            if ctx.options.interactive {
                include = crate::plan::prompt_interactive_change(
                    ctx.ui,
                    &format!("{} {}", label, local.get_name()),
                    remote_clone.as_ref(),
                    &local,
                    RoleRepresentation::SECRET_PREFIX,
                )?;
            }
            if include {
                changed_files.push(path);
                if is_update {
                    summary.updated += 1;
                } else {
                    summary.created += 1;
                }
            }
        }
    }

    Ok((changed_files, summary))
}
