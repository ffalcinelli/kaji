#![allow(missing_docs)]
//! Apply module for applying local configuration changes to Keycloak.

pub mod authenticator_config;
pub mod components;
pub mod generic;
pub mod realm;

macro_rules! spawn_apply_stage {
    ($set:expr, $client:expr, $dir:expr, $secrets_path:expr, $resolver:expr, $planned_files:expr, $realm_name:expr, $profile:expr, $review:expr, $ui:expr, $yes:expr, $prune:expr, [ $($t:ty),* ]) => {
        $(
            let client_clone = $client.clone();
            let dir_clone = $dir.clone();
            let secrets_path_clone = Arc::clone(&$secrets_path);
            let resolver_clone = Arc::clone(&$resolver);
            let planned_files_clone = Arc::clone(&$planned_files);
            let realm_name_clone = $realm_name.to_string();
            let profile_clone = $profile.clone();
            let ui_clone = Arc::clone(&$ui);
            let review_clone = $review;
            let yes_clone = $yes;
            let prune_clone = $prune;
            $set.spawn(async move {
                let ctx = crate::apply::ApplyContext {
                    client: &client_clone,
                    workspace_dir: dir_clone,
                    secrets_path: secrets_path_clone,
                    resolver: resolver_clone,
                    planned_files: planned_files_clone,
                    realm_name: &realm_name_clone,
                    profile: profile_clone,
                    review: review_clone,
                    ui: ui_clone,
                    yes: yes_clone,
                    prune: prune_clone,
                };
                generic::apply_resources::<$t>(
                    ctx
                )
                .await
            });
        )*
    };
}

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

            let $update_id = id;
            let _ = &$update_id;
            let $update_rep = &$rep;
            $update_expr.await.with_context(|| {
                format!(
                    "Failed to update {} '{}' in realm '{}'",
                    $resource_name,
                    $rep.get_name(),
                    $realm_name
                )
            })?;
            eprintln!(
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
            eprintln!(
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

/// Reconciles local configuration files in the workspace directory with the remote Keycloak server.
///
/// # Errors
/// Returns an error if the workspace does not exist, network communication fails,
/// or authentication details are invalid.
pub struct ApplyArgs<'a> {
    pub client: &'a KeycloakClient,
    pub workspace_dir: PathBuf,
    pub realms_to_apply: &'a [String],
    pub yes: bool,
    pub review: bool,
    pub prune: bool,
    pub ui: Arc<dyn Ui>,
    pub resolver: Arc<dyn SecretResolver>,
    pub profile: Option<String>,
}

/// Reconciles local configuration files in the workspace directory with the remote Keycloak server,
/// optionally pruning orphaned remote resources.
pub async fn run(args: ApplyArgs<'_>) -> Result<()> {
    let ApplyArgs {
        client,
        workspace_dir,
        realms_to_apply,
        yes,
        review,
        prune,
        ui,
        resolver,
        profile,
    } = args;
    if !workspace_dir.exists() {
        anyhow::bail!("Input directory {:?} does not exist", workspace_dir);
    }

    let secrets_file = if let Some(p) = &profile {
        if let Ok(profile_obj) = crate::load_profile(&workspace_dir, p).await {
            profile_obj
                .secrets_file
                .clone()
                .unwrap_or_else(|| ".secrets".to_string())
        } else {
            ".secrets".to_string()
        }
    } else {
        ".secrets".to_string()
    };
    let secrets_path = Arc::new(workspace_dir.join(secrets_file));

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
                    eprintln!("Aborted.");
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
                eprintln!("Aborted.");
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
        eprintln!(
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
        let secrets_path = Arc::clone(&secrets_path);

        set.spawn(async move {
            eprintln!(
                "\n{} {}",
                ACTION,
                style(format!("Applying realm: {}", realm_name))
                    .cyan()
                    .bold()
            );

            apply_single_realm(ApplyContext {
                client: &realm_client,
                workspace_dir: realm_dir,
                secrets_path,
                resolver,
                planned_files,
                realm_name: realm_name.as_str(),
                profile,
                review,
                ui,
                yes,
                prune,
            })
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

pub struct ApplyContext<'a> {
    pub client: &'a KeycloakClient,
    pub workspace_dir: PathBuf,
    pub secrets_path: Arc<PathBuf>,
    pub resolver: Arc<dyn SecretResolver>,
    pub planned_files: Arc<Option<HashSet<PathBuf>>>,
    pub realm_name: &'a str,
    pub profile: Option<String>,
    pub review: bool,
    pub ui: Arc<dyn Ui>,
    pub yes: bool,
    pub prune: bool,
}

async fn apply_single_realm(ctx: ApplyContext<'_>) -> Result<()> {
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
    } = ctx;
    // Stage 0: Realms
    realm::apply_realm(crate::apply::ApplyContext {
        client,
        workspace_dir: workspace_dir.clone(),
        secrets_path: Arc::clone(&secrets_path),
        resolver: Arc::clone(&resolver),
        planned_files: Arc::clone(&planned_files),
        realm_name,
        profile: profile.clone(),
        review,
        ui: Arc::clone(&ui),
        yes,
        prune,
    })
    .await?;

    // Stage 1: Identity Providers, Roles
    {
        let mut set = JoinSet::new();
        spawn_apply_stage!(
            set,
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
            [IdentityProviderRepresentation, RoleRepresentation]
        );
        crate::utils::join_all_tasks(set, None).await?;
    }

    // Stage 2: Clients, Client Scopes, Authentication Flows, Required Actions, Groups
    {
        let mut set = JoinSet::new();
        spawn_apply_stage!(
            set,
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
            [
                ClientRepresentation,
                ClientScopeRepresentation,
                AuthenticationFlowRepresentation,
                RequiredActionProviderRepresentation,
                GroupRepresentation
            ]
        );
        crate::utils::join_all_tasks(set, None).await?;
    }

    // Stage 3: Users, Components, Keys
    {
        let mut set = JoinSet::new();
        spawn_apply_stage!(
            set,
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
            [UserRepresentation]
        );

        let client_ac = client.clone();
        let dir_ac = workspace_dir.clone();
        let secrets_path_ac = Arc::clone(&secrets_path);
        let res_ac = Arc::clone(&resolver);
        let plan_ac = Arc::clone(&planned_files);
        let rn_ac = realm_name.to_string();
        let p_ac = profile.clone();
        let ui_ac = Arc::clone(&ui);
        set.spawn(async move {
            authenticator_config::apply_authenticator_configs(crate::apply::ApplyContext {
                client: &client_ac,
                workspace_dir: dir_ac,
                secrets_path: secrets_path_ac,
                resolver: res_ac,
                planned_files: plan_ac,
                realm_name: &rn_ac,
                profile: p_ac,
                review,
                ui: ui_ac,
                yes,
                prune: false, // prune not used in this call
            })
            .await
        });

        let client_co = client.clone();
        let dir_co = workspace_dir.clone();
        let secrets_path_co = Arc::clone(&secrets_path);
        let res_co = Arc::clone(&resolver);
        let plan_co = Arc::clone(&planned_files);
        let rn_co = realm_name.to_string();
        let p_co = profile.clone();
        let ui_co = Arc::clone(&ui);
        set.spawn(async move {
            components::apply_components_or_keys(
                crate::apply::ApplyContext {
                    client: &client_co,
                    workspace_dir: dir_co,
                    secrets_path: secrets_path_co,
                    resolver: res_co,
                    planned_files: plan_co,
                    realm_name: &rn_co,
                    profile: p_co,
                    review: false,
                    ui: ui_co,
                    yes,
                    prune: false,
                },
                "components",
            )
            .await
        });

        let client_ke = client.clone();
        let dir_ke = workspace_dir.clone();
        let secrets_path_ke = Arc::clone(&secrets_path);
        let res_ke = Arc::clone(&resolver);
        let plan_ke = Arc::clone(&planned_files);
        let rn_ke = realm_name.to_string();
        let p_ke = profile.clone();
        let ui_ke = Arc::clone(&ui);
        set.spawn(async move {
            components::apply_components_or_keys(
                crate::apply::ApplyContext {
                    client: &client_ke,
                    workspace_dir: dir_ke,
                    secrets_path: secrets_path_ke,
                    resolver: res_ke,
                    planned_files: plan_ke,
                    realm_name: &rn_ke,
                    profile: p_ke,
                    review: false,
                    ui: ui_ke,
                    yes,
                    prune: false,
                },
                "keys",
            )
            .await
        });

        crate::utils::join_all_tasks(set, None).await?;
    }

    Ok(())
}
