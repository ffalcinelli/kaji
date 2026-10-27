#![allow(clippy::collapsible_if)]

use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, KeycloakResource,
};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, create_progress_bar};
use crate::utils::yaml::{deserialize_value, list_yaml_files, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;

use std::sync::Arc;

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
    let Some(mut files) = list_yaml_files(&resources_dir, profile.as_deref()).await? else {
        return Ok(());
    };
    if let Some(plan) = planned_files.as_ref() {
        files.retain(|path| plan.contains(path));
    }
    if files.is_empty() {
        return Ok(());
    }

    // 1. Fetch remote configs
    let remote_configs = client
        .get_authenticator_configs_internal()
        .await
        .context("Failed to get authenticator configs from Keycloak")?;
    let remote_map: HashMap<String, AuthenticatorConfigRepresentation> = remote_configs
        .into_iter()
        .filter_map(|c| c.alias.clone().map(|alias| (alias, c)))
        .collect();

    // 2. Executions of the local flows, to find the execution a new config belongs to. Flow
    // files that cannot be parsed were already reported by the flows stage.
    let local_flows_dir = workspace_dir.join(AuthenticationFlowRepresentation::DIR_NAME);
    let mut local_flows_map: HashMap<
        String,
        Vec<crate::models::AuthenticationExecutionExportRepresentation>,
    > = HashMap::new();
    let flow_files = list_yaml_files(&local_flows_dir, profile.as_deref())
        .await?
        .unwrap_or_default();
    for flow_path in flow_files {
        if let Ok(flow_val) = load_yaml_with_overlay(&flow_path, profile.as_deref()).await {
            if let Ok(flow) = serde_json::from_value::<AuthenticationFlowRepresentation>(flow_val) {
                if let (Some(flow_alias), Some(executions)) =
                    (flow.alias, flow.authentication_executions)
                {
                    local_flows_map.insert(flow_alias, executions);
                }
            }
        }
    }

    let pb = create_progress_bar(files.len() as u64, "Applying authenticator configs");

    // 3. Process each config
    for path in files {
        async {
            let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            let local_val_before_sub = val.clone();
            substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
            let local_val_resolved = val.clone();
            let mut local_config: AuthenticatorConfigRepresentation = deserialize_value(val)?;

            let alias = local_config
                .alias
                .clone()
                .context("The authenticator config has no 'alias'")?;

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
                        return Ok(());
                    }
                }
                let remote_id = remote.id.clone().with_context(|| {
                    format!("Remote authenticator config '{}' has no ID", alias)
                })?;
                local_config.id = Some(remote_id.clone());
                client
                    .update_resource(&remote_id, &local_config)
                    .await
                    .with_context(|| format!("Failed to update authenticator config '{}'", alias))?;
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
                        return Ok(());
                    }
                }

                // Find an execution in local flows referencing this alias
                let mut referencing_execution = None; // (flow_alias, provider_id)
                for (flow_alias, executions) in &local_flows_map {
                    for exec in executions {
                        if exec.authenticator_config.as_deref() == Some(&alias) {
                            if let Some(provider_id) = &exec.authenticator {
                                referencing_execution =
                                    Some((flow_alias.clone(), provider_id.clone()));
                                break;
                            }
                        }
                    }
                    if referencing_execution.is_some() {
                        break;
                    }
                }

                let (flow_alias, provider_id) = referencing_execution
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Hint: Keycloak ties a config to one execution: set `authenticatorConfig: {}` on an execution of a flow in {}",
                            alias,
                            local_flows_dir.display()
                        )
                    })
                    .with_context(|| {
                        format!(
                            "No local authentication execution references authenticator config '{}'",
                            alias
                        )
                    })?;

                // Find the matching direct child execution of the remote flow without a config.
                // (Configs belong to exactly one execution in Keycloak.)
                let remote_exec = client
                    .get_flow_executions(&flow_alias)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to list the executions of authentication flow '{}'",
                            flow_alias
                        )
                    })?
                    .into_iter()
                    .filter(|e| e.level.unwrap_or(0) == 0 && !e.is_subflow())
                    .filter(|e| e.provider_id.as_deref() == Some(provider_id.as_str()))
                    .min_by_key(|e| e.authentication_config.is_some())
                    .with_context(|| {
                        format!(
                            "Authentication flow '{}' on the server has no '{}' execution for authenticator config '{}'",
                            flow_alias, provider_id, alias
                        )
                    })?;

                let execution_id = remote_exec.id.with_context(|| {
                    format!(
                        "Execution '{}' of flow '{}' has no ID",
                        provider_id, flow_alias
                    )
                })?;

                // Create config on Keycloak
                let new_config_id = client
                    .create_authenticator_config_for_execution(&execution_id, &local_config)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to attach authenticator config '{}' to execution '{}' of flow '{}'",
                            alias, provider_id, flow_alias
                        )
                    })?;

                crate::utils::ui::report(
                    &pb,
                    format!(
                        "  {} Created authenticator config {} (associated with execution {})",
                        SUCCESS_CREATE, alias, execution_id
                    ),
                );
                final_id = new_config_id;
            }

            match client
                .get_resource::<AuthenticatorConfigRepresentation>(&final_id)
                .await
            {
                Err(e) => crate::utils::ui::report(
                    &pb,
                    crate::apply::generic::enrichment_skipped_warning(
                        "authenticator config",
                        &alias,
                        &e,
                    ),
                ),
                Ok(enriched) => {
                    crate::apply::generic::check_and_update_enrichment(
                        &path,
                        profile.as_deref(),
                        crate::apply::generic::LocalSource {
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
                    .await?
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await
        .with_context(|| path.display().to_string())?;

        pb.inc(1);
    }
    pb.finish_with_message("Applied authenticator configs");
    Ok(())
}
