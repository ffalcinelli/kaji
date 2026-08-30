use crate::client::KeycloakClient;
use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, ComponentRepresentation, GroupRepresentation,
    IdentityProviderRepresentation, KeycloakResource, RequiredActionProviderRepresentation,
    ResourceMeta, RoleRepresentation, UserRepresentation,
};
use crate::utils::to_sorted_yaml_with_secrets;
use crate::utils::ui::{CHECK, SEARCH, SUCCESS, WARN};
use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, theme::ColorfulTheme};
use sanitize_filename::sanitize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;

/// Exports/inspects remote Keycloak server configuration into local workspace files.
///
/// # Errors
/// Returns an error if Keycloak server query or file writing fails.
pub async fn run(
    client: &KeycloakClient,
    workspace_dir: PathBuf,
    realms_to_inspect: &[String],
    yes: bool,
) -> Result<()> {
    if !fs::try_exists(&workspace_dir)
        .await
        .context("Failed to check output directory")?
    {
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create output directory")?;
    }

    let realms = if realms_to_inspect.is_empty() {
        let all_realms = client
            .get_realms()
            .await
            .context("Failed to fetch realms")?;
        all_realms.into_iter().map(|r| r.realm).collect()
    } else {
        realms_to_inspect.to_vec()
    };

    let all_secrets = Arc::new(Mutex::new(BTreeMap::new()));
    let prompt_mutex = Arc::new(Mutex::new(()));

    let mut set = tokio::task::JoinSet::new();

    for realm_name in realms {
        let mut realm_client = client.clone();
        realm_client.set_target_realm(realm_name.clone());
        let realm_dir = workspace_dir.join(&realm_name);
        let all_secrets = Arc::clone(&all_secrets);
        let prompt_mutex = Arc::clone(&prompt_mutex);
        let realm_name_owned = realm_name.clone();

        set.spawn(async move {
            {
                let _lock = prompt_mutex.lock().await;
                eprintln!(
                    "\n{} {}",
                    SEARCH,
                    style(format!("Inspecting realm: {}", realm_name_owned))
                        .cyan()
                        .bold()
                );
            }
            inspect_realm(
                &realm_client,
                &realm_name_owned,
                realm_dir,
                all_secrets,
                yes,
                prompt_mutex,
            )
            .await
        });
    }

    crate::utils::join_all_tasks(set, Some("Task panicked")).await?;

    let secrets_lock = all_secrets.lock().await;
    if !secrets_lock.is_empty() {
        let env_path = workspace_dir.join(".secrets");
        let mut env_content = String::new();
        for (key, value) in secrets_lock.iter() {
            env_content.push_str(&format!("{}={}\n", key, value));
        }

        let mut existing_env = fs::read_to_string(&env_path).await.unwrap_or_default();
        if !existing_env.ends_with('\n') && !existing_env.is_empty() {
            existing_env.push('\n');
        }

        let new_content = format!("{}{}", existing_env, env_content);
        write_if_changed_with_mutex(&env_path, &new_content, yes, Arc::clone(&prompt_mutex))
            .await?;
        eprintln!(
            "{} {}",
            CHECK,
            style("Exported secrets to .secrets").green()
        );
    }

    Ok(())
}

async fn write_if_changed_with_mutex(
    path: &Path,
    content: &str,
    yes: bool,
    prompt_mutex: Arc<Mutex<()>>,
) -> Result<()> {
    if let Ok(existing) = fs::read_to_string(path).await {
        if existing == content {
            return Ok(());
        }

        if !yes {
            let _lock = prompt_mutex.lock().await;
            if !Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "File {:?} already exists with different content. Overwrite?",
                    path
                ))
                .default(false)
                .interact()?
            {
                eprintln!(
                    "{} {}",
                    WARN,
                    style(format!("Skipping {:?}", path)).yellow()
                );
                return Ok(());
            }
        }
    }

    crate::utils::write_secure(path, content).await?;

    Ok(())
}

