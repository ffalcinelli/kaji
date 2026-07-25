use crate::client::KeycloakClient;
use crate::models::RealmRepresentation;
use crate::utils::secrets::{SecretResolver, substitute_secrets};
use crate::utils::ui::{SUCCESS_UPDATE, Ui};
use crate::utils::yaml::load_yaml_with_overlay;
use anyhow::{Context, Result};
use console::style;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

#[allow(clippy::too_many_arguments)]
pub async fn apply_realm(
    client: &KeycloakClient,
    workspace_dir: &std::path::Path,
    secrets_path: Arc<PathBuf>,
    resolver: Arc<dyn SecretResolver>,
    planned_files: Arc<Option<HashSet<PathBuf>>>,
    realm_name: &str,
    profile: Option<String>,
    ui: Arc<dyn Ui>,
    yes: bool,
) -> Result<()> {
    // 1. Apply Realm
    let realm_path = workspace_dir.join("realm.yaml");
    if let Some(plan) = &*planned_files
        && !plan.contains(&realm_path)
    {
        return Ok(());
    }
    if async_fs::try_exists(&realm_path).await? {
        let mut val = load_yaml_with_overlay(&realm_path, profile.as_deref()).await?;
        let local_val_before_sub = val.clone();
        substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
        let realm_rep: RealmRepresentation = serde_json::from_value(val)?;
        client
            .update_realm(&realm_rep)
            .await
            .with_context(|| format!("Failed to update realm '{}'", realm_name))?;
        eprintln!(
            "  {} {}",
            SUCCESS_UPDATE,
            style("Updated realm configuration").cyan()
        );

        if let Ok(enriched) = client.get_realm().await {
            crate::apply::generic::check_and_update_enrichment(
                client,
                &realm_path,
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
    Ok(())
}
