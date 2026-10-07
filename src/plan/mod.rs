#![allow(missing_docs)]
//! Plan module for calculating diffs and detecting configuration drift.

pub mod client_roles;
pub mod components;
pub mod generic;
pub mod plan_file;
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
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};

use anyhow::{Context, Result};
use console::{Style, style};
use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

#[deprecated(note = "Use PlanArgs.verbose instead")]
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
    /// Remote resources not declared locally (removed only by `apply --prune`).
    pub orphaned: usize,
}

impl PlanSummary {
    pub fn add(&mut self, other: &PlanSummary) {
        self.created += other.created;
        self.updated += other.updated;
        self.orphaned += other.orphaned;
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
    pub verbose: bool,
}

/// Calculates configuration drift and compiles a list of planned modifications.
///
/// # Errors
/// Returns an error if directory read fails or Keycloak connection fails.
pub async fn run(args: PlanArgs<'_>) -> Result<()> {
    run_with_outcome(args, true).await.map(|_| ())
}

/// Calculates configuration drift and returns the plan summary.
///
/// When `write_plan` is false (e.g. `kaji drift`), the `.kajiplan` file is left untouched.
///
/// # Errors
/// Returns an error if directory read fails or Keycloak connection fails.
pub async fn run_with_outcome(args: PlanArgs<'_>, write_plan: bool) -> Result<PlanSummary> {
    let PlanArgs {
        client,
        workspace_dir,
        changes_only,
        interactive,
        realms_to_plan,
        ui,
        resolver,
        profile,
        verbose,
    } = args;

    if !async_fs::try_exists(&workspace_dir).await? {
        return Err(anyhow::anyhow!(
            "Hint: Create the workspace directory first or use `kaji init`."
        ))
        .with_context(|| format!("Input directory {:?} does not exist", workspace_dir));
    }

    let realms = if realms_to_plan.is_empty() {
        crate::utils::discover_realms(&workspace_dir).await?
    } else {
        realms_to_plan.to_vec()
    };

    if realms.is_empty() {
        eprintln!(
            "{} {}",
            WARN,
            style(format!("No realms found to plan in {:?}", workspace_dir)).yellow()
        );
        return Ok(PlanSummary::default());
    }

    let mut set = tokio::task::JoinSet::new();
    let mut sequential = Vec::new();

    for realm_name in realms {
        let mut realm_client = client.clone();
        realm_client.set_target_realm(realm_name.clone());
        let realm_dir = workspace_dir.join(&realm_name);
        let resolver = Arc::clone(&resolver);
        let ui = Arc::clone(&ui);
        let profile = profile.clone();

        let task = async move {
            eprintln!(
                "\n{} {}",
                ACTION,
                style(format!("Planning changes for realm: {}", realm_name))
                    .cyan()
                    .bold()
            );

            let mut changed_files = Vec::new();
            let mut summary = PlanSummary::default();
            #[allow(deprecated)]
            let is_verbose = verbose || VERBOSE.load(std::sync::atomic::Ordering::Relaxed);
            let options = PlanOptions {
                changes_only,
                interactive,
                verbose: is_verbose,
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
        };
        if interactive {
            // Prompts of concurrent realms would interleave: plan one realm at a time.
            sequential.push(task.await?);
        } else {
            set.spawn(task);
        }
    }

    let mut changed_files = Vec::new();
    let mut total_summary = PlanSummary::default();
    let concurrent = crate::utils::join_all_tasks(set, None).await?;
    for res in sequential.into_iter().chain(concurrent) {
        let (files, summary) = res;
        changed_files.extend(files);
        total_summary.add(&summary);
    }
    changed_files.sort();

    let plan_path = workspace_dir.join(plan_file::PLAN_FILE_NAME);
    if changed_files.is_empty() {
        if write_plan {
            match async_fs::remove_file(&plan_path).await {
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        eprintln!(
            "\n{} {}",
            CHECK,
            style("No changes planned. Your infrastructure is in sync.")
                .green()
                .bold()
        );
    } else {
        if write_plan {
            plan_file::PlanFile::build(&workspace_dir, profile.as_deref(), &changed_files)
                .await?
                .write(&workspace_dir)
                .await?;
        }
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
    if total_summary.orphaned > 0 {
        eprintln!(
            "{} {}",
            WARN,
            style(format!(
                "{} remote resource(s) are not declared locally and would be deleted by `apply --prune`.",
                total_summary.orphaned
            ))
            .yellow()
        );
    }

    Ok(total_summary)
}

use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, GroupRepresentation, IdentityProviderRepresentation,
    KeycloakResource, RequiredActionProviderRepresentation, RoleRepresentation, UserRepresentation,
};

/// Analyzes authentication flow dependencies to detect shared sub-flows and explain how they will
/// be reconciled.
///
/// When Keycloak processes parent flows during `apply`, it may auto-create sub-flows or return
/// `409 Conflict` if applied out of order. `kaji` solves this by topologically staging leaf and
/// shared sub-flows before parent flows, and automatically adopting remote flows upon 409 conflict.
async fn check_flow_subflow_collisions(ctx: &PlanContext<'_>) -> Result<()> {
    let flows_dir = ctx
        .workspace_dir
        .join(AuthenticationFlowRepresentation::DIR_NAME);
    if !async_fs::try_exists(&flows_dir).await? {
        return Ok(());
    }

    // 1. Load all local flow representations from YAML files.
    let mut local_flows: Vec<AuthenticationFlowRepresentation> = Vec::new();
    let mut entries = async_fs::read_dir(&flows_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !is_yaml_file(&path) {
            continue;
        }
        if is_overlay_file(&path, ctx.profile.as_deref()) {
            continue;
        }
        let val = load_yaml_with_overlay(&path, ctx.profile.as_deref()).await?;
        if let Ok(flow) = serde_json::from_value::<AuthenticationFlowRepresentation>(val) {
            local_flows.push(flow);
        }
    }
    if local_flows.is_empty() {
        return Ok(());
    }

    // 2. Fetch remote flows to determine which local flows are "to create" (not yet in Keycloak).
    let remote_flows = match ctx
        .client
        .get_resources::<AuthenticationFlowRepresentation>()
        .await
    {
        Ok(flows) => flows,
        Err(e) if crate::client::is_not_found(&e) => Vec::new(),
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "Failed to fetch authentication flows for realm '{}' during sub-flow collision check",
                    ctx.realm_name
                )
            });
        }
    };
    let remote_aliases: HashSet<String> = remote_flows
        .iter()
        .filter_map(|f| f.get_identity())
        .collect();

