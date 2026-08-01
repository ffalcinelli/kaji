#![allow(clippy::collapsible_if)]
use crate::client::KeycloakClient;
use crate::models::{ComponentRepresentation, KeycloakResource};
use crate::utils::secrets::{SecretResolver, substitute_secrets};
use anyhow::{Context, Result};
use std::collections::{HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

pub type ComponentKey = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub fn build_component_indices(
    existing_components: impl IntoIterator<Item = ComponentRepresentation>,
) -> (
    HashMap<String, ComponentRepresentation>,
    HashMap<ComponentKey, ComponentRepresentation>,
) {
    let mut by_identity: HashMap<String, ComponentRepresentation> = HashMap::new();
    let mut by_details: HashMap<ComponentKey, ComponentRepresentation> = HashMap::new();

    for c in existing_components {
        if let Some(id) = c.get_identity() {
            by_identity.insert(id, c.clone());
        }
        let key = (
            c.name.clone(),
            c.sub_type.clone(),
            c.provider_id.clone(),
            c.parent_id.clone(),
        );
        by_details.insert(key, c);
    }
    (by_identity, by_details)
}

use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};

#[allow(clippy::too_many_arguments)]
pub async fn process_component_file(
    path: PathBuf,
    client: KeycloakClient,
    by_identity: Arc<HashMap<String, ComponentRepresentation>>,
    by_details: Arc<HashMap<ComponentKey, ComponentRepresentation>>,
    secrets_path: Arc<PathBuf>,
    resolver: Arc<dyn SecretResolver>,
    realm_name: String,
    profile: Option<String>,
    ui: Arc<dyn Ui>,
    yes: bool,
) -> Result<()> {
    let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
    let local_val_before_sub = val.clone();
    substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
    let mut component_rep: ComponentRepresentation = serde_json::from_value(val)?;

    let existing = if let Some(identity) = component_rep.get_identity() {
        by_identity.get(&identity).or_else(|| {
            let key = (
                component_rep.name.clone(),
                component_rep.sub_type.clone(),
                component_rep.provider_id.clone(),
                component_rep.parent_id.clone(),
            );
            by_details.get(&key)
        })
    } else {
        let key = (
            component_rep.name.clone(),
            component_rep.sub_type.clone(),
            component_rep.provider_id.clone(),
            component_rep.parent_id.clone(),
        );
        by_details.get(&key)
    };

    let id_opt = existing.and_then(|e| e.id.as_ref());
    crate::handle_upsert! {
        client: client,
        realm: realm_name,
        rep: component_rep,
        id_opt: id_opt,
        id_field: id,
        resource_name: "component",
        update_call: |id, rep| client.update_component(id, rep),
        create_call: |rep| client.create_component(rep)
    }

    let mut final_id = None;
    if let Some(id) = id_opt {
        final_id = Some(id.clone());
    } else {
        if let Ok(fresh_comps) = client.get_components().await {
            if let Some(fresh) = fresh_comps.into_iter().find(|c| {
                c.name == component_rep.name
                    && c.parent_id == component_rep.parent_id
                    && c.provider_id == component_rep.provider_id
            }) {
                if let Some(id) = fresh.id {
                    final_id = Some(id);
                }
            }
        }
    }

    if let Some(id) = final_id {
        if let Ok(enriched) = client.get_resource::<ComponentRepresentation>(&id).await {
            crate::apply::generic::check_and_update_enrichment(
                &client,
                &path,
                &local_val_before_sub,
                &enriched,
                &realm_name,
                &secrets_path,
                &*ui,
                yes,
            )
            .await?;
        }
    }

    Ok(())
}

use crate::utils::ui::Ui;

#[allow(clippy::collapsible_if)]
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
        review: _,
        ui,
        yes,
        prune: _,
    } = ctx;

    let components_dir = workspace_dir.join(dir_name);
    if !async_fs::try_exists(&components_dir).await? {
        return Ok(());
    }

    let existing_components = client
        .get_components()
        .await
        .with_context(|| format!("Failed to get components/keys for realm '{}'", realm_name))?;

    let (by_identity, by_details) = build_component_indices(existing_components);
    let by_identity = Arc::new(by_identity);
    let by_details = Arc::new(by_details);

    let mut entries = async_fs::read_dir(&components_dir).await?;
    let mut set = JoinSet::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if let Some(plan) = &*planned_files
            && !plan.contains(&path)
        {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        // Skip overlay files themselves
        if is_overlay_file(&path, profile.as_deref()) {
            continue;
        }

        let client = client.clone();
        let by_identity = Arc::clone(&by_identity);
        let by_details = Arc::clone(&by_details);
        let resolver = Arc::clone(&resolver);
        let realm_name = realm_name.to_string();
        let profile = profile.clone();
        let secrets_path = Arc::clone(&secrets_path);
        let ui = Arc::clone(&ui);
        set.spawn(async move {
            process_component_file(
                path,
                client,
                by_identity,
                by_details,
                secrets_path,
                resolver,
                realm_name,
                profile,
                ui,
                yes,
            )
            .await
        });
    }
    crate::utils::join_all_tasks(set, None).await?;
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

    #[test]
    fn test_build_component_indices() {
        let comps = vec![
            ComponentRepresentation {
                id: Some("id1".to_string()),
                name: Some("comp1".to_string()),
                provider_id: Some("prov1".to_string()),
                provider_type: Some("type1".to_string()),
                sub_type: Some("sub1".to_string()),
                parent_id: Some("parent1".to_string()),
                config: None,
                extra: Default::default(),
            },
            ComponentRepresentation {
                id: None,
                name: Some("comp2".to_string()),
                provider_id: Some("prov2".to_string()),
                provider_type: Some("type2".to_string()),
                sub_type: None,
                parent_id: Some("parent2".to_string()),
                config: None,
                extra: Default::default(),
            },
        ];

        let (by_identity, by_details) = build_component_indices(comps);

        // get_identity for ComponentRepresentation returns `id.or_else(|| name)`.
        // First component has id "id1", so it uses "id1".
        // Second component has no id, so it uses "comp2".
        assert_eq!(by_identity.len(), 2);
        assert!(by_identity.contains_key("id1"));
        assert!(by_identity.contains_key("comp2"));

        assert_eq!(by_details.len(), 2);
        let key1 = (
            Some("comp1".to_string()),
            Some("sub1".to_string()),
            Some("prov1".to_string()),
            Some("parent1".to_string()),
        );
        let key2 = (
            Some("comp2".to_string()),
            None,
            Some("prov2".to_string()),
            Some("parent2".to_string()),
        );
        assert!(by_details.contains_key(&key1));
        assert!(by_details.contains_key(&key2));
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
