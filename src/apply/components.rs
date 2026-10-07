#![allow(clippy::collapsible_if)]
//! Reconciliation of components (`components/` and `keys/` directories).
//!
//! Component IDs and `parentId`s are environment specific, so components are matched by a
//! portable key: `(providerType, subType, name, parent)`, where `parent` is empty for realm-level
//! components and `providerType/name` of the parent component otherwise (e.g. LDAP mappers).
//! Parent IDs found in local files are resolved through the local files themselves, so a
//! workspace exported from one environment applies to another. When writing, `parentId` is
//! rewritten to the target realm ID or the target parent component ID, and realm-level
//! components are applied before their children.

use crate::client::KeycloakClient;
use crate::models::{ComponentRepresentation, KeycloakResource};
use crate::utils::secrets::{SecretResolver, substitute_secrets};
use crate::utils::ui::Ui;
use crate::utils::yaml::{is_overlay_file, is_yaml_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

/// Directories holding component files.
pub const COMPONENT_DIRS: &[&str] = &["components", "keys"];

fn label(c: &ComponentRepresentation) -> String {
    format!(
        "{}/{}",
        c.provider_type.as_deref().unwrap_or_default(),
        c.name.as_deref().unwrap_or_default()
    )
}

/// Matches local components to remote ones and resolves parent IDs across environments.
pub struct ComponentResolver {
    realm_id: Option<String>,
    remote: Vec<ComponentRepresentation>,
    /// Local component ID -> `providerType/name`, from every local component file.
    local_parents: HashMap<String, String>,
}

impl ComponentResolver {
    /// Builds a resolver from the remote components and every local component file of the realm.
    ///
    /// # Errors
    /// Returns an error if a local component file cannot be read or parsed.
    pub async fn load(
        client: &KeycloakClient,
        workspace_dir: &Path,
        profile: Option<&str>,
        remote: Vec<ComponentRepresentation>,
    ) -> Result<Self> {
        let realm_id = client
            .get_realm()
            .await
            .ok()
            .and_then(|r| r.extra.get("id").and_then(|v| v.as_str()).map(String::from));
        let mut local_parents = HashMap::new();
        for (_, local) in read_local_components(workspace_dir, profile).await? {
            if let Some(id) = &local.id {
                local_parents.insert(id.clone(), label(&local));
            }
        }
        Ok(Self {
            realm_id,
            remote,
            local_parents,
        })
    }

    fn remote_parent(&self, parent_id: Option<&str>) -> String {
        match parent_id {
            None => String::new(),
            Some(p) if Some(p) == self.realm_id.as_deref() => String::new(),
            Some(p) => self
                .remote
                .iter()
                .find(|c| c.id.as_deref() == Some(p))
                .map(label)
                .unwrap_or_default(),
        }
    }

    fn local_parent(&self, parent_id: Option<&str>) -> String {
        match parent_id {
            None => String::new(),
            Some(p) if Some(p) == self.realm_id.as_deref() => String::new(),
            Some(p) => self.local_parents.get(p).cloned().unwrap_or_else(|| {
                // Same-environment parent, or a realm ID from another environment.
                self.remote_parent(Some(p))
            }),
        }
    }

    /// Returns true if the local component belongs directly to the realm.
    pub fn is_realm_level(&self, local: &ComponentRepresentation) -> bool {
        self.local_parent(local.parent_id.as_deref()).is_empty()
    }

    /// Finds the remote component matching a local one by portable key.
    pub fn find_remote(&self, local: &ComponentRepresentation) -> Option<&ComponentRepresentation> {
        let parent = self.local_parent(local.parent_id.as_deref());
        self.remote.iter().find(|r| {
            r.name == local.name
                && (local.provider_type.is_none() || r.provider_type == local.provider_type)
                && (local.sub_type.is_none() || r.sub_type == local.sub_type)
                && self.remote_parent(r.parent_id.as_deref()) == parent
        })
    }

    /// Returns the `parentId` to send to the target server for a local component.
    ///
    /// # Errors
    /// Returns an error if the parent component does not exist on the server.
    pub fn target_parent_id(&self, local: &ComponentRepresentation) -> Result<Option<String>> {
        let parent = self.local_parent(local.parent_id.as_deref());
        if parent.is_empty() {
            return Ok(self.realm_id.clone().or_else(|| local.parent_id.clone()));
        }
        self.remote
            .iter()
            .find(|c| label(c) == parent && self.remote_parent(c.parent_id.as_deref()).is_empty())
            .and_then(|c| c.id.clone())
            .map(Some)
            .with_context(|| {
                format!(
                    "Parent component '{}' of component '{}' does not exist",
                    parent,
                    local.get_name()
                )
            })
    }
}

/// Lists the component files of a directory (overlays excluded).
async fn component_files(dir: &Path, profile: Option<&str>) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut entries = match async_fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(e) => return Err(e.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if is_yaml_file(&path) && !is_overlay_file(&path, profile) {
            files.push(path);
        }
    }
    Ok(files)
}

