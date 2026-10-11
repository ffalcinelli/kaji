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
    #[arg(long, env = "KEYCLOAK_URL", global = true)]
    pub server: Option<String>,

    /// Keycloak Realms to consider. If empty, all realms are considered.
    #[arg(long, env = "KEYCLOAK_REALMS", global = true, value_delimiter = ',')]
    pub realms: Vec<String>,

    /// Keycloak Admin User
    #[arg(long, env = "KEYCLOAK_USER", global = true)]
    pub user: Option<String>,

    /// Keycloak Admin Password
    #[arg(long, env = "KEYCLOAK_PASSWORD", global = true, hide_env_values = true)]
    pub password: Option<String>,

    /// Keycloak Client ID (for client credentials grant)
    #[arg(long, env = "KEYCLOAK_CLIENT_ID", global = true)]
    pub client_id: Option<String>,

    /// Keycloak Client Secret (for client credentials grant)
    #[arg(
        long,
        env = "KEYCLOAK_CLIENT_SECRET",
        global = true,
        hide_env_values = true
    )]
    pub client_secret: Option<String>,

    /// Profile name to load from profiles/ directory
    #[arg(long, short = 'p', global = true)]
    pub profile: Option<String>,

    /// Keycloak request timeout in seconds
    #[arg(long, global = true)]
    pub timeout: Option<u64>,

    /// Maximum number of concurrent HTTP requests sent to Keycloak (default 16)
    #[arg(long, env = "KAJI_CONCURRENCY", global = true)]
    pub concurrency: Option<usize>,

    /// Realm used to obtain the admin token (default: master)
    #[arg(long, env = "KEYCLOAK_AUTH_REALM", global = true)]
    pub auth_realm: Option<String>,

    /// Allow plain HTTP connections to non-local Keycloak servers (credentials are sent unencrypted)
    #[arg(long, env = "KAJI_ALLOW_INSECURE_HTTP", global = true)]
    pub allow_insecure_http: bool,

    /// HashiCorp Vault URL
    #[arg(long, env = "VAULT_ADDR", global = true)]
    pub vault_addr: Option<String>,

    /// HashiCorp Vault Token
    #[arg(long, env = "VAULT_TOKEN", global = true, hide_env_values = true)]
    pub vault_token: Option<String>,

    /// Path to a custom TOML configuration file
    #[arg(long, env = "KAJI_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// IDs of the arguments explicitly passed on the command line (as opposed to environment
    /// variables). Explicit flags take precedence over profile values.
    #[arg(skip)]
    pub explicit_args: Vec<String>,
}

impl Cli {
    /// Connection arguments whose precedence depends on where their value came from.
    const SOURCE_TRACKED_ARGS: &'static [&'static str] = &[
        "server",
        "user",
        "password",
        "client_id",
        "client_secret",
        "auth_realm",
        "vault_addr",
        "vault_token",
    ];

    /// Parses the process arguments, recording which arguments were explicitly given as flags.
    pub fn parse_with_sources() -> Self {
        Self::from_matches_with_sources(&<Self as clap::CommandFactory>::command().get_matches())
    }

    /// Parses the given arguments, recording which arguments were explicitly given as flags.
    pub fn try_parse_from_with_sources<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = <Self as clap::CommandFactory>::command().try_get_matches_from(args)?;
        Ok(Self::from_matches_with_sources(&matches))
    }

    fn from_matches_with_sources(matches: &clap::ArgMatches) -> Self {
        let mut cli =
            <Self as clap::FromArgMatches>::from_arg_matches(matches).unwrap_or_else(|e| e.exit());
        cli.explicit_args = Self::SOURCE_TRACKED_ARGS
            .iter()
            .filter(|id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
            .map(|id| id.to_string())
            .collect();
        cli
    }

    /// Returns true if the argument was explicitly passed as a command-line flag.
    pub fn is_explicit(&self, id: &str) -> bool {
        self.explicit_args.iter().any(|a| a == id)
    }
}

impl fmt::Debug for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            command,
            server,
            realms,
            user,
            password,
            client_id,
            client_secret,
            profile,
            timeout,
            concurrency,
            auth_realm,
            allow_insecure_http,
            vault_addr,
            vault_token,
            config,
            explicit_args,
        } = self;

        f.debug_struct("Cli")
            .field("command", command)
            .field("server", server)
            .field("realms", realms)
            .field("user", user)
            .field("password", &password.as_ref().map(|_| "********"))
            .field("client_id", client_id)
            .field("client_secret", &client_secret.as_ref().map(|_| "********"))
            .field("profile", profile)
            .field("timeout", timeout)
            .field("concurrency", concurrency)
            .field("auth_realm", auth_realm)
            .field("allow_insecure_http", allow_insecure_http)
            .field("vault_addr", vault_addr)
            .field("vault_token", &vault_token.as_ref().map(|_| "********"))
            .field("config", config)
            .field("explicit_args", explicit_args)
            .finish()
    }
}

/// List of subcommands supported by `kaji`.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Inspect the current Keycloak configuration and dump to files
    #[command(
        visible_alias = "sync",
        visible_alias = "pull",
        visible_alias = "export"
    )]
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
    #[command(visible_alias = "push", visible_alias = "deploy")]
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
    /// Maximum number of concurrent HTTP requests sent to Keycloak
    pub concurrency: Option<usize>,
    /// Realm used to obtain the admin token (default: master)
    pub auth_realm: Option<String>,
    /// Allow plain HTTP connections to non-local Keycloak servers
    pub allow_insecure_http: Option<bool>,
    /// HashiCorp Vault URL
    pub vault_addr: Option<String>,
    /// HashiCorp Vault Token
    pub vault_token: Option<String>,
    /// Workspace directory
    pub workspace: Option<PathBuf>,
}
