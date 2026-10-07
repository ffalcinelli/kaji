# kaji — Steer Your Keycloak Configuration

[![CI](https://github.com/ffalcinelli/kaji/actions/workflows/ci.yml/badge.svg)](https://github.com/ffalcinelli/kaji/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/ffalcinelli/kaji/graph/badge.svg)](https://app.codecov.io/gh/ffalcinelli/kaji)
[![docs.rs](https://img.shields.io/docsrs/kaji)](https://docs.rs/kaji)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![Rust Version](https://img.shields.io/badge/rust-1.85%2B-blue.svg)

**Disclaimer**: This project is experimentally written almost entirely by AI, so any usage should keep this in mind and that the execution of this software is at your own risk.

`kaji` (舵, Japanese for *helm* or *rudder*) is a robust CLI tool for the **declarative management** of [Keycloak](https://www.keycloak.org/) configurations. Just as a ship's helm holds its course through any conditions, `kaji` steers your Keycloak identity infrastructure to a stable, locked, declared state — enabling version control, automated testing, and reliable drift detection.

---

## 📺 Screenshots

### Interactive Plan Mode
> Previewing changes before applying them with interactive confirmation.

![kaji plan screenshot](https://raw.githubusercontent.com/ffalcinelli/kaji/main/assets/kaji-plan.png)

```text
$ kaji plan --interactive
💡 Calculating diff for realm 'master'...

  Clients:
    [+] my-new-app (Create)
    [~] admin-cli (Update)
        - root_url: "http://localhost:8080" -> "https://idp.example.com"
    [-] legacy-app (Delete)

? Apply change to client 'my-new-app'? (y/n)
```

### Interactive CLI Menu
> Scaffolding resources without writing YAML by hand.

![kaji cli screenshot](https://raw.githubusercontent.com/ffalcinelli/kaji/main/assets/kaji-cli.png)

```text
$ kaji cli
💡 Welcome to kaji interactive CLI!
? What would you like to do?
❯ Create User
  Change User Password
  Create Client
  Create Role
  Create Group
  Create Identity Provider
  Create Client Scope
  Rotate Keys
  Exit
```

---

## 🚀 Key Features

- **Blazing Fast Performance**: Utilizes Rust's `tokio` for highly concurrent API interactions and parallel I/O operations.
- **Declarative State**: Define your desired Keycloak state in human-readable YAML files.
- **Environment Profiles & Overlays**: Manage multiple environments (Dev, Staging, Prod) with zero configuration duplication.
- **Dependency-Aware Reconciliation**: Guaranteed correct application order through staged reconciliation (e.g., Realms -> Roles -> Users).
- **Bootstrapping & Scaffolding**: Easily initialize new project configurations (`kaji.toml` / `.kaji.toml`) with the `init` command.
- **Inspect & Export**: Bootstrap your project by exporting existing Keycloak configurations to local files.
- **Dry-Run Planning**: Preview exactly what changes will be applied with detailed diffs and summaries.
- **Interactive Review**: Confirm individual changes before they are applied to the server using the `--review` flag.
- **Drift Detection**: Identify discrepancies between your local configuration and the live server.
- **Secret Masking & Resolution**: Native support for Environment Variables and HashiCorp Vault.
- **Resource Support**: Realms, Roles, Identity Providers, Clients, Client Scopes, Groups, Users, Authentication Flows, Required Actions, and Components (including Keys).

---

## 🛠️ Installation

### Install Pre-built Binaries

**macOS and Linux:**
```bash
curl -LsSf https://raw.githubusercontent.com/ffalcinelli/kaji/main/scripts/install.sh | sh
```

**Windows:**
```powershell
powershell -c "irm https://raw.githubusercontent.com/ffalcinelli/kaji/main/scripts/install.ps1 | iex"
```

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable) and Cargo.

### Building from Source

```bash
git clone https://github.com/ffalcinelli/kaji.git
cd kaji
cargo build --release
sudo cp target/release/kaji /usr/local/bin/
```

---

## 🛠️ Development

This project uses `cargo-husky` to manage Git hooks. To set up your development environment:

1.  Clone the repository.
2.  Run `cargo test`.

Running tests will automatically install the Git hooks in your `.git/hooks` directory. The pre-commit hook ensures that `cargo fmt` and `cargo clippy` pass before any code is committed.

### Live Keycloak tests

`tests/real_integration_test.rs` runs against a real Keycloak (pinned to **26.8.0** in `docker-compose.yml`). The tests are skipped unless `KAJI_IT_URL` is set:

```bash
# Ports are configurable in case 8080/9000 are taken
KAJI_IT_PORT=8180 KAJI_IT_MGMT_PORT=9180 docker compose up -d --wait
KAJI_IT_URL=http://localhost:8180 cargo test --test real_integration_test
```

Tests marked `#[ignore = "known bug: ..."]` document confirmed defects that are not fixed yet (see the Known Issues section in [AGENTS.md](AGENTS.md)). In CI they run in the `Live Keycloak` workflow (on `main`, manual dispatch, or PRs labeled `live-tests`).

---

## 🌍 Environment Profiles

`kaji` allows you to manage multiple Keycloak instances (e.g., Development, Staging, Production) using a native **Profiles** system.

### 1. Define a Profile
Create a YAML file in the `profiles/` directory:

**`profiles/prod.yaml`**
```yaml
server_url: "https://keycloak.prod.example.com"
client_id: "kaji-cli"
client_secret: "${PROD_KAJI_SECRET}"
secrets_file: ".secrets.prod"  # Load environment secrets from this file
timeout: 30                    # Keycloak request timeout in seconds (optional)
```

Placeholders in profile connection fields (`server_url`, `client_id`, `client_secret`, `user`, `password`, `vault_addr`, `vault_token`) are resolved from the environment and the profile's `secrets_file`.

### 2. Use Overlays
Avoid duplicating entire resource files for small environment-specific changes. Create an overlay file matching the pattern `resource.{profile}.yaml`:

**`workspace/my-realm/clients/my-app.yaml` (Base)**
```yaml
clientId: my-app
enabled: true
redirectUris:
  - "http://localhost:3000/*"
```

**`workspace/my-realm/clients/my-app.prod.yaml` (Overlay)**
```yaml
redirectUris:
  - "https://app.example.com/*"
```

When running with `--profile prod`, `kaji` deep-merges the overlay onto the base configuration.

A file is only treated as an overlay when its base file exists next to it (`my-app.prod.yaml` requires `my-app.yaml` or `my-app.yml`). Resources whose names contain dots, such as a client `app.test`, are therefore never mistaken for overlays.

When `apply` notices that Keycloak enriched a resource, it offers to update the local file. Files that have an active overlay are never rewritten, so profile-specific values cannot leak into the base file.

---

## ⚙️ Configuration

`kaji` uses environment variables for connection and authentication. You can export these in your shell or use a `.secrets` file.

| Variable | Description | Default |
| :--- | :--- | :--- |
| `KEYCLOAK_URL` | Base URL (e.g., `http://localhost:8080`) | **Required** |
| `KEYCLOAK_USER` | Admin username | |
| `KEYCLOAK_PASSWORD` | Admin password | |
| `KEYCLOAK_CLIENT_ID` | Client ID for auth | `admin-cli` |
| `KEYCLOAK_CLIENT_SECRET` | Client Secret (if using client credentials) | |
| `VAULT_ADDR` | HashiCorp Vault URL | |
| `VAULT_TOKEN` | HashiCorp Vault Token | |
| `KEYCLOAK_TIMEOUT` | Keycloak request timeout in seconds | `10` |

### Configuration File (`kaji.toml` / `.kaji.toml`)

`kaji` supports persisting connection and workspace settings in a TOML configuration file. By default, it searches for `kaji.toml` and then `.kaji.toml` in the current working directory. You can also specify a custom configuration file path using the global `--config` flag or the `KAJI_CONFIG` environment variable.

#### Example: `kaji.toml`
```toml
# Keycloak Server URL
server = "http://localhost:8080"

# Target realms (if empty, all realms are considered)
realms = ["master", "myrealm"]

# Admin User
user = "admin"

# Client ID used for auth
client_id = "kaji-cli"

# Default environment profile to load
profile = "dev"

# Workspace directory containing configuration files (defaults to "workspace")
workspace = "kaji-workspace"

# HashiCorp Vault Settings
vault_addr = "http://vault:8200"
vault_token = "my-vault-token"

# Keycloak request timeout in seconds (optional)
timeout = 10

# Maximum concurrent HTTP requests to Keycloak (optional, default 16;
# also --concurrency or KAJI_CONCURRENCY)
concurrency = 16

# Realm used to obtain the admin token (optional, default "master";
# also --auth-realm or KEYCLOAK_AUTH_REALM)
auth_realm = "master"

# Allow plain HTTP to a non-local server, e.g. http://keycloak:8080 inside a cluster
# (optional; also --allow-insecure-http or KAJI_ALLOW_INSECURE_HTTP). HTTPS is otherwise
# required except for localhost, 127.0.0.1 and ::1.
allow_insecure_http = false
```

Profiles accept `auth_realm` and `allow_insecure_http` as well.

`kaji` refreshes its admin access token automatically during long runs, retries transient Keycloak responses (HTTP 429/502/503/504, honoring `Retry-After`), and fetches paginated lists such as users completely.

#### Precedence / Priority Rules
When resolving settings, `kaji` merges settings from different sources in the following priority order (highest to lowest):
1. **CLI Flags** (explicitly passed on command line, e.g. `--server` or `--workspace`; values coming from environment variables such as `KEYCLOAK_URL` rank below the profile)
2. **Profile Configuration** (loaded from `profiles/` directory when `--profile` or `profile` is specified)
3. **Environment Variables** (e.g. `KEYCLOAK_URL`, `KEYCLOAK_CLIENT_ID`)
4. **Config File** (`kaji.toml` / `.kaji.toml`)
5. **Fallback Defaults** (e.g., `"admin-cli"` for client ID, `"workspace"` for workspace)

### Workspace Structure

```text
workspace/
├── .secrets                   # Default secrets file
├── profiles/
│   └── prod.yaml              # Profile definition
├── my-realm/                  # Realm folder
    ├── realm.yaml             # Main realm settings
    ├── clients/
    │   ├── my-app.yaml        # Base resource
    │   └── my-app.prod.yaml   # Environment overlay
    └── roles/
        └── admin.yaml
```

---

## 📖 Command Reference

### `inspect` *(Aliases: `sync`, `pull`, `export`)*
Exports the remote server state to local YAML files.
```bash
# Export everything to 'my-workspace' (using inspect, sync, pull, or export)
kaji inspect --workspace my-workspace --yes
kaji sync --workspace my-workspace --yes
```

### `validate`
Ensures your local YAML files are syntactically correct and follow the Keycloak model. Supports environment profiles (skips partial overlays like `*.prod.yaml` when validating base configs, or deep-merges them when `--profile` is specified). Checks include:
- Required fields are present and non-empty (realm name, client ID, role name, flow alias, etc.)
- **Authentication flow aliases do not contain characters forbidden by Keycloak** (`(`, `)`, `[`, `]`, `{`, `}`, `/`, `\`)
- **No duplicate authentication flow aliases** within the same workspace
- **Cycle detection (DFS)** and reference validation for authentication flows and sub-flows
```bash
kaji validate
kaji -p prod validate
```

### `plan`
Calculates the "diff" between local files and the remote server. By default, it shows a minimal, clean unified diff (collapsed with 3 lines of context).

In addition to the diff output, `plan` also runs a **sub-flow check** for authentication flows and reports sub-flows shared by several parent flows or created together with their parent.
```bash
# Plan for a specific profile
kaji plan --profile prod

# Show full resource diffs instead of collapsed unified diffs
kaji plan --verbose

# Interactive: decide for each change whether to include it.
# Prompts you with: Yes (include), No (skip), Show Full Diff (to expand)
kaji plan --interactive
```

`plan` writes a `.kajiplan` file containing the changed files (relative to the workspace), the profile, and a content hash of each file and its overlay. `apply` refuses plans made for a different profile or whose files changed after planning; run `kaji plan` again in that case.

Local files may be **partial**. For the realm, clients, client scopes and users, Keycloak keeps fields that a file omits, so `plan` only compares the declared keys. Roles, groups and identity providers are replaced as a whole by Keycloak: omitted fields are shown as removals, because `apply` really clears them (for identity providers this includes the whole `config`).

`plan` also lists remote resources that are not declared locally. They are only deleted by `apply --prune`.

### `apply`
Reconciles the remote state. It follows a **staged application order** (Realm → Roles/Client Scopes/Required Actions → Flows/Groups → Identity Providers/Clients → Users/Authenticator Configs/Components/Keys → realm flow bindings) so that every referenced resource exists before it is used, even when bootstrapping a new realm.

**`apply` automatically runs `validate` first** (pure local file I/O — no network cost). If validation fails, apply aborts immediately with a clear error message pointing to the offending file, before making any API calls to Keycloak. Validation enforces execution requirement enums, ensures subflows specify valid aliases, and detects circular dependency graphs using DFS.

Authentication flows use Keycloak's export format: each flow file lists its `authenticationExecutions` (`authenticator` or `authenticatorFlow: true` + `flowAlias`, `requirement`, optional `priority` and `authenticatorConfig` alias), and sub-flows live in their own files with `topLevel: false`. Flows are applied in **topological tiers** (sub-flows first), then their executions are reconciled: missing executions are added (sub-flows are linked), undeclared ones are removed, and requirements/priorities are updated. Executions without `priority` are ordered as listed. Built-in flows only accept requirement/priority changes; copy a built-in flow to change its structure. Leaving out `authenticationExecutions` keeps the remote executions untouched.

> Flow files exported by kaji versions before this change contain execution rows in the wrong format; run `kaji inspect` again to regenerate them.
```bash
# Apply planned changes for production
kaji apply --profile prod --yes

# Review mode: confirm each change before application
kaji apply --profile prod --review

# Prune mode: delete remote resources that are not declared in local config files
kaji apply --profile prod --prune
```

`apply` creates realms that do not exist yet (`realm.yaml`'s `realm` must match its directory name) and registers required actions that are not registered.

**Relationships** use Keycloak's export format and are reconciled through their dedicated endpoints (Keycloak ignores most of them on create/update):

| Where | Keys |
| :--- | :--- |
| Client files | `defaultClientScopes`, `optionalClientScopes` |
| User files | `groups` (paths such as `/org/team`), `realmRoles`, `clientRoles: {clientId: [role]}` |
| Group files | `realmRoles`, `clientRoles`, nested `subGroups` (sub-groups that are no longer declared are deleted) |
| Role files | `composites: {realm: [role], client: {clientId: [role]}}` |
| `clients/<clientId>/roles/<role>.yaml` | Client roles |

A relationship key that a file does not declare is left untouched.

Workspaces are **portable across environments**: server-assigned IDs are not exported by `inspect` (except component IDs used for parent references), are ignored by `plan`, and are never written back by `apply`. Components are matched by type, name and parent rather than by ID, so an LDAP provider and its mappers exported from one environment apply cleanly to another.

**Prune** only considers resource types whose directory exists in the realm (an empty directory means "manage this type, nothing declared"). It never deletes Keycloak's own resources:
- built-in flows (`builtIn: true`), default client scopes (`profile`, `email`, `acr`, `basic`, `organization`, ...), and system clients (including the `<realm>-realm` clients in `master`);
- default roles;
- required actions (they are never unregistered);
- service-account users, and all users of the `master` realm.

### `drift`
A read-only variant of `plan --changes-only`. It never writes `.kajiplan`, and it **exits with code 2 when drift is detected** (0 when in sync, 1 on errors), which makes it suitable for CI. By default, it prints collapsed unified diffs.
```bash
kaji drift --profile prod

# Show full resource diffs for configuration drift
kaji drift --verbose
```

### `clean`
Removes the realm directories of the workspace (or only those given with `--realms`) and the `.kajiplan` file. Profiles, `.secrets*` files and any other file in the workspace are kept.
```bash
kaji clean --yes
```

### `cli`
An interactive menu to generate resource scaffolds or perform quick actions.
```bash
kaji cli
```

### `init`
Scaffolds an initial `kaji.toml` / `.kaji.toml` project configuration. Automatically pre-fills settings from the environment where available.
```bash
# Non-interactive mode (uses environment variables to pre-fill, otherwise writes empty/default configuration)
kaji init

# Interactive mode (asks you for configuration parameters step-by-step)
kaji init --interactive
```

---

## 🔐 Secret Management

`kaji` is designed with security in mind. During `inspect`, it detects sensitive fields and replaces them with placeholders.

### Resolution Strategies

1. **Environment Variables**: Placeholders like `${VAR_NAME}` are resolved from the environment or a local `.secrets` file.
2. **HashiCorp Vault**: Placeholders like `${vault:mount/path#field}` are resolved from a live Vault instance using the KV2 engine.

Only `${UPPER_SNAKE_CASE}` names and `${vault:...}` references are placeholders. Anything else is left untouched, so Keycloak's own localization keys like `${client_account}` or `${profileScopeConsentText}` work as-is. Placeholders can be embedded in longer strings (`https://${HOST}/callback`). To write a literal `${NAME}`, escape it as `$${NAME}`.

#### Example 1: `confidential-client.yaml` (using Environment Variable)
```yaml
clientId: internal-api
name: Internal API Service
enabled: true
publicClient: false
secret: ${KEYCLOAK_CLIENT_INTERNAL_API_SECRET}
redirectUris:
  - "https://api.example.com/*"
serviceAccountsEnabled: true
```

#### Example 2: `vault-client.yaml` (using HashiCorp Vault)
```yaml
clientId: api-gateway
name: API Gateway
enabled: true
publicClient: false
# Format: ${vault:mount/path#field}
secret: ${vault:secret/data/kaji/clients/api-gateway#secret}
redirectUris:
  - "https://gateway.example.com/*"
protocol: openid-connect
```

### Usage Workflow

1. Run `kaji inspect` to bootstrap your local configuration.
2. Sensitive values are automatically replaced with `${KEYCLOAK_...}` placeholders and saved to a `.secrets` file. Re-running `inspect` updates existing keys in place instead of duplicating them, and never overwrites a stored secret with Keycloak's `**********` mask.
3. **DO NOT commit the `.secrets` file**.
4. (Optional) Replace placeholders with `vault:` syntax if using HashiCorp Vault.
5. Provide secrets via environment variables or set `VAULT_ADDR` and `VAULT_TOKEN`.
6. Run `kaji apply` to synchronize changes.

---

## 📅 Versioning

`kaji` uses [Calendar Versioning (CalVer)](https://calver.org/) with the format `YYMM.MICRO.MODIFIER` (e.g., `2603.1.0`).
- **YYMM**: The year and month of the release (e.g., `2603` for March 2026).
- **MICRO**: Increments for each release within the same month.
- **MODIFIER**: Typically `0`, used for specific hotfixes.

This format provides an immediate understanding of how recent your installed version is.

---

## 🤝 Credits

`kaji` is built for and relies on the excellent work of the [Keycloak](https://www.keycloak.org/) project and its community. Keycloak is an open-source identity and access management solution.

---

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

---

## 🛡️ Security Policy

Please refer to the [Security Policy](SECURITY.md) for information on reporting vulnerabilities and security best practices.
