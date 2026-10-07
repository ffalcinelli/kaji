//! Reconciliation of relationships that Keycloak's main create/update endpoints ignore.
//!
//! Verified against Keycloak 26.8.0: a client's `defaultClientScopes`/`optionalClientScopes`
//! are only honored when the client is created, so later changes go through the dedicated
//! `/clients/{id}/{default,optional}-client-scopes/{scopeId}` endpoints.

use crate::client::KeycloakClient;
use crate::models::ClientRepresentation;
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

/// Reads a declared list of strings from a resource's flattened fields.
///
/// Returns `None` when the key is absent (the relationship is not managed by the file).
pub fn declared_names(
    extra: &HashMap<String, serde_json::Value>,
    key: &str,
) -> Option<Vec<String>> {
    extra.get(key).and_then(|v| v.as_array()).map(|items| {
        items
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect()
    })
}

/// Reconciles the default and optional client scopes of a client with the declared lists.
pub async fn reconcile_client_scopes(
    client: &KeycloakClient,
    rep: &ClientRepresentation,
    client_internal_id: &str,
) -> Result<()> {
    let kinds = [
        ("defaultClientScopes", "default-client-scopes"),
        ("optionalClientScopes", "optional-client-scopes"),
    ];
    let desired: Vec<(&str, Option<Vec<String>>)> = kinds
        .iter()
        .map(|(key, path)| (*path, declared_names(&rep.extra, key)))
        .collect();
    if desired.iter().all(|(_, d)| d.is_none()) {
        return Ok(());
    }

    let scope_ids: HashMap<String, String> = client
        .get_client_scopes()
        .await?
        .into_iter()
        .filter_map(|s| Some((s.name?, s.id?)))
        .collect();
    let mut current: HashMap<&str, HashSet<String>> = HashMap::new();
    for (path, wanted) in &desired {
        if wanted.is_some() {
            let names = client
                .get_client_scope_links(client_internal_id, path)
                .await?
                .into_iter()
                .collect();
            current.insert(*path, names);
        }
    }
    let name = rep.get_client_id_or_unknown();

    // Removals first, so a scope can move between the default and optional lists.
    for (path, wanted) in &desired {
        let Some(wanted) = wanted else { continue };
        for scope in current[path].iter().filter(|s| !wanted.contains(*s)) {
            let id = scope_ids
                .get(scope)
                .with_context(|| format!("Client scope '{}' not found", scope))?;
            client
                .unlink_client_scope(client_internal_id, path, id)
                .await
                .with_context(|| {
                    format!(
                        "Failed to remove {} '{}' from client '{}'",
                        path, scope, name
                    )
                })?;
            crate::utils::ui::log_line(format!(
                "    {} Removed {} {} from client {}",
                SUCCESS_UPDATE, path, scope, name
            ));
        }
    }
    for (path, wanted) in &desired {
        let Some(wanted) = wanted else { continue };
        for scope in wanted.iter().filter(|s| !current[path].contains(*s)) {
            let id = scope_ids.get(scope).with_context(|| {
                format!(
                    "Client scope '{}' referenced by client '{}' not found",
                    scope, name
                )
            })?;
            client
                .link_client_scope(client_internal_id, path, id)
                .await
                .with_context(|| {
                    format!("Failed to add {} '{}' to client '{}'", path, scope, name)
                })?;
            crate::utils::ui::log_line(format!(
                "    {} Added {} {} to client {}",
                SUCCESS_CREATE, path, scope, name
            ));
        }
    }
    Ok(())
}

/// Reads declared client roles (`clientRoles: {clientId: [role, ...]}`).
pub fn declared_client_roles(
    extra: &HashMap<String, serde_json::Value>,
) -> Option<HashMap<String, Vec<String>>> {
    extra
        .get("clientRoles")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(client_id, roles)| {
                    let names = roles
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|r| r.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    (client_id.clone(), names)
                })
                .collect()
        })
}

