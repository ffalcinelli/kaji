# GEMINI.md - kaji (舵 — Helm/Rudder)

This document serves as the internal developer guide for `kaji`. It explains the architecture, design decisions, and workflows for extending the tool.

## 🏛️ Architecture Overview

`kaji` follows a **Reconciliation Loop** pattern, similar to Kubernetes controllers.

1.  **Desired State**: Defined in local YAML files within the workspace. Support for **Environment Profiles & Overlays** allows for multi-environment configurations (e.g., `realm.yaml` + `realm.prod.yaml`).
2.  **Current State**: Fetched from the Keycloak Admin API.
3.  **Diff Engine (`src/plan/`)**: Compares the two states to identify what needs to be Created, Updated, or Deleted. It generates a `.kajiplan` file in the workspace containing the list of files that have pending changes.
4.  **Reconciler (`src/apply/`)**: Executes the necessary API calls to bring the Current State in line with the Desired State. It uses **Dependency-Aware (Staged) Reconciliation** to ensure resources are applied in the correct order (e.g., Realms before Roles, Roles before Users). Supports optional pruning/deletion of orphaned remote resources not declared in the workspace configuration using the `--prune` flag (excluding protected system resources like default clients/roles).

### Core Modules

-   `src/client.rs`: Low-level wrapper for the Keycloak Admin REST API. Handles authentication and provides a **generic CRUD interface** for Keycloak resources.
-   `src/models.rs`: Serde-based representations of Keycloak resources. Defines the `KeycloakResource` and `ResourceMeta` traits for generic resource management.
-   `src/inspect.rs`: Deep-scans the remote Keycloak server and serializes resources into local files using a **generic, parallelized inspection pipeline**.
-   `src/plan/`: Contains the logic for calculating diffs. Uses a **generic planning engine** (`generic.rs`) for most resource types.
-   `src/apply/`: Contains the logic for applying changes. Uses a **generic reconciliation engine** (`generic.rs`) and a **staged application pipeline** to ensure reliability.
-   `src/utils/secrets/`: Manages secret resolution (Env, Vault, etc.).
-   `src/utils/yaml.rs`: Handles YAML deep-merging and profile-specific overlays.
-   `src/utils/ui.rs`: Centralized module for CLI output formatting, emoji management, and **indicatif progress bars**.
-   `src/init.rs`: Scaffolds the initial `kaji.toml` / `.kaji.toml` configuration files.

---

## 🛠️ Staged Reconciliation Pipeline

To prevent race conditions and ensure correct dependency handling, `apply` is executed in stages:

1.  **Stage 0**: Realms (Foundation).
2.  **Stage 1**: Identity Providers, Roles (Infrastructure).
3.  **Stage 2**: Clients, Client Scopes, Authentication Flows, Required Actions, Groups (Structure).
4.  **Stage 3**: Users, Authenticator Configs, Components, Keys (Data & Final Config).

## 🔄 Keycloak Resource Enrichment

During the reconciliation (`apply`) process, Keycloak may enrich resources with default values, read-only system attributes, or server-assigned identifiers (IDs). When `kaji` detects differences between the local representation and the enriched one returned by Keycloak:
1. It recursively maps any user-defined secret placeholders (e.g. `${VAR_NAME}`) from the original local file to the enriched representation to prevent them from being lost or overwritten by redacted/actual secret values.
2. It prompts the user (defaulting to Yes) to update the local representation to match the enriched Keycloak representation.
3. If the `--yes` (`-y`) option flag is passed, the update is accepted automatically without prompting.
4. Any newly generated secrets (such as client secrets) are extracted and appended to the secrets file.

---

## 🌍 Environment Profiles & Overlays

`kaji` supports multi-environment configurations via the `--profile` (`-p`) flag.

### Profiles
Profiles are stored in the `profiles/` directory (e.g., `profiles/prod.yaml`). They define environment-specific connection details:
```yaml
server_url: "https://keycloak.prod.example.com"
client_id: "kaji-cli"
client_secret: "${PROD_KAJI_SECRET}"
secrets_file: ".secrets.prod"
```

### Overlays
For any resource `resource.yaml`, `kaji` looks for `resource.{profile}.yaml` and deep-merges it onto the base configuration if that profile is active. This is handled by `src/utils/yaml.rs`.

---

## ⚙️ Project-Level Configuration File (`kaji.toml` / `.kaji.toml`)

`kaji` supports project-level settings to override defaults and connection variables persistently. 

### Architecture & Pipeline
1. **Schema Definition**: The `Config` struct is defined in [src/args.rs](src/args.rs). It represents optional fields for Keycloak connection credentials, vault parameters, the workspace folder, and request timeouts.
2. **File Lookup**: In [src/lib.rs](src/lib.rs), `load_config_file` checks for:
   * A custom config path specified via `--config` CLI flag or `KAJI_CONFIG` env var.
   * `kaji.toml` in the current working directory.
   * `.kaji.toml` in the current working directory.
3. **Merging Logic**: In `run_app` in `src/lib.rs`, the loaded `Config` is merged into the parsed CLI `Cli` struct. Settings are resolved using a strictly defined precedence logic:
   * **CLI Flags** > **Active Profile** > **Environment Variables** > **TOML Configuration** > **Default Fallbacks**.

---

## 🛠️ Adding a New Resource Support

To support a new Keycloak resource (e.g., "Event Listeners"):

