pub mod authenticator_config;
pub mod components;
pub mod generic;
pub mod realm;

#[cfg(test)]
pub mod test_utils;

#[macro_export]
macro_rules! handle_upsert {
    (
        client: $client:expr,
        realm: $realm_name:expr,
        rep: $rep:expr,
        id_opt: $id_expr:expr,
        id_field: $id_field:ident,
        resource_name: $resource_name:expr,
        update_call: |$update_id:ident, $update_rep:ident| $update_expr:expr,
        create_call: |$create_rep:ident| $create_expr:expr
    ) => {
        if let Some(id) = $id_expr {
            $rep.$id_field = Some(id.clone());
            #[allow(unused_variables)]
            let $update_id = id;
            let $update_rep = &$rep;
            $update_expr.await.with_context(|| {
                format!(
                    "Failed to update {} '{}' in realm '{}'",
                    $resource_name,
                    $rep.get_name(),
                    $realm_name
                )
            })?;
            println!(
                "  {} {}",
                $crate::utils::ui::SUCCESS_UPDATE,
                console::style(format!("Updated {} {}", $resource_name, $rep.get_name())).cyan()
            );
        } else {
            $rep.$id_field = None;
            let $create_rep = &$rep;
            $create_expr.await.with_context(|| {
                format!(
                    "Failed to create {} '{}' in realm '{}'",
                    $resource_name,
                    $rep.get_name(),
                    $realm_name
                )
            })?;
            println!(
                "  {} {}",
                $crate::utils::ui::SUCCESS_CREATE,
                console::style(format!("Created {} {}", $resource_name, $rep.get_name())).green()
            );
        }
    };
}

