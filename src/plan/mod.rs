pub mod components;
pub mod generic;
pub mod realm;

use crate::client::KeycloakClient;
use crate::utils::secrets::{SecretResolver, obfuscate_secrets};
use crate::utils::ui::{ACTION, CHECK, MEMO, Ui, WARN};

use anyhow::Result;
use console::{Style, style};
use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

#[derive(Debug, Clone, Copy)]
pub struct PlanOptions {
    pub changes_only: bool,
    pub interactive: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlanSummary {
    pub created: usize,
    pub updated: usize,
}

impl PlanSummary {
    pub fn add(&mut self, other: &PlanSummary) {
        self.created += other.created;
        self.updated += other.updated;
    }

    pub fn total(&self) -> usize {
        self.created + self.updated
    }
}

pub struct PlanContext<'a> {
    pub client: &'a KeycloakClient,
    pub workspace_dir: &'a std::path::Path,
    pub options: PlanOptions,
    pub resolver: Arc<dyn SecretResolver>,
    pub realm_name: &'a str,
    pub ui: &'a dyn Ui,
    pub profile: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &KeycloakClient,
    workspace_dir: PathBuf,
    changes_only: bool,
    interactive: bool,
    realms_to_plan: &[String],
    ui: Arc<dyn Ui>,
    resolver: Arc<dyn SecretResolver>,
    profile: Option<String>,
) -> Result<()> {
    if !workspace_dir.exists() {
        anyhow::bail!("Input directory {:?} does not exist", workspace_dir);
    }

    let realms = if realms_to_plan.is_empty() {
        let mut dirs = Vec::new();
        let mut entries = async_fs::read_dir(&workspace_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir() {
                dirs.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        dirs
    } else {
        realms_to_plan.to_vec()
    };

    if realms.is_empty() {
        println!(
            "{} {}",
            WARN,
            style(format!("No realms found to plan in {:?}", workspace_dir)).yellow()
        );
        return Ok(());
    }

    let mut set = tokio::task::JoinSet::new();

    for realm_name in realms {
        let mut realm_client = client.clone();
        realm_client.set_target_realm(realm_name.clone());
        let realm_dir = workspace_dir.join(&realm_name);
        let resolver = Arc::clone(&resolver);
        let ui = Arc::clone(&ui);
        let profile = profile.clone();

        set.spawn(async move {
            println!(
                "\n{} {}",
                ACTION,
                style(format!("Planning changes for realm: {}", realm_name))
                    .cyan()
                    .bold()
            );

            let mut changed_files = Vec::new();
            let mut summary = PlanSummary::default();
            let options = PlanOptions {
                changes_only,
                interactive,
            };
            let ctx = PlanContext {
                client: &realm_client,
                workspace_dir: &realm_dir,
                options,
                resolver,
                realm_name: &realm_name,
                ui: ui.as_ref(),
                profile,
            };
            plan_single_realm(ctx, &mut changed_files, &mut summary).await?;

            Ok::<(Vec<PathBuf>, PlanSummary), anyhow::Error>((changed_files, summary))
        });
    }

    let mut changed_files = Vec::new();
    let mut total_summary = PlanSummary::default();
    for res in crate::utils::join_all_tasks(set, None).await? {
        let (files, summary) = res;
        changed_files.extend(files);
        total_summary.add(&summary);
    }
    changed_files.sort();

    let plan_file = workspace_dir.join(".kajiplan");
    if changed_files.is_empty() {
        if async_fs::try_exists(&plan_file).await? {
            async_fs::remove_file(&plan_file).await?;
        }
        println!(
            "\n{} {}",
            CHECK,
            style("No changes planned. Your infrastructure is in sync.")
                .green()
                .bold()
        );
    } else {
        let content = serde_json::to_string_pretty(&changed_files)?;
        async_fs::write(&plan_file, content).await?;
        println!(
            "\n{} {}",
            MEMO,
            style(format!(
                "Plan summary: {} to create, {} to update ({} total changes).",
                total_summary.created,
                total_summary.updated,
                total_summary.total()
            ))
            .cyan()
            .bold()
        );
    }

    Ok(())
}

use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, GroupRepresentation, IdentityProviderRepresentation,
    RequiredActionProviderRepresentation, RoleRepresentation, UserRepresentation,
};

async fn plan_single_realm(
    ctx: PlanContext<'_>,
    changed_files: &mut Vec<PathBuf>,
    summary: &mut PlanSummary,
) -> Result<()> {
    let (
        (mut realm_changes, realm_summary),
        (mut role_changes, role_summary),
        (mut client_changes, client_summary),
        (mut idp_changes, idp_summary),
        (mut client_scope_changes, client_scope_summary),
        (mut group_changes, group_summary),
        (mut user_changes, user_summary),
        (mut auth_flow_changes, auth_flow_summary),
        (mut required_action_changes, required_action_summary),
        (mut config_changes, config_summary),
        (mut component_changes, component_summary),
        (mut key_changes, key_summary),
        _,
    ) = tokio::try_join!(
        realm::plan_realm(&ctx),
        generic::plan_resources::<RoleRepresentation>(&ctx),
        generic::plan_resources::<ClientRepresentation>(&ctx),
        generic::plan_resources::<IdentityProviderRepresentation>(&ctx),
        generic::plan_resources::<ClientScopeRepresentation>(&ctx),
        generic::plan_resources::<GroupRepresentation>(&ctx),
        generic::plan_resources::<UserRepresentation>(&ctx),
        generic::plan_resources::<AuthenticationFlowRepresentation>(&ctx),
        generic::plan_resources::<RequiredActionProviderRepresentation>(&ctx),
        generic::plan_resources::<AuthenticatorConfigRepresentation>(&ctx),
        components::plan_components_or_keys(&ctx, "components"),
        components::plan_components_or_keys(&ctx, "keys"),
        components::check_keys_drift(ctx.client, ctx.options, ctx.realm_name),
    )?;

    changed_files.append(&mut realm_changes);
    changed_files.append(&mut role_changes);
    changed_files.append(&mut client_changes);
    changed_files.append(&mut idp_changes);
    changed_files.append(&mut client_scope_changes);
    changed_files.append(&mut group_changes);
    changed_files.append(&mut user_changes);
    changed_files.append(&mut auth_flow_changes);
    changed_files.append(&mut required_action_changes);
    changed_files.append(&mut config_changes);
    changed_files.append(&mut component_changes);
    changed_files.append(&mut key_changes);

    summary.add(&realm_summary);
    summary.add(&role_summary);
    summary.add(&client_summary);
    summary.add(&idp_summary);
    summary.add(&client_scope_summary);
    summary.add(&group_summary);
    summary.add(&user_summary);
    summary.add(&auth_flow_summary);
    summary.add(&required_action_summary);
    summary.add(&config_summary);
    summary.add(&component_summary);
    summary.add(&key_summary);

    Ok(())
}

pub fn print_diff<T: Serialize>(
    name: &str,
    old: Option<&T>,
    new: &T,
    changes_only: bool,
    prefix: &str,
) -> Result<bool> {
    let old_yaml = if let Some(o) = old {
        let mut val = serde_json::to_value(o)?;
        obfuscate_secrets(&mut val, prefix);
        crate::utils::to_sorted_yaml(&val)?
    } else {
        String::new()
    };

    let mut new_val = serde_json::to_value(new)?;
    obfuscate_secrets(&mut new_val, prefix);
    let new_yaml = crate::utils::to_sorted_yaml(&new_val)?;

    let diff = TextDiff::from_lines(&old_yaml, &new_yaml);
    let changed = diff.ratio() < 1.0;

    if changed {
        println!("\n{} Changes for {}:", MEMO, name);
        for change in diff.iter_all_changes() {
            let (sign, style) = match change.tag() {
                ChangeTag::Delete => ("-", Style::new().red()),
                ChangeTag::Insert => ("+", Style::new().green()),
                ChangeTag::Equal => (" ", Style::new().dim()),
            };
            print!("{}{}", style.apply_to(sign).bold(), style.apply_to(change));
        }
    } else if !changes_only {
        println!("{} No changes for {}", CHECK, name);
    }
    Ok(changed)
}
