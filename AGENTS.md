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

*   [`src/client.rs`](src/client.rs): Wrapper around the Keycloak Admin REST API. Handles authentication and provides a **generic CRUD interface** for resources. All requests go through `execute`: a `Session` shared by every clone refreshes the access token before it expires (refresh-token grant, falling back to a new login) and once more on HTTP 401; HTTP 429/502/503/504 are retried with backoff (connection errors only for GET); a semaphore bounds in-flight requests (`with_concurrency`, `--concurrency`, default `DEFAULT_CONCURRENCY`). Paginated list endpoints (users, groups) are read with `get_paginated`. HTTPS is required except for local servers or with `with_allow_insecure_http`; admin tokens come from `with_auth_realm` (default `master`). Automatically extracts generated resource IDs from HTTP 201 `Location` response headers to prevent post-creation query storms. Creation goes through the `KeycloakResourceMapping::create` hook, which resource types can override (required actions are registered via `register-required-action` and then configured with PUT). `is_not_found` detects HTTP 404 errors (used to create missing realms).
*   [`src/models.rs`](src/models.rs): Strongly-typed Serde representations of Keycloak resources. Implements the `KeycloakResource` and `ResourceMeta` traits for generic resource management. Normalizes group identities (trimming leading slashes) to cleanly align local and remote representations.
*   [`src/inspect.rs`](src/inspect.rs): Scans the remote Keycloak instance and serializes resources into local workspace files using a parallelized pipeline. Server IDs are stripped (`clear_metadata`) except for components, whose IDs local child components reference in `parentId`. Routes file overwrite prompts through the `Ui` trait abstraction (`run_with_ui_and_secrets`), exporting discovered secrets to profile-configured secrets files. Supported CLI aliases: `sync`, `pull`, `export`.
*   [`src/plan/`](src/plan/): Calculates diffs and writes the plan. Server IDs (`id`, `containerId`, ...) are ignored in diffs because they are environment specific and never applied. For types whose update endpoint keeps omitted fields (`KeycloakResource::PARTIAL_UPDATES`: realm, clients, client scopes, users — verified on 26.8.0) only keys declared locally are compared (`print_resource_diff`); roles, groups and identity providers reset omitted fields, so omissions there are shown as removals. Empty objects/arrays are treated like absent fields, and array elements are paired by identity (`id`, `name`, ...) when projecting partial files. Uses the generic planning engine in `generic.rs`. Pre-validates the workspace via `validate::run_with_profile` before initiating remote Keycloak queries. Receives `verbose: bool` directly in `PlanArgs` without static mutable state. Supports collapsed unified diff formatting (3 context lines) by default, `--verbose` full diff view, and interactive expansion choices during confirmation. Also runs a **sub-flow check** for authentication flows: identifies shared sub-flows and explains topological staging and auto-adoption. Lists remote resources not declared locally (`PlanSummary::orphaned`, deleted only by `apply --prune`). [`plan_file.rs`](src/plan/plan_file.rs) defines the versioned `.kajiplan` format: workspace-relative paths, the profile, and a sha256 of each base file plus overlay. `run_with_outcome(args, write_plan)` lets `drift` plan without touching `.kajiplan`; `drift` returns `DriftDetected`, which the binary maps to exit code 2.
*   [`src/apply/`](src/apply/): Reconciles resources. Uses the generic reconciliation engine in `generic.rs` and stage-specific modules. Leverages generated IDs from `Location` headers during creation to eliminate unnecessary full-list queries. Employs **topological dependency ordering** (e.g. leaf/shared sub-flows applied in Tier 0 before dependent parent flows in Tier 1+) and **graceful 409 Conflict auto-adoption** to eliminate Keycloak flow race conditions. Serializes interactive user confirmations and `.secrets` file updates across concurrent tasks using a shared prompt mutex, and handles JSON placeholder restoration via segmented path navigation (`PathSegment`). Supports optional pruning/deletion of orphaned remote resources via the `--prune` flag; `is_protected_resource` uses server flags (`builtIn`, `serviceAccountClientId`) plus lists of default clients, scopes and roles, never unregisters required actions, and never prunes `master` users. Prune runs even when a resource directory is empty. Realms that do not exist are created (POST) instead of updated. Components ([`src/apply/components.rs`](src/apply/components.rs)) are matched by the portable key `(providerType, subType, name, parent)` via `ComponentResolver` (shared with plan); local `parentId`s are resolved through local component files, rewritten to the target realm or parent component ID on write, and realm-level components are applied before child components (e.g. LDAP mappers). `apply` verifies `.kajiplan` (profile and content hashes) before using it. **`apply` runs `validate` automatically** before any Keycloak API calls are made.
*   [`src/apply/relations.rs`](src/apply/relations.rs): Relationships Keycloak's main endpoints ignore (verified on 26.8.0), reconciled through `KeycloakResourceMapping::post_save` and loaded for plan/inspect through `load_relations` (plan only loads what the local file declares): client default/optional scope links, user `groups`/`realmRoles`/`clientRoles`, group role mappings and nested `subGroups`, role `composites` (`{realm: [...], client: {clientId: [...]}}`). A relationship key that is absent from a file is not managed.
*   [`src/apply/client_roles.rs`](src/apply/client_roles.rs) / [`src/plan/client_roles.rs`](src/plan/client_roles.rs): Client roles stored as `clients/<sanitized clientId>/roles/<role>.yaml` (inspect, plan, apply, prune; roles of system clients are never pruned) and the role composites pass. Role composites are stripped from role create/update bodies (`pre_save`) and reconciled once all roles exist (`DEFERRED_RELATIONS`).
*   [`src/validate.rs`](src/validate.rs): Validates local configurations against expected structures and constraints. Supports environment profiles (`run_with_profile`), skipping partial standalone overlays (`*.{profile}.yaml`) and deep-merging them when a profile is specified. Checks include `realm.yaml`'s `realm` matching its directory name (otherwise apply would rename the realm), forbidden characters, duplicate aliases, valid execution requirement enums, subflow reference integrity, and **cycle detection (DFS)** in authentication sub-flow dependency graphs.
*   [`src/clean.rs`](src/clean.rs): Removes realm directories (all discovered realms, or `--realms`) and `.kajiplan`; profiles, secrets files and anything else in the workspace are never deleted. Realm names are validated with `utils::validate_realm_name` (also applied to `--realms` globally and to realm names discovered by `inspect`).
*   [`src/init.rs`](src/init.rs): Scaffolds the initial `kaji.toml` / `.kaji.toml` configuration files.
*   [`src/cli/`](src/cli/): Interactive CLI scaffolding menu. Styled with `dialoguer`'s `ColorfulTheme` and uses `FuzzySelect` for real-time query filtering. Auto-discovers existing realms in the workspace directory. Supports key rotation in both `keys/` and `components/` directories.
*   [`src/utils/secrets/`](src/utils/secrets/): Manages secret resolution (Env, HashiCorp Vault with cached lookup). Resolves any non-vault environment variable from `.secrets` or process environment and enforces error reporting on missing placeholders. Supports compound and embedded placeholders (`"${HOST}:${PORT}"`). Only `${UPPER_SNAKE_CASE}` and `${vault:...}` are placeholders (`parse_segments`, `is_placeholder_name`); Keycloak localization keys such as `${client_account}` stay literal and `$${X}` escapes a placeholder. `merge_secrets_content` merges secrets files by key. Diffs show secrets as `<redacted:sha256-prefix>` (changes stay visible, nothing leaks); remote `**********` masks are excluded from comparisons; string arrays under secret keys (component configs) are masked and extracted too.
*   [`src/utils.rs`](src/utils.rs): Common utilities, including `discover_realms` for unified workspace realm discovery filtering out `.*`, `profiles`, and `target`. `write_secure` writes atomically (0600 temp file + rename). `join_all_tasks` always awaits every task and aggregates all failures (never aborts in-flight siblings).
*   [`src/utils/yaml.rs`](src/utils/yaml.rs): Handles YAML serialization, sorting, and profile-specific deep-merging using `serde_yaml_ng`. Unifies `.yaml` and `.yml` extension support across all resource loaders, overlays, and pruning. `is_overlay_file` only treats `<stem>.<profile>.yaml` as an overlay when the base `<stem>.yaml|yml` exists; `find_overlay_path` locates the active overlay.
*   [`src/utils/ui.rs`](src/utils/ui.rs): CLI visual formatting, progress bars (`indicatif`), emojis, and styling (`DialoguerUi`). All progress bars share one `MultiProgress`; `DialoguerUi` prompts run inside `suspend_progress` so bars never draw over them, and `report`/`log_line` fall back to stderr when bars are hidden (non-terminal output such as CI logs).