use crate::client::KeycloakClient;
use crate::models::{
    AuthenticationFlowRepresentation, ClientRepresentation, ClientScopeRepresentation,
    GroupRepresentation, IdentityProviderRepresentation, RequiredActionProviderRepresentation,
    RoleRepresentation, UserRepresentation,
};
use crate::utils::secrets::SecretResolver;
pub use crate::utils::ui::{ACTION, SUCCESS_CREATE, SUCCESS_UPDATE, Ui, WARN};
use anyhow::Result;
use console::style;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &KeycloakClient,
    workspace_dir: PathBuf,
    realms_to_apply: &[String],
    yes: bool,
    review: bool,
    ui: Arc<dyn Ui>,
    resolver: Arc<dyn SecretResolver>,
    profile: Option<String>,
) -> Result<()> {
    if !workspace_dir.exists() {
        anyhow::bail!("Input directory {:?} does not exist", workspace_dir);
    }

    // Check for .kajiplan
    let plan_path = workspace_dir.join(".kajiplan");
    let planned_files = if plan_path.exists() {
        let content = async_fs::read_to_string(&plan_path).await?;
        let items: Vec<PathBuf> = serde_json::from_str(&content)?;
        if items.is_empty() {
            if !yes {
                let proceed = ui.confirm(
                    "No planned changes found. Send everything to Keycloak anyway?",
                    false,
                )?;
                if !proceed {
                    println!("Aborted.");
                    return Ok(());
                }
            }

            Arc::new(None)
        } else {
            let hashset: HashSet<PathBuf> = items.into_iter().collect();
            Arc::new(Some(hashset))
        }
    } else {
        if !yes {
            let proceed = ui.confirm(
                "No planned changes found. Send everything to Keycloak anyway?",
                false,
            )?;
            if !proceed {
                println!("Aborted.");
                return Ok(());
            }
        }
        Arc::new(None)
    };

    let realms = if realms_to_apply.is_empty() {
        let mut dirs = Vec::new();
        let mut entries = async_fs::read_dir(&workspace_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                dirs.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        dirs
    } else {
        realms_to_apply.to_vec()
    };

    if realms.is_empty() {
        println!(
            "{} {}",
            WARN,
            style(format!("No realms found to apply in {:?}", workspace_dir)).yellow()
        );
        return Ok(());
    }

    let mut set = tokio::task::JoinSet::new();

    for realm_name in realms {
        let mut realm_client = client.clone();
        realm_client.set_target_realm(realm_name.clone());
        let realm_dir = workspace_dir.join(&realm_name);
        let resolver = Arc::clone(&resolver);
        let planned_files = Arc::clone(&planned_files);
        let profile = profile.clone();
        let ui = Arc::clone(&ui);

        set.spawn(async move {
            println!(
                "\n{} {}",
                ACTION,
                style(format!("Applying realm: {}", realm_name))
                    .cyan()
                    .bold()
            );

            apply_single_realm(
                &realm_client,
                realm_dir,
                resolver,
                planned_files,
                &realm_name,
                profile,
                review,
                ui,
            )
            .await
        });
    }

    crate::utils::join_all_tasks(set, None).await?;

    // Success - remove plan
    if plan_path.exists() {
        let _ = async_fs::remove_file(plan_path).await;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_single_realm(
    client: &KeycloakClient,
    workspace_dir: PathBuf,
    resolver: Arc<dyn SecretResolver>,
    planned_files: Arc<Option<HashSet<PathBuf>>>,
    realm_name: &str,
    profile: Option<String>,
    review: bool,
    ui: Arc<dyn Ui>,
) -> Result<()> {
    // Stage 0: Realms
    realm::apply_realm(
        client,
        &workspace_dir,
        Arc::clone(&resolver),
        Arc::clone(&planned_files),
        realm_name,
        profile.clone(),
    )
    .await?;

    // Stage 1: Identity Providers, Roles
    {
        let mut set = JoinSet::new();
        let client1 = client.clone();
        let dir1 = workspace_dir.clone();
        let res1 = Arc::clone(&resolver);
        let plan1 = Arc::clone(&planned_files);
        let rn1 = realm_name.to_string();
        let p1 = profile.clone();
        let ui1 = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<IdentityProviderRepresentation>(
                &client1, &dir1, res1, plan1, &rn1, p1, review, ui1,
            )
            .await
        });

        let client2 = client.clone();
        let dir2 = workspace_dir.clone();
        let res2 = Arc::clone(&resolver);
        let plan2 = Arc::clone(&planned_files);
        let rn2 = realm_name.to_string();
        let p2 = profile.clone();
        let ui2 = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<RoleRepresentation>(
                &client2, &dir2, res2, plan2, &rn2, p2, review, ui2,
            )
            .await
        });
        crate::utils::join_all_tasks(set, None).await?;
    }

    // Stage 2: Clients, Client Scopes, Authentication Flows, Required Actions, Groups
    {
        let mut set = JoinSet::new();

        let client_cl = client.clone();
        let dir_cl = workspace_dir.clone();
        let res_cl = Arc::clone(&resolver);
        let plan_cl = Arc::clone(&planned_files);
        let rn_cl = realm_name.to_string();
        let p_cl = profile.clone();
        let ui_cl = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<ClientRepresentation>(
                &client_cl, &dir_cl, res_cl, plan_cl, &rn_cl, p_cl, review, ui_cl,
            )
            .await
        });

        let client_sc = client.clone();
        let dir_sc = workspace_dir.clone();
        let res_sc = Arc::clone(&resolver);
        let plan_sc = Arc::clone(&planned_files);
        let rn_sc = realm_name.to_string();
        let p_sc = profile.clone();
        let ui_sc = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<ClientScopeRepresentation>(
                &client_sc, &dir_sc, res_sc, plan_sc, &rn_sc, p_sc, review, ui_sc,
            )
            .await
        });

        let client_fl = client.clone();
        let dir_fl = workspace_dir.clone();
        let res_fl = Arc::clone(&resolver);
        let plan_fl = Arc::clone(&planned_files);
        let rn_fl = realm_name.to_string();
        let p_fl = profile.clone();
        let ui_fl = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<AuthenticationFlowRepresentation>(
                &client_fl, &dir_fl, res_fl, plan_fl, &rn_fl, p_fl, review, ui_fl,
            )
            .await
        });

        let client_ra = client.clone();
        let dir_ra = workspace_dir.clone();
        let res_ra = Arc::clone(&resolver);
        let plan_ra = Arc::clone(&planned_files);
        let rn_ra = realm_name.to_string();
        let p_ra = profile.clone();
        let ui_ra = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<RequiredActionProviderRepresentation>(
                &client_ra, &dir_ra, res_ra, plan_ra, &rn_ra, p_ra, review, ui_ra,
            )
            .await
        });

        let client_gr = client.clone();
        let dir_gr = workspace_dir.clone();
        let res_gr = Arc::clone(&resolver);
        let plan_gr = Arc::clone(&planned_files);
        let rn_gr = realm_name.to_string();
        let p_gr = profile.clone();
        let ui_gr = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<GroupRepresentation>(
                &client_gr, &dir_gr, res_gr, plan_gr, &rn_gr, p_gr, review, ui_gr,
            )
            .await
        });

        crate::utils::join_all_tasks(set, None).await?;
    }

    // Stage 3: Users, Components, Keys
    {
        let mut set = JoinSet::new();

        let client_us = client.clone();
        let dir_us = workspace_dir.clone();
        let res_us = Arc::clone(&resolver);
        let plan_us = Arc::clone(&planned_files);
        let rn_us = realm_name.to_string();
        let p_us = profile.clone();
        let ui_us = Arc::clone(&ui);
        set.spawn(async move {
            generic::apply_resources::<UserRepresentation>(
                &client_us, &dir_us, res_us, plan_us, &rn_us, p_us, review, ui_us,
            )
            .await
        });

        let client_ac = client.clone();
        let dir_ac = workspace_dir.clone();
        let res_ac = Arc::clone(&resolver);
        let plan_ac = Arc::clone(&planned_files);
        let rn_ac = realm_name.to_string();
        let p_ac = profile.clone();
        let ui_ac = Arc::clone(&ui);
        set.spawn(async move {
            authenticator_config::apply_authenticator_configs(
                &client_ac, &dir_ac, res_ac, plan_ac, &rn_ac, p_ac, review, ui_ac,
            )
            .await
        });

        let client_co = client.clone();
        let dir_co = workspace_dir.clone();
        let res_co = Arc::clone(&resolver);
        let plan_co = Arc::clone(&planned_files);
        let rn_co = realm_name.to_string();
        let p_co = profile.clone();
        set.spawn(async move {
            components::apply_components_or_keys(
                &client_co,
                &dir_co,
                "components",
                res_co,
                plan_co,
                &rn_co,
                p_co,
            )
            .await
        });

        let client_ke = client.clone();
        let dir_ke = workspace_dir.clone();
        let res_ke = Arc::clone(&resolver);
        let plan_ke = Arc::clone(&planned_files);
        let rn_ke = realm_name.to_string();
        let p_ke = profile.clone();
        set.spawn(async move {
            components::apply_components_or_keys(
                &client_ke, &dir_ke, "keys", res_ke, plan_ke, &rn_ke, p_ke,
            )
            .await
        });

        crate::utils::join_all_tasks(set, None).await?;
    }

    Ok(())
}