fn names_of(roles: &[crate::models::RoleRepresentation]) -> Vec<String> {
    let mut names: Vec<String> = roles.iter().map(|r| r.name.clone()).collect();
    names.sort();
    names
}

/// Loads role mappings of `owner` into `realmRoles`/`clientRoles` of `extra`.
///
/// With `declared`, only the keys present there are loaded.
pub async fn load_role_mappings(
    client: &KeycloakClient,
    owner: &[&str],
    extra: &mut HashMap<String, serde_json::Value>,
    declared: Option<&HashMap<String, serde_json::Value>>,
) -> Result<()> {
    let want_realm = declared.is_none_or(|d| d.contains_key("realmRoles"));
    let want_clients = declared.is_none_or(|d| d.contains_key("clientRoles"));
    if !want_realm && !want_clients {
        return Ok(());
    }
    let mappings = client.get_role_mappings(owner).await?;
    if want_realm {
        extra.insert(
            "realmRoles".to_string(),
            serde_json::json!(names_of(&mappings.realm_mappings)),
        );
    }
    if want_clients {
        let clients: serde_json::Map<String, serde_json::Value> = mappings
            .client_mappings
            .iter()
            .filter(|(_, m)| !m.mappings.is_empty())
            .map(|(client_id, m)| (client_id.clone(), serde_json::json!(names_of(&m.mappings))))
            .collect();
        extra.insert(
            "clientRoles".to_string(),
            serde_json::Value::Object(clients),
        );
    }
    Ok(())
}

/// Loads a user's groups and role mappings (see [`load_role_mappings`]).
pub async fn load_user_relations(
    client: &KeycloakClient,
    user: &mut crate::models::UserRepresentation,
    declared: Option<&HashMap<String, serde_json::Value>>,
) -> Result<()> {
    let Some(id) = user.id.clone() else {
        return Ok(());
    };
    if declared.is_none_or(|d| d.contains_key("groups")) {
        let mut paths = client.get_user_group_paths(&id).await?;
        paths.sort();
        user.extra
            .insert("groups".to_string(), serde_json::json!(paths));
    }
    load_role_mappings(client, &["users", &id], &mut user.extra, declared).await
}