/// Reads every local component (without secret substitution, for identity purposes only).
async fn read_local_components(
    workspace_dir: &Path,
    profile: Option<&str>,
) -> Result<Vec<(PathBuf, ComponentRepresentation)>> {
    let mut components = Vec::new();
    for dir in COMPONENT_DIRS {
        for path in component_files(&workspace_dir.join(dir), profile).await? {
            let val = load_yaml_with_overlay(&path, profile).await?;
            let rep: ComponentRepresentation = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;
            components.push((path, rep));
        }
    }
    Ok(components)
}

#[allow(clippy::too_many_arguments)]
pub async fn process_component_file(
    path: PathBuf,
    client: KeycloakClient,
    components: Arc<ComponentResolver>,
    secrets_path: Arc<PathBuf>,
    resolver: Arc<dyn SecretResolver>,
    realm_name: String,
    profile: Option<String>,
    review: bool,
    ui: Arc<dyn Ui>,
    yes: bool,
    prompt_mutex: Arc<tokio::sync::Mutex<()>>,
) -> Result<()> {
    let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
    let local_val_before_sub = val.clone();
    substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
    let local_val_resolved = val.clone();
    let mut component_rep: ComponentRepresentation = serde_json::from_value(val)?;

    let id_opt = components
        .find_remote(&component_rep)
        .and_then(|e| e.id.clone());

    if review {
        let action = if id_opt.is_some() { "update" } else { "create" };
        let proceed = {
            let _lock = prompt_mutex.lock().await;
            ui.confirm(
                &format!(
                    "Do you want to {} component '{}'?",
                    action,
                    component_rep.get_name()
                ),
                true,
            )?
        };
        if !proceed {
            return Ok(());
        }
    }

    component_rep.parent_id = components.target_parent_id(&component_rep)?;
    let id_ref = id_opt.as_ref();
    crate::handle_upsert! {
        client: client,
        realm: realm_name,
        rep: component_rep,
        id_opt: id_ref,
        id_field: id,
        resource_name: "component",
        update_call: |id, rep| client.update_component(id, rep),
        create_call: |rep| client.create_resource(rep)
    }

    let final_id = match id_opt {
        Some(id) => Some(id),
        None => client.get_components().await.ok().and_then(|fresh| {
            fresh
                .into_iter()
                .find(|c| {
                    c.name == component_rep.name
                        && c.parent_id == component_rep.parent_id
                        && c.provider_id == component_rep.provider_id
                })
                .and_then(|c| c.id)
        }),
    };

    if let Some(id) = final_id {
        if let Ok(enriched) = client.get_resource::<ComponentRepresentation>(&id).await {
            crate::apply::generic::check_and_update_enrichment(
                &path,
                profile.as_deref(),
                crate::apply::generic::LocalSource {
                    before_sub: &local_val_before_sub,
                    resolved: &local_val_resolved,
                },
                &enriched,
                &realm_name,
                &secrets_path,
                &*ui,
                yes,
                prompt_mutex,
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn apply_components_or_keys(
    ctx: crate::apply::ApplyContext<'_>,
    dir_name: &str,
) -> Result<()> {
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
        prune: _,
        prompt_mutex,
    } = ctx;

    let mut files = component_files(&workspace_dir.join(dir_name), profile.as_deref()).await?;
    if let Some(plan) = &*planned_files {
        files.retain(|path| plan.contains(path));
    }
    if files.is_empty() {
        return Ok(());
    }

    // Realm-level components first (children such as LDAP mappers need their parent's ID).
    let mut tiers: [Vec<PathBuf>; 2] = [Vec::new(), Vec::new()];
    let initial = Arc::new(
        ComponentResolver::load(
            client,
            &workspace_dir,
            profile.as_deref(),
            client.get_components().await.with_context(|| {
                format!("Failed to get components/keys for realm '{}'", realm_name)
            })?,
        )
        .await?,
    );
    for path in files {
        let raw: ComponentRepresentation =
            serde_json::from_value(load_yaml_with_overlay(&path, profile.as_deref()).await?)
                .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;
        let tier = usize::from(!initial.is_realm_level(&raw));
        tiers[tier].push(path);
    }

    let mut components = initial;
    for (index, tier) in tiers.into_iter().enumerate() {
        if tier.is_empty() {
            continue;
        }
        if index > 0 {
            components = Arc::new(
                ComponentResolver::load(
                    client,
                    &workspace_dir,
                    profile.as_deref(),
                    client.get_components().await?,
                )
                .await?,
            );
        }
        let mut set = JoinSet::new();
        for path in tier {
            let client = client.clone();
            let components = Arc::clone(&components);
            let resolver = Arc::clone(&resolver);
            let realm_name = realm_name.to_string();
            let profile = profile.clone();
            let secrets_path = Arc::clone(&secrets_path);
            let ui = Arc::clone(&ui);
            let prompt_mutex = Arc::clone(&prompt_mutex);
            set.spawn(async move {
                process_component_file(
                    path,
                    client,
                    components,
                    secrets_path,
                    resolver,
                    realm_name,
                    profile,
                    review,
                    ui,
                    yes,
                    prompt_mutex,
                )
                .await
            });
        }
        crate::utils::join_all_tasks(set, None).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::apply::test_utils::start_mock_server;

    use super::*;
    use crate::client::KeycloakClient;
    use crate::utils::secrets::EnvResolver;

    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn comp(v: serde_json::Value) -> ComponentRepresentation {
        serde_json::from_value(v).unwrap()
    }

    fn resolver(
        remote: Vec<ComponentRepresentation>,
        local: &[(&str, &str, &str)],
    ) -> ComponentResolver {
        ComponentResolver {
            realm_id: Some("realm-uuid".to_string()),
            remote,
            local_parents: local
                .iter()
                .map(|(id, t, n)| (id.to_string(), format!("{}/{}", t, n)))
                .collect(),
        }
    }

    #[test]
    fn test_component_keys_are_portable() {
        let remote = vec![
            comp(
                serde_json::json!({"id": "prod-ldap", "name": "ldap", "providerType": "org.keycloak.storage.UserStorageProvider", "parentId": "realm-uuid"}),
            ),
            comp(
                serde_json::json!({"id": "prod-mapper", "name": "username", "providerType": "org.keycloak.storage.ldap.mappers.LDAPStorageMapper", "parentId": "prod-ldap"}),
            ),
            comp(
                serde_json::json!({"id": "prod-rsa", "name": "rsa-generated", "providerType": "org.keycloak.keys.KeyProvider", "subType": null, "parentId": "realm-uuid"}),
            ),
        ];
        // Local files exported from another environment: different ids and realm id.
        let r = resolver(
            remote,
            &[(
                "dev-ldap",
                "org.keycloak.storage.UserStorageProvider",
                "ldap",
            )],
        );
        let ldap = comp(
            serde_json::json!({"id": "dev-ldap", "name": "ldap", "providerType": "org.keycloak.storage.UserStorageProvider", "parentId": "dev-realm-uuid"}),
        );
        let mapper = comp(
            serde_json::json!({"id": "dev-mapper", "name": "username", "providerType": "org.keycloak.storage.ldap.mappers.LDAPStorageMapper", "parentId": "dev-ldap"}),
        );
        let rsa = comp(
            serde_json::json!({"name": "rsa-generated", "providerType": "org.keycloak.keys.KeyProvider"}),
        );

        assert_eq!(
            r.find_remote(&ldap).and_then(|c| c.id.as_deref()),
            Some("prod-ldap")
        );
        assert_eq!(
            r.find_remote(&mapper).and_then(|c| c.id.as_deref()),
            Some("prod-mapper")
        );
        assert_eq!(
            r.find_remote(&rsa).and_then(|c| c.id.as_deref()),
            Some("prod-rsa")
        );
        assert!(r.is_realm_level(&ldap));
        assert!(!r.is_realm_level(&mapper));

        assert_eq!(
            r.target_parent_id(&ldap).unwrap().as_deref(),
            Some("realm-uuid")
        );
        assert_eq!(
            r.target_parent_id(&mapper).unwrap().as_deref(),
            Some("prod-ldap")
        );
    }

    #[test]
    fn test_missing_parent_component_is_an_error() {
        let r = resolver(
            vec![],
            &[(
                "dev-ldap",
                "org.keycloak.storage.UserStorageProvider",
                "ldap",
            )],
        );
        let mapper = comp(
            serde_json::json!({"name": "username", "providerType": "m", "parentId": "dev-ldap"}),
        );
        assert!(r.find_remote(&mapper).is_none());
        let err = r.target_parent_id(&mapper).unwrap_err();
        assert!(err.to_string().contains("ldap"), "{err}");
    }

    #[tokio::test]
    async fn test_apply_components_error_paths() -> Result<()> {
        let (server_url, call_count) = start_mock_server().await?;
        let mut client = KeycloakClient::new(server_url);
        client.set_target_realm("test".to_string());
        client.set_token("mock_token".to_string());

        let temp = tempdir()?;
        let components_dir = temp.path().join("components");
        fs::create_dir(&components_dir)?;
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let secrets_path = Arc::new(temp.path().join(".secrets"));
        let ui = Arc::new(crate::utils::ui::MockUi {
            inputs: std::sync::Mutex::new(Vec::new()),
            confirms: std::sync::Mutex::new(Vec::new()),
            selects: std::sync::Mutex::new(Vec::new()),
            passwords: std::sync::Mutex::new(Vec::new()),
        });

        // 1. Test update failure
        call_count.store(0, std::sync::atomic::Ordering::SeqCst);
        let comp_existing = components_dir.join("existing.yaml");
        fs::write(comp_existing, "name: Existing Component\nid: existing-id")?;

        let res = apply_components_or_keys(
            crate::apply::ApplyContext {
                client: &client,
                workspace_dir: temp.path().to_path_buf(),
                secrets_path: secrets_path.clone(),
                resolver: Arc::clone(&resolver) as Arc<dyn SecretResolver>,
                planned_files: Arc::new(None),
                realm_name: "test",
                profile: None,
                review: false,
                ui: ui.clone(),
                yes: true,
                prune: false,
                prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
            },
            "components",
        )
        .await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Failed to update component")
        );

        fs::remove_file(components_dir.join("existing.yaml"))?;

        // 2. Test create failure
        call_count.store(0, std::sync::atomic::Ordering::SeqCst);
        let comp_new = components_dir.join("new.yaml");
        fs::write(comp_new, "name: New Component\nproviderId: new-provider")?;

        let res = apply_components_or_keys(
            crate::apply::ApplyContext {
                client: &client,
                workspace_dir: temp.path().to_path_buf(),
                secrets_path: secrets_path.clone(),
                resolver: Arc::clone(&resolver) as Arc<dyn SecretResolver>,
                planned_files: Arc::new(None),
                realm_name: "test",
                profile: None,
                review: false,
                ui: ui.clone(),
                yes: true,
                prune: false,
                prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
            },
            "components",
        )
        .await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Failed to create component")
        );

        Ok(())
    }
}
