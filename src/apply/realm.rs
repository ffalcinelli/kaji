use crate::models::RealmRepresentation;
use crate::utils::secrets::substitute_secrets;
use crate::utils::ui::{INFO, SUCCESS_CREATE, SUCCESS_UPDATE};
use crate::utils::yaml::load_yaml_with_overlay;
use anyhow::{Context, Result};
use console::style;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;

/// Realm attributes that bind an authentication flow by alias.
///
/// Keycloak rejects a realm update (HTTP 500) when one of them references a flow that does not
/// exist yet, so bindings to flows created later in the same apply are deferred.
pub const REALM_FLOW_BINDINGS: &[&str] = &[
    "browserFlow",
    "registrationFlow",
    "directGrantFlow",
    "resetCredentialsFlow",
    "clientAuthenticationFlow",
    "dockerAuthenticationFlow",
    "firstBrokerLoginFlow",
];

/// Realm state carried from Stage 0 to the final stage of an apply.
pub struct PendingRealm {
    path: PathBuf,
    before_sub: Value,
    resolved: Value,
    deferred_bindings: Map<String, Value>,
}

impl PendingRealm {
    /// Flow bindings that will be applied after authentication flows exist.
    pub fn deferred_bindings(&self) -> &Map<String, Value> {
        &self.deferred_bindings
    }
}

fn without_keys(val: &Value, keys: &[&str]) -> Value {
    let mut val = val.clone();
    if let Some(obj) = val.as_object_mut() {
        for key in keys {
            obj.remove(*key);
        }
    }
    val
}

/// Stage 0: creates or updates the realm, deferring flow bindings to flows that do not exist yet.
///
/// Returns `None` when there is no `realm.yaml` to apply (missing or excluded by the plan).
pub async fn apply_realm(ctx: crate::apply::ApplyContext<'_>) -> Result<Option<PendingRealm>> {
    let crate::apply::ApplyContext {
        client,
        workspace_dir,
        resolver,
        planned_files,
        realm_name,
        profile,
        review,
        ui,
        prompt_mutex,
        ..
    } = ctx;

    let realm_path = workspace_dir.join("realm.yaml");
    if let Some(plan) = &*planned_files
        && !plan.contains(&realm_path)
    {
        return Ok(None);
    }
    if !async_fs::try_exists(&realm_path).await? {
        return Ok(None);
    }

    let mut val = load_yaml_with_overlay(&realm_path, profile.as_deref()).await?;
    let before_sub = val.clone();
    substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
    let resolved = val.clone();

    let exists = match client.get_realm().await {
        Ok(_) => true,
        Err(e) if crate::client::is_not_found(&e) => false,
        Err(e) => return Err(e).with_context(|| format!("Failed to get realm '{}'", realm_name)),
    };

    if review {
        let action = if exists { "update" } else { "create" };
        let proceed = {
            let _lock = prompt_mutex.lock().await;
            ui.confirm(
                &format!("Do you want to {} realm '{}'?", action, realm_name),
                true,
            )?
        };
        if !proceed {
            return Ok(None);
        }
    }

    if !exists {
        // Built-in flows only exist once the realm does: create it without any binding first.
        let create_rep: RealmRepresentation =
            serde_json::from_value(without_keys(&resolved, REALM_FLOW_BINDINGS))?;
        client
            .create_realm(&create_rep)
            .await
            .with_context(|| format!("Failed to create realm '{}'", realm_name))?;
        eprintln!(
            "  {} {}",
            SUCCESS_CREATE,
            style(format!("Created realm {}", realm_name)).green()
        );
    }

    let remote_flows: HashSet<String> = client
        .get_authentication_flows_raw()
        .await
        .with_context(|| {
            format!(
                "Failed to get authentication flows of realm '{}'",
                realm_name
            )
        })?
        .into_iter()
        .filter_map(|f| f.alias)
        .collect();

    let mut deferred_bindings = Map::new();
    if let Some(obj) = resolved.as_object() {
        for key in REALM_FLOW_BINDINGS {
            if let Some(Value::String(alias)) = obj.get(*key)
                && !remote_flows.contains(alias)
            {
                deferred_bindings.insert(key.to_string(), Value::String(alias.clone()));
            }
        }
    }
    let deferred_keys: Vec<&str> = deferred_bindings.keys().map(String::as_str).collect();

    let update_rep: RealmRepresentation =
        serde_json::from_value(without_keys(&resolved, &deferred_keys))?;
    client
        .update_realm(&update_rep)
        .await
        .with_context(|| format!("Failed to update realm '{}'", realm_name))?;
    if exists {
        eprintln!(
            "  {} {}",
            SUCCESS_UPDATE,
            style("Updated realm configuration").cyan()
        );
    }
    if !deferred_bindings.is_empty() {
        eprintln!(
            "  {} {}",
            INFO,
            style(format!(
                "Deferring realm flow bindings until flows exist: {}",
                deferred_keys.join(", ")
            ))
            .dim()
        );
    }

    Ok(Some(PendingRealm {
        path: realm_path,
        before_sub,
        resolved,
        deferred_bindings,
    }))
}

/// Final stage: applies deferred flow bindings and offers to sync Keycloak's enrichment locally.
pub async fn finish_realm(
    ctx: crate::apply::ApplyContext<'_>,
    pending: Option<PendingRealm>,
) -> Result<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let crate::apply::ApplyContext {
        client,
        secrets_path,
        realm_name,
        profile,
        ui,
        yes,
        prompt_mutex,
        ..
    } = ctx;

    if !pending.deferred_bindings.is_empty() {
        let current = client
            .get_realm()
            .await
            .with_context(|| format!("Failed to get realm '{}'", realm_name))?;
        let mut current_val = serde_json::to_value(current)?;
        if let Some(obj) = current_val.as_object_mut() {
            for (key, value) in &pending.deferred_bindings {
                obj.insert(key.clone(), value.clone());
            }
        }
        let rep: RealmRepresentation = serde_json::from_value(current_val)?;
        client.update_realm(&rep).await.with_context(|| {
            format!(
                "Failed to bind authentication flows of realm '{}'",
                realm_name
            )
        })?;
        eprintln!(
            "  {} {}",
            SUCCESS_UPDATE,
            style("Applied realm flow bindings").cyan()
        );
    }

    if let Ok(enriched) = client.get_realm().await {
        crate::apply::generic::check_and_update_enrichment(
            crate::apply::generic::LocalSource {
                path: &pending.path,
                profile: profile.as_deref(),
                before_sub: &pending.before_sub,
                resolved: &pending.resolved,
            },
            &enriched,
            realm_name,
            &secrets_path,
            &*ui,
            yes,
            prompt_mutex,
        )
        .await?;
    }
    Ok(())
}
