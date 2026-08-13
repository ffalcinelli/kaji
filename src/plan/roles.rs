#![allow(clippy::collapsible_if)]

use crate::models::{KeycloakResource, ResourceMeta, RoleRepresentation};
use crate::plan::{PlanContext, PlanSummary, print_diff};
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::SPARKLE;
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

pub async fn plan_roles(ctx: &PlanContext<'_>) -> Result<(Vec<PathBuf>, PlanSummary)> {
    let mut changed_files = Vec::new();
    let mut summary = PlanSummary::default();

    // 1. Discover all role files (both realm roles and client roles)
    let mut role_files = Vec::new();

    // Realm roles dir: <workspace_dir>/roles
    let realm_roles_dir = ctx.workspace_dir.join("roles");
    if async_fs::try_exists(&realm_roles_dir).await? {
        let mut entries = async_fs::read_dir(&realm_roles_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|ext| ext == "yaml")
                && !is_overlay_file(&path, ctx.profile.as_deref())
            {
                role_files.push(path);
            }
        }
    }

    // Client roles dirs: <workspace_dir>/clients/<client_id>/roles
    let clients_dir = ctx.workspace_dir.join("clients");
    if async_fs::try_exists(&clients_dir).await? {
        let mut client_entries = async_fs::read_dir(&clients_dir).await?;
        while let Some(client_entry) = client_entries.next_entry().await? {
            let client_path = client_entry.path();
            if client_path.is_dir() {
                let client_roles_dir = client_path.join("roles");
                if async_fs::try_exists(&client_roles_dir).await? {
                    let mut role_entries = async_fs::read_dir(&client_roles_dir).await?;
                    while let Some(role_entry) = role_entries.next_entry().await? {
                        let path = role_entry.path();
                        if path.is_file()
                            && path.extension().is_some_and(|ext| ext == "yaml")
                            && !is_overlay_file(&path, ctx.profile.as_deref())
                        {
                            role_files.push(path);
                        }
                    }
                }
            }
        }
    }

    if role_files.is_empty() {
        return Ok((changed_files, summary));
    }

    // 2. Fetch remote realm roles
    let remote_realm_roles = ctx
        .client
        .get_roles()
        .await
        .with_context(|| format!("Failed to get roles for realm '{}'", ctx.realm_name))?;
    let realm_roles_map: HashMap<String, RoleRepresentation> = remote_realm_roles
        .into_iter()
        .map(|r| (r.name.clone(), r))
        .collect();

    // 3. Fetch remote client roles for all clients
    let mut client_roles_map: HashMap<(String, String), RoleRepresentation> = HashMap::new();
    if let Ok(clients) = ctx.client.get_clients().await {
        for cl in clients {
            if let (Some(client_id), Some(client_uuid)) = (cl.client_id, cl.id) {
                let c_roles = ctx
                    .client
                    .get_client_roles(&client_uuid)
                    .await
                    .unwrap_or_default();
                for r in c_roles {
                    client_roles_map.insert((client_id.clone(), r.name.clone()), r);
                }
            }
        }
    }

    let realm_roles_map = Arc::new(realm_roles_map);
    let client_roles_map = Arc::new(client_roles_map);

    let mut set = tokio::task::JoinSet::new();

    for path in role_files {
        let resolver = Arc::clone(&ctx.resolver);
        let realm_roles_map = Arc::clone(&realm_roles_map);
        let client_roles_map = Arc::clone(&client_roles_map);
        let profile = ctx.profile.clone();

        set.spawn(async move {
            let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            substitute_secrets(&mut val, resolver).await?;
            let local: RoleRepresentation = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize role YAML file {:?}", path))?;

            // Check if this is a client role
            let is_client_role =
                local.client_role || path.components().any(|c| c.as_os_str() == "clients");

            let target_client_id = if is_client_role {
                let mut client_id = None;
                let components: Vec<_> = path.components().collect();
                for i in 0..components.len() {
                    if components[i].as_os_str() == "clients" && i + 1 < components.len() {
                        client_id =
                            Some(components[i + 1].as_os_str().to_string_lossy().to_string());
                        break;
                    }
                }
                client_id.or_else(|| local.container_id.clone())
            } else {
                None
            };

            let remote = if let Some(ref cid) = target_client_id {
                client_roles_map
                    .get(&(cid.clone(), local.name.clone()))
                    .cloned()
            } else {
                realm_roles_map.get(&local.name).cloned()
            };

            Ok::<
                (
                    RoleRepresentation,
                    PathBuf,
                    Option<RoleRepresentation>,
                    Option<String>,
                ),
                anyhow::Error,
            >((local, path, remote, target_client_id))
        });
    }

    for res in crate::utils::join_all_tasks(set, None).await? {
        let (local, path, remote, target_client_id) = res;
        let is_update = remote.is_some();
        let mut remote_clone = None;

        let label = if let Some(ref cid) = target_client_id {
            format!("client role ({})", cid)
        } else {
            "role".to_string()
        };

        let changed = if let Some(r) = remote {
            let mut rc = r.clone();
            if !local.has_id() {
                rc.clear_metadata();
            }
            let diff_name = format!("{} {}", label, local.get_name());
            let ch = print_diff(
                &diff_name,
                Some(&rc),
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                RoleRepresentation::SECRET_PREFIX,
            )?;
            remote_clone = Some(rc);
            ch
        } else {
            eprintln!("\n{} Will create {} {}", SPARKLE, label, local.get_name());
            print_diff(
                &format!("{} {}", label, local.get_name()),
                None::<&RoleRepresentation>,
                &local,
                ctx.options.changes_only,
                ctx.options.verbose,
                RoleRepresentation::SECRET_PREFIX,
            )?
        };

        if changed {
            let mut include = true;
            if ctx.options.interactive {
                include = crate::plan::prompt_interactive_change(
                    ctx.ui,
                    &format!("{} {}", label, local.get_name()),
                    remote_clone.as_ref(),
                    &local,
                    RoleRepresentation::SECRET_PREFIX,
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
    async fn test_plan_roles_no_dir() {
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

        let (files, summary) = plan_roles(&ctx).await.unwrap();
        assert!(files.is_empty());
        assert_eq!(summary.created, 0);
        assert_eq!(summary.updated, 0);
    }

    #[tokio::test]
    async fn test_plan_roles_realm_and_client_roles() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let _m_realm_roles = server
            .mock("GET", "/admin/realms/test/roles")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"id": "r1-id", "name": "existing-realm-role"}
                ]"#,
            )
            .create_async()
            .await;

        let _m_clients = server
            .mock("GET", "/admin/realms/test/clients")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"id": "c-uuid-1", "clientId": "app-1"}
                ]"#,
            )
            .create_async()
            .await;

        let _m_client_roles = server
            .mock("GET", "/admin/realms/test/clients/c-uuid-1/roles")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"id": "cr1-id", "name": "existing-client-role", "clientRole": true}
                ]"#,
            )
            .create_async()
            .await;

        let mut client = KeycloakClient::new(url);
        client.set_target_realm("test".to_string());
        client.set_token("token".to_string());

        let dir = tempfile::tempdir().unwrap();
        let ws_dir = dir.path().to_path_buf();

        // Create realm role files
        let realm_roles_dir = ws_dir.join("roles");
        fs::create_dir_all(&realm_roles_dir).unwrap();
        fs::write(
            realm_roles_dir.join("existing-realm-role.yaml"),
            "name: existing-realm-role\ndescription: Updated\n",
        )
        .unwrap();
        fs::write(
            realm_roles_dir.join("new-realm-role.yaml"),
            "name: new-realm-role\n",
        )
        .unwrap();
        fs::write(realm_roles_dir.join("skip.txt"), "not yaml").unwrap();
        fs::write(realm_roles_dir.join("role.prod.yaml"), "overlay").unwrap();

        // Create client role files
        let client_roles_dir = ws_dir.join("clients/app-1/roles");
        fs::create_dir_all(&client_roles_dir).unwrap();
        fs::write(
            client_roles_dir.join("existing-client-role.yaml"),
            "name: existing-client-role\ndescription: Updated\n",
        )
        .unwrap();
        fs::write(
            client_roles_dir.join("new-client-role.yaml"),
            "name: new-client-role\n",
        )
        .unwrap();

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

        let (files, summary) = plan_roles(&ctx).await.unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(summary.created, 2);
        assert_eq!(summary.updated, 2);
    }

    #[tokio::test]
    async fn test_plan_roles_interactive() {
        let mut server = Server::new_async().await;
        let url = server.url();

        let _m_realm_roles = server
            .mock("GET", "/admin/realms/test/roles")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[]"#)
            .create_async()
            .await;

        let _m_clients = server
            .mock("GET", "/admin/realms/test/clients")
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
        let roles_dir = ws_dir.join("roles");
        fs::create_dir_all(&roles_dir).unwrap();
        fs::write(roles_dir.join("r1.yaml"), "name: r1\n").unwrap();
        fs::write(roles_dir.join("r2.yaml"), "name: r2\n").unwrap();

        let ui = Arc::new(MockUi {
            inputs: std::sync::Mutex::new(vec![]),
            confirms: std::sync::Mutex::new(vec![]),
            selects: std::sync::Mutex::new(vec![0, 1]),
            passwords: std::sync::Mutex::new(vec![]),
        });
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let ctx = PlanContext {
            client: &client,
            workspace_dir: &ws_dir,
            options: PlanOptions {
                changes_only: false,
                interactive: true,
                verbose: false,
            },
            realm_name: "test",
            ui: ui.as_ref(),
            resolver,
            profile: None,
        };

        let (files, summary) = plan_roles(&ctx).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(summary.created, 1);
    }
}
