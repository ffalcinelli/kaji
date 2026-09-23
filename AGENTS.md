# 🧭 AGENTS.md — AI Agent Developer Guide for kaji

Welcome! This document serves as the canonical developer guide and instructions directory for AI coding assistants (agents) working on the `kaji` repository. It outlines the project's architecture, core modules, staged reconciliation pipeline, profile overlays, coding conventions, testing strategies, and development commands.

---

## 🏛️ Architecture Overview

`kaji` (舵 — Japanese for *helm* or *rudder*) follows a **Reconciliation Loop** pattern, similar to Kubernetes controllers, to steer Keycloak server state to a stable, declared state.

1. **Desired State**: Defined in local YAML configurations within the workspace. Supports environment-specific overlays (e.g. `realm.yaml` deep-merged with `realm.prod.yaml`).
2. **Current State**: Fetched dynamically from the Keycloak Admin API.
3. **Diff Engine ([`src/plan/`](src/plan/))**: Compares Desired and Current states to identify what needs to be created, updated, or deleted. It generates a `.kajiplan` file listing the files with pending changes.
4. **Reconciler ([`src/apply/`](src/apply/))**: Executes API requests to resolve the differences. It uses a **Staged Reconciliation Pipeline** to apply changes in order of dependencies. Supports optional pruning/deletion of orphaned remote resources not declared in the workspace configuration using the `--prune` flag (excluding protected system resources like default clients/roles).

---

## 🏗️ Core Modules

All business logic is located in the [`src/`](src/) directory:

*   [`src/client.rs`](src/client.rs): Wrapper around the Keycloak Admin REST API. Handles authentication and provides a **generic CRUD interface** for resources.
*   [`src/models.rs`](src/models.rs): Strongly-typed Serde representations of Keycloak resources. Implements the `KeycloakResource` and `ResourceMeta` traits for generic resource management.
*   [`src/inspect.rs`](src/inspect.rs): Scans the remote Keycloak instance and serializes resources into local workspace files using a parallelized pipeline. Supported CLI aliases: `sync`, `pull`, `export`.
*   [`src/plan/`](src/plan/): Calculates diffs and writes the plan. Uses the generic planning engine in `generic.rs`. Supports collapsed unified diff formatting (3 context lines) by default, `--verbose` full diff view, and interactive expansion choices during confirmation. Also runs a **sub-flow check** for authentication flows: identifies shared sub-flows and explains topological staging and auto-adoption.
*   [`src/apply/`](src/apply/): Reconciles resources. Uses the generic reconciliation engine in `generic.rs` and stage-specific modules. Employs **topological dependency ordering** (e.g. leaf/shared sub-flows applied in Tier 0 before dependent parent flows in Tier 1+) and **graceful 409 Conflict auto-adoption** to eliminate Keycloak flow race conditions. Supports optional pruning/deletion of orphaned remote resources via the `--prune` flag. **`apply` runs `validate` automatically** before any Keycloak API calls are made.
*   [`src/validate.rs`](src/validate.rs): Validates local configurations against expected structures and constraints. Checks include forbidden characters, duplicate aliases, valid execution requirement enums, subflow reference integrity, and **cycle detection (DFS)** in authentication sub-flow dependency graphs.
*   [`src/clean.rs`](src/clean.rs): Removes unreferenced or invalid configuration files from the workspace.
*   [`src/init.rs`](src/init.rs): Scaffolds the initial `kaji.toml` / `.kaji.toml` configuration files.
*   [`src/cli/`](src/cli/): Interactive CLI scaffolding menu. Styled with `dialoguer`'s `ColorfulTheme` and uses `FuzzySelect` for real-time query filtering. Auto-discovers existing realms in the workspace directory.
*   [`src/utils/secrets/`](src/utils/secrets/): Manages secret resolution (Env, HashiCorp Vault with cached lookup).
*   [`src/utils/yaml.rs`](src/utils/yaml.rs): Handles YAML serialization, sorting, and profile-specific deep-merging using `serde_yaml_ng`.
*   [`src/utils/ui.rs`](src/utils/ui.rs): CLI visual formatting, progress bars (`indicatif`), emojis, and styling (`DialoguerUi`).

---

## 🛠️ Staged Reconciliation Pipeline

To prevent race conditions, resources are reconciled sequentially across stages:

| Stage | Resources Applied | Category |
| :--- | :--- | :--- |
| **Stage 0** | Realms | Foundation |
| **Stage 1** | Identity Providers, Roles | Infrastructure |
| **Stage 2** | Clients, Client Scopes, Authentication Flows, Required Actions, Groups | Structure |
| **Stage 3** | Users, Authenticator Configs, Components, Keys | Data & Final Config |

