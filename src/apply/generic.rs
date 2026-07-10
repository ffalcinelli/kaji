use crate::client::KeycloakClient;
use crate::models::{KeycloakResource, ResourceMeta};
use crate::utils::secrets::{SecretResolver, substitute_secrets};
pub use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE};
use crate::utils::ui::{Ui, create_progress_bar};
use crate::utils::yaml::{is_overlay_file, load_yaml_with_overlay};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs as async_fs;
use tokio::task::JoinSet;

#[allow(clippy::too_many_arguments)]
pub async fn apply_resources<T>(
    client: &KeycloakClient,
    workspace_dir: &std::path::Path,
    resolver: Arc<dyn SecretResolver>,
    planned_files: Arc<Option<HashSet<PathBuf>>>,
    realm_name: &str,
    profile: Option<String>,
    review: bool,
    ui: Arc<dyn Ui>,
) -> Result<()>
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
    let dir_name = T::DIR_NAME;
    let resources_dir = workspace_dir.join(dir_name);
    if !async_fs::try_exists(&resources_dir).await? {
        return Ok(());
    }

    let existing_resources = client
        .get_resources::<T>()
        .await
        .with_context(|| format!("Failed to get {} for realm '{}'", T::LABEL, realm_name))?;

    let existing_map: HashMap<String, String> = existing_resources
        .into_iter()
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

    let mut entries = async_fs::read_dir(&resources_dir).await?;
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
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        // Skip overlay files themselves
        if is_overlay_file(&path, profile.as_deref()) {
            continue;
        }
        files.push(path);
    }

    if files.is_empty() {
        return Ok(());
    }

    let pb = create_progress_bar(files.len() as u64, &format!("Applying {}", T::LABEL));
    let mut set = JoinSet::new();

    for path in files {
        let client = client.clone();
        let existing_map = Arc::clone(&existing_map);
        let resolver = Arc::clone(&resolver);
        let realm_name = realm_name.to_string();
        let profile = profile.clone();
        let ui = Arc::clone(&ui);
        let pb = pb.clone();

        set.spawn(async move {
            let mut val = load_yaml_with_overlay(&path, profile.as_deref()).await?;
            substitute_secrets(&mut val, Arc::clone(&resolver)).await?;
            let mut rep: T = serde_json::from_value(val)
                .with_context(|| format!("Failed to deserialize YAML file: {:?}", path))?;

            let identity = rep.get_identity().with_context(|| {
                format!("Failed to get identity for {} in {:?}", T::LABEL, path)
            })?;

            let id_opt = existing_map.get(&identity);

            if review {
                let action = if id_opt.is_some() { "update" } else { "create" };
                let proceed = ui.confirm(
                    &format!(
                        "Do you want to {} {} '{}'?",
                        action,
                        T::LABEL,
                        rep.get_name()
                    ),
                    true,
                )?;
                if !proceed {
                    pb.inc(1);
                    return Ok::<(), anyhow::Error>(());
                }
            }

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
                pb.println(format!(
                    "  {} Updated {} {}",
                    SUCCESS_UPDATE,
                    T::LABEL,
                    rep.get_name()
                ));
            } else {
                rep.set_id(None);
                client.create_resource(&rep).await.with_context(|| {
                    format!(
                        "Failed to create {} '{}' in realm '{}'",
                        T::LABEL,
                        rep.get_name(),
                        realm_name
                    )
                })?;
                pb.println(format!(
                    "  {} Created {} {}",
                    SUCCESS_CREATE,
                    T::LABEL,
                    rep.get_name()
                ));
            }
            pb.inc(1);
            Ok::<(), anyhow::Error>(())
        });
    }

    crate::utils::join_all_tasks(set, None).await?;
    pb.finish_with_message(format!("Applied {}", T::LABEL));
    Ok(())
}
