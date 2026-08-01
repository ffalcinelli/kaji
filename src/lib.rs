#![warn(missing_docs)]
//! `kaji` is a declarative configuration management CLI tool for Keycloak.
//!
//! It brings GitOps workflows to identity infrastructure, allowing you to define,
//! validate, plan, apply, and drift-detect Keycloak configurations.

/// Staged reconciliation logic for Keycloak resources.
pub mod apply;
/// Command-line argument parser and command definitions.
pub mod args;
/// Logic to clean up configuration files in the workspace.
pub mod clean;
/// Scaffolding for interactive command-line initialization.
pub mod cli;
/// Keycloak Admin REST API HTTP client wrapper.
pub mod client;
/// Scaffolding for project configuration.
pub mod init;
/// Inspection pipeline to bootstrap local configuration files.
pub mod inspect;
/// Strongly-typed representations of Keycloak resources.
pub mod models;
/// Diff calculation and drift planning.
pub mod plan;
/// Helper utilities (secrets resolvers, YAML helpers, terminal UI).
pub mod utils;
/// Validation of local workspace YAML configuration files.
pub mod validate;

use anyhow::{Context, Result};
use args::{Cli, Commands, Config};
use client::KeycloakClient;
use console::{Emoji, style};
use std::collections::HashMap;
use std::sync::Arc;
use utils::secrets::vault::VaultResolver;
use utils::secrets::{CompositeResolver, EnvResolver, SecretResolver};

static ACTION: Emoji<'_, '_> = Emoji("🚀 ", ">> ");
static SEARCH: Emoji<'_, '_> = Emoji("🔍 ", "> ");

/// Connection profile details for a target environment.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct Profile {
    /// Keycloak server base URL.
    pub server_url: String,
    /// Client ID used for client credentials grant.
    pub client_id: Option<String>,
    /// Client secret used for client credentials grant.
    pub client_secret: Option<String>,
    /// Username for administrator credentials login.
    pub user: Option<String>,
    /// Password for administrator credentials login.
    pub password: Option<String>,
    /// Relative path to the secret variables file.
    pub secrets_file: Option<String>,
    /// Address of HashiCorp Vault server (optional).
    pub vault_addr: Option<String>,
    /// Token for HashiCorp Vault server (optional).
    pub vault_token: Option<String>,
    /// Timeout in seconds (optional).
    pub timeout: Option<u64>,
}

/// Loads a profile configuration file from the `profiles/` directory in the workspace.
///
/// # Errors
/// Returns an error if the profile file fails to load or parse as YAML.
pub async fn load_profile(workspace: &std::path::Path, name: &str) -> Result<Profile> {
    let profile_path = workspace.join("profiles").join(format!("{}.yaml", name));
    let content = tokio::fs::read_to_string(&profile_path)
        .await
        .with_context(|| format!("Failed to read profile file: {:?}", profile_path))?;
    let profile: Profile = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse profile file: {:?}", profile_path))?;
    Ok(profile)
}