    let to_create: HashSet<String> = local_flows
        .iter()
        .filter_map(|f| f.alias.clone())
        .filter(|alias| !remote_aliases.contains(alias))
        .collect();

    // 3. Track sub-flow references across parent flows to detect shared flows and dependencies
    let mut subflow_referrers: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for flow in &local_flows {
        let parent = flow.alias.as_deref().unwrap_or("unknown");
        for sub_alias in flow.subflow_aliases() {
            subflow_referrers
                .entry(sub_alias)
                .or_default()
                .push(parent.to_string());
        }
    }

    for (sub_alias, parents) in &subflow_referrers {
        let is_to_create = to_create.contains(sub_alias.as_str());
        if parents.len() > 1 {
            let status = if is_to_create {
                "to create"
            } else {
                "existing"
            };
            eprintln!(
                "\n{} Authentication flow '{}' is a shared sub-flow ({}) referenced by: {}. \
                 Kaji will topologically stage and reconcile it across parent flows.",
                CHECK,
                sub_alias,
                status,
                parents.join(", ")
            );
        } else if is_to_create {
            eprintln!(
                "\n{} Authentication flow '{}' is marked to CREATE and referenced as a \
                 sub-flow inside flow '{}'. Kaji will topologically stage and auto-adopt it during apply.",
                ACTION, sub_alias, parents[0]
            );
        }
    }

