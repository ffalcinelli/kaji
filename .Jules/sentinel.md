## 2025-02-14 - Prevent clap from leaking secrets in help menus
**Vulnerability:** The CLI exposed sensitive environment variables (e.g., `KEYCLOAK_PASSWORD`, `VAULT_TOKEN`) in the `--help` output because `clap` natively prints the current values of `env` fallback arguments when they are populated.
**Learning:** `clap` parses and caches environment variable values dynamically. If an argument uses `env = "SECRET_NAME"` without explicit configuration, running `--help` will print `[env: SECRET_NAME=actual_secret_value]` directly to standard output, risking exposure in CI logs or terminal scrollback.
**Prevention:** Always use `hide_env_values = true` (e.g., `#[arg(env = "SECRET_NAME", hide_env_values = true)]`) when binding sensitive environment variables to `clap` structs to suppress value rendering in help menus while retaining native parsing.
## 2025-02-28 - Implement Strict URL Canonicalization for VaultResolver Path Traversal

**Vulnerability:** The VaultResolver used a naive substring check (`if full_path.contains("..")`) to prevent directory traversal. This both created false positives (e.g. blocking `my..secret`) and failed to account for URL encoding (`%2e%2e`) or absolute path bypasses when constructing requests.
**Learning:** Naive string checks are insufficient for URL path validation. When paths are appended to URLs and sent via an HTTP client (like `reqwest`), the client or underlying URL parser will normalize segments, potentially bypassing simple string filters.
**Prevention:** Always use proper URL canonicalization when joining user-provided paths. Parse a base `reqwest::Url` with a trailing slash, `.join()` the user path to it, and enforce that the resulting URL strictly `.starts_with()` the parsed base URL string.
## 2024-08-06 - Enforce TLS for Vault connections
**Vulnerability:** VaultResolver allowed insecure (plain HTTP) connections in non-local environments, risking exposure of sensitive vault tokens and fetched secrets.
**Learning:** `reqwest::Client` defaults to supporting HTTP. When handling sensitive secrets from external stores, we must explicitly reject plain HTTP at the URL parsing stage to prevent accidental misconfiguration from exposing credentials over the network.
**Prevention:** Explicitly validate URL schemes against `https` (allowing `http` strictly for localhost/127.0.0.1) and configure `reqwest::Client::builder().https_only(!is_localhost)` when instantiating clients for secret resolvers.
## 2024-05-24 - Enforce TLS for Keycloak reqwest Client
**Vulnerability:** The `reqwest::Client` in `KeycloakClient::new` and `KeycloakClient::with_timeout` was not explicitly enforcing TLS, allowing potential plain HTTP connections to the Keycloak API which could expose sensitive credentials.
**Learning:** By default, `reqwest` does not mandate TLS unless strictly configured. Sensitive API clients must explicitly call `.https_only(true)` if they are connecting to remote hosts.
**Prevention:** Explicitly parse the URL scheme/host and apply `.https_only(!is_localhost)` (where `is_localhost` checks for `localhost` or `127.0.0.1`) when initializing HTTP clients for secure services.
## 2025-02-28 - TOCTOU Vulnerability in File Creation
**Vulnerability:** The `write_secure` helper function for writing sensitive YAML files had a Time-of-Check to Time-of-Use (TOCTOU) vulnerability. It checked if a file existed and applied `0o600` permissions by path before opening the file. An attacker could exploit this by swapping the file with a symlink after the check but before the file was opened, leading to sensitive data being written elsewhere or unauthorized permission changes.
**Learning:** Checking file existence and applying permissions via paths in sequence is inherently racy. While `OpenOptions::mode` sets permissions securely during *creation*, it does not apply them to existing files.
**Prevention:** Always operate on the file descriptor rather than the path. Open the file first with `OpenOptions` and apply secure permissions directly to the file descriptor (via `file.set_permissions()`) before writing sensitive data.
