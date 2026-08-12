#![allow(clippy::collapsible_if)]

use crate::apply::ApplyContext;
use crate::models::RoleRepresentation;
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::fs as async_fs;

pub async fn apply_roles(ctx: ApplyContext<'_>) -> Result<()> {
    let ApplyContext {
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
        ..
    } = ctx;

    // 1. Discover all role files (both realm roles and client roles)
    let mut files = Vec::new();

    let realm_roles_dir = workspace_dir.join("roles");
    if async_fs::try_exists(&realm_roles_dir).await? {
        let mut entries = async_fs::read_dir(&realm_roles_dir).await?;
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
            if is_overlay_file(&path, profile.as_deref()) {
                continue;
            }
            files.push(path);
        }
    }

    let clients_dir = workspace_dir.join("clients");
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
                        if is_overlay_file(&path, profile.as_deref()) {
                            continue;
                        }
                        files.push(path);
                    }
                }
            }
        }
    }

    if files.is_empty() {
        return Ok(());
    }

    // 2. Fetch remote client map (clientId -> uuid)
    let client_uuid_map = client.get_client_uuid_map().await.unwrap_or_default();

    // 3. Fetch remote realm roles
    let remote_realm_roles = client.get_roles().await.unwrap_or_default();
    let realm_roles_map: HashMap<String, RoleRepresentation> = remote_realm_roles
        .clone()
        .into_iter()
        .map(|r| (r.name.clone(), r))
        .collect();

    // 4. Fetch remote client roles
    let mut client_roles_map: HashMap<(String, String), RoleRepresentation> = HashMap::new();
    for (client_id, client_uuid) in &client_uuid_map {
        if let Ok(c_roles) = client.get_client_roles(client_uuid).await {
            for r in c_roles {
                client_roles_map.insert((client_id.clone(), r.name.clone()), r);
            }
        }
    }

    let pb = create_progress_bar(files.len() as u64, "Applying roles");

    // 5. Process each role file
    for path in files {
        let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
        let local_val_before_sub = val.clone();
        substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
        let mut rep: RoleRepresentation = serde_json::from_value(val)
            .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

        let is_client_role =
            rep.client_role || path.components().any(|c| c.as_os_str() == "clients");

        let target_client_id = if is_client_role {
            let mut client_id = None;
            let components: Vec<_> = path.components().collect();
            for i in 0..components.len() {
                if components[i].as_os_str() == "clients" && i + 1 < components.len() {
                    client_id = Some(components[i + 1].as_os_str().to_string_lossy().to_string());
                    break;
                }
            }
            client_id.or_else(|| rep.container_id.clone())
        } else {
            None
        };

        if let Some(ref cid) = target_client_id {
            // Client role processing
            let client_uuid = client_uuid_map.get(cid).with_context(|| {
                format!(
                    "Could not resolve client ID '{}' for client role '{}'",
                    cid, rep.name
                )
            })?;

            let remote_opt = client_roles_map.get(&(cid.clone(), rep.name.clone()));

            if review {
                let action = if remote_opt.is_some() {
                    "update"
                } else {
                    "create"
                };
                let proceed = ui.confirm(
                    &format!(
                        "Do you want to {} client role '{}' ({})?",
                        action, rep.name, cid
                    ),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }

            rep.client_role = true;
            rep.container_id = Some(client_uuid.clone());

            let final_id;
            if let Some(remote) = remote_opt {
                client
                    .update_client_role(client_uuid, &rep.name, &rep)
                    .await
                    .with_context(|| format!("Failed to update client role '{}'", rep.name))?;
                pb.println(format!(
                    "  {} Updated client role {} ({})",
                    SUCCESS_UPDATE, rep.name, cid
                ));
                final_id = remote.id.clone();
            } else {
                client
                    .create_client_role(client_uuid, &rep)
                    .await
                    .with_context(|| format!("Failed to create client role '{}'", rep.name))?;
                pb.println(format!(
                    "  {} Created client role {} ({})",
                    SUCCESS_CREATE, rep.name, cid
                ));

                // Fetch created client role to get ID if needed
                let fresh = client
                    .get_client_roles(client_uuid)
                    .await
                    .unwrap_or_default();
                final_id = fresh
                    .into_iter()
                    .find(|r| r.name == rep.name)
                    .and_then(|r| r.id);
            }

            if let Some(id) = final_id {
                let enriched_res = client.get_resource::<RoleRepresentation>(&id).await;
                if let Ok(enriched) = enriched_res {
                    crate::apply::generic::check_and_update_enrichment(
                        client,
                        &path,
                        &local_val_before_sub,
                        &enriched,
                        realm_name,
                        &secrets_path,
                        &*ui,
                        yes,
                    )
                    .await?;
                }
            }
        } else {
            // Realm role processing
            let remote_opt = realm_roles_map.get(&rep.name);

            if review {
                let action = if remote_opt.is_some() {
                    "update"
                } else {
                    "create"
                };
                let proceed = ui.confirm(
                    &format!("Do you want to {} realm role '{}'?", action, rep.name),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }

            let final_id;
            if let Some(remote) = remote_opt {
                let id = remote.id.as_deref().unwrap_or(&rep.name);
                rep.id = remote.id.clone();
                client
                    .update_role(id, &rep)
                    .await
                    .with_context(|| format!("Failed to update realm role '{}'", rep.name))?;
                pb.println(format!(
                    "  {} Updated realm role {}",
                    SUCCESS_UPDATE, rep.name
                ));
                final_id = remote.id.clone();
            } else {
                rep.id = None;
                client
                    .create_role(&rep)
                    .await
                    .with_context(|| format!("Failed to create realm role '{}'", rep.name))?;
                pb.println(format!(
                    "  {} Created realm role {}",
                    SUCCESS_CREATE, rep.name
                ));

                let fresh = client.get_roles().await.unwrap_or_default();
                final_id = fresh
                    .into_iter()
                    .find(|r| r.name == rep.name)
                    .and_then(|r| r.id);
            }

            if let Some(id) = final_id {
                let enriched_res = client.get_resource::<RoleRepresentation>(&id).await;
                if let Ok(enriched) = enriched_res {
                    crate::apply::generic::check_and_update_enrichment(
                        client,
                        &path,
                        &local_val_before_sub,
                        &enriched,
                        realm_name,
                        &secrets_path,
                        &*ui,
                        yes,
                    )
                    .await?;
                }
            }
        }

        pb.inc(1);
    }
    pb.finish_with_message("Applied roles");

    // Optional pruning
    if prune {
        let mut declared_realm_roles = HashSet::new();
        let mut declared_client_roles = HashSet::new(); // (client_id, role_name)

        if async_fs::try_exists(&realm_roles_dir).await? {
            let mut entries = async_fs::read_dir(&realm_roles_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "yaml")
                    && !is_overlay_file(&path, profile.as_deref())
                {
                    if let Ok(content) = async_fs::read_to_string(&path).await {
                        if let Ok(r) = serde_yaml::from_str::<RoleRepresentation>(&content) {
                            declared_realm_roles.insert(r.name);
                        }
                    }
                }
            }
        }

        if async_fs::try_exists(&clients_dir).await? {
            let mut client_entries = async_fs::read_dir(&clients_dir).await?;
            while let Some(client_entry) = client_entries.next_entry().await? {
                let client_path = client_entry.path();
                if client_path.is_dir() {
                    let client_id = client_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let client_roles_dir = client_path.join("roles");
                    if async_fs::try_exists(&client_roles_dir).await? {
                        let mut role_entries = async_fs::read_dir(&client_roles_dir).await?;
                        while let Some(role_entry) = role_entries.next_entry().await? {
                            let path = role_entry.path();
                            if path.extension().is_some_and(|ext| ext == "yaml")
                                && !is_overlay_file(&path, profile.as_deref())
                            {
                                if let Ok(content) = async_fs::read_to_string(&path).await {
                                    if let Ok(r) =
                                        serde_yaml::from_str::<RoleRepresentation>(&content)
                                    {
                                        declared_client_roles
                                            .insert((client_id.to_string(), r.name));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let default_role = format!("default-roles-{}", realm_name);
        let protected_realm_roles = ["offline_access", "uma_authorization", &default_role];

        for remote in &remote_realm_roles {
            if !declared_realm_roles.contains(&remote.name)
                && !protected_realm_roles.contains(&remote.name.as_str())
            {
                let proceed = if yes {
                    true
                } else {
                    ui.confirm(
                        &format!("Prune/Delete remote realm role '{}'?", remote.name),
                        false,
                    )?
                };

                if proceed {
                    if let Some(ref id) = remote.id {
                        client.delete_role(id).await?;
                    } else {
                        client.delete_role(&remote.name).await?;
                    }
                    eprintln!("  Removed/Pruned realm role {}", remote.name);
                }
            }
        }

        for ((client_id, role_name), remote) in &client_roles_map {
            if !declared_client_roles.contains(&(client_id.clone(), role_name.clone())) {
                let proceed = if yes {
                    true
                } else {
                    ui.confirm(
                        &format!(
                            "Prune/Delete remote client role '{}' ({})?",
                            role_name, client_id
                        ),
                        false,
                    )?
                };

                if proceed {
                    if let Some(client_uuid) = client_uuid_map.get(client_id) {
                        client.delete_client_role(client_uuid, role_name).await?;
                    } else if let Some(ref id) = remote.id {
                        client.delete_role(id).await?;
                    }
                    eprintln!("  Removed/Pruned client role {} ({})", role_name, client_id);
                }
            }
        }
    }

    Ok(())
}
