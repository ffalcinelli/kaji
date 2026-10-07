//! Planning of client roles (`clients/<clientId>/roles/<role>.yaml`).

use crate::apply::client_roles::{client_role_dirs, clients_by_dir_name, role_files};
use crate::client::KeycloakResourceMapping;
use crate::models::{KeycloakResource, ResourceMeta, RoleRepresentation};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SPARKLE, WARN};
use crate::utils::yaml::load_yaml_with_overlay;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use super::{PlanContext, PlanSummary, print_diff};

/// Diffs every client role file against the server.
///
/// # Errors
/// Returns an error if a file cannot be parsed or Keycloak cannot be queried.
pub async fn plan_client_roles(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    let dirs = client_role_dirs(ctx.workspace_dir).await?;
    if dirs.is_empty() {
        return Ok((changed_files, summary));
    }
    let clients = match clients_by_dir_name(ctx.client).await {
        Ok(clients) => clients,
        Err(e) if crate::client::is_not_found(&e) => HashMap::new(),
        Err(e) => return Err(e),
    };

    for dir in dirs {
        let remote_client = clients.get(&dir.dir_name);
        let client_label = remote_client
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| dir.dir_name.clone());
        let existing: HashMap<String, RoleRepresentation> = match remote_client {
            Some((uuid, _)) => ctx
                .client
                .get_client_roles(uuid)
                .await?
                .into_iter()
                .map(|r| (r.name.clone(), r))
                .collect(),
            None => HashMap::new(),
        };

        let mut declared = HashSet::new();
        for path in role_files(&dir.roles_dir, ctx.profile.as_deref()).await? {
            let mut val = load_yaml_with_overlay(&path, ctx.profile.as_deref()).await?;
            substitute_secrets(&mut val, Arc::clone(&ctx.resolver)).await?;
            let mut local: RoleRepresentation = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize YAML file {:?}", path))?;
            declared.insert(local.name.clone());
            let name = format!("role {} of client {}", local.name, client_label);

            let changed = match existing.get(&local.name) {
                Some(remote) => {
                    let mut rc = remote.clone();
                    rc.load_relations(ctx.client, Some(&local)).await?;
                    rc.clear_metadata();
                    // Client role flag and container are implied by the directory.
                    rc.client_role = local.client_role;
                    local.clear_metadata();
                    let changed = print_diff(
                        &name,
                        Some(&rc),
                        &local,
                        ctx.options.changes_only,
                        ctx.options.verbose,
                        RoleRepresentation::SECRET_PREFIX,
                    )?;
                    if changed {
                        summary.updated += 1;
                    }
                    changed
                }
                None => {
                    eprintln!("\n{} Will create {}", SPARKLE, name);
                    print_diff(
                        &name,
                        None::<&RoleRepresentation>,
                        &local,
                        ctx.options.changes_only,
                        ctx.options.verbose,
                        RoleRepresentation::SECRET_PREFIX,
                    )?;
                    summary.created += 1;
                    true
                }
            };
            if changed {
                let include = !ctx.options.interactive
                    || super::prompt_interactive_change(
                        ctx.ui,
                        &name,
                        existing.get(&local.name),
                        &local,
                        RoleRepresentation::SECRET_PREFIX,
                    )?;
                if include {
                    changed_files.push(path);
                } else if existing.contains_key(&local.name) {
                    summary.updated -= 1;
                } else {
                    summary.created -= 1;
                }
            }
        }

        let protected = remote_client
            .is_some_and(|(_, id)| crate::apply::generic::is_protected_client(id, ctx.realm_name));
        let mut orphaned: Vec<&String> =
            existing.keys().filter(|n| !declared.contains(*n)).collect();
        if !protected && !orphaned.is_empty() {
            orphaned.sort();
            eprintln!(
                "\n{} {} remote roles of client {} not declared locally (deleted only by `apply --prune`): {}",
                WARN,
                orphaned.len(),
                client_label,
                orphaned
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            summary.orphaned += orphaned.len();
        }
    }
    Ok((changed_files, summary))
}