### 🔐 Authentication Flows & Shared Sub-flows
Authentication flows in Keycloak frequently contain sub-flows, some of which may be shared across multiple top-level flows (e.g. MFA sub-flows). `kaji` handles these seamlessly:
1. **Validation & Cycle Detection**: `validate` validates requirement enums (`REQUIRED`, `ALTERNATIVE`, `OPTIONAL`, `CONDITIONAL`, `DISABLED`), verifies that sub-flow executions declare `flowAlias`, and runs depth-first search (DFS) cycle detection to prevent circular references before API calls.
2. **Topological Dependency Tiers**: During `apply`, flow files are automatically partitioned into dependency tiers (Tier 0: leaf/shared sub-flows without dependencies; Tier 1+: dependent parent flows). Each tier is applied in order, while resources within a tier are processed concurrently.
3. **Graceful 409 Conflict Auto-Adoption**: If Keycloak auto-creates a sub-flow container during parent flow import or due to shared references, `kaji` catches the HTTP 409 Conflict, invalidates the cache, queries Keycloak for the generated flow ID, adopts it, and reconciles the flow via PUT update without error.

---

## 🔄 Keycloak Resource Enrichment

During the reconciliation (`apply`) process, Keycloak may enrich resources with default values, read-only system attributes, or server-assigned identifiers (IDs). When `kaji` detects differences between the local representation and the enriched one returned by Keycloak:
1. It recursively maps any user-defined secret placeholders (e.g. `${VAR_NAME}`) from the original local file to the enriched representation to prevent them from being lost or overwritten by redacted/actual secret values.
2. It prompts the user (defaulting to Yes) to update the local representation to match the enriched Keycloak representation.
3. If the `--yes` (`-y`) option flag is passed, the update is accepted automatically without prompting.
4. Any newly generated secrets (such as client secrets) are extracted and appended to the secrets file.

---

## 🌍 Environment Profiles & Overlays

Multi-environment configurations are managed using the `--profile` (`-p`) flag:

### Profiles
Profiles are stored in the `profiles/` directory (e.g., `profiles/prod.yaml`). They define environment-specific connection details:
```yaml
server_url: "https://keycloak.prod.example.com"
client_id: "kaji-cli"
client_secret: "${PROD_KAJI_SECRET}"
secrets_file: ".secrets.prod"
```

### Overlays
For a resource `name.yaml`, `kaji` searches for `name.{profile}.yaml` and deep-merges it onto the base configuration at runtime. For example:
- Base: `workspace/my-realm/clients/my-app.yaml` (`redirectUris: ["http://localhost:3000/*"]`)
- Overlay: `workspace/my-realm/clients/my-app.prod.yaml` (`redirectUris: ["https://app.example.com/*"]`)

When running with `--profile prod`, `kaji` deep-merges the overlay onto the base configuration.

---

## ⚙️ Project Configuration File (`kaji.toml` / `.kaji.toml`)

Project connection defaults, request timeouts, and workspace parameters can be declared inside `kaji.toml` or `.kaji.toml` files in the current working directory. The configuration file is parsed at startup and merged with command-line inputs.

### Architecture & Pipeline
1. **Schema Definition**: The `Config` struct is defined in [`src/args.rs`](src/args.rs). It represents optional fields for Keycloak connection credentials, vault parameters, workspace folder, and request timeouts.
2. **File Lookup**: In [`src/lib.rs`](src/lib.rs), `load_config_file` checks for:
   * A custom config path specified via `--config` CLI flag or `KAJI_CONFIG` env var.
   * `kaji.toml` in the current working directory.
   * `.kaji.toml` in the current working directory.
3. **Merging Logic**: In `run_app` in [`src/lib.rs`](src/lib.rs), the loaded `Config` is merged into the parsed CLI `Cli` struct.

Settings are resolved in the following precedence order:
1. **CLI Flags** (highest)
2. **Profile Configuration**
3. **Environment Variables**
4. **TOML Configuration**
5. **Fallback Defaults** (lowest)

---

## 🔐 Secret Management & Resolution

Secrets are managed via the `SecretResolver` trait using three resolution strategies:
*   **EnvResolver**: Resolves `${VAR_NAME}` from the environment or a `.secrets` file.
*   **VaultResolver**: Resolves `${vault:mount/path#field}` from HashiCorp Vault KV2. Utilizes a thread-safe in-memory cache (`tokio::sync::Mutex<HashMap>`) to avoid duplicate/redundant HTTP GET requests to Vault for the same secret path during planning/applying.
*   **CompositeResolver**: Chains resolvers in prioritized order.

### Inspection Masking Heuristic
During `inspect`, any field matching secret patterns is masked using the placeholder `${KEYCLOAK_<RESOURCE_TYPE>_<RESOURCE_NAME>_<FIELD_NAME>}` and exported to `.secrets`:
*   Contains `secret` (case-insensitive)
*   Contains `password`
*   Matches exactly `value` (for certain component configurations)
*   Matches exactly `hashedValue`

---

## 🛠️ Adding a New Resource Support

To support a new Keycloak resource (e.g., "Event Listeners"):

1.  **Update [`src/models.rs`](src/models.rs)**:
    - Add the `struct` for the resource.
    - Implement `KeycloakResource` (for name/ID handling and API paths).
    - Implement `ResourceMeta` (to define labels and secret prefixes).
