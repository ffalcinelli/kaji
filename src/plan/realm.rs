use crate::models::KeycloakResource;
use crate::utils::secrets::substitute_secrets;
use crate::utils::yaml::{from_value_at, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use super::{PlanContext, PlanSummary};

pub async fn plan_realm(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();
    let realm_path = crate::utils::yaml::find_realm_file(ctx.workspace_dir).await;
    let mut val = match load_yaml_with_overlay(&realm_path, ctx.profile.as_deref()).await {
        Ok(v) => v,
        Err(e) => {
            if e.root_cause()
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok((changed_files, summary));
            }
            return Err(e);
        }
    };

    substitute_secrets(&mut val, Arc::clone(&ctx.resolver))
        .await
        .with_context(|| realm_path.display().to_string())?;
    let local_realm: crate::models::RealmRepresentation = from_value_at(val, &realm_path)?;

    // We handle the case where remote realm fetch might fail (e.g. if we are creating it)
    // by treating it as None (creation). However, usually plan is run against existing realm.
    let remote_realm = match ctx.client.get_realm().await {
        Ok(r) => Some(r),
        Err(e) => {
            if crate::client::is_not_found(&e) {
                None
            } else {
                return Err(e).context("Failed to get the realm settings from Keycloak");
            }
        }
    };

    // The server-assigned realm id is environment specific: ignore it unless declared locally.
    let remote_realm = remote_realm.map(|mut r| {
        if !local_realm.extra.contains_key("id") {
            r.clear_metadata();
        }
        r
    });

    let is_update = remote_realm.is_some();
    if super::print_resource_diff(
        "Realm",
        remote_realm.as_ref(),
        &local_realm,
        ctx.options.changes_only,
        ctx.options.verbose,
        "realm",
        crate::models::RealmRepresentation::PARTIAL_UPDATES,
    )? {
        let mut include = true;
        if ctx.options.interactive {
            include = super::prompt_interactive_resource_change(
                ctx.ui,
                "Realm",
                remote_realm.as_ref(),
                &local_realm,
                "realm",
                crate::models::RealmRepresentation::PARTIAL_UPDATES,
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
    Ok((changed_files, summary))
}
