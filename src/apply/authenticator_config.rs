#![allow(clippy::collapsible_if)]

use crate::client::KeycloakClient;
use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, KeycloakResource,
};
use crate::utils::secrets::{SecretResolver, substitute_secrets};
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, Ui, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

#[allow(clippy::too_many_arguments)]
pub async fn apply_authenticator_configs(
    client: &KeycloakClient,
    workspace_dir: &std::path::Path,
    resolver: Arc<dyn SecretResolver>,
    planned_files: Arc<Option<HashSet<PathBuf>>>,
    _realm_name: &str,
    profile: Option<String>,
    review: bool,
    ui: Arc<dyn Ui>,
) -> Result<()> {
    let resources_dir = workspace_dir.join(AuthenticatorConfigRepresentation::DIR_NAME);
    if !async_fs::try_exists(&resources_dir).await? {
        return Ok(());
    }

    // 1. Fetch remote configs
    let remote_configs = client.get_authenticator_configs_internal().await?;
    let remote_map: HashMap<String, AuthenticatorConfigRepresentation> = remote_configs
        .into_iter()
        .filter_map(|c| c.alias.clone().map(|alias| (alias, c)))
        .collect();

    // 2. Read local config files
    let mut entries = async_fs::read_dir(&resources_dir).await?;
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
        if path.extension().is_none_or(|ext| ext != "yaml") {
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

    let pb = create_progress_bar(files.len() as u64, "Applying authenticator configs");

    // 3. Process each config
    for path in files {
        let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
        substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
        let mut local_config: AuthenticatorConfigRepresentation = serde_json::from_value(val)
            .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

        let alias = local_config
            .alias
            .clone()
            .context("Config is missing 'alias'")?;

        if let Some(remote) = remote_map.get(&alias) {
            // Config exists! Update it
            if review {
                let proceed = ui.confirm(
                    &format!("Do you want to update authenticator config '{}'?", alias),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }
            let remote_id = remote.id.clone().context("Remote config is missing 'id'")?;
            local_config.id = Some(remote_id.clone());
            client.update_resource(&remote_id, &local_config).await?;
            pb.println(format!(
                "  {} Updated authenticator config {}",
                SUCCESS_UPDATE, alias
            ));
        } else {
            // New config! Create it
            if review {
                let proceed = ui.confirm(
                    &format!("Do you want to create authenticator config '{}'?", alias),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }

            // Find an execution in local flows referencing this alias
            let local_flows_dir = workspace_dir.join("authentication-flows");
            let mut referencing_execution = None; // (flow_alias, provider_id)
            if async_fs::try_exists(&local_flows_dir).await? {
                let mut flow_entries = async_fs::read_dir(&local_flows_dir).await?;
                while let Some(flow_entry) = flow_entries.next_entry().await? {
                    let flow_path = flow_entry.path();
                    if flow_path.extension().is_some_and(|ext| ext == "yaml") {
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
                                    (&flow.alias, &flow.authentication_executions)
                                {
                                    for exec in executions {
                                        if exec.authenticator_config.as_deref() == Some(&alias) {
                                            if let Some(provider_id) = &exec.authenticator {
                                                referencing_execution =
                                                    Some((flow_alias.clone(), provider_id.clone()));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if referencing_execution.is_some() {
                        break;
                    }
                }
            }

            let (flow_alias, provider_id) = referencing_execution.context(format!(
                "Could not find any local authentication execution referencing authenticator config '{}'",
                alias
            ))?;

            // Fetch remote executions for this flow to get the execution ID
            let remote_executions = client.get_flow_executions(&flow_alias).await?;
            let remote_exec = remote_executions
                .into_iter()
                .find(|e| e.authenticator.as_deref() == Some(&provider_id))
                .context(format!(
                    "Could not find remote execution with provider '{}' in flow '{}'",
                    provider_id, flow_alias
                ))?;

            let execution_id = remote_exec.id.context("Remote execution is missing 'id'")?;

            // Create config on Keycloak
            let created_config = client
                .create_authenticator_config_for_execution(&execution_id, &local_config)
                .await?;
            let new_config_id = created_config
                .id
                .context("Created config is missing 'id'")?;

            pb.println(format!(
                "  {} Created authenticator config {} (associated with execution {})",
                SUCCESS_CREATE, alias, execution_id
            ));

            // Link this config to all other referencing executions
            // Scan all remote flows & executions
            let remote_flows = client.get_authentication_flows_raw().await?;
            for remote_flow in remote_flows {
                if let Some(r_flow_alias) = &remote_flow.alias {
                    if let Ok(executions) = client.get_flow_executions(r_flow_alias).await {
                        for mut exec in executions {
                            // Check if this execution is supposed to be linked locally
                            // (we find it in local flows by flow_alias and provider_id)
                            let mut should_link = false;
                            // Search local flows
                            if async_fs::try_exists(&local_flows_dir).await? {
                                let mut flow_entries = async_fs::read_dir(&local_flows_dir).await?;
                                while let Some(flow_entry) = flow_entries.next_entry().await? {
                                    let flow_path = flow_entry.path();
                                    if flow_path.extension().is_some_and(|ext| ext == "yaml") {
                                        if is_overlay_file(&flow_path, profile.as_deref()) {
                                            continue;
                                        }
                                        if let Ok(flow_val) =
                                            load_yaml_with_overlay(&flow_path, profile.as_deref())
                                                .await
                                        {
                                            if let Ok(flow) = serde_json::from_value::<
                                                AuthenticationFlowRepresentation,
                                            >(
                                                flow_val
                                            ) {
                                                if flow.alias.as_ref() == Some(r_flow_alias) {
                                                    if let Some(loc_execs) =
                                                        &flow.authentication_executions
                                                    {
                                                        for loc_exec in loc_execs {
                                                            if loc_exec.authenticator
                                                                == exec.authenticator
                                                            {
                                                                if loc_exec
                                                                    .authenticator_config
                                                                    .as_deref()
                                                                    == Some(&alias)
                                                                {
                                                                    should_link = true;
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if should_link {
                                        break;
                                    }
                                }
                            }

                            // If it should link, and is not already linked to this config ID:
                            if should_link
                                && exec.authenticator_config.as_ref() != Some(&new_config_id)
                            {
                                exec.authenticator_config = Some(new_config_id.clone());
                                client.update_flow_execution(r_flow_alias, &exec).await?;
                            }
                        }
                    }
                }
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("Applied authenticator configs");
    Ok(())
}