2.  **Update [`src/inspect.rs`](src/inspect.rs)**: Add a `spawn_inspect::<NewResourceRepresentation>(...)` call in the `inspect_realm` function.
3.  **Update [`src/plan/mod.rs`](src/plan/mod.rs)**: Add the new resource to `plan_single_realm` using `generic::plan_resources`.
4.  **Update [`src/apply/mod.rs`](src/apply/mod.rs)**: Add the new resource to the appropriate stage in `apply_single_realm` using `generic::apply_resources`.
5.  **Update [`src/validate.rs`](src/validate.rs)**: (Optional) Add specific validation rules.
6.  **Update [`src/cli/`](src/cli/)**: (Optional) Add interactive scaffolding for the new resource.

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

## 📜 Coding Conventions

1.  **Rust Edition**: Use Rust 2024 (as defined in `Cargo.toml`).
2.  **Asynchronous by Default**: All I/O and API operations must use `tokio`.
3.  **Concurrency**: Use `tokio::task::JoinSet` to parallelize independent resource operations. Avoid blocking Tokio worker threads.
4.  **Generic Abstractions**: Prefer using the generic CRUD methods in `KeycloakClient` and the `KeycloakResource`/`ResourceMeta` traits to avoid boilerplate.
5.  **Error Handling**: Use `anyhow::Context` for descriptive error chains, including specific resource identifiers (e.g., realm name).
6.  **Formatting**: Run `cargo fmt --all -- --check` before every commit and ensure all formatting issues are resolved.
7.  **Clippy**: Ensure `cargo clippy -- -D warnings` passes without warnings.
8.  **Serialization**: Prefer `serde_yaml_ng` for YAML operations to ensure compatibility with modern YAML features.
9.  **Documentation Updates**: Always update relevant documentation (`README.md`, `AGENTS.md`, and `.jules/` guides) whenever you introduce new features, modify reconciliation stages, or alter module structures to prevent documentation drift.

---

## 🧪 Development & Quality Checklist

Before completing changes, agents **MUST** ensure the following suite executes successfully:

```bash
# 1. Format code check
cargo fmt --all -- --check

# 2. Clippy lints
cargo clippy -- -D warnings

# 3. Complete test suite
cargo test

# 4. Dependency security audit
cargo audit

# 5. Code coverage
cargo tarpaulin --out Xml

# 6. Benchmarks (if modifying plan/apply paths)
cargo bench
```

### Testing Strategy & Layout
*   **Unit Tests**: Located inline in modules (e.g., `src/utils/secrets.rs`).
*   **Integration Tests**: Located in [`tests/`](tests/). Uses local Axum mock servers (in `tests/common/mod.rs`) and mockito.
*   **Real Integration**: Run against a live Keycloak instance. See [`tests/real_integration_test.rs`](tests/real_integration_test.rs).
*   **Ultimate & Models Coverage**: [`tests/ultimate_coverage_test.rs`](tests/ultimate_coverage_test.rs) and [`tests/models_coverage_test.rs`](tests/models_coverage_test.rs) provide comprehensive checks for resource handling.
*   **Benchmarks**: Located in [`benches/`](benches/). Used to monitor performance for large workspaces with thousands of files.

---

## 📂 Specialized Agent Guidelines (`.jules/`)

Detailed technical deep-dives for specialized topics are located under the [`.jules/`](.jules/) directory:

*   **[Testing & Validation](.jules/testing.md)**: Details the testing strategy (Axum-based mock servers, unit/integration test rules, and cargo-tarpaulin coverage).
*   **[Performance Guidelines](.jules/performance.md)**: Rules for writing async-first code, preventing thread blocking, and working with Criterion benchmarks.
*   **[Security Guidelines](.jules/security.md)**: Explains secret resolution flow, custom debug logs obfuscation, masking rules, and safe file permissions.
*   **[Architecture Guidelines](.jules/architect.md)**: Staged reconciliation pipeline architecture and topological dependency ordering.
*   **[Jules Instructions](.jules/instructions.md)**: Context for Google Jules and automated agent workflows.

---

## 🚀 Future Roadmap

-   [x] Parallel reconciliation (apply changes concurrently for resources within a realm).
-   [x] Generic refactor for `inspect.rs`.
-   [x] Integration with HashiCorp Vault for secret resolution.
-   [ ] Support for custom SPIs and provider configurations.
-   [x] Support for multiple environment profiles (e.g., `prod.yaml`, `staging.yaml`).
-   [x] Generic refactor for `plan.rs` and `apply.rs` (similar to `inspect.rs`).

---

## 📚 Documentation Integrity Rule

AI agents **MUST** ensure that all project documentation is kept fully up to date. Whenever making changes to codebase features, reconciliation stages, modules, or models:
*   Immediately update relevant developer guides: [AGENTS.md](AGENTS.md) and files under [.jules/](.jules/).
*   Update [README.md](README.md) if user-facing CLI behavior, flags, or configuration options change.
*   Avoid duplicating sections or copy-pasting information across files; instead, reference/link to details in other files where possible.
