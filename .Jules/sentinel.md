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