/// Reconciles the direct role mappings of `owner` with the declared realm and client roles.
pub async fn reconcile_role_mappings(
    client: &KeycloakClient,
    owner: &[&str],
    owner_label: &str,
    realm_roles: Option<Vec<String>>,
    client_roles: Option<HashMap<String, Vec<String>>>,
) -> Result<()> {
    if realm_roles.is_none() && client_roles.is_none() {
        return Ok(());
    }
    let current = client.get_role_mappings(owner).await?;

    if let Some(wanted) = realm_roles {
        let wanted: HashSet<String> = wanted.into_iter().collect();
        let remove: Vec<_> = current
            .realm_mappings
            .iter()
            .filter(|r| !wanted.contains(&r.name))
            .cloned()
            .collect();
        let have: HashSet<&String> = current.realm_mappings.iter().map(|r| &r.name).collect();
        let mut add = Vec::new();
        for name in wanted.iter().filter(|n| !have.contains(n)) {
            add.push(client.get_realm_role(name).await.with_context(|| {
                format!(
                    "Realm role '{}' assigned to {} not found",
                    name, owner_label
                )
            })?);
        }
        apply_mapping_change(
            client,
            owner,
            &["realm"],
            owner_label,
            "realm roles",
            remove,
            add,
        )
        .await?;
    }

    if let Some(wanted) = client_roles {
        let client_ids: HashMap<String, String> = client
            .get_clients()
            .await?
            .into_iter()
            .filter_map(|c| Some((c.client_id?, c.id?)))
            .collect();
        let mut all_clients: HashSet<String> = wanted.keys().cloned().collect();
        all_clients.extend(current.client_mappings.keys().cloned());
        for client_id in all_clients {
            let wanted_names: HashSet<String> = wanted
                .get(&client_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let have: Vec<crate::models::RoleRepresentation> = current
                .client_mappings
                .get(&client_id)
                .map(|m| m.mappings.clone())
                .unwrap_or_default();
            let internal_id = client_ids
                .get(&client_id)
                .with_context(|| format!("Client '{}' not found", client_id))?;
            let remove: Vec<_> = have
                .iter()
                .filter(|r| !wanted_names.contains(&r.name))
                .cloned()
                .collect();
            let have_names: HashSet<&String> = have.iter().map(|r| &r.name).collect();
            let mut add = Vec::new();
            for name in wanted_names.iter().filter(|n| !have_names.contains(n)) {
                add.push(
                    client
                        .get_client_role(internal_id, name)
                        .await
                        .with_context(|| {
                            format!(
                                "Client role '{}' of client '{}' assigned to {} not found",
                                name, client_id, owner_label
                            )
                        })?,
                );
            }
            let what = format!("roles of client {}", client_id);
            apply_mapping_change(
                client,
                owner,
                &["clients", internal_id],
                owner_label,
                &what,
                remove,
                add,
            )
            .await?;
        }
    }
    Ok(())
}

async fn apply_mapping_change(
    client: &KeycloakClient,
    owner: &[&str],
    target: &[&str],
    owner_label: &str,
    what: &str,
    remove: Vec<crate::models::RoleRepresentation>,
    add: Vec<crate::models::RoleRepresentation>,
) -> Result<()> {
    if !remove.is_empty() {
        client
            .change_role_mappings(owner, target, &remove, false)
            .await
            .with_context(|| format!("Failed to remove {} from {}", what, owner_label))?;
        crate::utils::ui::log_line(format!(
            "    {} Removed {} {} from {}",
            SUCCESS_UPDATE,
            what,
            names_of(&remove).join(", "),
            owner_label
        ));
    }
    if !add.is_empty() {
        client
            .change_role_mappings(owner, target, &add, true)
            .await
            .with_context(|| format!("Failed to add {} to {}", what, owner_label))?;
        crate::utils::ui::log_line(format!(
            "    {} Added {} {} to {}",
            SUCCESS_CREATE,
            what,
            names_of(&add).join(", "),
            owner_label
        ));
    }
    Ok(())
}

/// Reconciles a user's group membership and role mappings.
pub async fn reconcile_user(
    client: &KeycloakClient,
    user: &crate::models::UserRepresentation,
    user_id: &str,
) -> Result<()> {
    let label = format!("user {}", user.username.as_deref().unwrap_or(user_id));
    if let Some(wanted) = declared_names(&user.extra, "groups") {
        let normalize = |p: &str| format!("/{}", p.trim_start_matches('/'));
        let wanted: HashSet<String> = wanted.iter().map(|p| normalize(p)).collect();
        let current: HashSet<String> = client
            .get_user_group_paths(user_id)
            .await?
            .iter()
            .map(|p| normalize(p))
            .collect();
        for path in current.difference(&wanted) {
            let group = client.get_group_by_path(path).await?;
            let id = group
                .id
                .with_context(|| format!("Group '{}' has no ID", path))?;
            client.set_user_group(user_id, &id, false).await?;
            crate::utils::ui::log_line(format!(
                "    {} Removed {} from group {}",
                SUCCESS_UPDATE, label, path
            ));
        }
        for path in wanted.difference(&current) {
            let group = client
                .get_group_by_path(path)
                .await
                .with_context(|| format!("Group '{}' of {} not found", path, label))?;
            let id = group
                .id
                .with_context(|| format!("Group '{}' has no ID", path))?;
            client.set_user_group(user_id, &id, true).await?;
            crate::utils::ui::log_line(format!(
                "    {} Added {} to group {}",
                SUCCESS_CREATE, label, path
            ));
        }
    }
    reconcile_role_mappings(
        client,
        &["users", user_id],
        &label,
        declared_names(&user.extra, "realmRoles"),
        declared_client_roles(&user.extra),
    )
    .await
}

/// Reconciles a group's role mappings and, when declared, its sub-groups (recursively).
///
/// Declared sub-groups are matched by name: missing ones are created, existing ones updated,
/// and sub-groups that are no longer declared are deleted.
#[async_recursion::async_recursion]
#[allow(clippy::double_must_use)]
pub async fn reconcile_group(
    client: &KeycloakClient,
    group: &crate::models::GroupRepresentation,
    group_id: &str,
) -> Result<()> {
    let label = format!("group {}", group.get_name_or_unknown());
    reconcile_role_mappings(
        client,
        &["groups", group_id],
        &label,
        declared_names(&group.extra, "realmRoles"),
        declared_client_roles(&group.extra),
    )
    .await?;

    let Some(wanted) = group.sub_groups.as_ref() else {
        return Ok(());
    };
    let current = client.get_group_children(group_id).await?;
    for existing in &current {
        let declared = wanted.iter().any(|w| w.name == existing.name);
        if !declared {
            let id = existing.id.as_deref().context("Sub-group has no ID")?;
            client.delete_group(id).await.with_context(|| {
                format!(
                    "Failed to delete sub-group '{}' of {}",
                    existing.get_name_or_unknown(),
                    label
                )
            })?;
            crate::utils::ui::log_line(format!(
                "    {} Removed sub-group {} from {}",
                SUCCESS_UPDATE,
                existing.get_name_or_unknown(),
                label
            ));
        }
    }
    for child in wanted {
        let mut body = child.clone();
        body.clear_server_fields();
        body.sub_groups = None;
        let existing = current.iter().find(|c| c.name == child.name);
        let child_id = match existing.and_then(|c| c.id.clone()) {
            Some(id) => {
                body.id = Some(id.clone());
                client.update_group(&id, &body).await.with_context(|| {
                    format!(
                        "Failed to update sub-group '{}' of {}",
                        child.get_name_or_unknown(),
                        label
                    )
                })?;
                id
            }
            None => {
                let created = client
                    .create_child_group(group_id, &body)
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to create sub-group '{}' of {}",
                            child.get_name_or_unknown(),
                            label
                        )
                    })?;
                crate::utils::ui::log_line(format!(
                    "    {} Created sub-group {} in {}",
                    SUCCESS_CREATE,
                    child.get_name_or_unknown(),
                    label
                ));
                match created {
                    Some(id) => id,
                    None => client
                        .get_group_children(group_id)
                        .await?
                        .into_iter()
                        .find(|c| c.name == child.name)
                        .and_then(|c| c.id)
                        .context("Created sub-group not found")?,
                }
            }
        };
        reconcile_group(client, child, &child_id).await?;
    }
    Ok(())
}

