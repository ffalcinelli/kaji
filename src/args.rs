use clap::{Parser, Subcommand};
use std::fmt;
use std::path::PathBuf;

/// The main CLI configuration for `kaji`.
#[derive(Parser)]
#[command(name = "kaji", author, version, about, long_about = None)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// Keycloak Server URL
    #[arg(long, env = "KEYCLOAK_URL")]
    pub server: Option<String>,

    /// Keycloak Realms to consider. If empty, all realms are considered.
    #[arg(long, env = "KEYCLOAK_REALMS", value_delimiter = ',')]
    pub realms: Vec<String>,

    /// Keycloak Admin User
    #[arg(long, env = "KEYCLOAK_USER")]
    pub user: Option<String>,

    /// Keycloak Admin Password
    #[arg(long, env = "KEYCLOAK_PASSWORD", hide_env_values = true)]
    pub password: Option<String>,

    /// Keycloak Client ID (for client credentials grant)
    #[arg(long, env = "KEYCLOAK_CLIENT_ID")]
    pub client_id: Option<String>,

    /// Keycloak Client Secret (for client credentials grant)
    #[arg(long, env = "KEYCLOAK_CLIENT_SECRET", hide_env_values = true)]
    pub client_secret: Option<String>,

    /// Profile name to load from profiles/ directory
    #[arg(long, short = 'p')]
    pub profile: Option<String>,

    /// Keycloak request timeout in seconds
    #[arg(long)]
    pub timeout: Option<u64>,

    /// HashiCorp Vault URL
    #[arg(long, env = "VAULT_ADDR")]
    pub vault_addr: Option<String>,

    /// HashiCorp Vault Token
    #[arg(long, env = "VAULT_TOKEN", hide_env_values = true)]
    pub vault_token: Option<String>,

    /// Path to a custom TOML configuration file
    #[arg(long, env = "KAJI_CONFIG")]
    pub config: Option<PathBuf>,
}

impl fmt::Debug for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cli")
            .field("command", &self.command)
            .field("server", &self.server)
            .field("realms", &self.realms)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "********"))
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "********"),
            )
            .field("profile", &self.profile)
            .field("timeout", &self.timeout)
            .field("vault_addr", &self.vault_addr)
            .field(
                "vault_token",
                &self.vault_token.as_ref().map(|_| "********"),
            )
            .finish()
    }
}

/// List of subcommands supported by `kaji`.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Inspect the current Keycloak configuration and dump to files
    Inspect {
        /// Workspace directory for configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,

        /// Skip confirmation prompt when overwriting local files
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,
    },
    /// Validate the local Keycloak configuration files
    Validate {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,
    },
    /// Apply the local Keycloak configuration to the server
    Apply {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,

        /// Ask for confirmation before applying each resource
        #[arg(long, short = 'r', default_value = "false")]
        review: bool,

        /// Prune remote resources that are not declared in the workspace configuration
        #[arg(long, default_value = "false")]
        prune: bool,
    },
    /// Plan the application of the local Keycloak configuration
    Plan {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,

        /// Show only changes, suppressing "No changes" messages
        #[arg(long, short = 'c')]
        changes_only: bool,

        /// Ask interactively whether to include each change in the plan
        #[arg(long, short = 'i', default_value = "false")]
        interactive: bool,

        /// Show full resource diff instead of unified diff of changes
        #[arg(long, short = 'v', default_value = "false")]
        verbose: bool,
    },
    /// Check for drift between local configuration and server
    Drift {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,

        /// Show full resource diff instead of unified diff of changes
        #[arg(long, short = 'v', default_value = "false")]
        verbose: bool,
    },
    /// Interactive CLI mode to generate local configuration
    Cli {
        /// Workspace directory for configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,
    },
    /// Clean the local configuration files
    Clean {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w')]
        workspace: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,
    },
    /// Scaffold an initial kaji.toml / .kaji.toml configuration file
    Init {
        /// Use interactive mode to prompt for configuration values
        #[arg(long, short = 'i', default_value = "false")]
        interactive: bool,

        /// Path to write the configuration file (defaults to kaji.toml)
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
    },
}

/// The schema of `.kaji.toml` / `kaji.toml` configuration file.
#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
pub struct Config {
    /// Keycloak Server URL
    pub server: Option<String>,
    /// Keycloak Realms to steer
    pub realms: Option<Vec<String>>,
    /// Keycloak Admin User
    pub user: Option<String>,
    /// Keycloak Client ID
    pub client_id: Option<String>,
    /// Environment Profile Name
    pub profile: Option<String>,
    /// Keycloak request timeout in seconds
    pub timeout: Option<u64>,
    /// HashiCorp Vault URL
    pub vault_addr: Option<String>,
    /// HashiCorp Vault Token
    pub vault_token: Option<String>,
    /// Workspace directory
    pub workspace: Option<PathBuf>,
}
