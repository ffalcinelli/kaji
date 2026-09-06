#![allow(missing_docs)]
//! Plan module for calculating diffs and detecting configuration drift.

pub mod components;
pub mod generic;
pub mod realm;

macro_rules! plan_generic_resources {
    ($ctx:expr, $changed_files:expr, $summary:expr, [ $($t:ty),* ]) => {
        $(
            let (mut files, sum) = generic::plan_resources::<$t>($ctx).await?;
            $changed_files.append(&mut files);
            $summary.add(&sum);
        )*
    };
}

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

pub static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
pub struct PlanOptions {
    pub changes_only: bool,
    pub interactive: bool,
    pub verbose: bool,
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

pub struct PlanArgs<'a> {
    pub client: &'a KeycloakClient,
    pub workspace_dir: PathBuf,
    pub changes_only: bool,
    pub interactive: bool,
    pub realms_to_plan: &'a [String],
    pub ui: Arc<dyn Ui>,
    pub resolver: Arc<dyn SecretResolver>,
    pub profile: Option<String>,
}

/// Calculates configuration drift and compiles a list of planned modifications.
///
/// # Errors
/// Returns an error if directory read fails or Keycloak connection fails.
pub async fn run(args: PlanArgs<'_>) -> Result<()> {
    let PlanArgs {
        client,
        workspace_dir,
        changes_only,
        interactive,
        realms_to_plan,
        ui,
        resolver,
        profile,
    } = args;

    if !workspace_dir.exists() {
        return Err(anyhow::anyhow!(
            "Hint: Create the workspace directory first or use `kaji init`."
        )
        .context(format!(
            "Input directory {:?} does not exist",
            workspace_dir
        )));
    }

    let realms = if realms_to_plan.is_empty() {
        let mut dirs = Vec::new();
        let mut entries = async_fs::read_dir(&workspace_dir).await?;
        let mut join_set = tokio::task::JoinSet::new();
        while let Some(entry) = entries.next_entry().await? {
            join_set.spawn(async move {
                let is_dir = entry.file_type().await?.is_dir();
                Ok::<(bool, String), anyhow::Error>((
                    is_dir,
                    entry.file_name().to_string_lossy().to_string(),
                ))
            });
        }
        while let Some(res) = join_set.join_next().await {
            let (is_dir, name) = res??;
            if is_dir {
                dirs.push(name);
            }
        }
        dirs
    } else {
        realms_to_plan.to_vec()
    };

    if realms.is_empty() {
        eprintln!(
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
            eprintln!(
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
                verbose: VERBOSE.load(std::sync::atomic::Ordering::Relaxed),
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
        eprintln!(
            "\n{} {}",
            CHECK,
            style("No changes planned. Your infrastructure is in sync.")
                .green()
                .bold()
        );
    } else {
        let content = serde_json::to_string_pretty(&changed_files)?;
        async_fs::write(&plan_file, content).await?;
        eprintln!(
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
    // 1. Plan realm
    let (mut realm_changes, realm_summary) = realm::plan_realm(&ctx).await?;
    changed_files.append(&mut realm_changes);
    summary.add(&realm_summary);

    // 2. Plan generic resources
    plan_generic_resources!(
        &ctx,
        changed_files,
        summary,
        [
            RoleRepresentation,
            ClientRepresentation,
            IdentityProviderRepresentation,
            ClientScopeRepresentation,
            GroupRepresentation,
            UserRepresentation,
            AuthenticationFlowRepresentation,
            RequiredActionProviderRepresentation,
            AuthenticatorConfigRepresentation
        ]
    );

    // 3. Plan custom components and keys
    let ((mut component_changes, component_summary), (mut key_changes, key_summary), _) = tokio::try_join!(
        components::plan_components_or_keys(&ctx, "components"),
        components::plan_components_or_keys(&ctx, "keys"),
        components::check_keys_drift(ctx.client, ctx.options, ctx.realm_name),
    )?;

    changed_files.append(&mut component_changes);
    changed_files.append(&mut key_changes);

    summary.add(&component_summary);
    summary.add(&key_summary);

    Ok(())
}

pub fn prompt_interactive_change<T: Serialize>(
    ui: &dyn Ui,
    name: &str,
    old: Option<&T>,
    new: &T,
    prefix: &str,
) -> Result<bool> {
    let selections = &["Yes", "No", "Show Full Diff"];
    loop {
        let selection = ui.select("Include this change in the plan?", selections, 0)?;
        match selection {
            0 => return Ok(true),
            1 => return Ok(false),
            2 => {
                print_diff(name, old, new, false, true, prefix)?;
            }
            _ => {}
        }
    }
}

pub fn print_diff<T: Serialize>(
    name: &str,
    old: Option<&T>,
    new: &T,
    changes_only: bool,
    verbose: bool,
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
        if verbose {
            for change in diff.iter_all_changes() {
                let (sign, style) = match change.tag() {
                    ChangeTag::Delete => ("-", Style::new().red()),
                    ChangeTag::Insert => ("+", Style::new().green()),
                    ChangeTag::Equal => (" ", Style::new().dim()),
                };
                print!("{}{}", style.apply_to(sign).bold(), style.apply_to(change));
            }
        } else {
            for (idx, hunk) in diff.grouped_ops(3).iter().enumerate() {
                if idx > 0 {
                    println!("{}", style("...").dim());
                }

                let old_start = hunk.first().map(|op| op.old_range().start).unwrap_or(0);
                let old_end = hunk.last().map(|op| op.old_range().end).unwrap_or(0);
                let new_start = hunk.first().map(|op| op.new_range().start).unwrap_or(0);
                let new_end = hunk.last().map(|op| op.new_range().end).unwrap_or(0);

                let old_len = old_end - old_start;
                let new_len = new_end - new_start;

                let mut header = String::from("@@");
                if old_len == 1 {
                    header.push_str(&format!(" -{}", old_start + 1));
                } else {
                    header.push_str(&format!(" -{},{}", old_start + 1, old_len));
                }
                if new_len == 1 {
                    header.push_str(&format!(" +{}", new_start + 1));
                } else {
                    header.push_str(&format!(" +{},{}", new_start + 1, new_len));
                }
                header.push_str(" @@");
                println!("{}", style(header).cyan());

                for op in hunk {
                    for change in diff.iter_changes(op) {
                        let (sign, style) = match change.tag() {
                            ChangeTag::Delete => ("-", Style::new().red()),
                            ChangeTag::Insert => ("+", Style::new().green()),
                            ChangeTag::Equal => (" ", Style::new().dim()),
                        };
                        print!("{}{}", style.apply_to(sign).bold(), style.apply_to(change));
                    }
                }
            }
        }
    } else if !changes_only {
        println!("{} No changes for {}", CHECK, name);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Clone)]
    struct DummyResource {
        name: String,
        value: i32,
        secret: String,
    }

    #[test]
    fn test_print_diff_no_changes() {
        let dummy = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", Some(&dummy), &dummy, false, false, "").unwrap();
        assert_eq!(result, false);
    }

    #[test]
    fn test_print_diff_with_changes_hunk() {
        let old = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };
        let new = DummyResource {
            name: "test".to_string(),
            value: 43,
            secret: "secret_value".to_string(),
        };

        // changes_only = true, non-verbose (hunk printing)
        let result = print_diff("Dummy", Some(&old), &new, true, false, "").unwrap();
        assert_eq!(result, true);
    }

    #[test]
    fn test_print_diff_no_changes_changes_only() {
        let dummy = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", Some(&dummy), &dummy, true, false, "").unwrap();
        assert_eq!(result, false);
    }

    #[test]
    fn test_print_diff_new_resource() {
        let new = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", None, &new, false, false, "").unwrap();
        assert_eq!(result, true);
    }

    #[test]
    fn test_print_diff_verbose() {
        let old = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };
        let new = DummyResource {
            name: "test".to_string(),
            value: 43,
            secret: "secret_value".to_string(),
        };

        // Verbose diff printing
        let result = print_diff("Dummy", Some(&old), &new, false, true, "").unwrap();
        assert_eq!(result, true);
    }
}
