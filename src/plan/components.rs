use crate::client::KeycloakClient;
use crate::models::{ComponentRepresentation, KeycloakResource};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SPARKLE, WARN};
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use console::style;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs as async_fs;

use super::{PlanContext, PlanOptions, PlanSummary, print_diff};

pub async fn plan_components_or_keys(
    ctx: &PlanContext<'_>,
    dir_name: &str,
) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    let components_dir = ctx.workspace_dir.join(dir_name);
    let mut entries = match async_fs::read_dir(&components_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((changed_files, summary)),
        Err(e) => return Err(e.into()),
    };

    let existing_components = match ctx.client.get_components().await {
        Ok(components) => components,
        // The realm does not exist yet: everything declared locally will be created.
        Err(e) if crate::client::is_not_found(&e) => Vec::new(),
        Err(e) => {
            return Err(e).with_context(|| {
                format!("Failed to get components for realm '{}'", ctx.realm_name)
            });
        }
    };
    let components = Arc::new(
        crate::apply::components::ComponentResolver::load(
            ctx.client,
            ctx.workspace_dir,
            ctx.profile.as_deref(),
            existing_components,
        )
        .await?,
    );

    let mut set = tokio::task::JoinSet::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if is_yaml_file(&path) {
            // Skip overlay files themselves
            if is_overlay_file(&path, ctx.profile.as_deref()) {
                continue;
            }

            let resolver = Arc::clone(&ctx.resolver);
            let components = Arc::clone(&components);
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

                let remote = components.find_remote(&local_component).cloned();

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
            // Matched by portable key: IDs and parent IDs are environment specific.
            let mut rc = r.clone();
            rc.id = local_component.id.clone();
            rc.parent_id = local_component.parent_id.clone();
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
