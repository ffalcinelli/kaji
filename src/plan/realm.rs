use crate::utils::secrets::substitute_secrets;
use crate::utils::yaml::load_yaml_with_overlay;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

use super::{print_diff, PlanContext, PlanSummary};

pub async fn plan_realm(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    let realm_path = ctx.workspace_dir.join("realm.yaml");
    if async_fs::try_exists(&realm_path).await? {
        let mut val = load_yaml_with_overlay(&realm_path, ctx.profile.as_deref()).await?;
        substitute_secrets(&mut val, Arc::clone(&ctx.resolver)).await?;
        let local_realm: crate::models::RealmRepresentation = serde_json::from_value(val)
            .with_context(|| format!("Failed to deserialize YAML file: {:?}", realm_path))?;

        // We handle the case where remote realm fetch might fail (e.g. if we are creating it)
        // by treating it as None (creation). However, usually plan is run against existing realm.
        let remote_realm = match ctx.client.get_realm().await {
            Ok(r) => Some(r),
            Err(e) => {
                // Check if it's a 404 (Not Found)
                if e.to_string().contains("404") {
                    None
                } else {
                    return Err(e).with_context(|| {
                        format!("Failed to get realm '{}' from Keycloak", ctx.realm_name)
                    });
                }
            }
        };

        let is_update = remote_realm.is_some();
        if print_diff(
            "Realm",
            remote_realm.as_ref(),
            &local_realm,
            ctx.options.changes_only,
            ctx.options.verbose,
            "realm",
        )? {
            let mut include = true;
            if ctx.options.interactive {
                include = super::prompt_interactive_change(
                    ctx.ui,
                    "Realm",
                    remote_realm.as_ref(),
                    &local_realm,
                    "realm",
                )?;
            }
            if include {
                changed_files.push(realm_path);
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
