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

/// Error returned by `kaji drift` when the server differs from the workspace.
///
/// The binary maps it to exit code 2 so CI pipelines can tell drift apart from failures.
#[derive(Debug)]
pub struct DriftDetected(pub usize);

impl std::fmt::Display for DriftDetected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Drift detected: {} resource(s) differ from the workspace",
            self.0
        )
    }
}

impl std::error::Error for DriftDetected {}
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
    /// Realm used to obtain the admin token (optional, default `master`).
    #[serde(default)]
    pub auth_realm: Option<String>,
    /// Allow plain HTTP connections to a non-local server (optional).
    #[serde(default)]
    pub allow_insecure_http: Option<bool>,
}

/// Loads a profile configuration file from the `profiles/` directory in the workspace.
///
/// # Errors
/// Returns an error if the profile file fails to load or parse as YAML.
pub async fn load_profile(workspace: &std::path::Path, name: &str) -> Result<Profile> {
    let profiles_dir = workspace.join("profiles");
    let yaml_path = profiles_dir.join(format!("{}.yaml", name));
    let yml_path = profiles_dir.join(format!("{}.yml", name));

    let profile_path = if tokio::fs::try_exists(&yaml_path).await.unwrap_or(false) {
        yaml_path
    } else if tokio::fs::try_exists(&yml_path).await.unwrap_or(false) {
        yml_path
    } else {
        yaml_path
    };

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
        if tokio::fs::try_exists(&kaji_toml).await.unwrap_or(false) {
            Some(kaji_toml)
        } else {
            let dot_kaji_toml = cwd.join(".kaji.toml");
            if tokio::fs::try_exists(&dot_kaji_toml).await.unwrap_or(false) {
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

/// Chooses between a CLI/env value and a profile value.
///
/// Precedence: explicit CLI flag > profile > environment variable / TOML configuration.
fn pick(
    cli: &Cli,
    id: &str,
    cli_val: Option<String>,
    profile_val: Option<String>,
) -> Option<String> {
    if cli.is_explicit(id) {
        cli_val.or(profile_val)
    } else {
        profile_val.or(cli_val)
    }
}

/// Effective Keycloak connection settings after applying precedence rules.
struct ConnectionSettings {
    server: String,
    client_id: String,
    client_secret: Option<String>,
    user: Option<String>,
    password: Option<String>,
}

impl ConnectionSettings {
    fn resolve(cli: &Cli, profile: Option<&Profile>) -> Result<Self> {
        let server = pick(
            cli,
            "server",
            cli.server.clone(),
            profile.map(|p| p.server_url.clone()),
        )
        .context("Hint: Try running `kaji init` to generate a default config, or pass `--server`.")
        .context("Keycloak server URL not provided (neither via --server nor --profile)")?;
        let client_id = pick(
            cli,
            "client_id",
            cli.client_id.clone(),
            profile.and_then(|p| p.client_id.clone()),
        )
        .unwrap_or_else(|| "admin-cli".to_string());
        Ok(Self {
            server,
            client_id,
            client_secret: pick(
                cli,
                "client_secret",
                cli.client_secret.clone(),
                profile.and_then(|p| p.client_secret.clone()),
            ),
            user: pick(
                cli,
                "user",
                cli.user.clone(),
                profile.and_then(|p| p.user.clone()),
            ),
            password: pick(
                cli,
                "password",
                cli.password.clone(),
                profile.and_then(|p| p.password.clone()),
            ),
        })
    }
}

/// Resolves `${VAR}` placeholders in a profile's connection fields from the environment and the
/// profile's secrets file (e.g. `client_secret: "${PROD_KAJI_SECRET}"`).
///
/// # Errors
/// Returns an error if a placeholder cannot be resolved.
pub async fn resolve_profile_placeholders(
    mut profile: Profile,
    workspace: &std::path::Path,
) -> Result<Profile> {
    let secrets_file = profile.secrets_file.as_deref().unwrap_or(".secrets");
    let env_path = workspace.join(secrets_file);
    let mut vars = std::env::vars().collect::<HashMap<String, String>>();
    if let Ok(iter) = dotenvy::from_path_iter(&env_path) {
        for (k, v) in iter.flatten() {
            vars.insert(k, v);
        }
    }
    let resolver = EnvResolver::new(vars);

    profile.server_url = utils::secrets::substitute_string(&profile.server_url, &resolver)
        .await
        .context("Failed to resolve profile 'server_url'")?;
    for (name, field) in [
        ("client_id", &mut profile.client_id),
        ("client_secret", &mut profile.client_secret),
        ("user", &mut profile.user),
        ("password", &mut profile.password),
        ("vault_addr", &mut profile.vault_addr),
        ("vault_token", &mut profile.vault_token),
    ] {
        if let Some(value) = field.as_mut() {
            *value = utils::secrets::substitute_string(value, &resolver)
                .await
                .with_context(|| format!("Failed to resolve profile '{}'", name))?;
        }
    }
    Ok(profile)
}

/// Initializes a `KeycloakClient` by logging in using credentials from the CLI or active profile.
///
/// # Errors
/// Returns an error if connection URL is missing or login authentication fails.
pub async fn init_client(cli: &Cli, profile: Option<&Profile>) -> Result<KeycloakClient> {
    let ConnectionSettings {
        server,
        client_id,
        client_secret,
        user,
        password,
    } = ConnectionSettings::resolve(cli, profile)?;

    let timeout_secs = cli.timeout.unwrap_or(10);
    let auth_realm = pick(
        cli,
        "auth_realm",
        cli.auth_realm.clone(),
        profile.and_then(|p| p.auth_realm.clone()),
    )
    .unwrap_or_else(|| "master".to_string());
    let allow_insecure_http =
        cli.allow_insecure_http || profile.and_then(|p| p.allow_insecure_http) == Some(true);
    let mut client = KeycloakClient::new(server)
        .with_allow_insecure_http(allow_insecure_http)
        .with_timeout(std::time::Duration::from_secs(timeout_secs))
        .with_concurrency(cli.concurrency.unwrap_or(client::DEFAULT_CONCURRENCY))
        .with_auth_realm(auth_realm);
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
    let mut vars = std::env::vars().collect::<HashMap<String, String>>();
    if tokio::fs::try_exists(&env_path).await.unwrap_or(false)
        && let Ok(iter) = dotenvy::from_path_iter(&env_path)
    {
        for item in iter.flatten() {
            vars.insert(item.0, item.1);
        }
    }

    let mut resolvers: Vec<Box<dyn SecretResolver>> = Vec::new();

    let vault_addr = pick(
        cli,
        "vault_addr",
        cli.vault_addr.clone(),
        profile.and_then(|p| p.vault_addr.clone()),
    );
    let vault_token = pick(
        cli,
        "vault_token",
        cli.vault_token.clone(),
        profile.and_then(|p| p.vault_token.clone()),
    );

    if let (Some(addr), Some(token)) = (vault_addr, vault_token) {
        resolvers.push(Box::new(VaultResolver::new(&addr, &token)?));
    }

    resolvers.push(Box::new(EnvResolver::new(vars)));

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
    let ui = Arc::new(crate::utils::ui::DialoguerUi::new());
    let secrets_file = profile.and_then(|p| p.secrets_file.as_deref());
    inspect::run_with_ui_and_secrets(
        &client,
        workspace.to_path_buf(),
        &cli.realms,
        yes,
        ui,
        secrets_file,
    )
    .await?;
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
    validate::run_with_profile(workspace.to_path_buf(), &cli.realms, cli.profile.as_deref())
        .await?;
    Ok(())
}

fn check_credentials_presence(cli: &Cli, profile: Option<&Profile>) -> Result<()> {
    let ConnectionSettings {
        client_secret,
        user,
        password,
        ..
    } = ConnectionSettings::resolve(cli, profile)?;

    if client_secret.is_none() && (user.is_none() || password.is_none()) {
        return Err(anyhow::anyhow!(
            "Hint: Provide credentials via --user/--password flags, KEYCLOAK_USER/KEYCLOAK_PASSWORD env vars, or in your config file."
        ))
        .context("Missing authentication credentials");
    }
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
    check_credentials_presence(cli, profile)?;
    // 1. Validate local workspace before touching Keycloak (pure file I/O — no network)
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
    validate::run_with_profile(workspace.to_path_buf(), &cli.realms, cli.profile.as_deref())
        .await
        .context("Pre-apply validation failed. Fix the issues above before running apply.")?;

    // 2. Connect to Keycloak and apply
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
    check_credentials_presence(cli, profile)?;
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
    validate::run_with_profile(workspace.to_path_buf(), &cli.realms, cli.profile.as_deref())
        .await
        .context("Pre-plan validation failed. Fix the issues above before running plan.")?;

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
    plan::run(plan::PlanArgs {
        client: &client,
        workspace_dir: workspace.to_path_buf(),
        changes_only,
        interactive,
        realms_to_plan: &cli.realms,
        ui: Arc::new(crate::utils::ui::DialoguerUi::new()),
        resolver,
        profile: cli.profile.clone(),
        verbose,
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
    check_credentials_presence(cli, profile)?;
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
    validate::run_with_profile(workspace.to_path_buf(), &cli.realms, cli.profile.as_deref())
        .await
        .context("Pre-drift validation failed. Fix the issues above before running drift.")?;

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
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &client,
            workspace_dir: workspace.to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: &cli.realms,
            ui: Arc::new(crate::utils::ui::DialoguerUi::new()),
            resolver,
            profile: cli.profile.clone(),
            verbose,
        },
        false,
    )
    .await?;
    if summary.total() > 0 {
        return Err(DriftDetected(summary.total()).into());
    }
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
    if cli.realms.is_empty() {
        cli.realms = config.realms.clone().unwrap_or_default();
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
    if cli.concurrency.is_none() {
        cli.concurrency = config.concurrency;
    }
    if cli.auth_realm.is_none() {
        cli.auth_realm = config.auth_realm.clone();
    }
    if !cli.allow_insecure_http {
        cli.allow_insecure_http = config.allow_insecure_http == Some(true);
    }
    if cli.vault_addr.is_none() {
        cli.vault_addr = config.vault_addr.clone();
    }
    if cli.vault_token.is_none() {
        cli.vault_token = config.vault_token.clone();
    }

    for realm in &cli.realms {
        utils::validate_realm_name(realm)?;
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
        let loaded = load_profile(&workspace, p).await?;
        Some(resolve_profile_placeholders(loaded, &workspace).await?)
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

    fn profile_with(server: &str, user: Option<&str>) -> Profile {
        Profile {
            server_url: server.to_string(),
            client_id: None,
            client_secret: None,
            user: user.map(String::from),
            password: None,
            secrets_file: None,
            vault_addr: None,
            vault_token: None,
            timeout: None,
            auth_realm: None,
            allow_insecure_http: None,
        }
    }

    #[test]
    fn test_explicit_flags_override_profile() {
        let profile = profile_with("https://profile", Some("profile-user"));

        // Values from environment variables (not explicit flags) lose to the profile.
        let cli = Cli::try_parse_from_with_sources(["kaji", "validate"]).unwrap();
        let mut cli = cli;
        cli.server = Some("https://env".to_string());
        cli.user = Some("env-user".to_string());
        let settings = ConnectionSettings::resolve(&cli, Some(&profile)).unwrap();
        assert_eq!(settings.server, "https://profile");
        assert_eq!(settings.user.as_deref(), Some("profile-user"));

        // Explicit flags win over the profile.
        let cli = Cli::try_parse_from_with_sources([
            "kaji",
            "validate",
            "--server",
            "https://flag",
            "--user",
            "flag-user",
        ])
        .unwrap();
        let settings = ConnectionSettings::resolve(&cli, Some(&profile)).unwrap();
        assert_eq!(settings.server, "https://flag");
        assert_eq!(settings.user.as_deref(), Some("flag-user"));
        assert_eq!(settings.client_id, "admin-cli");
    }

    #[tokio::test]
    async fn test_resolve_profile_placeholders() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".secrets.prod"),
            "PROD_KAJI_SECRET=s3cr3t\nPROD_HOST=kc.example.com\n",
        )
        .unwrap();
        let mut profile = profile_with("https://${PROD_HOST}", None);
        profile.secrets_file = Some(".secrets.prod".to_string());
        profile.client_secret = Some("${PROD_KAJI_SECRET}".to_string());

        let resolved = resolve_profile_placeholders(profile.clone(), dir.path())
            .await
            .unwrap();
        assert_eq!(resolved.server_url, "https://kc.example.com");
        assert_eq!(resolved.client_secret.as_deref(), Some("s3cr3t"));

        profile.password = Some("${KAJI_TEST_UNDEFINED_PROFILE_VAR}".to_string());
        let err = resolve_profile_placeholders(profile, dir.path())
            .await
            .unwrap_err();
        assert!(format!("{:#}", err).contains("KAJI_TEST_UNDEFINED_PROFILE_VAR"));
    }

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
