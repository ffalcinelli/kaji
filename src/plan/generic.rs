use crate::models::{KeycloakResource, ResourceMeta};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::SPARKLE;
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

use super::{print_diff, PlanContext, PlanSummary};

#[rustfmt::skip]
pub async fn plan_resources<T>(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)>
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
    let dir_name = T::DIR_NAME;
    let resources_dir = ctx.workspace_dir.join(dir_name);
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    if !async_fs::try_exists(&resources_dir).await? {
        return Ok((changed_files, summary));
    }

    let existing_resources =
        ctx.client.get_resources::<T>().await.with_context(|| format!("Failed to get {} for realm '{}'", T::LABEL, ctx.realm_name))?;

    let existing_map: HashMap<String, T> = existing_resources
        .into_iter()
        .filter_map(|r| r.get_identity().map(|id| (id, r)))
        .collect();
    let existing_map = Arc::new(existing_map);

    let mut set = tokio::task::JoinSet::new();
    let mut entries = async_fs::read_dir(&resources_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "yaml") {
            // Skip overlay files themselves
            if is_overlay_file(&path, ctx.profile.as_deref()) {
                continue;
            }

            let resolver = Arc::clone(&ctx.resolver);
            let existing_map = Arc::clone(&existing_map);
            let realm_name = ctx.realm_name.to_string();
            let profile = ctx.profile.clone();

            set.spawn(async move {
                let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
                substitute_secrets(&mut val, resolver).await?;
                let local: T = serde_json::from_value(val).with_context(|| format!("Failed to deserialize YAML file {:?} in realm '{}'", path, realm_name))?;

                let identity = local.get_identity().with_context(|| format!("Failed to get identity for {} in {:?} in realm '{}'", T::LABEL, path, realm_name))?;
                let remote = existing_map.get(&identity).cloned();

                Ok::<(T, PathBuf, Option<T>), anyhow::Error>((local, path, remote))
            });
        }
    }

    for res in crate::utils::join_all_tasks(set, None).await? {
        let (local, path, remote) = res;

        let is_update = remote.is_some();
        let mut remote_clone = None;
        let changed = if let Some(r) = remote {
            let mut rc = r.clone();
            if !local.has_id() {
                rc.clear_metadata();
            }
            let diff_name = format!("{} {}", T::LABEL, local.get_name());
            let ch = print_diff(&diff_name, Some(&rc), &local, ctx.options.changes_only, ctx.options.verbose, T::SECRET_PREFIX)?;
            remote_clone = Some(rc);
            ch
        } else {
            eprintln!("\n{} Will create {}", SPARKLE, T::LABEL);
            print_diff(&format!("{} {}", T::LABEL, local.get_name()), None::<&T>, &local, ctx.options.changes_only, ctx.options.verbose, T::SECRET_PREFIX)?
        };

        if changed {
            let mut include = true;
            if ctx.options.interactive {
                include = super::prompt_interactive_change(ctx.ui, &format!("{} {}", T::LABEL, local.get_name()), remote_clone.as_ref(), &local, T::SECRET_PREFIX)?;
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
