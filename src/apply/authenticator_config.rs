#![allow(clippy::collapsible_if)]

use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, KeycloakResource,
};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;

use std::sync::Arc;
use tokio::fs as async_fs;

pub async fn apply_authenticator_configs(ctx: crate::apply::ApplyContext<'_>) -> Result<()> {
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
        prompt_mutex,
        ..
    } = ctx;

    let resources_dir = workspace_dir.join(AuthenticatorConfigRepresentation::DIR_NAME);
    let mut entries = match async_fs::read_dir(&resources_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    // 1. Fetch remote configs
    let remote_configs = client.get_authenticator_configs_internal().await?;
    let remote_map: HashMap<String, AuthenticatorConfigRepresentation> = remote_configs
        .into_iter()
        .filter_map(|c| c.alias.clone().map(|alias| (alias, c)))
        .collect();

    // 2. Read local config files
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if planned_files
            .as_ref()
            .as_ref()
            .is_some_and(|plan| !plan.contains(&path))
        {
            continue;
        }
        if !is_yaml_file(&path) {
            continue;
        }
        if is_overlay_file(&path, profile.as_deref()) {
            continue;
        }
        files.push(path);
    }

    if files.is_empty() {
        return Ok(());
    }

    // Pre-parse local flows
    let local_flows_dir = workspace_dir.join("authentication-flows");
    let mut local_flows_map: HashMap<
        String,
        Vec<crate::models::AuthenticationExecutionExportRepresentation>,
    > = HashMap::new();
    match async_fs::read_dir(&local_flows_dir).await {
        Ok(mut flow_entries) => {
            while let Some(flow_entry) = flow_entries.next_entry().await? {
                let flow_path = flow_entry.path();
                if is_yaml_file(&flow_path) {
                    if is_overlay_file(&flow_path, profile.as_deref()) {
                        continue;
                    }
                    if let Ok(flow_val) =
                        load_yaml_with_overlay(&flow_path, profile.as_deref()).await
                    {
                        if let Ok(flow) =
                            serde_json::from_value::<AuthenticationFlowRepresentation>(flow_val)
                        {
                            if let (Some(flow_alias), Some(executions)) =
                                (flow.alias, flow.authentication_executions)
                            {
                                local_flows_map.insert(flow_alias, executions);
                            }
                        }
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    let pb = create_progress_bar(files.len() as u64, "Applying authenticator configs");

    // 3. Process each config
    for path in files {
        let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
        let local_val_before_sub = val.clone();
        substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
        let local_val_resolved = val.clone();
        let mut local_config: AuthenticatorConfigRepresentation = serde_json::from_value(val)
            .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

        let alias = local_config
            .alias
            .clone()
            .context("Config is missing 'alias'")?;

        let final_id;

        if let Some(remote) = remote_map.get(&alias) {
            // Config exists! Update it
            if review {
                let proceed = {
                    let _lock = prompt_mutex.lock().await;
                    ui.confirm(
                        &format!("Do you want to update authenticator config '{}'?", alias),
                        true,
                    )?
                };
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }
            let remote_id = remote.id.clone().context("Remote config is missing 'id'")?;
            local_config.id = Some(remote_id.clone());
            client.update_resource(&remote_id, &local_config).await?;
            crate::utils::ui::report(
                &pb,
                format!(
                    "  {} Updated authenticator config {}",
                    SUCCESS_UPDATE, alias
                ),
            );
            final_id = remote_id;
        } else {
            // New config! Create it
            if review {
                let proceed = {
                    let _lock = prompt_mutex.lock().await;
                    ui.confirm(
                        &format!("Do you want to create authenticator config '{}'?", alias),
                        true,
                    )?
                };
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }

            // Find an execution in local flows referencing this alias
            let mut referencing_execution = None; // (flow_alias, provider_id)
            for (flow_alias, executions) in &local_flows_map {
                for exec in executions {
                    if exec.authenticator_config.as_deref() == Some(&alias) {
                        if let Some(provider_id) = &exec.authenticator {
                            referencing_execution = Some((flow_alias.clone(), provider_id.clone()));
                            break;
                        }
                    }
                }
                if referencing_execution.is_some() {
                    break;
                }
            }

            let (flow_alias, provider_id) = referencing_execution.with_context(|| format!(
                "Could not find any local authentication execution referencing authenticator config '{}'",
                alias
            ))?;

            // Find the matching direct child execution of the remote flow without a config.
            // (Configs belong to exactly one execution in Keycloak.)
            let remote_exec = client
                .get_flow_executions(&flow_alias)
                .await?
                .into_iter()
                .filter(|e| e.level.unwrap_or(0) == 0 && !e.is_subflow())
                .filter(|e| e.provider_id.as_deref() == Some(provider_id.as_str()))
                .min_by_key(|e| e.authentication_config.is_some())
                .with_context(|| {
                    format!(
                        "Could not find remote execution with provider '{}' in flow '{}'",
                        provider_id, flow_alias
                    )
                })?;

            let execution_id = remote_exec.id.context("Remote execution is missing 'id'")?;

            // Create config on Keycloak
            let new_config_id = client
                .create_authenticator_config_for_execution(&execution_id, &local_config)
                .await?;

            crate::utils::ui::report(
                &pb,
                format!(
                    "  {} Created authenticator config {} (associated with execution {})",
                    SUCCESS_CREATE, alias, execution_id
                ),
            );
            final_id = new_config_id;
        }

        if let Ok(enriched) = client
            .get_resource::<AuthenticatorConfigRepresentation>(&final_id)
            .await
        {
            crate::apply::generic::check_and_update_enrichment(
                crate::apply::generic::LocalSource {
                    path: &path,
                    profile: profile.as_deref(),
                    before_sub: &local_val_before_sub,
                    resolved: &local_val_resolved,
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

        pb.inc(1);
    }
    pb.finish_with_message("Applied authenticator configs");
    Ok(())
}
