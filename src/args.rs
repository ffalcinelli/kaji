use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "kcd", author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Keycloak Server URL
    #[arg(long, env = "KEYCLOAK_URL", required_unless_present = "profile")]
    pub server: Option<String>,

    /// Keycloak Realms to consider. If empty, all realms are considered.
    #[arg(long, env = "KEYCLOAK_REALMS", value_delimiter = ',')]
    pub realms: Vec<String>,

    /// Keycloak Admin User
    #[arg(long, env = "KEYCLOAK_USER")]
    pub user: Option<String>,

    /// Keycloak Admin Password
    #[arg(skip)]
    pub password: Option<String>,

    /// Keycloak Client ID (for client credentials grant)
    #[arg(long, env = "KEYCLOAK_CLIENT_ID", default_value = "admin-cli")]
    pub client_id: String,

    /// Keycloak Client Secret (for client credentials grant)
    #[arg(skip)]
    pub client_secret: Option<String>,

    /// Profile name to load from profiles/ directory
    #[arg(long, short = 'p')]
    pub profile: Option<String>,

    /// HashiCorp Vault URL
    #[arg(long, env = "VAULT_ADDR")]
    pub vault_addr: Option<String>,

    /// HashiCorp Vault Token
    #[arg(long, env = "VAULT_TOKEN")]
    pub vault_token: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inspect the current Keycloak configuration and dump to files
    Inspect {
        /// Workspace directory for configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,

        /// Skip confirmation prompt when overwriting local files
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,
    },
    /// Validate the local Keycloak configuration files
    Validate {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,
    },
    /// Apply the local Keycloak configuration to the server
    Apply {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,

        /// Skip confirmation prompt
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,

        /// Ask for confirmation before applying each resource
        #[arg(long, short = 'r', default_value = "false")]
        review: bool,
    },
    /// Plan the application of the local Keycloak configuration
    Plan {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,

        /// Show only changes, suppressing "No changes" messages
        #[arg(long, short = 'c')]
        changes_only: bool,

        /// Ask interactively whether to include each change in the plan
        #[arg(long, short = 'i', default_value = "false")]
        interactive: bool,
    },
    /// Check for drift between local configuration and server
    Drift {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,
    },
    /// Interactive CLI mode to generate local configuration
    Cli {
        /// Workspace directory for configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,
    },
    /// Clean the local configuration files
    Clean {
        /// Workspace directory containing configuration files
        #[arg(long, short = 'w', default_value = "workspace")]
        workspace: PathBuf,

        /// Skip confirmation prompt
        #[arg(long, short = 'y', default_value = "false")]
        yes: bool,
    },
}