    Ok(())
}

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

    // Client roles live under clients/<clientId>/roles/
    let (mut role_changes, role_summary) = client_roles::plan_client_roles(&ctx).await?;
    changed_files.append(&mut role_changes);
    summary.add(&role_summary);

    // 3. Warn about potential 409 sub-flow collisions in authentication flows
    check_flow_subflow_collisions(&ctx).await?;

    // 4. Plan custom components and keys
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

/// Removes object keys whose value is an empty object or array (recursively).
fn drop_empty_collections(value: &mut serde_json::Value) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                drop_empty_collections(v);
            }
            map.retain(|_, v| match v {
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
        }
        Value::Array(items) => {
            for v in items {
                drop_empty_collections(v);
            }
        }
        _ => {}
    }
}

/// Removes object keys from `remote` that `local` does not declare (recursively).
///
/// Arrays are compared as a whole once declared; elements of arrays of objects are projected
/// when both arrays have the same length.
fn project_onto(remote: &mut serde_json::Value, local: &serde_json::Value) {
    use serde_json::Value;
    match (remote, local) {
        (Value::Object(r), Value::Object(l)) => {
            r.retain(|k, _| l.contains_key(k));
            for (k, rv) in r.iter_mut() {
                if let Some(lv) = l.get(k) {
                    project_onto(rv, lv);
                }
            }
        }
        (Value::Array(r), Value::Array(l)) => {
            // Pair elements by identity (array order may differ), by position as a fallback.
            const KEYS: &[&str] = &[
                "id",
                "clientId",
                "name",
                "alias",
                "authenticator",
                "flowAlias",
            ];
            let identity = |v: &Value| {
                KEYS.iter()
                    .find_map(|k| v.get(*k).map(|id| (*k, id.clone())))
            };
            let same_len = r.len() == l.len();
            for (index, rv) in r.iter_mut().enumerate() {
                let local = match identity(rv) {
                    Some((key, id)) => l.iter().find(|lv| lv.get(key) == Some(&id)),
                    None if same_len => l.get(index),
                    None => None,
                };
                if let Some(lv) = local {
                    project_onto(rv, lv);
                }
            }
        }
        _ => {}
    }
}