/// Loads configuration settings from `kaji.toml` / `.kaji.toml` if present in current directory,
/// or from a custom configuration path.
///
/// # Errors
/// Returns an error if the config file fails to read or parse as TOML.
pub async fn load_config_file(custom_path: Option<&std::path::Path>) -> Result<Config> {
    let path = if let Some(p) = custom_path {
        Some(p.to_path_buf())
    } else {
        let cwd = std::env::current_dir()?;
        let kaji_toml = cwd.join("kaji.toml");
        if kaji_toml.exists() {
            Some(kaji_toml)
        } else {
            let dot_kaji_toml = cwd.join(".kaji.toml");
            if dot_kaji_toml.exists() {
                Some(dot_kaji_toml)
            } else {
                None
            }
        }
    };

    if let Some(config_path) = path {
        let content = tokio::fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {:?}", config_path))?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

/// Initializes a `KeycloakClient` by logging in using credentials from the CLI or active profile.
///
/// # Errors
/// Returns an error if connection URL is missing or login authentication fails.
pub async fn init_client(cli: &Cli, profile: Option<&Profile>) -> Result<KeycloakClient> {
    let server = profile
        .map(|p| p.server_url.clone())
        .or_else(|| cli.server.clone())
        .context("Hint: Try running `kaji init` to generate a default config, or pass `--server`.")
        .context("Keycloak server URL not provided (neither via --server nor --profile)")?;

    let client_id = profile
        .and_then(|p| p.client_id.clone())
        .or_else(|| cli.client_id.clone())
        .unwrap_or_else(|| "admin-cli".to_string());

    let client_secret = profile
        .and_then(|p| p.client_secret.clone())
        .or_else(|| cli.client_secret.clone());

    let user = profile
        .and_then(|p| p.user.clone())
        .or_else(|| cli.user.clone());

    let password = profile
        .and_then(|p| p.password.clone())
        .or_else(|| cli.password.clone());

    let timeout_secs = cli.timeout.unwrap_or(10);
    let mut client =
        KeycloakClient::new(server).with_timeout(std::time::Duration::from_secs(timeout_secs));
    client
        .login(
            &client_id,
            client_secret.as_deref(),
            user.as_deref(),
            password.as_deref(),
        )
        .await
        .context("Login failed")?;
    Ok(client)
}

/// Initializes secret resolvers (environment variables and/or Vault) to substitute secret tokens.
///
/// # Errors
/// Returns an error if any vault address is invalid or resolvers cannot be set up.
pub async fn init_secrets(
    cli: &Cli,
    workspace: &std::path::Path,
    profile: Option<&Profile>,
) -> Result<Arc<dyn SecretResolver>> {
    // Load secrets from profile-specific secrets file or default .secrets
    let secrets_file = profile
        .and_then(|p| p.secrets_file.as_deref())
        .unwrap_or(".secrets");

    let env_path = workspace.join(secrets_file);
    if env_path.exists() {
        dotenvy::from_path(&env_path).ok();
    }

    let mut resolvers: Vec<Box<dyn SecretResolver>> = Vec::new();

    let vault_addr = profile
        .and_then(|p| p.vault_addr.clone())
        .or_else(|| cli.vault_addr.clone());

    let vault_token = profile
        .and_then(|p| p.vault_token.clone())
        .or_else(|| cli.vault_token.clone());

    if let (Some(addr), Some(token)) = (vault_addr, vault_token) {
        resolvers.push(Box::new(VaultResolver::new(&addr, &token)?));
    }

    resolvers.push(Box::new(EnvResolver::new(
        std::env::vars().collect::<HashMap<String, String>>(),
    )));

    Ok(Arc::new(CompositeResolver::new(resolvers)))
}

async fn handle_inspect(
    cli: &Cli,
    profile: Option<&Profile>,
    workspace: &std::path::Path,
    yes: bool,
) -> Result<()> {
    let client = init_client(cli, profile).await?;
    eprintln!(
        "{} {}",
        SEARCH,
        style(format!(
            "Inspecting Keycloak configuration into {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    inspect::run(&client, workspace.to_path_buf(), &cli.realms, yes).await?;
    Ok(())
}

async fn handle_validate(cli: &Cli, workspace: &std::path::Path) -> Result<()> {
    eprintln!(
        "{} {}",
        SEARCH,
        style(format!(
            "Validating Keycloak configuration from {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    validate::run(workspace.to_path_buf(), &cli.realms).await?;
    Ok(())
}

async fn handle_apply(
    cli: &Cli,
    profile: Option<&Profile>,
    workspace: &std::path::Path,
    yes: bool,
    review: bool,
    prune: bool,
) -> Result<()> {
    let client = init_client(cli, profile).await?;
    let resolver = init_secrets(cli, workspace, profile).await?;
    eprintln!(
        "{} {}",
        ACTION,
        style(format!(
            "Applying Keycloak configuration from {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    apply::run(apply::ApplyArgs {
        client: &client,
        workspace_dir: workspace.to_path_buf(),
        realms_to_apply: &cli.realms,
        yes,
        review,
        prune,
        ui: Arc::new(crate::utils::ui::DialoguerUi::new()),
        resolver,
        profile: cli.profile.clone(),
    })
    .await?;
    Ok(())
}

async fn handle_plan(
    cli: &Cli,
    profile: Option<&Profile>,
    workspace: &std::path::Path,
    changes_only: bool,
    interactive: bool,
    verbose: bool,
) -> Result<()> {
    let client = init_client(cli, profile).await?;
    let resolver = init_secrets(cli, workspace, profile).await?;
    eprintln!(
        "{} {}",
        SEARCH,
        style(format!(
            "Planning Keycloak configuration from {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    plan::VERBOSE.store(verbose, std::sync::atomic::Ordering::Relaxed);
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace.to_path_buf(),
        changes_only,
        interactive,
        realms_to_plan: &cli.realms,
        ui: Arc::new(crate::utils::ui::DialoguerUi::new()),
        resolver,
        profile: cli.profile.clone(),
    })
    .await?;
    Ok(())
}

async fn handle_drift(
    cli: &Cli,
    profile: Option<&Profile>,
    workspace: &std::path::Path,
    verbose: bool,
) -> Result<()> {
    let client = init_client(cli, profile).await?;
    let resolver = init_secrets(cli, workspace, profile).await?;
    eprintln!(
        "{} {}",
        SEARCH,
        style(format!(
            "Checking drift for Keycloak configuration from {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    plan::VERBOSE.store(verbose, std::sync::atomic::Ordering::Relaxed);
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace.to_path_buf(),
        changes_only: true,
        interactive: false,
        realms_to_plan: &cli.realms,
        ui: Arc::new(crate::utils::ui::DialoguerUi::new()),
        resolver,
        profile: cli.profile.clone(),
    })
    .await?;
    Ok(())
}

async fn handle_cli(workspace: &std::path::Path) -> Result<()> {
    cli::run(
        workspace.to_path_buf(),
        &crate::utils::ui::DialoguerUi::new(),
    )
    .await?;
    Ok(())
}

async fn handle_clean(cli: &Cli, workspace: &std::path::Path, yes: bool) -> Result<()> {
    eprintln!(
        "{} {}",
        ACTION,
        style(format!(
            "Cleaning up Keycloak configuration in {:?}",
            workspace
        ))
        .cyan()
        .bold()
    );
    clean::run(
        workspace.to_path_buf(),
        yes,
        &cli.realms,
        &crate::utils::ui::DialoguerUi::new(),
    )
    .await?;
    Ok(())
}

/// Standard entry point that resolves config workspaces and executes command handlers.
///
/// # Errors
/// Returns an error if command execution or network request fails.
pub async fn run_app(cli: Cli) -> Result<()> {
    // 1. Load configuration file
    let config = load_config_file(cli.config.as_deref()).await?;

    // 2. Merge config file settings into Cli
    let mut cli = cli;
    if cli.server.is_none() {
        cli.server = config.server.clone();
    }
    if cli.realms.is_empty() && config.realms.is_some() {
        cli.realms = config.realms.clone().unwrap();
    }
    if cli.user.is_none() {
        cli.user = config.user.clone();
    }
    if cli.client_id.is_none() {
        cli.client_id = config.client_id.clone();
    }
    if cli.profile.is_none() {
        cli.profile = config.profile.clone();
    }
    if cli.vault_addr.is_none() {
        cli.vault_addr = config.vault_addr.clone();
    }
    if cli.vault_token.is_none() {
        cli.vault_token = config.vault_token.clone();
    }

    // 3. Fallback default for client_id
    if cli.client_id.is_none() {
        cli.client_id = Some("admin-cli".to_string());
    }

    // 4. Resolve workspace directory
    let raw_workspace = match &cli.command {
        Commands::Inspect { workspace, .. } => workspace.clone(),
        Commands::Validate { workspace } => workspace.clone(),
        Commands::Apply { workspace, .. } => workspace.clone(),
        Commands::Plan { workspace, .. } => workspace.clone(),
        Commands::Drift { workspace, .. } => workspace.clone(),
        Commands::Cli { workspace } => workspace.clone(),
        Commands::Clean { workspace, .. } => workspace.clone(),
        Commands::Init { .. } => None,
    };
    let workspace = raw_workspace
        .or(config.workspace.clone())
        .unwrap_or_else(|| std::path::PathBuf::from("workspace"));

    // 5. Load profile if requested
    let profile = if let Some(p) = &cli.profile {
        Some(load_profile(&workspace, p).await?)
    } else {
        None
    };

    // Resolve timeout based on:
    // CLI Flags > Active Profile > Environment Variables > TOML Configuration > Default Fallbacks
    let resolved_timeout = cli
        .timeout
        .or_else(|| profile.as_ref().and_then(|p| p.timeout))
        .or_else(|| {
            std::env::var("KEYCLOAK_TIMEOUT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
        })
        .or(config.timeout)
        .unwrap_or(10);
    cli.timeout = Some(resolved_timeout);

    // 6. Execute subcommand handlers
    match &cli.command {
        Commands::Inspect { yes, .. } => {
            handle_inspect(&cli, profile.as_ref(), &workspace, *yes).await?;
        }
        Commands::Validate { .. } => {
            handle_validate(&cli, &workspace).await?;
        }
        Commands::Apply {
            yes, review, prune, ..
        } => {
            handle_apply(&cli, profile.as_ref(), &workspace, *yes, *review, *prune).await?;
        }
        Commands::Plan {
            changes_only,
            interactive,
            verbose,
            ..
        } => {
            handle_plan(
                &cli,
                profile.as_ref(),
                &workspace,
                *changes_only,
                *interactive,
                *verbose,
            )
            .await?;
        }
        Commands::Drift { verbose, .. } => {
            handle_drift(&cli, profile.as_ref(), &workspace, *verbose).await?;
        }
        Commands::Cli { .. } => {
            handle_cli(&workspace).await?;
        }
        Commands::Clean { yes, .. } => {
            handle_clean(&cli, &workspace, *yes).await?;
        }
        Commands::Init {
            interactive,
            output,
        } => {
            init::run(
                *interactive,
                output.clone(),
                &crate::utils::ui::DialoguerUi::new(),
            )
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_load_profile_success() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let profiles_dir = workspace.join("profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let profile_path = profiles_dir.join("test_prof.yaml");
        let yaml_content = r#"
server_url: "http://localhost:8080"
client_id: "test-client"
"#;
        std::fs::write(&profile_path, yaml_content).unwrap();

        let profile = load_profile(workspace, "test_prof").await.unwrap();
        assert_eq!(profile.server_url, "http://localhost:8080");
        assert_eq!(profile.client_id, Some("test-client".to_string()));
    }

    #[tokio::test]
    async fn test_load_profile_missing_file() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();

        let result = load_profile(workspace, "non_existent").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to read profile file:"));
    }

    #[tokio::test]
    async fn test_load_profile_invalid_yaml() {
        let dir = tempdir().unwrap();
        let workspace = dir.path();
        let profiles_dir = workspace.join("profiles");
        std::fs::create_dir_all(&profiles_dir).unwrap();

        let profile_path = profiles_dir.join("invalid.yaml");
        let yaml_content = "server_url: [invalid_yaml";
        std::fs::write(&profile_path, yaml_content).unwrap();

        let result = load_profile(workspace, "invalid").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to parse profile file:"));
    }

    #[tokio::test]
    async fn test_load_config_file_success() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("kaji.toml");
        let toml_content = r#"
server = "http://localhost:8080"
realms = ["master"]
client_id = "test-client-id"
workspace = "test-ws"
"#;
        std::fs::write(&config_path, toml_content).unwrap();

        let config = load_config_file(Some(&config_path)).await.unwrap();
        assert_eq!(config.server, Some("http://localhost:8080".to_string()));
        assert_eq!(config.realms, Some(vec!["master".to_string()]));
        assert_eq!(config.client_id, Some("test-client-id".to_string()));
        assert_eq!(config.workspace, Some(std::path::PathBuf::from("test-ws")));
    }

    #[tokio::test]
    async fn test_load_config_file_missing() {
        let config = load_config_file(None).await.unwrap();
        assert!(config.server.is_none());
        assert!(config.realms.is_none());
    }

    #[tokio::test]
    async fn test_load_config_file_explicit_missing_error() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("non_existent.toml");
        let result = load_config_file(Some(&config_path)).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read config file:")
        );
    }

    #[tokio::test]
    async fn test_load_config_file_invalid() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("invalid.toml");
        std::fs::write(&config_path, "server = [invalid").unwrap();

        let result = load_config_file(Some(&config_path)).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to parse config file:")
        );
    }
}