/// Maps client UUIDs to client IDs (and back) for role references.
pub async fn client_id_maps(
    client: &KeycloakClient,
) -> Result<(HashMap<String, String>, HashMap<String, String>)> {
    let mut by_internal_id = HashMap::new();
    let mut by_client_id = HashMap::new();
    for c in client.get_clients().await? {
        if let (Some(internal_id), Some(client_id)) = (c.id, c.client_id) {
            by_internal_id.insert(internal_id.clone(), client_id.clone());
            by_client_id.insert(client_id, internal_id);
        }
    }
    Ok((by_internal_id, by_client_id))
}

/// Loads a role's composites into `composites` (Keycloak export format:
/// `{realm: [names], client: {clientId: [names]}}`).
pub async fn load_role_composites(
    client: &KeycloakClient,
    role: &mut crate::models::RoleRepresentation,
) -> Result<()> {
    let Some(id) = role.id.clone() else {
        return Ok(());
    };
    let composites = client.get_role_composites(&id).await?;
    let (by_internal_id, _) = client_id_maps(client).await?;
    let mut realm = Vec::new();
    let mut clients: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for c in composites {
        if c.client_role {
            let client_id = c
                .container_id
                .as_ref()
                .and_then(|internal_id| by_internal_id.get(internal_id))
                .cloned()
                .unwrap_or_default();
            clients.entry(client_id).or_default().push(c.name);
        } else {
            realm.push(c.name);
        }
    }
    realm.sort();
    let mut value = serde_json::Map::new();
    if !realm.is_empty() {
        value.insert("realm".to_string(), serde_json::json!(realm));
    }
    if !clients.is_empty() {
        for names in clients.values_mut() {
            names.sort();
        }
        value.insert("client".to_string(), serde_json::json!(clients));
    }
    role.extra
        .insert("composites".to_string(), serde_json::Value::Object(value));
    Ok(())
}

