use crate::models::{AuthenticationFlowRepresentation, KeycloakResource, ResourceMeta};
use crate::plan::{PlanContext, PlanSummary, print_diff};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::SPARKLE;
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

pub async fn plan_authentication_flows(
    ctx: &PlanContext<'_>,
) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let resources_dir = ctx
        .workspace_dir
        .join(AuthenticationFlowRepresentation::DIR_NAME);
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();

    if !async_fs::try_exists(&resources_dir).await? {
        return Ok((changed_files, summary));
    }

    let existing_flows = ctx
        .client
        .get_raw_flows_with_executions()
        .await
        .with_context(|| {
            format!(
                "Failed to get authentication flows for realm '{}'",
                ctx.realm_name
            )
        })?;

    let existing_map: HashMap<String, AuthenticationFlowRepresentation> = existing_flows
        .into_iter()
        .filter_map(|f| f.alias.clone().map(|alias| (alias, f)))
        .collect();
    let existing_map = Arc::new(existing_map);

    let mut set = tokio::task::JoinSet::new();
    let mut entries = async_fs::read_dir(&resources_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "yaml") {
            if is_overlay_file(&path, ctx.profile.as_deref()) {
                continue;
            }

            let resolver = Arc::clone(&ctx.resolver);
            let existing_map = Arc::clone(&existing_map);
            let profile = ctx.profile.clone();

            set.spawn(async move {
                let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
                substitute_secrets(&mut val, resolver).await?;
                let local: AuthenticationFlowRepresentation = serde_json::from_value(val)
                    .with_context(|| format!("Failed to deserialize YAML file {:?}", path))?;

                let identity = local
                    .alias
                    .clone()
                    .context("Authentication flow missing 'alias'")?;
                let remote = existing_map.get(&identity).cloned();

                Ok::<
                    (
                        AuthenticationFlowRepresentation,
                        PathBuf,
                        Option<AuthenticationFlowRepresentation>,
                    ),
                    anyhow::Error,
                >((local, path, remote))
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
            let diff_name = format!(
                "{} {}",
                AuthenticationFlowRepresentation::LABEL,
                local.get_name()
            );
            let ch = print_diff(
                &diff_name,
                Some(&rc),
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                AuthenticationFlowRepresentation::SECRET_PREFIX,
            )?;
            remote_clone = Some(rc);
            ch
        } else {
            eprintln!(
                "\n{} Will create {} {}",
                SPARKLE,
                AuthenticationFlowRepresentation::LABEL,
                local.get_name()
            );
            print_diff(
                &format!(
                    "{} {}",
                    AuthenticationFlowRepresentation::LABEL,
                    local.get_name()
                ),
                None::<&AuthenticationFlowRepresentation>,
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                AuthenticationFlowRepresentation::SECRET_PREFIX,
            )?
        };

        if changed {
            let mut include = true;
            if ctx.options.interactive {
                include = crate::plan::prompt_interactive_change(
                    ctx.ui,
                    &format!(
                        "{} {}",
                        AuthenticationFlowRepresentation::LABEL,
                        local.get_name()
                    ),
                    remote_clone.as_ref(),
                    &local,
                    AuthenticationFlowRepresentation::SECRET_PREFIX,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::KeycloakClient;
    use crate::plan::PlanOptions;
    use crate::utils::secrets::EnvResolver;
    use crate::utils::ui::MockUi;
    use mockito::Server;
    use std::fs;

    #[tokio::test]
    async fn test_plan_auth_flow_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        let ui = Arc::new(MockUi {
            inputs: std::sync::Mutex::new(vec![]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        });
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let ctx = PlanContext {
            client: &client,
            workspace_dir: dir.path(),
            options: PlanOptions {
                changes_only: false,
                interactive: false,
                verbose: false,
            },
            realm_name: "test",
            ui: ui.as_ref(),
            resolver,
            profile: None,
        };

        let (files, summary) = plan_authentication_flows(&ctx).await.unwrap();
        assert!(files.is_empty());
        assert_eq!(summary.created, 0);
    }

    #[tokio::test]
    async fn test_plan_auth_flow_create_and_update() {
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

        let _m_execs = server
            .mock(
                "GET",
                "/admin/realms/test/authentication/flows/existing-flow/executions",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
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
            "alias: existing-flow\ndescription: Updated\n",
        )
        .unwrap();
        fs::write(flows_dir.join("new-flow.yaml"), "alias: new-flow\n").unwrap();
        fs::write(flows_dir.join("flow.prod.yaml"), "overlay").unwrap();
        fs::write(flows_dir.join("skip.txt"), "text").unwrap();

        let ui = Arc::new(MockUi {
            inputs: std::sync::Mutex::new(vec![]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![]),
            passwords: std::sync::Mutex::new(vec![]),
        });
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let ctx = PlanContext {
            client: &client,
            workspace_dir: &ws_dir,
            options: PlanOptions {
                changes_only: false,
                interactive: false,
                verbose: true,
            },
            realm_name: "test",
            ui: ui.as_ref(),
            resolver,
            profile: None,
        };

        let (files, summary) = plan_authentication_flows(&ctx).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(summary.created, 1);
        assert_eq!(summary.updated, 1);
    }
}
