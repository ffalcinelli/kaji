#![allow(clippy::collapsible_if)]

use crate::apply::ApplyContext;
use crate::models::{
    AuthenticationExecutionExportRepresentation, AuthenticationFlowRepresentation, KeycloakResource,
};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use tokio::fs as async_fs;

pub async fn apply_authentication_flows(ctx: ApplyContext<'_>) -> Result<()> {
    let ApplyContext {
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
        prune,
        ..
    } = ctx;

    let resources_dir = workspace_dir.join(AuthenticationFlowRepresentation::DIR_NAME);
    if !async_fs::try_exists(&resources_dir).await? {
        return Ok(());
    }

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

    let remote_flows = client
        .get_authentication_flows_raw()
        .await
        .unwrap_or_default();
    let remote_flows_map: HashMap<String, AuthenticationFlowRepresentation> = remote_flows
        .into_iter()
        .filter_map(|f| f.alias.clone().map(|alias| (alias, f)))
        .collect();

    let pb = create_progress_bar(files.len() as u64, "Applying authentication flows");

    for path in files {
        let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
        let local_val_before_sub = val.clone();
        substitute_secrets(&mut val, std::sync::Arc::clone(&resolver)).await?;
        let mut flow: AuthenticationFlowRepresentation = serde_json::from_value(val)
            .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

        let flow_alias = flow
            .alias
            .clone()
            .context("Authentication flow missing 'alias'")?;

        let remote_flow_opt = remote_flows_map.get(&flow_alias);

        // 1. Reconcile top-level flow container
        let final_id;
        if let Some(remote) = remote_flow_opt {
            if review {
                let proceed = ui.confirm(
                    &format!(
                        "Do you want to update authentication flow '{}'?",
                        flow_alias
                    ),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }
            let remote_id = remote.id.clone().context("Remote flow missing 'id'")?;
            flow.id = Some(remote_id.clone());
            client.update_authentication_flow(&remote_id, &flow).await?;
            pb.println(format!(
                "  {} Updated authentication flow container {}",
                SUCCESS_UPDATE, flow_alias
            ));
            final_id = remote_id;
        } else {
            if review {
                let proceed = ui.confirm(
                    &format!(
                        "Do you want to create authentication flow '{}'?",
                        flow_alias
                    ),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    continue;
                }
            }
            flow.id = None;
            client.create_authentication_flow(&flow).await?;
            pb.println(format!(
                "  {} Created authentication flow container {}",
                SUCCESS_CREATE, flow_alias
            ));

            let fresh = client
                .get_authentication_flows_raw()
                .await
                .unwrap_or_default();
            final_id = fresh
                .into_iter()
                .find(|f| f.alias.as_deref() == Some(&flow_alias))
                .and_then(|f| f.id)
                .unwrap_or_default();
        }

        // 2. Reconcile child executions & subflows
        if let Some(local_executions) = &flow.authentication_executions {
            reconcile_flow_executions(client, &flow_alias, local_executions, prune, yes, &*ui)
                .await?;
        }

        if !final_id.is_empty() {
            let enriched_res = client
                .get_resource::<AuthenticationFlowRepresentation>(&final_id)
                .await;
            if let Ok(enriched) = enriched_res {
                crate::apply::generic::check_and_update_enrichment(
                    client,
                    &path,
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

        pb.inc(1);
    }

    pb.finish_with_message("Applied authentication flows");
    Ok(())
}

async fn reconcile_flow_executions(
    client: &crate::client::KeycloakClient,
    flow_alias: &str,
    local_executions: &[AuthenticationExecutionExportRepresentation],
    prune: bool,
    yes: bool,
    ui: &dyn crate::utils::ui::Ui,
) -> Result<()> {
    let mut remote_executions = client
        .get_flow_executions(flow_alias)
        .await
        .unwrap_or_default();

    for local_exec in local_executions {
        if local_exec.authenticator_flow == Some(true) {
            // It's a subflow
            let subflow_alias = local_exec
                .flow_alias
                .as_deref()
                .or(local_exec.authenticator.as_deref());
            let provider = local_exec.authenticator.as_deref().unwrap_or("basic-flow");

            if let Some(sub_alias) = subflow_alias {
                let mut remote_exec = remote_executions
                    .iter()
                    .find(|e| {
                        e.flow_alias.as_deref() == Some(sub_alias)
                            || e.authenticator.as_deref() == Some(sub_alias)
                    })
                    .cloned();

                if remote_exec.is_none() {
                    let _ = client
                        .add_subflow_to_flow(flow_alias, sub_alias, provider, None)
                        .await;
                    remote_executions = client
                        .get_flow_executions(flow_alias)
                        .await
                        .unwrap_or_default();
                    remote_exec = remote_executions
                        .iter()
                        .find(|e| {
                            e.flow_alias.as_deref() == Some(sub_alias)
                                || e.authenticator.as_deref() == Some(sub_alias)
                        })
                        .cloned();
                }

                if let Some(mut re) = remote_exec {
                    if local_exec.requirement.is_some() && re.requirement != local_exec.requirement
                    {
                        re.requirement = local_exec.requirement.clone();
                        let _ = client.update_flow_execution(flow_alias, &re).await;
                    }
                }
            }
        } else if let Some(provider) = &local_exec.authenticator {
            // It's an authenticator execution
            let mut remote_exec = remote_executions
                .iter()
                .find(|e| e.authenticator.as_deref() == Some(provider.as_str()))
                .cloned();

            if remote_exec.is_none() {
                let _ = client.add_execution_to_flow(flow_alias, provider).await;
                remote_executions = client
                    .get_flow_executions(flow_alias)
                    .await
                    .unwrap_or_default();
                remote_exec = remote_executions
                    .iter()
                    .find(|e| e.authenticator.as_deref() == Some(provider.as_str()))
                    .cloned();
            }

            if let Some(mut re) = remote_exec {
                if local_exec.requirement.is_some() && re.requirement != local_exec.requirement {
                    re.requirement = local_exec.requirement.clone();
                    let _ = client.update_flow_execution(flow_alias, &re).await;
                }
            }
        }
    }

    if prune {
        for remote_exec in &remote_executions {
            let is_declared = local_executions.iter().any(|le| {
                if le.authenticator_flow == Some(true) {
                    le.flow_alias == remote_exec.flow_alias
                        || le.authenticator == remote_exec.authenticator
                } else {
                    le.authenticator == remote_exec.authenticator
                }
            });

            if !is_declared {
                let exec_id_opt = &remote_exec.id;
                if let Some(exec_id) = exec_id_opt {
                    let proceed = if yes {
                        true
                    } else {
                        ui.confirm(
                            &format!(
                                "Prune execution/subflow in flow '{}' (ID: {})?",
                                flow_alias, exec_id
                            ),
                            false,
                        )?
                    };
                    if proceed {
                        let _ = client.delete_execution(exec_id).await;
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::KeycloakClient;
    use crate::utils::secrets::EnvResolver;
    use crate::utils::ui::MockUi;
    use mockito::Server;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_apply_auth_flow_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        let ui = Arc::new(MockUi {
            inputs: std::sync::Mutex::new(vec![]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        });
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let ctx = ApplyContext {
            client: &client,
            workspace_dir: dir.path().to_path_buf(),
            secrets_path: Arc::new(dir.path().join(".secrets")),
            resolver,
            planned_files: Arc::new(None),
            realm_name: "test",
            profile: None,
            review: false,
            ui,
            yes: true,
            prune: false,
        };

        apply_authentication_flows(ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_apply_auth_flow_reconcile_and_prune() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let _m_flows = server
            .mock("GET", "/admin/realms/test/authentication/flows")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"id": "f-1", "alias": "existing-flow", "providerId": "basic-flow"}
                ]"#,
            )
            .create_async()
            .await;

        let _m_post_flow = server
            .mock("POST", "/admin/realms/test/authentication/flows")
            .with_status(201)
            .create_async()
            .await;

        let _m_put_flow = server
            .mock("PUT", "/admin/realms/test/authentication/flows/f-1")
            .with_status(204)
            .create_async()
            .await;

        let _m_execs_existing = server
            .mock(
                "GET",
                "/admin/realms/test/authentication/flows/existing-flow/executions",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"id": "e-1", "authenticator": "auth-cookie", "requirement": "ALTERNATIVE"},
                    {"id": "e-orphan", "authenticator": "auth-otp", "requirement": "DISABLED"}
                ]"#,
            )
            .create_async()
            .await;

        let _m_execs_new = server
            .mock(
                "GET",
                "/admin/realms/test/authentication/flows/new-flow/executions",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let _m_add_exec = server
            .mock(
                "POST",
                "/admin/realms/test/authentication/flows/new-flow/executions/execution",
            )
            .with_status(201)
            .create_async()
            .await;

        let _m_add_subflow = server
            .mock(
                "POST",
                "/admin/realms/test/authentication/flows/new-flow/executions/flow",
            )
            .with_status(201)
            .create_async()
            .await;

        let _m_update_exec = server
            .mock(
                "PUT",
                "/admin/realms/test/authentication/flows/existing-flow/executions",
            )
            .with_status(204)
            .create_async()
            .await;

        let _m_del_exec = server
            .mock(
                "DELETE",
                "/admin/realms/test/authentication/executions/e-orphan",
            )
            .with_status(204)
            .create_async()
            .await;

        let mut client = KeycloakClient::new(url);
        client.set_target_realm("test".to_string());
        client.set_token("token".to_string());

        let dir = tempfile::tempdir().unwrap();
        let ws_dir = dir.path().to_path_buf();
        let flows_dir = ws_dir.join("authentication-flows");
        fs::create_dir_all(&flows_dir).unwrap();

        fs::write(
            flows_dir.join("existing-flow.yaml"),
            r#"
alias: existing-flow
authenticationExecutions:
  - authenticator: auth-cookie
    requirement: REQUIRED
"#,
        )
        .unwrap();

        fs::write(
            flows_dir.join("new-flow.yaml"),
            r#"
alias: new-flow
authenticationExecutions:
  - authenticator: auth-password
    requirement: REQUIRED
  - authenticator: sub-flow-1
    authenticatorFlow: true
    requirement: REQUIRED
"#,
        )
        .unwrap();

        let ui = Arc::new(MockUi {
            inputs: std::sync::Mutex::new(vec![]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        });
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let ctx = ApplyContext {
            client: &client,
            workspace_dir: ws_dir,
            secrets_path: Arc::new(dir.path().join(".secrets")),
            resolver,
            planned_files: Arc::new(None),
            realm_name: "test",
            profile: None,
            review: false,
            ui,
            yes: true,
            prune: true,
        };

        apply_authentication_flows(ctx).await.unwrap();
    }
}
