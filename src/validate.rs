use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, ComponentRepresentation, GroupRepresentation,
    IdentityProviderRepresentation, RealmRepresentation, RequiredActionProviderRepresentation,
    RoleRepresentation, UserRepresentation,
};
use crate::utils::ui::{CHECK, SEARCH, SUCCESS, WARN};
use anyhow::{Context, Result};
use console::style;
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::task::JoinSet;

async fn read_yaml_files<T: DeserializeOwned + Send + 'static>(
    dir: &Path,
    file_type: &str,
) -> Result<Vec<(PathBuf, T)>> {
    let mut results = Vec::new();
    if fs::try_exists(dir).await? {
        let mut entries = fs::read_dir(dir).await?;
        let mut join_set = JoinSet::new();
        let file_type_str = file_type.to_string();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "yaml") {
                let ft = file_type_str.clone();
                join_set.spawn(async move {
                    let content = fs::read_to_string(&path)
                        .await
                        .with_context(|| format!("Failed to read {} file {:?}", ft, path))?;
                    let item: T = serde_yaml::from_str(&content)
                        .with_context(|| format!("Failed to parse {} file {:?}", ft, path))?;
                    Ok::<(PathBuf, T), anyhow::Error>((path, item))
                });
            }
        }

        while let Some(res) = join_set.join_next().await {
            results.push(res??);
        }
    }
    Ok(results)
}

/// Validates the structure and syntax of local YAML configuration files.
///
/// # Errors
/// Returns an error if validation fails or a file cannot be parsed.
pub async fn run(workspace_dir: PathBuf, realms_to_validate: &[String]) -> Result<()> {
    if !fs::try_exists(&workspace_dir).await? {
        return Err(anyhow::anyhow!(
            "Hint: Create the workspace directory first or use `kaji init`."
        )
        .context(format!(
            "Input directory {:?} does not exist",
            workspace_dir
        )));
    }

    let realms = if realms_to_validate.is_empty() {
        let mut dirs = Vec::new();
        let mut entries = fs::read_dir(&workspace_dir).await?;
        let mut join_set = JoinSet::new();
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
        realms_to_validate.to_vec()
    };

    if realms.is_empty() {
        eprintln!(
            "{} {}",
            WARN,
            style(format!(
                "No realms found to validate in {:?}",
                workspace_dir
            ))
            .yellow()
        );
        return Ok(());
    }

    for realm_name in &realms {
        eprintln!(
            "\n{} {}",
            SEARCH,
            style(format!("Validating realm: {}", realm_name))
                .cyan()
                .bold()
        );
        let realm_dir = workspace_dir.join(realm_name);
        validate_realm(realm_dir).await?;
        eprintln!(
            "  {} {}",
            SUCCESS,
            style(format!("Successfully validated realm: {}", realm_name))
                .green()
                .bold()
        );
    }
    Ok(())
}

async fn validate_realm_config(workspace_dir: &Path) -> Result<()> {
    let realm_path = workspace_dir.join("realm.yaml");
    let realm_content = fs::read_to_string(&realm_path).await.with_context(|| {
        format!(
            "realm.yaml not found or failed to read in {:?}",
            workspace_dir
        )
    })?;
    let realm: RealmRepresentation =
        serde_yaml::from_str(&realm_content).context("Failed to parse realm.yaml")?;

    if realm.realm.is_empty() {
        anyhow::bail!("Realm name is empty in realm.yaml");
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Realm configuration is valid:").dim(),
        style(&realm.realm).green()
    );
    Ok(())
}

fn validate_roles(roles: &[(PathBuf, RoleRepresentation)]) -> Result<()> {
    let mut role_names = HashSet::new();
    for (path, role) in roles {
        if role.name.is_empty() {
            anyhow::bail!("Role name is empty in {:?}", path);
        }
        if role_names.contains(&role.name) {
            anyhow::bail!("Duplicate role name: {}", role.name);
        }
        role_names.insert(role.name.clone());
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated roles:").dim(),
        style(roles.len()).green()
    );
    Ok(())
}

fn validate_clients(clients: &[(PathBuf, ClientRepresentation)]) -> Result<()> {
    for (path, client) in clients {
        if client.client_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Client ID is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated clients:").dim(),
        style(clients.len()).green()
    );
    Ok(())
}

fn validate_idps(idps: &[(PathBuf, IdentityProviderRepresentation)]) -> Result<()> {
    for (path, idp) in idps {
        if idp.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Identity Provider alias is missing or empty in {:?}", path);
        }
        if idp.provider_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Identity Provider providerId is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated Identity Providers:").dim(),
        style(idps.len()).green()
    );
    Ok(())
}