---

## 🛠️ Staged Reconciliation Pipeline

To prevent race conditions, resources are reconciled sequentially across stages:

| Stage | Resources Applied | Category |
| :--- | :--- | :--- |
| **Stage 0** | Realm (created if missing; flow bindings to flows not yet on the server are deferred) | Foundation |
| **Stage 1** | Roles, Client Scopes, Required Actions | Infrastructure |
| **Stage 2** | Authentication Flows | Structure |
| **Stage 3** | Identity Providers (reference flows), Clients (reference client scopes and flows; scope links reconciled) | Integration |
| **Stage 4** | Client roles (`clients/<clientId>/roles/`), role composites, then Groups (sub-groups and role mappings) | Authorization |
| **Stage 5** | Users (groups and role mappings), Authenticator Configs, Components, Keys | Data & Final Config |
| **Stage 6** | Realm deferred flow bindings (`browserFlow`, ...) and realm enrichment sync | Finalization |

Keycloak rejects (HTTP 500) realms and identity providers that reference a flow that does not exist, and resolves `defaultClientScopes` only against existing scopes; the order above guarantees references exist before they are used. See `realm::apply_realm`/`realm::finish_realm` in [`src/apply/realm.rs`](src/apply/realm.rs).

### 🔐 Authentication Flows & Shared Sub-flows
Authentication flows in Keycloak frequently contain sub-flows, some of which may be shared across multiple top-level flows (e.g. MFA sub-flows). `kaji` handles these seamlessly:
1. **Validation & Cycle Detection**: `validate` validates requirement enums (`REQUIRED`, `ALTERNATIVE`, `OPTIONAL`, `CONDITIONAL`, `DISABLED`), verifies that sub-flow executions declare `flowAlias`, and runs depth-first search (DFS) cycle detection to prevent circular references before API calls.
2. **Topological Dependency Tiers**: During `apply`, flow files are automatically partitioned into dependency tiers (Tier 0: leaf/shared sub-flows without dependencies; Tier 1+: dependent parent flows). Each tier is applied in order, while resources within a tier are processed concurrently.
3. **Graceful 409 Conflict Auto-Adoption**: If Keycloak auto-creates a sub-flow container during parent flow import or due to shared references, `kaji` catches the HTTP 409 Conflict, invalidates the cache, queries Keycloak for the generated flow ID, adopts it, and reconciles the flow via PUT update without error.
4. **Remote representation**: `GET /authentication/flows` only lists top-level flows, already in Keycloak's export format (`authenticator`, `authenticatorConfig` alias, `flowAlias`, `requirement`, `priority`), the same format as flow files. Sub-flows are discovered through the `/flows/{alias}/executions` rows (`AuthenticationExecutionInfoRepresentation`: `providerId`, `authenticationConfig` ID, `flowId`, `level`) and fetched by ID, so `inspect` exports them as separate `topLevel: false` files. Keycloak's misspelled `autheticatorFlow` duplicate is dropped on deserialization.
5. **Execution reconciliation** ([`src/apply/flow_executions.rs`](src/apply/flow_executions.rs)): `POST`/`PUT /authentication/flows` ignore executions, so after a flow is created/updated the `KeycloakResourceMapping::post_save` hook matches declared executions to the flow's direct children (by provider or sub-flow alias, in order), deletes undeclared ones, adds missing ones (`POST /authentication/executions`; sub-flows applied in an earlier tier are linked by ID via `flowId`, others are created inline), and updates requirement/priority (`PUT /flows/{alias}/executions`). Executions without `priority` get 10, 20, 30, ... Built-in flows only accept requirement/priority changes. A flow file without `authenticationExecutions` leaves remote executions untouched. Standalone sub-flows are invisible to Keycloak's flow list until linked, so `KeycloakClient::remember_flow_id` records the IDs of flows applied in the run.
6. **Authenticator configs** (Stage 4) are created for the execution that references them (`POST /executions/{id}/config`, ID returned in the `Location` header). Keycloak ties a config to exactly one execution.

