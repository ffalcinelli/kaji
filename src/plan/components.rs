use crate::client::KeycloakClient;
use crate::models::{ComponentRepresentation, KeycloakResource};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SPARKLE, WARN};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use console::style;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs as async_fs;

use super::{print_diff, PlanContext, PlanOptions, PlanSummary};

pub async fn plan_components_or_keys(
    ctx: &PlanContext<'_>,
    dir_name: &str,
) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    let components_dir = ctx.workspace_dir.join(dir_name);
    if async_fs::try_exists(&components_dir).await? {
        let existing_components =
            ctx.client.get_components().await.with_context(|| {
                format!("Failed to get components for realm '{}'", ctx.realm_name)
            })?;
        let mut by_identity: HashMap<String, ComponentRepresentation> = HashMap::new();
        type ComponentKey = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let mut by_details: HashMap<ComponentKey, ComponentRepresentation> = HashMap::new();

        for c in existing_components {
            if let Some(id) = c.get_identity() {
                by_identity.insert(id, c.clone());
            }
            let key = (
                c.name.clone(),
                c.sub_type.clone(),
                c.provider_id.clone(),
                c.parent_id.clone(),
            );
            by_details.insert(key, c);
        }

        let by_identity = Arc::new(by_identity);
        let by_details = Arc::new(by_details);

        let mut set = tokio::task::JoinSet::new();
        let mut entries = async_fs::read_dir(&components_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "yaml") {
                // Skip overlay files themselves
                if is_overlay_file(&path, ctx.profile.as_deref()) {
                    continue;
                }

                let resolver = Arc::clone(&ctx.resolver);
                let by_identity = by_identity.clone();
                let by_details = by_details.clone();
                let realm_name = ctx.realm_name.to_string();
                let profile = ctx.profile.clone();

                set.spawn(async move {
                    let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
                    substitute_secrets(&mut val, resolver).await?;
                    let local_component: ComponentRepresentation = serde_json::from_value(val)
                        .with_context(|| {
                            format!(
                                "Failed to deserialize YAML file {:?} in realm '{}'",
                                path, realm_name
                            )
                        })?;

                    let remote = local_component
                        .get_identity()
                        .and_then(|id| by_identity.get(&id))
                        .or_else(|| {
                            let key = (
                                local_component.name.clone(),
                                local_component.sub_type.clone(),
                                local_component.provider_id.clone(),
                                local_component.parent_id.clone(),
                            );
                            by_details.get(&key)
                        })
                        .cloned();

                    Ok::<
                        (
                            ComponentRepresentation,
                            PathBuf,
                            Option<ComponentRepresentation>,
                        ),
                        anyhow::Error,
                    >((local_component, path, remote))
                });
            }
        }

        for res in crate::utils::join_all_tasks(set, None).await? {
            let (local_component, path, remote) = res;

            let is_update = remote.is_some();
            let mut remote_clone = None;
            let changed = if let Some(r) = remote {
                let mut rc = r.clone();
                if local_component.id.is_none() {
                    rc.id = None;
                }
                let prefix = if dir_name == "keys" {
                    "key"
                } else {
                    "component"
                };
                let diff_name = format!("Component {}", local_component.get_name());
                let ch = print_diff(
                    &diff_name,
                    Some(&rc),
                    &local_component,
                    ctx.options.changes_only,
                    ctx.options.verbose,
                    prefix,
                )?;
                remote_clone = Some(rc);
                ch
            } else {
                eprintln!(
                    "\n{} Will create Component: {}",
                    SPARKLE,
                    local_component.get_name()
                );
                let prefix = if dir_name == "keys" {
                    "key"
                } else {
                    "component"
                };
                let diff_name = format!("Component {}", local_component.get_name());
                print_diff(
                    &diff_name,
                    None::<&ComponentRepresentation>,
                    &local_component,
                    ctx.options.changes_only,
                    ctx.options.verbose,
                    prefix,
                )?
            };

            if changed {
                let mut include = true;
                if ctx.options.interactive {
                    let prefix = if dir_name == "keys" {
                        "key"
                    } else {
                        "component"
                    };
                    include = super::prompt_interactive_change(
                        ctx.ui,
                        &format!("Component {}", local_component.get_name()),
                        remote_clone.as_ref(),
                        &local_component,
                        prefix,
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
    }
    Ok((changed_files, summary))
}

pub async fn check_keys_drift(
    client: &KeycloakClient,
    options: PlanOptions,
    realm_name: &str,
) -> Result<()> {
    if !options.changes_only {
        return Ok(());
    }

    let keys_metadata = match client.get_keys().await {
        Ok(km) => km,
        Err(_) => return Ok(()), // Ignore if not available
    };

    if let Some(keys) = keys_metadata.keys {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System clock is before UNIX EPOCH")?
            .as_millis() as i64;
        let thirty_days = 30 * 24 * 60 * 60 * 1000; // 30 days in ms

        for key in keys {
            if key.status.as_deref() == Some("ACTIVE")
                && key
                    .valid_to
                    .is_some_and(|valid_to| valid_to > 0 && valid_to - now < thirty_days)
            {
                let provider_id = key.provider_id.as_deref().unwrap_or("unknown");
                eprintln!(
                    "{} Warning: Active key (providerId: {}) in realm '{}' is near expiration or expired! Consider rotating keys.",
                    WARN,
                    style(provider_id).yellow(),
                    realm_name
                );
            }
        }
    }

    Ok(())
}
