#![allow(clippy::collapsible_if)]
use crate::models::{KeycloakResource, ResourceMeta};
use crate::utils::secrets::substitute_secrets;
pub use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE};
use crate::utils::ui::{Ui, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

async fn order_files_topologically<T>(
    files: Vec<std::path::PathBuf>,
    profile: Option<&str>,
) -> Vec<Vec<std::path::PathBuf>>
where
    T: KeycloakResource,
{
    if T::DIR_NAME != crate::models::AuthenticationFlowRepresentation::DIR_NAME || files.len() <= 1
    {
        return vec![files];
    }

    // Parse each flow file to find its declared alias and referenced sub-flows
    let mut file_info: Vec<(std::path::PathBuf, String, Vec<String>)> = Vec::new();
    for path in files {
        if let Ok(val) = load_yaml_with_overlay(&path, profile).await {
            if let Ok(flow) =
                serde_json::from_value::<crate::models::AuthenticationFlowRepresentation>(val)
            {
                let alias = flow.alias.clone().unwrap_or_default();
                let subflows = flow.subflow_aliases();
                file_info.push((path, alias, subflows));
                continue;
            }
        }
        file_info.push((path, String::new(), Vec::new()));
    }

    let mut remaining = file_info;
    let mut tiers = Vec::new();
    let mut satisfied_aliases: HashSet<String> = HashSet::new();

    while !remaining.is_empty() {
        let mut current_tier = Vec::new();
        let mut next_remaining = Vec::new();

        let pending_aliases: HashSet<String> = remaining
            .iter()
            .map(|(_, alias, _)| alias.clone())
            .filter(|a| !a.is_empty())
            .collect();

        for (path, alias, subflows) in remaining {
            // A flow can be applied if all its local subflow dependencies are already satisfied
            let dependencies_ready = subflows
                .iter()
                .all(|sub| !pending_aliases.contains(sub) || satisfied_aliases.contains(sub));

            if dependencies_ready {
                current_tier.push((path, alias));
            } else {
                next_remaining.push((path, alias, subflows));
            }
        }

        if current_tier.is_empty() {
            // Unresolvable cycle or mutual dependency: fall back to applying all remaining in one tier
            let fallback: Vec<std::path::PathBuf> =
                next_remaining.into_iter().map(|(p, _, _)| p).collect();
            tiers.push(fallback);
            break;
        }

        let mut tier_paths = Vec::new();
        for (path, alias) in current_tier {
            if !alias.is_empty() {
                satisfied_aliases.insert(alias);
            }
            tier_paths.push(path);
        }

        tiers.push(tier_paths);
        remaining = next_remaining;
    }

    tiers
}

#[allow(clippy::too_many_arguments)]
pub async fn apply_resources<T>(ctx: crate::apply::ApplyContext<'_>) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + crate::client::KeycloakResourceMapping
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Send
        + Sync
        + Clone
        + 'static,
{
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
        prune,
        prompt_mutex,
    } = ctx;

    let dir_name = T::DIR_NAME;
    let resources_dir = workspace_dir.join(dir_name);
    let mut entries = match async_fs::read_dir(&resources_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let existing_resources = client
        .get_resources::<T>()
        .await
        .with_context(|| format!("Failed to get {} for realm '{}'", T::LABEL, realm_name))?;

    let existing_map: HashMap<String, String> = existing_resources
        .iter()
        .filter_map(|r| {
            let identity = r.get_identity();
            let id = r.get_id();
            match (identity, id) {
                (Some(identity), Some(id)) => Some((identity, id.to_string())),
                _ => None,
            }
        })
        .collect();
    let existing_map = Arc::new(existing_map);

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
        if !is_yaml_file(&path) {
            continue;
        }
        // Skip overlay files themselves
        if is_overlay_file(&path, profile.as_deref()) {
            continue;
        }
        files.push(path);
    }

    // Without files there is nothing to apply, but an empty directory still means
    // "manage this resource type", so pruning must run.
    if files.is_empty() && !prune {
        return Ok(());
    }

    let pb = create_progress_bar(files.len() as u64, &format!("Applying {}", T::LABEL));
    let tiers = order_files_topologically::<T>(files, profile.as_deref()).await;

    for tier_files in tiers {
        let mut set = JoinSet::new();

        for path in tier_files {
            let client = client.clone();
            let existing_map = Arc::clone(&existing_map);
            let resolver = Arc::clone(&resolver);
            let realm_name = realm_name.to_string();
            let profile = profile.clone();
            let ui = Arc::clone(&ui);
            let pb = pb.clone();
            let secrets_path = Arc::clone(&secrets_path);
            let prompt_mutex = Arc::clone(&prompt_mutex);

            set.spawn(async move {
                let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
                let local_val_before_sub = val.clone();
                substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
                let local_val_resolved = val.clone();
                let mut rep: T = serde_json::from_value(val)
                    .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

                let identity = rep.get_identity().with_context(|| {
                    format!("Failed to get identity for {} in {:?}", T::LABEL, path)
                })?;

                let id_opt = existing_map.get(&identity);

                if review {
                    let action = if id_opt.is_some() { "update" } else { "create" };
                    let proceed = {
                        let _lock = prompt_mutex.lock().await;
                        ui.confirm(
                            &format!(
                                "Do you want to {} {} '{}'?",
                                action,
                                T::LABEL,
                                rep.get_name()
                            ),
                            true,
                        )?
                    };
                    if !proceed {
                        pb.inc(1);
                        return Ok::<(), anyhow::Error>(());
                    }
                }

                let mut final_id = None;
                if let Some(id) = id_opt {
                    rep.set_id(Some(id.clone()));
                    client.update_resource(id, &rep).await.with_context(|| {
                        format!(
                            "Failed to update {} '{}' in realm '{}'",
                            T::LABEL,
                            rep.get_name(),
                            realm_name
                        )
                    })?;
                    crate::utils::ui::report(&pb, format!(
                        "  {} Updated {} {}",
                        SUCCESS_UPDATE,
                        T::LABEL,
                        rep.get_name()
                    ));
                    final_id = Some(id.clone());
                } else {
                    rep.set_id(None);
                    let create_result = client.create_resource(&rep).await;
                    match create_result {
                        Ok(maybe_id) => {
                            crate::utils::ui::report(&pb, format!(
                                "  {} Created {} {}",
                                SUCCESS_CREATE,
                                T::LABEL,
                                rep.get_name()
                            ));

                            if let Some(id) = maybe_id {
                                final_id = Some(id);
                            } else {
                                // Fallback: Fetch resources to get the generated ID of the created resource
                                let fresh_resources = client.get_resources::<T>().await?;
                                if let Some(fresh) = fresh_resources
                                    .into_iter()
                                    .find(|r| r.get_identity() == Some(identity.clone()))
                                {
                                    if let Some(id) = fresh.get_id() {
                                        final_id = Some(id.to_string());
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            let is_conflict = err_str.contains("409")
                                || err_str.to_lowercase().contains("conflict")
                                || err_str.to_lowercase().contains("already exists");

                            let mut adopted = false;
                            if is_conflict {
                                client.invalidate_resource_cache::<T>();
                                if let Ok(fresh_resources) = client.get_resources::<T>().await {
                                    if let Some(fresh) = fresh_resources
                                        .into_iter()
                                        .find(|r| r.get_identity() == Some(identity.clone()))
                                    {
                                        if let Some(existing_id) = fresh.get_id() {
                                            let mut update_rep = rep.clone();
                                            update_rep.set_id(Some(existing_id.to_string()));
                                            if let Ok(()) = client
                                                .update_resource(existing_id, &update_rep)
                                                .await
                                            {
                                                crate::utils::ui::report(&pb, format!(
                                                    "  {} Reconciled existing {} {} (adopted after conflict)",
                                                    SUCCESS_UPDATE,
                                                    T::LABEL,
                                                    rep.get_name()
                                                ));
                                                final_id = Some(existing_id.to_string());
                                                adopted = true;
                                            }
                                        }
                                    }
                                }
                            }

                            if !adopted {
                                return Err(err).with_context(|| {
                                    format!(
                                        "Failed to create {} '{}' in realm '{}'",
                                        T::LABEL,
                                        rep.get_name(),
                                        realm_name
                                    )
                                });
                            }
                        }
                    }
                }

                if let Some(id) = &final_id {
                    rep.post_save(&client, id).await.with_context(|| {
                        format!(
                            "Failed to reconcile {} '{}' in realm '{}'",
                            T::LABEL,
                            rep.get_name(),
                            realm_name
                        )
                    })?;
                }

                if let Some(id) = final_id {
                    if let Ok(mut enriched) = client.get_resource::<T>(&id).await {
                        // Relationships declared locally are not part of the single GET.
                        if !T::DEFERRED_RELATIONS {
                            enriched.load_relations(&client, Some(&rep)).await?;
                        }
                        rep.prepare_enriched(&mut enriched);
                        check_and_update_enrichment(
                            &path,
profile.as_deref(),
LocalSource { before_sub: &local_val_before_sub, resolved: &local_val_resolved },
                            &enriched,
                            &realm_name,
                            &secrets_path,
                            &*ui,
                            yes,
                            Arc::clone(&prompt_mutex),
                        )
                        .await?;
                    }
                }

                pb.inc(1);
                Ok::<(), anyhow::Error>(())
            });
        }

        crate::utils::join_all_tasks(set, None).await?;
    }

    pb.finish_with_message(format!("Applied {}", T::LABEL));

    if prune {
        let mut declared = HashSet::new();
        match async_fs::read_dir(&resources_dir).await {
            Ok(mut entries) => {
                while let Some(entry) = entries.next_entry().await? {
                    let path = entry.path();
                    if !is_yaml_file(&path) {
                        continue;
                    }
                    if is_overlay_file(&path, profile.as_deref()) {
                        continue;
                    }
                    if let Ok(val) = load_yaml_with_overlay(&path, profile.as_deref()).await {
                        if let Ok(rep) = serde_json::from_value::<T>(val) {
                            if let Some(identity) = rep.get_identity() {
                                declared.insert(identity);
                            }
                        }
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        for remote in &existing_resources {
            if let (Some(identity), Some(id)) = (remote.get_identity(), remote.get_id()) {
                if !declared.contains(&identity) {
                    if is_protected_resource(remote, &identity, realm_name) {
                        continue;
                    }

                    let proceed = if yes {
                        true
                    } else {
                        let _lock = prompt_mutex.lock().await;
                        ui.confirm(
                            &format!("Prune/Delete remote {} '{}'?", T::LABEL, remote.get_name()),
                            false,
                        )?
                    };

                    if proceed {
                        client.delete_resource::<T>(id).await.with_context(|| {
                            format!("Failed to prune {} '{}'", T::LABEL, remote.get_name())
                        })?;
                        eprintln!("  Removed/Pruned {} {}", T::LABEL, remote.get_name());
                    }
                }
            }
        }
    }

    Ok(())
}

/// Built-in client scopes created by Keycloak for every realm.
const PROTECTED_CLIENT_SCOPES: &[&str] = &[
    "profile",
    "email",
    "address",
    "phone",
    "offline_access",
    "roles",
    "web-origins",
    "microprofile-jwt",
    "acr",
    "basic",
    "role_list",
    "saml_organization",
    "organization",
    "service_account",
    "AuthnContextClassRef",
];

/// Built-in authentication flows (fallback when the server does not report `builtIn`).
const PROTECTED_FLOWS: &[&str] = &[
    "browser",
    "direct grant",
    "registration",
    "registration form",
    "reset credentials",
    "clients",
    "first broker login",
    "saml ecp",
    "docker auth",
    "http challenge",
];

/// Returns true if the client is a system client that must never be pruned.
///
/// Besides the per-realm system clients, the `master` realm owns one `<realm>-realm`
/// management client for every realm on the server.
pub fn is_protected_client(client_id: &str, realm_name: &str) -> bool {
    const PROTECTED: &[&str] = &[
        "admin-cli",
        "security-admin-console",
        "account",
        "account-console",
        "broker",
        "realm-management",
    ];
    PROTECTED.contains(&client_id) || (realm_name == "master" && client_id.ends_with("-realm"))
}

/// Returns true if a remote resource is managed by Keycloak itself and must never be pruned.
pub fn is_protected_resource<T>(remote: &T, identity: &str, realm_name: &str) -> bool
where
    T: KeycloakResource + serde::Serialize,
{
    let value = serde_json::to_value(remote).unwrap_or_default();
    if value.get("builtIn").and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    match T::API_PATH {
        "clients" => is_protected_client(identity, realm_name),
        "roles" => {
            let default_role = format!("default-roles-{}", realm_name);
            let mut protected = vec!["offline_access", "uma_authorization", &default_role];
            if realm_name == "master" {
                protected.extend(["admin", "create-realm"]);
            }
            protected.contains(&identity)
        }
        "client-scopes" => PROTECTED_CLIENT_SCOPES.contains(&identity),
        // Sub-flows are managed through their parent's executions, never pruned on their own.
        "authentication/flows" => {
            PROTECTED_FLOWS.contains(&identity)
                || value.get("topLevel").and_then(|v| v.as_bool()) == Some(false)
        }
        // Unregistering required actions is never what a prune should do.
        "authentication/required-actions" => true,
        // Service-account users belong to their client; master users include the admin login.
        "users" => realm_name == "master" || value.get("serviceAccountClientId").is_some(),
        _ => false,
    }
}

/// Value Keycloak returns in place of stored secrets (IdP client secrets, LDAP bind credentials, ...).
const KEYCLOAK_MASKED_SECRET: &str = "**********";

/// Local values of the file that produced an applied resource.
pub struct LocalSource<'a> {
    /// Local value (base + overlay) before secret substitution.
    pub before_sub: &'a serde_json::Value,
    /// Local value (base + overlay) after secret substitution.
    pub resolved: &'a serde_json::Value,
}

#[allow(clippy::too_many_arguments)]
pub async fn check_and_update_enrichment<T>(
    path: &std::path::Path,
    profile: Option<&str>,
    local: LocalSource<'_>,
    enriched: &T,
    realm_name: &str,
    secrets_path: &std::path::Path,
    ui: &dyn Ui,
    yes: bool,
    prompt_mutex: Arc<tokio::sync::Mutex<()>>,
) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Clone,
{
    let LocalSource {
        before_sub: local_val_before_sub,
        resolved: local_val_resolved,
    } = local;

    // The local value is base + overlay merged: writing it back would leak profile-specific
    // values into the base file, so leave both files untouched.
    if crate::utils::yaml::find_overlay_path(path, profile)
        .await
        .is_some()
    {
        let _lock = prompt_mutex.lock().await;
        crate::utils::ui::log_line(format!(
            "  {} Skipping local update of {} '{}': its file has a '{}' profile overlay",
            crate::utils::ui::INFO,
            T::LABEL,
            enriched.get_name(),
            profile.unwrap_or_default()
        ));
        return Ok(());
    }

    let mut enriched_raw = serde_json::to_value(enriched.clone())?;

    // Server identifiers and read-only server data are environment specific: never introduce
    // ones the user did not declare, and never replace declared ones with server values.
    if let (Some(obj), Some(local_obj)) = (
        enriched_raw.as_object_mut(),
        local_val_before_sub.as_object(),
    ) {
        // `access` (the caller's permissions), `createdTimestamp` and `subGroupCount` are
        // read-only server data as well.
        for key in [
            "id",
            "internalId",
            "parentId",
            "containerId",
            "access",
            "createdTimestamp",
            "subGroupCount",
        ] {
            match local_obj.get(key) {
                Some(local_id) if !local_id.is_null() => {
                    obj.insert(key.to_string(), local_id.clone());
                }
                _ => {
                    obj.remove(key);
                }
            }
        }
    }

    // Keycloak masks some stored secrets: keep what the user declared instead of the mask.
    restore_masked_values(&mut enriched_raw, local_val_before_sub);
    // Never drop what the user declared: write-only fields and references applied in a later
    // stage (e.g. an execution's `authenticatorConfig`) are absent from the server response.
    restore_missing_declared(&mut enriched_raw, local_val_before_sub);

    let mut enriched_val = enriched_raw.clone();
    let mut new_secrets = std::collections::BTreeMap::new();
    let prefix = format!("realm_{}_{}", realm_name, T::SECRET_PREFIX);
    crate::utils::secrets::extract_secrets(&mut enriched_val, &prefix, &mut new_secrets);

    let mut placeholders = Vec::new();
    find_placeholders(local_val_before_sub, &mut Vec::new(), &mut placeholders);
    for (p, placeholder) in &placeholders {
        let target =
            locate_placeholder_target(&enriched_raw, local_val_before_sub, local_val_resolved, p);
        if let Some(target) = target {
            set_value_at_path(
                &mut enriched_val,
                &target,
                serde_json::Value::String(placeholder.clone()),
            );
        }
    }

    let mut sorted_local_val = local_val_before_sub.clone();
    crate::utils::recursive_sort(&mut sorted_local_val);
    let local_yaml = serde_yaml::to_string(&sorted_local_val)?;

    crate::utils::recursive_sort(&mut enriched_val);
    let enriched_yaml = serde_yaml::to_string(&enriched_val)?;

    // Only persist extracted secrets that the written file still references.
    new_secrets.retain(|k, v| {
        v != KEYCLOAK_MASKED_SECRET && enriched_yaml.contains(&format!("${{{}}}", k))
    });

    if local_yaml != enriched_yaml {
        let proceed = if yes {
            true
        } else {
            let _lock = prompt_mutex.lock().await;
            ui.confirm(
                &format!(
                    "Keycloak enriched the representation of {} '{}'. Update the local file?",
                    T::LABEL,
                    enriched.get_name()
                ),
                true,
            )?
        };

        if proceed {
            crate::utils::write_secure(path, &enriched_yaml).await?;
            let _lock = prompt_mutex.lock().await;
            append_secrets(secrets_path, &new_secrets).await?;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

fn find_placeholders(
    val: &serde_json::Value,
    current_path: &mut Vec<PathSegment>,
    placeholders: &mut Vec<(Vec<PathSegment>, String)>,
) {
    match val {
        serde_json::Value::String(s) => {
            if crate::utils::secrets::contains_placeholder(s) {
                placeholders.push((current_path.clone(), s.clone()));
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                current_path.push(PathSegment::Key(k.clone()));
                find_placeholders(v, current_path, placeholders);
                current_path.pop();
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                current_path.push(PathSegment::Index(i));
                find_placeholders(v, current_path, placeholders);
                current_path.pop();
            }
        }
        _ => {}
    }
}

fn get_value_at_path<'a>(
    val: &'a serde_json::Value,
    segments: &[PathSegment],
) -> Option<&'a serde_json::Value> {
    segments
        .iter()
        .try_fold(val, |current, segment| match segment {
            PathSegment::Key(k) => current.get(k),
            PathSegment::Index(i) => current.get(*i),
        })
}

/// Finds where a local placeholder belongs in the enriched value.
///
/// The path is trusted when the enriched value there equals the resolved local value (or the
/// field is a secret Keycloak does not return). Inside arrays, Keycloak may reorder elements, so
/// the element holding the resolved value is searched for instead of trusting the index.
fn locate_placeholder_target(
    enriched_raw: &serde_json::Value,
    local_before_sub: &serde_json::Value,
    local_resolved: &serde_json::Value,
    path: &[PathSegment],
) -> Option<Vec<PathSegment>> {
    let resolved = get_value_at_path(local_resolved, path);
    let original = get_value_at_path(local_before_sub, path);
    match get_value_at_path(enriched_raw, path) {
        None => {
            // Field not returned by Keycloak (e.g. write-only secret): keep the placeholder
            // only if its parent exists so it is not lost.
            let parent_exists = path.len() <= 1
                || get_value_at_path(enriched_raw, &path[..path.len() - 1]).is_some();
            parent_exists.then(|| path.to_vec())
        }
        Some(current) if resolved.is_none_or(|r| r == current) || original == Some(current) => {
            Some(path.to_vec())
        }
        Some(serde_json::Value::String(s)) if s == KEYCLOAK_MASKED_SECRET => Some(path.to_vec()),
        Some(_) => {
            let resolved = resolved?;
            let (last, parent) = path.split_last()?;
            if !matches!(last, PathSegment::Index(_)) {
                return None;
            }
            let siblings = get_value_at_path(enriched_raw, parent)?.as_array()?;
            let idx = siblings.iter().position(|v| v == resolved)?;
            let mut target = parent.to_vec();
            target.push(PathSegment::Index(idx));
            Some(target)
        }
    }
}

/// Re-inserts object keys declared locally but missing from the enriched value.
///
/// Objects are merged recursively. Array elements are paired by an identity key
/// (`authenticator`, `flowAlias`, `clientId`, `name`, `alias`), or by position when they have
/// none and both arrays have the same length.
fn restore_missing_declared(enriched: &mut serde_json::Value, local: &serde_json::Value) {
    match (enriched, local) {
        (serde_json::Value::Object(enriched_map), serde_json::Value::Object(local_map)) => {
            for (key, local_val) in local_map {
                match enriched_map.get_mut(key) {
                    Some(enriched_val) => restore_missing_declared(enriched_val, local_val),
                    None if !local_val.is_null() => {
                        enriched_map.insert(key.clone(), local_val.clone());
                    }
                    None => {}
                }
            }
        }
        (serde_json::Value::Array(enriched_arr), serde_json::Value::Array(local_arr)) => {
            let same_len = enriched_arr.len() == local_arr.len();
            let mut used = vec![false; enriched_arr.len()];
            for (index, local_val) in local_arr.iter().enumerate() {
                let target = match identity_of(local_val) {
                    Some((key, id)) => enriched_arr
                        .iter()
                        .enumerate()
                        .position(|(j, e)| !used[j] && e.get(key) == Some(id)),
                    None if same_len => Some(index),
                    None => None,
                };
                if let Some(j) = target {
                    used[j] = true;
                    restore_missing_declared(&mut enriched_arr[j], local_val);
                }
            }
        }
        _ => {}
    }
}

/// Identity of an array element used to pair local and enriched elements.
fn identity_of(val: &serde_json::Value) -> Option<(&'static str, &serde_json::Value)> {
    const IDENTITY_KEYS: &[&str] = &["authenticator", "flowAlias", "clientId", "name", "alias"];
    IDENTITY_KEYS
        .iter()
        .find_map(|key| val.get(*key).map(|v| (*key, v)))
}

/// Replaces Keycloak's `**********` masks with the value declared locally at the same path.
fn restore_masked_values(enriched: &mut serde_json::Value, local: &serde_json::Value) {
    fn walk(
        enriched: &mut serde_json::Value,
        local: &serde_json::Value,
        path: &mut Vec<PathSegment>,
    ) {
        match enriched {
            serde_json::Value::String(s) if s == KEYCLOAK_MASKED_SECRET => {
                if let Some(local_val) = get_value_at_path(local, path) {
                    *enriched = local_val.clone();
                }
            }
            serde_json::Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    path.push(PathSegment::Key(k.clone()));
                    walk(v, local, path);
                    path.pop();
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, v) in arr.iter_mut().enumerate() {
                    path.push(PathSegment::Index(i));
                    walk(v, local, path);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    walk(enriched, local, &mut Vec::new());
}

fn set_value_at_path(
    val: &mut serde_json::Value,
    segments: &[PathSegment],
    new_val: serde_json::Value,
) {
    if segments.is_empty() {
        return;
    }
    match &segments[0] {
        PathSegment::Key(k) => {
            if segments.len() == 1 {
                if let Some(obj) = val.as_object_mut() {
                    obj.insert(k.clone(), new_val);
                }
            } else if let Some(obj) = val.as_object_mut() {
                if let Some(next) = obj.get_mut(k) {
                    set_value_at_path(next, &segments[1..], new_val);
                }
            }
        }
        PathSegment::Index(idx) => {
            if segments.len() == 1 {
                if let Some(arr) = val.as_array_mut() {
                    if *idx < arr.len() {
                        arr[*idx] = new_val;
                    }
                }
            } else if let Some(arr) = val.as_array_mut() {
                if *idx < arr.len() {
                    set_value_at_path(&mut arr[*idx], &segments[1..], new_val);
                }
            }
        }
    }
}

pub async fn append_secrets(
    secrets_path: &std::path::Path,
    new_secrets: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if new_secrets.is_empty() {
        return Ok(());
    }

    let mut content = match tokio::fs::read_to_string(secrets_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };

    let mut existing = std::collections::HashMap::new();
    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            existing.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    let mut appended = false;
    for (k, v) in new_secrets {
        if !existing.contains_key(k) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            use std::fmt::Write;
            let _ = writeln!(&mut content, "{}={}", k, v);
            appended = true;
        }
    }

    if appended {
        crate::utils::write_secure(secrets_path, &content).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ClientRepresentation;
    use crate::utils::ui::MockUi;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_check_and_update_enrichment_unit() -> Result<()> {
        let temp = tempdir()?;
        let client_path = temp.path().join("client.yaml");
        let secrets_path = temp.path().join(".secrets");

        // Write pre-existing secrets
        fs::write(&secrets_path, "EXISTING_KEY=old_val\n")?;

        // 1. Write a local YAML value containing secret placeholders inside object and array
        let local_yaml = serde_json::json!({
            "id": null,
            "clientId": "test-client",
            "name": "Initial Name",
            "secret": "${CLIENT_SECRET}",
            "redirectUris": [
                "http://localhost",
                "${REDIRECT_URI_PLACEHOLDER}"
            ]
        });
        fs::write(&client_path, serde_yaml::to_string(&local_yaml)?)?;

        // 2. Prepare an enriched representation returned from keycloak.
        // It has a secret value "my-new-secret" (which will be extracted),
        // and some new client fields.
        let enriched_client = ClientRepresentation {
            id: Some("generated-id-123".to_string()),
            client_id: Some("test-client".to_string()),
            secret: None,
            name: Some("Enriched Name from Keycloak".to_string()),
            description: None,
            enabled: Some(true),
            protocol: None,
            redirect_uris: Some(vec![
                "http://localhost".to_string(),
                "enriched-redirect-uri".to_string(),
            ]),
            web_origins: None,
            public_client: None,
            bearer_only: None,
            service_accounts_enabled: None,
            extra: [("secret".to_string(), serde_json::json!("my-new-secret"))]
                .into_iter()
                .collect(),
        };

        // UI confirms update
        let ui = MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(vec![true]),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        };

        // Resolved local value: placeholders substituted with what Keycloak now reports.
        let mut resolved = local_yaml.clone();
        resolved["secret"] = serde_json::json!("my-new-secret");
        resolved["redirectUris"][1] = serde_json::json!("enriched-redirect-uri");

        // Call check_and_update_enrichment with yes = false, confirm = true
        check_and_update_enrichment(
            &client_path,
            None,
            LocalSource {
                before_sub: &local_yaml,
                resolved: &resolved,
            },
            &enriched_client,
            "test-realm",
            &secrets_path,
            &ui,
            false,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .await?;

        // 3. Verify that:
        // A. The local file was updated with enriched fields
        let content = fs::read_to_string(&client_path)?;
        let parsed: serde_json::Value = serde_yaml::from_str(&content)?;

        // - Server ID is not introduced into the file
        assert!(parsed.get("id").is_none());
        // - Name is updated
        assert_eq!(
            parsed.get("name").and_then(|v| v.as_str()),
            Some("Enriched Name from Keycloak")
        );
        // - Placeholders are preserved!
        assert_eq!(
            parsed.get("secret").and_then(|v| v.as_str()),
            Some("${CLIENT_SECRET}")
        );
        let redirect_uris = parsed
            .get("redirectUris")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(
            redirect_uris[0].as_str(),
            Some("${REDIRECT_URI_PLACEHOLDER}")
        );

        // B. New secret was appended to .secrets, while preserving the existing one!
        let secrets_content = fs::read_to_string(&secrets_path)?;
        assert!(secrets_content.contains("EXISTING_KEY=old_val"));
        // The secret already has a user placeholder: no duplicate entry under a generated name.
        assert!(!secrets_content.contains("my-new-secret"));

        Ok(())
    }

    #[tokio::test]
    async fn test_check_and_update_enrichment_with_slashed_keys() -> Result<()> {
        let temp = tempdir()?;
        let client_path = temp.path().join("client.yaml");
        let secrets_path = temp.path().join(".secrets");

        let local_yaml = serde_json::json!({
            "clientId": "test-client",
            "attributes": {
                "custom/url/endpoint": "${ENDPOINT_URL}",
                "user.attribute/department": "${DEPT_NAME}"
            }
        });
        fs::write(&client_path, serde_yaml::to_string(&local_yaml)?)?;

        let mut extra = HashMap::new();
        extra.insert(
            "attributes".to_string(),
            serde_json::json!({
                "custom/url/endpoint": "placeholder-to-overwrite",
                "user.attribute/department": "placeholder-to-overwrite",
                "other.field": "val"
            }),
        );

        let enriched_client = ClientRepresentation {
            id: Some("gen-id".to_string()),
            client_id: Some("test-client".to_string()),
            secret: None,
            name: None,
            description: None,
            enabled: Some(true),
            protocol: None,
            redirect_uris: None,
            web_origins: None,
            public_client: None,
            bearer_only: None,
            service_accounts_enabled: None,
            extra,
        };

        let ui = MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(vec![true]),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        };

        let mut resolved = local_yaml.clone();
        resolved["attributes"]["custom/url/endpoint"] =
            serde_json::json!("placeholder-to-overwrite");
        resolved["attributes"]["user.attribute/department"] =
            serde_json::json!("placeholder-to-overwrite");
        check_and_update_enrichment(
            &client_path,
            None,
            LocalSource {
                before_sub: &local_yaml,
                resolved: &resolved,
            },
            &enriched_client,
            "test-realm",
            &secrets_path,
            &ui,
            false,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .await?;

        let content = fs::read_to_string(&client_path)?;
        let parsed: serde_json::Value = serde_yaml::from_str(&content)?;
        assert_eq!(
            parsed["attributes"]["custom/url/endpoint"].as_str(),
            Some("${ENDPOINT_URL}")
        );
        assert_eq!(
            parsed["attributes"]["user.attribute/department"].as_str(),
            Some("${DEPT_NAME}")
        );
        Ok(())
    }

    fn auto_ui() -> MockUi {
        MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(Vec::new()),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        }
    }

    async fn enrich(
        dir: &std::path::Path,
        profile: Option<&str>,
        local: &serde_json::Value,
        resolved: &serde_json::Value,
        enriched: serde_json::Value,
    ) -> Result<(serde_json::Value, String)> {
        let path = dir.join("client.yaml");
        let secrets_path = dir.join(".secrets");
        let original = serde_yaml::to_string(local)?;
        fs::write(&path, &original)?;
        let enriched: ClientRepresentation = serde_json::from_value(enriched)?;
        check_and_update_enrichment(
            &path,
            profile,
            LocalSource {
                before_sub: local,
                resolved,
            },
            &enriched,
            "r",
            &secrets_path,
            &auto_ui(),
            true,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .await?;
        let parsed = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
        let secrets = fs::read_to_string(&secrets_path).unwrap_or_default();
        Ok((parsed, secrets))
    }

    #[tokio::test]
    async fn test_enrichment_skips_files_with_overlay() -> Result<()> {
        let temp = tempdir()?;
        fs::write(
            temp.path().join("client.prod.yaml"),
            "rootUrl: https://prod\n",
        )?;
        let local = serde_json::json!({"clientId": "c", "rootUrl": "https://prod"});
        let (parsed, _) = enrich(
            temp.path(),
            Some("prod"),
            &local,
            &local,
            serde_json::json!({"id": "x", "clientId": "c", "rootUrl": "https://prod", "enabled": true}),
        )
        .await?;
        // Base file is untouched: no prod values or server defaults leaked into it.
        assert_eq!(parsed, local);
        Ok(())
    }

    #[tokio::test]
    async fn test_enrichment_preserves_embedded_placeholders() -> Result<()> {
        let temp = tempdir()?;
        let local = serde_json::json!({"clientId": "c", "rootUrl": "https://${HOST}/app"});
        let resolved = serde_json::json!({"clientId": "c", "rootUrl": "https://h.example/app"});
        let (parsed, _) = enrich(
            temp.path(),
            None,
            &local,
            &resolved,
            serde_json::json!({"clientId": "c", "rootUrl": "https://h.example/app", "enabled": true}),
        )
        .await?;
        assert_eq!(parsed["rootUrl"], "https://${HOST}/app");
        assert_eq!(parsed["enabled"], true);
        Ok(())
    }

    #[tokio::test]
    async fn test_enrichment_follows_reordered_arrays() -> Result<()> {
        let temp = tempdir()?;
        let local = serde_json::json!({"clientId": "c", "redirectUris": ["${CB}", "https://b"]});
        let resolved =
            serde_json::json!({"clientId": "c", "redirectUris": ["https://a", "https://b"]});
        let (parsed, _) = enrich(
            temp.path(),
            None,
            &local,
            &resolved,
            serde_json::json!({"clientId": "c", "redirectUris": ["https://b", "https://a"]}),
        )
        .await?;
        let uris = parsed["redirectUris"].as_array().unwrap();
        assert!(uris.contains(&serde_json::json!("${CB}")));
        assert!(uris.contains(&serde_json::json!("https://b")));
        assert!(!uris.contains(&serde_json::json!("https://a")));
        Ok(())
    }

    #[tokio::test]
    async fn test_enrichment_keeps_masked_secret_placeholder() -> Result<()> {
        let temp = tempdir()?;
        let local =
            serde_json::json!({"clientId": "c", "attributes": {"clientSecret": "${IDP_SECRET}"}});
        let resolved = serde_json::json!({"clientId": "c", "attributes": {"clientSecret": "real"}});
        let (parsed, secrets) = enrich(
            temp.path(),
            None,
            &local,
            &resolved,
            serde_json::json!({"clientId": "c", "attributes": {"clientSecret": "**********"}, "enabled": true}),
        )
        .await?;
        assert_eq!(parsed["attributes"]["clientSecret"], "${IDP_SECRET}");
        assert!(!secrets.contains("**********"));
        assert!(!secrets.contains("${IDP_SECRET}"));
        Ok(())
    }

    #[tokio::test]
    async fn test_enrichment_extracts_new_generated_secret() -> Result<()> {
        let temp = tempdir()?;
        let local = serde_json::json!({"clientId": "c"});
        let (parsed, secrets) = enrich(
            temp.path(),
            None,
            &local,
            &local,
            serde_json::json!({"id": "x", "clientId": "c", "secret": "generated"}),
        )
        .await?;
        let placeholder = parsed["secret"].as_str().unwrap().to_string();
        assert!(placeholder.starts_with("${KEYCLOAK_"));
        assert!(secrets.contains("=generated"));
        assert!(parsed.get("id").is_none());
        Ok(())
    }

    #[test]
    fn test_restore_missing_declared() {
        let local = serde_json::json!({
            "alias": "f",
            "nullable": null,
            "executions": [
                {"authenticator": "a", "authenticatorConfig": "cfg"},
                {"authenticator": "b"}
            ],
            "nested": {"writeOnly": "x"}
        });
        let mut enriched = serde_json::json!({
            "alias": "f",
            "executions": [
                {"authenticator": "a", "priority": 10},
                {"authenticator": "b", "priority": 20}
            ],
            "nested": {}
        });
        restore_missing_declared(&mut enriched, &local);
        assert_eq!(enriched["executions"][0]["authenticatorConfig"], "cfg");

        assert_eq!(enriched["executions"][0]["priority"], 10);
        assert_eq!(enriched["nested"]["writeOnly"], "x");
        assert!(enriched.get("nullable").is_none());

        // Arrays of different lengths are paired by identity key.
        let local = serde_json::json!({"executions": [
            {"authenticator": "b", "authenticatorConfig": "cfg-b"}
        ]});
        let mut enriched = serde_json::json!({"executions": [
            {"authenticator": "a"},
            {"authenticator": "b", "priority": 20}
        ]});
        restore_missing_declared(&mut enriched, &local);
        assert!(
            enriched["executions"][0]
                .get("authenticatorConfig")
                .is_none()
        );
        assert_eq!(enriched["executions"][1]["authenticatorConfig"], "cfg-b");
    }

    fn protected<T>(json: serde_json::Value, realm: &str) -> bool
    where
        T: KeycloakResource + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let rep: T = serde_json::from_value(json).unwrap();
        let identity = rep.get_identity().unwrap();
        is_protected_resource(&rep, &identity, realm)
    }

    #[test]
    fn test_is_protected_resource_branches() {
        use crate::models::{
            AuthenticationFlowRepresentation, ClientRepresentation, ClientScopeRepresentation,
            GroupRepresentation, RequiredActionProviderRepresentation, RoleRepresentation,
            UserRepresentation,
        };
        use serde_json::json;

        // clients
        for c in [
            "admin-cli",
            "security-admin-console",
            "account",
            "realm-management",
        ] {
            assert!(protected::<ClientRepresentation>(
                json!({"clientId": c}),
                "r"
            ));
        }
        assert!(!protected::<ClientRepresentation>(
            json!({"clientId": "my-app"}),
            "r"
        ));
        assert!(protected::<ClientRepresentation>(
            json!({"clientId": "foo-realm"}),
            "master"
        ));
        assert!(!protected::<ClientRepresentation>(
            json!({"clientId": "foo-realm"}),
            "r"
        ));

        // roles
        assert!(protected::<RoleRepresentation>(
            json!({"name": "offline_access"}),
            "r"
        ));
        assert!(protected::<RoleRepresentation>(
            json!({"name": "default-roles-r"}),
            "r"
        ));
        assert!(protected::<RoleRepresentation>(
            json!({"name": "admin"}),
            "master"
        ));
        assert!(!protected::<RoleRepresentation>(
            json!({"name": "admin"}),
            "r"
        ));

        // client scopes, including those added in recent Keycloak versions
        for s in [
            "profile",
            "roles",
            "acr",
            "basic",
            "role_list",
            "organization",
            "service_account",
            "saml_organization",
            "AuthnContextClassRef",
        ] {
            assert!(
                protected::<ClientScopeRepresentation>(json!({"name": s}), "r"),
                "{s}"
            );
        }
        assert!(!protected::<ClientScopeRepresentation>(
            json!({"name": "custom"}),
            "r"
        ));

        // flows: builtIn flag wins, hard-coded list as fallback
        assert!(protected::<AuthenticationFlowRepresentation>(
            json!({"alias": "browser"}),
            "r"
        ));
        assert!(protected::<AuthenticationFlowRepresentation>(
            json!({"alias": "new-builtin", "builtIn": true}),
            "r"
        ));
        assert!(!protected::<AuthenticationFlowRepresentation>(
            json!({"alias": "custom", "builtIn": false}),
            "r"
        ));
        assert!(protected::<AuthenticationFlowRepresentation>(
            json!({"alias": "custom-sub", "topLevel": false}),
            "r"
        ));

        // required actions are never pruned
        assert!(protected::<RequiredActionProviderRepresentation>(
            json!({"alias": "CONFIGURE_TOTP"}),
            "r"
        ));

        // users: service accounts and master users are protected
        assert!(protected::<UserRepresentation>(
            json!({"username": "service-account-x", "serviceAccountClientId": "id"}),
            "r"
        ));
        assert!(protected::<UserRepresentation>(
            json!({"username": "admin"}),
            "master"
        ));
        assert!(!protected::<UserRepresentation>(
            json!({"username": "bob"}),
            "r"
        ));

        // other (e.g. groups)
        assert!(!protected::<GroupRepresentation>(json!({"name": "g"}), "r"));
    }

    #[tokio::test]
    async fn test_append_secrets_empty() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        let new_secrets = std::collections::BTreeMap::new();

        append_secrets(&secrets_path, &new_secrets).await?;

        assert!(!secrets_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_new_file() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "val1".to_string());
        new_secrets.insert("KEY2".to_string(), "val2".to_string());

        append_secrets(&secrets_path, &new_secrets).await?;

        assert!(secrets_path.exists());
        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("KEY1=val1\n"));
        assert!(content.contains("KEY2=val2\n"));
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_existing_file() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        fs::write(&secrets_path, "EXISTING_KEY=old_val\nKEY1=old_val1\n")?;

        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "new_val1".to_string()); // Should be ignored since KEY1 exists
        new_secrets.insert("KEY2".to_string(), "val2".to_string()); // Should be added

        append_secrets(&secrets_path, &new_secrets).await?;

        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("EXISTING_KEY=old_val\n"));
        assert!(content.contains("KEY1=old_val1\n"));
        assert!(!content.contains("KEY1=new_val1"));
        assert!(content.contains("KEY2=val2\n"));
        Ok(())
    }

    #[tokio::test]
    async fn test_append_secrets_missing_newline() -> Result<()> {
        let temp = tempdir()?;
        let secrets_path = temp.path().join(".secrets");
        fs::write(&secrets_path, "EXISTING_KEY=old_val")?; // No trailing newline

        let mut new_secrets = std::collections::BTreeMap::new();
        new_secrets.insert("KEY1".to_string(), "val1".to_string());

        append_secrets(&secrets_path, &new_secrets).await?;

        let content = fs::read_to_string(&secrets_path)?;
        assert!(content.contains("EXISTING_KEY=old_val\n"));
        assert!(content.contains("KEY1=val1\n"));
        Ok(())
    }
}