---

## 🔄 Keycloak Resource Enrichment

During the reconciliation (`apply`) process, Keycloak may enrich resources with default values, read-only system attributes, or server-assigned identifiers (IDs). When `kaji` detects differences between the local representation and the enriched one returned by Keycloak:
1. If the file has an active profile overlay, write-back is skipped: the local value is base + overlay merged, and writing it would leak profile values into the base file.
2. It recursively maps user-defined secret placeholders (including embedded ones such as `https://${HOST}/cb`) from the local file to the enriched representation. A placeholder is restored only if the enriched value at that path equals the resolved local value. Inside arrays that Keycloak reordered, the matching element is located instead. Keycloak's `**********` masks are replaced with the local value, and server identifiers (`id`, `internalId`, `parentId`, `containerId`) are never added when the local file did not declare them, nor replaced when it did (they are environment specific). Keys declared locally but missing from the server response (write-only fields, references applied in a later stage such as an execution's `authenticatorConfig`) are kept; array elements are paired by identity key (`authenticator`, `flowAlias`, `clientId`, `name`, `alias`).
3. It prompts the user (defaulting to Yes) to update the local representation to match the enriched Keycloak representation.
4. If the `--yes` (`-y`) option flag is passed, the update is accepted automatically without prompting.
5. Newly generated secrets (such as client secrets) are extracted and appended to the secrets file. Only secrets the written file actually references are added.