/// Reconciles the composites of a role with its declared `composites` (if any).
pub async fn reconcile_role_composites(
    client: &KeycloakClient,
    role: &crate::models::RoleRepresentation,
    role_id: &str,
    label: &str,
) -> Result<()> {
    let Some(declared) = role.extra.get("composites").and_then(|c| c.as_object()) else {
        return Ok(());
    };
    let wanted_realm: HashSet<String> = declared
        .get("realm")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let wanted_client: HashSet<(String, String)> = declared
        .get("client")
        .and_then(|c| c.as_object())
        .map(|m| {
            m.iter()
                .flat_map(|(client_id, names)| {
                    names
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|n| n.as_str().map(|n| (client_id.clone(), n.to_string())))
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .unwrap_or_default();

    let (by_internal_id, by_client_id) = client_id_maps(client).await?;
    let current = client.get_role_composites(role_id).await?;
    let key_of = |r: &crate::models::RoleRepresentation| {
        if r.client_role {
            let client_id = r
                .container_id
                .as_ref()
                .and_then(|internal_id| by_internal_id.get(internal_id))
                .cloned()
                .unwrap_or_default();
            (Some(client_id), r.name.clone())
        } else {
            (None, r.name.clone())
        }
    };

    let remove: Vec<_> = current
        .iter()
        .filter(|r| match key_of(r) {
            (None, name) => !wanted_realm.contains(&name),
            (Some(client_id), name) => !wanted_client.contains(&(client_id, name)),
        })
        .cloned()
        .collect();
    let have: HashSet<(Option<String>, String)> = current.iter().map(key_of).collect();
    let mut add = Vec::new();
    for name in &wanted_realm {
        if !have.contains(&(None, name.clone())) {
            add.push(client.get_realm_role(name).await.with_context(|| {
                format!("Composite realm role '{}' of {} not found", name, label)
            })?);
        }
    }
    for (client_id, name) in &wanted_client {
        if !have.contains(&(Some(client_id.clone()), name.clone())) {
            let internal_id = by_client_id
                .get(client_id)
                .with_context(|| format!("Client '{}' not found", client_id))?;
            add.push(
                client
                    .get_client_role(internal_id, name)
                    .await
                    .with_context(|| {
                        format!(
                            "Composite role '{}' of client '{}' for {} not found",
                            name, client_id, label
                        )
                    })?,
            );
        }
    }
    if !remove.is_empty() {
        client
            .change_role_composites(role_id, &remove, false)
            .await
            .with_context(|| format!("Failed to remove composites from {}", label))?;
        crate::utils::ui::log_line(format!(
            "    {} Removed composites {} from {}",
            SUCCESS_UPDATE,
            names_of(&remove).join(", "),
            label
        ));
    }
    if !add.is_empty() {
        client
            .change_role_composites(role_id, &add, true)
            .await
            .with_context(|| format!("Failed to add composites to {}", label))?;
        crate::utils::ui::log_line(format!(
            "    {} Added composites {} to {}",
            SUCCESS_CREATE,
            names_of(&add).join(", "),
            label
        ));
    }
    Ok(())
}