async fn inspect_resources<T>(
    client: &KeycloakClient,
    realm_name: &str,
    target_dir: Arc<PathBuf>,
    all_secrets: Arc<Mutex<BTreeMap<String, String>>>,
    yes: bool,
    prompt_mutex: Arc<Mutex<()>>,
) -> Result<()>
where
    T: KeycloakResource
        + ResourceMeta
        + crate::client::KeycloakResourceMapping
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Send
        + Sync
        + 'static,
{
    let resources = client
        .get_resources::<T>()
        .await
        .with_context(|| format!("Failed to fetch {} for realm '{}'", T::LABEL, realm_name))?;

    if !fs::try_exists(&*target_dir)
        .await
        .with_context(|| format!("Failed to check {} directory", T::LABEL))?
    {
        fs::create_dir_all(&*target_dir)
            .await
            .with_context(|| format!("Failed to create {} directory", T::LABEL))?;
    }

    let mut set = tokio::task::JoinSet::new();
    for res in resources {
        let target_dir = Arc::clone(&target_dir);
        let all_secrets = Arc::clone(&all_secrets);
        let realm_name = realm_name.to_string();
        let prompt_mutex = Arc::clone(&prompt_mutex);
        set.spawn(async move {
            let filename = format!("{}.yaml", sanitize(res.get_filename()));
            let path = target_dir.join(filename);
            let mut local_secrets = BTreeMap::new();
            let prefix = format!("realm_{}_{}", realm_name, T::SECRET_PREFIX);
            let yaml = to_sorted_yaml_with_secrets(&res, &prefix, &mut local_secrets).context(
                format!("Failed to serialize {} {}", T::LABEL, res.get_name()),
            )?;
            all_secrets.lock().await.extend(local_secrets);
            write_if_changed_with_mutex(&path, &yaml, yes, prompt_mutex).await
        });
    }
    crate::utils::join_all_tasks(set, Some("Task panicked")).await?;
    {
        let _lock = prompt_mutex.lock().await;
        eprintln!(
            "  {} {}",
            SUCCESS,
            style(format!(
                "Exported {} to {}/",
                T::LABEL,
                target_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ))
            .green()
        );
    }

    Ok(())
}

async fn inspect_realm(
    client: &KeycloakClient,
    realm_name: &str,
    workspace_dir: PathBuf,
    all_secrets: Arc<Mutex<BTreeMap<String, String>>>,
    yes: bool,
    prompt_mutex: Arc<Mutex<()>>,
) -> Result<()> {
    if !fs::try_exists(&workspace_dir)
        .await
        .context("Failed to check output directory")?
    {
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create output directory")?;
    }

    let mut set = tokio::task::JoinSet::new();
    let workspace_dir = Arc::new(workspace_dir);

    // Fetch realm configuration in parallel
    {
        let client = client.clone();
        let realm_name = realm_name.to_string();
        let workspace_dir = Arc::clone(&workspace_dir);
        let all_secrets = Arc::clone(&all_secrets);
        let prompt_mutex = Arc::clone(&prompt_mutex);
        set.spawn(async move {
            let realm = client.get_realm().await.context("Failed to fetch realm")?;
            let mut local_secrets = BTreeMap::new();
            let realm_prefix = format!("realm_{}", realm_name);
            let realm_yaml = to_sorted_yaml_with_secrets(&realm, &realm_prefix, &mut local_secrets)
                .context("Failed to serialize realm")?;
            all_secrets.lock().await.extend(local_secrets);

            let realm_path = workspace_dir.join("realm.yaml");
            write_if_changed_with_mutex(&realm_path, &realm_yaml, yes, Arc::clone(&prompt_mutex))
                .await?;
            {
                let _lock = prompt_mutex.lock().await;
                eprintln!(
                    "  {} {}",
                    SUCCESS,
                    style("Exported realm configuration to realm.yaml").green()
                );
            }
            Ok::<(), anyhow::Error>(())
        });
    }

    // Fetch resources in parallel
    spawn_inspect::<ClientRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<RoleRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<ClientScopeRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<IdentityProviderRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<GroupRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<UserRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<AuthenticationFlowRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<RequiredActionProviderRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<ComponentRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );
    spawn_inspect::<AuthenticatorConfigRepresentation>(
        &mut set,
        client,
        realm_name,
        &workspace_dir,
        &all_secrets,
        yes,
        &prompt_mutex,
    );

    crate::utils::join_all_tasks(set, Some("Task panicked")).await?;

    Ok(())
}

fn spawn_inspect<T>(
    set: &mut tokio::task::JoinSet<Result<()>>,
    client: &KeycloakClient,
    realm_name: &str,
    workspace_dir: &Arc<PathBuf>,
    all_secrets: &Arc<Mutex<BTreeMap<String, String>>>,
    yes: bool,
    prompt_mutex: &Arc<Mutex<()>>,
) where
    T: KeycloakResource
        + ResourceMeta
        + crate::client::KeycloakResourceMapping
        + serde::Serialize
        + for<'de> serde::Deserialize<'de>
        + Send
        + Sync
        + 'static,
{
    let client = client.clone();
    let realm_name = realm_name.to_string();
    let target_dir = Arc::new(workspace_dir.join(T::DIR_NAME));
    let all_secrets = Arc::clone(all_secrets);
    let prompt_mutex = Arc::clone(prompt_mutex);

    set.spawn(async move {
        inspect_resources::<T>(
            &client,
            &realm_name,
            target_dir,
            all_secrets,
            yes,
            prompt_mutex,
        )
        .await
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_write_if_changed_with_mutex() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("insecure.txt");
        let prompt_mutex = Arc::new(Mutex::new(()));

        // Write new file
        write_if_changed_with_mutex(&file_path, "test content", true, Arc::clone(&prompt_mutex))
            .await
            .unwrap();
        let content = fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "test content");

        // Overwrite file
        write_if_changed_with_mutex(&file_path, "new content", true, Arc::clone(&prompt_mutex))
            .await
            .unwrap();
        let content = fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "new content");
    }
}