fn validate_client_scopes(scopes: &[(PathBuf, ClientScopeRepresentation)]) -> Result<()> {
    for (path, scope) in scopes {
        if scope.name.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Client Scope name is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated client scopes:").dim(),
        style(scopes.len()).green()
    );
    Ok(())
}

fn validate_groups(groups: &[(PathBuf, GroupRepresentation)]) -> Result<()> {
    for (path, group) in groups {
        if group.name.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Group name is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated groups:").dim(),
        style(groups.len()).green()
    );
    Ok(())
}

fn validate_users(users: &[(PathBuf, UserRepresentation)]) -> Result<()> {
    for (path, user) in users {
        if user.username.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("User username is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated users:").dim(),
        style(users.len()).green()
    );
    Ok(())
}

fn validate_authentication_flows(
    flows: &[(PathBuf, AuthenticationFlowRepresentation)],
) -> Result<()> {
    for (path, flow) in flows {
        if flow.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Authentication Flow alias is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated authentication flows:").dim(),
        style(flows.len()).green()
    );
    Ok(())
}

fn validate_required_actions(
    actions: &[(PathBuf, RequiredActionProviderRepresentation)],
) -> Result<()> {
    for (path, action) in actions {
        if action.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Required Action alias is missing or empty in {:?}", path);
        }
        if action.provider_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Required Action providerId is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated required actions:").dim(),
        style(actions.len()).green()
    );
    Ok(())
}

fn validate_authenticator_configs(
    configs: &[(PathBuf, AuthenticatorConfigRepresentation)],
) -> Result<()> {
    for (path, config) in configs {
        if config.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Authenticator Config alias is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated authenticator configs:").dim(),
        style(configs.len()).green()
    );
    Ok(())
}

async fn validate_realm(workspace_dir: PathBuf) -> Result<()> {
    // 1. Validate Realm
    validate_realm_config(&workspace_dir).await?;

    // Read all resource directories concurrently
    let roles_dir = workspace_dir.join("roles");
    let clients_dir = workspace_dir.join("clients");
    let idps_dir = workspace_dir.join("identity-providers");
    let scopes_dir = workspace_dir.join("client-scopes");
    let groups_dir = workspace_dir.join("groups");
    let users_dir = workspace_dir.join("users");
    let flows_dir = workspace_dir.join("authentication-flows");
    let actions_dir = workspace_dir.join("required-actions");
    let configs_dir = workspace_dir.join("authenticator-configs");

    let (roles, clients, idps, scopes, groups, users, flows, actions, configs) = tokio::try_join!(
        read_yaml_files::<RoleRepresentation>(&roles_dir, "role"),
        read_yaml_files::<ClientRepresentation>(&clients_dir, "client"),
        read_yaml_files::<IdentityProviderRepresentation>(&idps_dir, "idp"),
        read_yaml_files::<ClientScopeRepresentation>(&scopes_dir, "client-scope"),
        read_yaml_files::<GroupRepresentation>(&groups_dir, "group"),
        read_yaml_files::<UserRepresentation>(&users_dir, "user"),
        read_yaml_files::<AuthenticationFlowRepresentation>(&flows_dir, "authentication-flow"),
        read_yaml_files::<RequiredActionProviderRepresentation>(&actions_dir, "required-action"),
        read_yaml_files::<AuthenticatorConfigRepresentation>(&configs_dir, "authenticator-config"),
    )?;

    // Validate resources
    validate_roles(&roles)?;
    validate_clients(&clients)?;
    validate_idps(&idps)?;
    validate_client_scopes(&scopes)?;
    validate_groups(&groups)?;
    validate_users(&users)?;
    validate_authentication_flows(&flows)?;
    validate_required_actions(&actions)?;
    validate_authenticator_configs(&configs)?;

    // Validate Components and Keys
    tokio::try_join!(
        validate_components_in_dir(&workspace_dir, "components"),
        validate_components_in_dir(&workspace_dir, "keys")
    )?;

    Ok(())
}

async fn validate_components_in_dir(workspace_dir: &Path, dir_name: &str) -> Result<()> {
    let dir = workspace_dir.join(dir_name);
    if fs::try_exists(&dir).await? {
        let components: Vec<(PathBuf, ComponentRepresentation)> =
            read_yaml_files(&dir, dir_name).await?;
        for (path, component) in &components {
            if let Some(name) = &component.name
                && name.is_empty()
            {
                anyhow::bail!("Component name is empty in {:?}", path);
            }
            if component
                .provider_id
                .as_deref()
                .unwrap_or_default()
                .is_empty()
            {
                anyhow::bail!("Component providerId is missing or empty in {:?}", path);
            }
        }
        eprintln!(
            "  {} {} {}",
            CHECK,
            style(format!("Validated {}:", dir_name)).dim(),
            style(components.len()).green()
        );
    }
    Ok(())
}