---

## 🌍 Environment Profiles & Overlays

Multi-environment configurations are managed using the `--profile` (`-p`) flag:

### Profiles
Profiles are stored in the `profiles/` directory (e.g., `profiles/prod.yaml` or `profiles/prod.yml`). They define environment-specific connection details:
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

When running with `--profile prod`, `kaji` deep-merges the overlay onto the base configuration. A file is only an overlay when its base file exists (see [README](README.md#2-use-overlays)).

Placeholders in profile connection fields are resolved by `resolve_profile_placeholders` in [`src/lib.rs`](src/lib.rs), using the environment and the profile's `secrets_file`.

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

Settings are resolved in the following precedence order (implemented by `pick`/`ConnectionSettings` in [`src/lib.rs`](src/lib.rs); `Cli::explicit_args` records, via clap's `value_source`, which values came from real command-line flags rather than environment variables):
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
*   **Real Integration**: Runs against a live Keycloak **26.8.0** when `KAJI_IT_URL` is set (skipped otherwise). See [`tests/real_integration_test.rs`](tests/real_integration_test.rs), the [README](README.md#live-keycloak-tests) for commands, and the `Live Keycloak` workflow for CI. Confirmed-but-unfixed defects are `#[ignore = "known bug: ..."]` tests that reference the Known Issues below. Keep `tests/common/mod.rs` mock payloads consistent with what the live server returns.
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

## 🐞 Known Issues

Confirmed defects that are not fixed yet. The numbers are referenced by `#[ignore = "known bug: ..."]` tests in [`tests/real_integration_test.rs`](tests/real_integration_test.rs). Items marked "verified" were reproduced against Keycloak 26.8.0.

**Keycloak API correctness**

**Robustness / UX**
*   **#20** Prune is not supported for components, keys and authenticator configs (deleting key providers or user storage is too risky to automate).
*   **#27** `main.rs` loads `.env` and `.secrets` from the current directory into the process environment, and environment variables also satisfy workspace placeholders: values from another environment's `.secrets` in the CWD can silently fill missing placeholders of a profile.

---

## 📚 Documentation Integrity Rule

AI agents **MUST** ensure that all project documentation is kept fully up to date. Whenever making changes to codebase features, reconciliation stages, modules, or models:
*   Immediately update relevant developer guides: [AGENTS.md](AGENTS.md) and files under [.jules/](.jules/).
*   Update [README.md](README.md) if user-facing CLI behavior, flags, or configuration options change.
*   Avoid duplicating sections or copy-pasting information across files; instead, reference/link to details in other files where possible.