/// Copies Keycloak's `**********` masks from the remote value onto the local value.
fn mask_like_remote(remote: &serde_json::Value, local: &mut serde_json::Value) {
    use serde_json::Value;
    match (remote, local) {
        (Value::String(r), local) if r == crate::utils::secrets::KEYCLOAK_MASK => {
            if local.is_string() {
                *local = Value::String(r.clone());
            }
        }
        (Value::Object(r), Value::Object(l)) => {
            for (k, rv) in r {
                if let Some(lv) = l.get_mut(k) {
                    mask_like_remote(rv, lv);
                }
            }
        }
        (Value::Array(r), Value::Array(l)) if r.len() == l.len() => {
            for (rv, lv) in r.iter().zip(l.iter_mut()) {
                mask_like_remote(rv, lv);
            }
        }
        _ => {}
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
    print_resource_diff(name, old, new, changes_only, verbose, prefix, false)
}

/// Like [`print_diff`]; with `partial_updates`, remote fields the local file omits are ignored
/// (for resource types whose Keycloak update endpoint leaves omitted fields unchanged).
pub fn print_resource_diff<T: Serialize>(
    name: &str,
    old: Option<&T>,
    new: &T,
    changes_only: bool,
    verbose: bool,
    prefix: &str,
    partial_updates: bool,
) -> Result<bool> {
    let mut new_val = serde_json::to_value(new)?;
    let old_yaml = if let Some(o) = old {
        let mut val = serde_json::to_value(o)?;
        // Keycloak never returns some stored secrets: they cannot be compared, so the local
        // value is shown as masked too instead of reporting a permanent change.
        mask_like_remote(&val, &mut new_val);
        if partial_updates {
            // Keycloak leaves omitted fields untouched: only compare what the file declares.
            project_onto(&mut val, &new_val);
        }
        // Keycloak returns empty collections for unset fields: they equal absent ones.
        drop_empty_collections(&mut val);
        drop_empty_collections(&mut new_val);
        obfuscate_secrets(&mut val, prefix);
        crate::utils::to_sorted_yaml(&val)?
    } else {
        String::new()
    };

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
        println!(
            "{} {}",
            CHECK,
            style(format!("No changes for {}", name)).green()
        );
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
    fn test_partial_local_file_only_compares_declared_keys() {
        let diff = |old: &serde_json::Value, new: &serde_json::Value| {
            print_resource_diff("c", Some(old), new, true, false, "client", true).unwrap()
        };
        let remote = serde_json::json!({
            "clientId": "app", "enabled": true, "publicClient": true,
            "attributes": {"a": "1", "b": "2"},
            "redirectUris": ["https://a", "https://b"]
        });
        let partial = serde_json::json!({"clientId": "app", "attributes": {"a": "1"}});
        assert!(!diff(&remote, &partial));
        // Without partial updates (e.g. roles), omitted fields are real changes.
        assert!(print_diff("c", Some(&remote), &partial, true, false, "client").unwrap());

        let changed = serde_json::json!({"clientId": "app", "attributes": {"a": "9"}});
        assert!(diff(&remote, &changed));

        // Declared arrays are compared as a whole.
        let fewer_uris = serde_json::json!({"clientId": "app", "redirectUris": ["https://a"]});
        assert!(diff(&remote, &fewer_uris));
    }

    #[test]
    fn test_partial_projection_pairs_array_elements_by_identity() {
        let remote = serde_json::json!({"name": "s", "protocolMappers": [
            {"id": "b", "name": "B", "config": {"k": "1", "extra": "x"}},
            {"id": "a", "name": "A", "config": {"k": "2"}}
        ]});
        let local = serde_json::json!({"name": "s", "protocolMappers": [
            {"id": "a", "name": "A", "config": {"k": "2"}},
            {"id": "b", "name": "B", "config": {"k": "1", "extra": "x"}}
        ]});
        assert!(
            !print_resource_diff("s", Some(&remote), &local, true, false, "scope", true).unwrap()
        );
    }

    #[test]
    fn test_empty_collections_equal_absent_fields() {
        let remote =
            serde_json::json!({"name": "r", "attributes": {}, "composites": {"realm": []}});
        let local = serde_json::json!({"name": "r"});
        assert!(!print_diff("r", Some(&remote), &local, true, false, "role").unwrap());
        let local_with = serde_json::json!({"name": "r", "attributes": {"a": ["1"]}});
        assert!(print_diff("r", Some(&remote), &local_with, true, false, "role").unwrap());
    }

    #[test]
    fn test_masked_remote_secret_is_not_a_change() {
        let remote = serde_json::json!({"alias": "idp", "config": {"clientSecret": "**********"}});
        let local = serde_json::json!({"alias": "idp", "config": {"clientSecret": "real"}});
        assert!(!print_diff("IdP", Some(&remote), &local, true, false, "idp").unwrap());

        let changed = serde_json::json!({"alias": "idp", "config": {"clientSecret": "new"}});
        let previous = serde_json::json!({"alias": "idp", "config": {"clientSecret": "old"}});
        assert!(print_diff("IdP", Some(&previous), &changed, true, false, "idp").unwrap());
    }

    #[test]
    fn test_print_diff_no_changes() {
        let dummy = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", Some(&dummy), &dummy, false, false, "").unwrap();
        assert!(!result);
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
        assert!(result);
    }

    #[test]
    fn test_print_diff_no_changes_changes_only() {
        let dummy = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", Some(&dummy), &dummy, true, false, "").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_print_diff_new_resource() {
        let new = DummyResource {
            name: "test".to_string(),
            value: 42,
            secret: "secret_value".to_string(),
        };

        let result = print_diff("Dummy", None, &new, false, false, "").unwrap();
        assert!(result);
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
        assert!(result);
    }
}