1.  **Update `models.rs`**: 
    - Add the `struct` for the resource.
    - Implement `KeycloakResource` (for name/ID handling and API paths).
    - Implement `ResourceMeta` (to define labels and secret prefixes).
2.  **Update `inspect.rs`**: Add a `spawn_inspect::<NewResourceRepresentation>(...)` call in the `inspect_realm` function.
3.  **Update `plan/mod.rs`**: Add the new resource to `plan_single_realm` using `generic::plan_resources`.
4.  **Update `apply/mod.rs`**: Add the new resource to the appropriate stage in `apply_single_realm` using `generic::apply_resources`.
5.  **Update `validate.rs`**: (Optional) Add specific validation rules.
6.  **Update `cli/`**: (Optional) Add interactive scaffolding for the new resource.

---

## 📺 Terminal UI & Diff Viewer Enhancements

### 1. Minimal Unified Diffs (Collapsed by Default)
To reduce terminal clutter, `kaji plan` and `kaji drift` default to showing collapsed unified diffs (with 3 context lines around changes). A `--verbose` or `-v` flag allows users to output full file diffs.

### 2. Interactive Diff Expansion
During `kaji plan --interactive`, the prompt is a selection menu with `Yes` (include change), `No` (skip change), and `Show Full Diff` (expand to full verbose diff). Selecting `Show Full Diff` displays the complete diff and prompts the user again.

### 3. Styled Fuzzy Scaffolding Menu
The interactive menu (`kaji cli`) utilizes `dialoguer::theme::ColorfulTheme` for polished, colorful CLI prompts. It replaces standard selects with `dialoguer::FuzzySelect`, allowing users to type to search and filter options instantly.

### 4. Workspace Realm Auto-Discovery
All scaffolding prompts dynamically scan the workspace to discover existing realms. The user is presented with a `FuzzySelect` list of discovered realms plus a `<Create New Realm...>` option, avoiding manual typing for existing projects.

---

## 🧪 Testing Strategy

`kaji` employs a multi-layered testing strategy:

### Unit Tests
Located within the modules themselves (e.g., `src/utils/secrets.rs`). Focused on pure logic like secret masking, path resolution, and YAML parsing.

### Integration Tests
Located in `tests/`.
-   **Common**: Shared utilities for setting up temporary workspaces and environment variables.
-   **Mocked Tests**: Use `mockito` to simulate Keycloak responses for various scenarios.
-   **Real Integration**: Requires a live Keycloak instance (configured via environment variables or **Profiles**). See `tests/real_integration_test.rs`.
-   **Ultimate Coverage**: `tests/ultimate_coverage_test.rs` and `tests/models_coverage_test.rs` provide comprehensive checks for resource handling.

### Benchmarks
Located in `benches/`. Used to monitor performance for large workspaces with thousands of files.

---

## 🔐 Secret Management Logic

Secret handling is managed via the `SecretResolver` trait, which allows for multiple resolution strategies:

- **EnvResolver**: Resolves `${VAR_NAME}` from the environment or a `.secrets` file.
- **VaultResolver**: Resolves `${vault:mount/path#field}` from a HashiCorp Vault KV2 engine. Utilizes a thread-safe in-memory cache (`tokio::sync::Mutex<HashMap>`) to avoid duplicate/redundant HTTP GET requests to Vault for the same secret path during planning/applying.
- **CompositeResolver**: Chains multiple resolvers in a prioritized order.

The masking heuristic during `inspect` looks for keys matching these patterns:
-   Contains `secret` (case-insensitive)
-   Contains `password`
-   Matches exactly `value` (for certain component configurations)
-   Matches exactly `hashedValue`

When detected, the value is replaced by `${KEYCLOAK_<RESOURCE_TYPE>_<RESOURCE_NAME>_<FIELD_NAME>}`.

---

## 📜 Coding Conventions

1.  **Asynchronous by Default**: All I/O and API operations must use `tokio`.
2.  **Concurrency**: Use `tokio::task::JoinSet` to parallelize independent resource operations.
3.  **Generic Abstractions**: Prefer using the generic CRUD methods in `KeycloakClient` and the `KeycloakResource`/`ResourceMeta` traits to avoid boilerplate.
4.  **Error Handling**: Use `anyhow::Context` for descriptive error chains, including specific resource identifiers (e.g., realm name).
5.  **Formatting**: Run `cargo fmt --all -- --check` before every commit and ensure all formatting issues are resolved.
6.  **Clippy**: Ensure `cargo clippy` passes without warnings.
7.  **Serialization**: Prefer `serde_yaml_ng` for YAML operations to ensure compatibility with modern YAML features.
8.  **Documentation Updates**: Always update relevant documentation (including `README.md`, `GEMINI.md`, `JULES.md`, `AGENTS.md`, and `.jules/` guides) whenever you introduce new features, modify reconciliation stages, or alter module structures to prevent documentation drift.


---

## 🚀 Future Roadmap

-   [x] Parallel reconciliation (apply changes concurrently for resources within a realm).
-   [x] Generic refactor for `inspect.rs`.
-   [x] Integration with HashiCorp Vault for secret resolution.
-   [ ] Support for custom SPIs and provider configurations.
-   [x] Support for multiple environment profiles (e.g., `prod.yaml`, `staging.yaml`).
-   [x] Generic refactor for `plan.rs` and `apply.rs` (similar to `inspect.rs`).
