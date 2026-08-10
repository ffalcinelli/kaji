# 🧭 AGENTS.md — AI Agent Developer Guide for kaji

Welcome! This document serves as the developer guide and instructions directory for AI coding assistants (agents) working on the `kaji` repository. It outlines the project's architecture, core modules, staged reconciliation pipeline, profile overlays, and development commands.

---

## 🏛️ Architecture Overview

`kaji` follows a **Reconciliation Loop** pattern, similar to Kubernetes controllers, to steer Keycloak server state to a stable, declared state.

1. **Desired State**: Defined in local YAML configurations within the workspace. Supports environment-specific overlays (e.g. `realm.yaml` deep-merged with `realm.prod.yaml`).
2. **Current State**: Fetched dynamically from the Keycloak Admin API.
3. **Diff Engine (`src/plan/`)**: Compares Desired and Current states to identify what needs to be created, updated, or deleted. It generates a `.kajiplan` file listing the files with pending changes.
4. **Reconciler (`src/apply/`)**: Executes API requests to resolve the differences. It uses a **Staged Reconciliation Pipeline** to apply changes in order of dependencies.

---

## 🏗️ Core Modules

All business logic is located in the `src/` directory:

*   [`src/client.rs`](src/client.rs): Wrapper around the Keycloak Admin REST API. Handles authentication and provides a **generic CRUD interface** for resources.
*   [`src/models.rs`](src/models.rs): Strongly-typed Serde representations of Keycloak resources. Implements the `KeycloakResource` and `ResourceMeta` traits.
*   [`src/inspect.rs`](src/inspect.rs): Scans the remote Keycloak instance and serializes resources into local workspace files using a parallelized pipeline. Supported CLI aliases: `sync`, `pull`, `export`.
*   [`src/plan/`](src/plan/): Calculates diffs and writes the plan. Uses the generic planning engine in `generic.rs`. Supports collapsed unified diff formatting (3 context lines) by default, `--verbose` full diff view, and interactive expansion choices during confirmation.
*   [`src/apply/`](src/apply/): Reconciles resources. Uses the generic reconciliation engine in `generic.rs` and stage-specific modules. Supports optional pruning/deletion of orphaned remote resources via the `--prune` flag.
*   [`src/validate.rs`](src/validate.rs): Validates local configurations against expected structures and constraints.
*   [`src/clean.rs`](src/clean.rs): Removes unreferenced or invalid configuration files from the workspace.
*   [`src/init.rs`](src/init.rs): Scaffolds the initial `kaji.toml` / `.kaji.toml` configuration files.
*   [`src/cli/`](src/cli/): Interactive CLI scaffolding menu. Styled with `dialoguer`'s `ColorfulTheme` and uses `FuzzySelect` for real-time query filtering. Auto-discovers existing realms in the workspace directory.
*   [`src/utils/secrets/`](src/utils/secrets/): Manages secret resolution (Env, HashiCorp Vault).
*   [`src/utils/yaml.rs`](src/utils/yaml.rs): Handles YAML serialization, sorting, and profile-specific deep-merging.
*   [`src/utils/ui.rs`](src/utils/ui.rs): CLI visual formatting, progress bars, emojis, and styling. Contains DialoguerUi which maps console prompts to ColorfulTheme and FuzzySelect.

---

## 🛠️ Staged Reconciliation Pipeline

To prevent race conditions, resources are reconciled sequentially across stages:

| Stage | Resources Applied | Category |
| :--- | :--- | :--- |
| **Stage 0** | Realms | Foundation |
| **Stage 1** | Identity Providers, Roles | Infrastructure |
| **Stage 2** | Clients, Client Scopes, Authentication Flows, Required Actions, Groups | Structure |
| **Stage 3** | Users, Authenticator Configs, Components, Keys | Data & Final Config |

## 🔄 Keycloak Resource Enrichment

During the reconciliation (`apply`) process, Keycloak may enrich resources with default values, read-only system attributes, or server-assigned identifiers (IDs). When `kaji` detects differences between the local representation and the enriched one returned by Keycloak:
1. It recursively maps any user-defined secret placeholders (e.g. `${VAR_NAME}`) from the original local file to the enriched representation to prevent them from being lost or overwritten by redacted/actual secret values.
2. It prompts the user (defaulting to Yes) to update the local representation to match the enriched Keycloak representation.
3. If the `--yes` (`-y`) option flag is passed, the update is accepted automatically without prompting.
4. Any newly generated secrets (such as client secrets) are extracted and appended to the secrets file.

---

## 🌍 Environment Profiles & Overlays

Multi-environment configurations are managed using the `--profile` (`-p`) flag:

1. **Profiles**: Contained in the `profiles/` directory (e.g., `profiles/prod.yaml`). Define server URLs, client credentials, and environment-specific secret files.
2. **Overlays**: For a resource `name.yaml`, `kaji` searches for `name.{profile}.yaml` and deep-merges it onto the base configuration at runtime.

---

## ⚙️ Project Configuration File (`kaji.toml` / `.kaji.toml`)

Project connection defaults, request timeouts, and workspace parameters can be declared inside `kaji.toml` or `.kaji.toml` files in the current working directory. The configuration file is parsed at startup and merged with command-line inputs.

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
*   **VaultResolver**: Resolves `${vault:mount/path#field}` from HashiCorp Vault KV2.
*   **CompositeResolver**: Chains resolvers in prioritized order.

During `inspect`, any field matching secret patterns (e.g. contains `secret`, `password`, or matches exactly `value`, `hashedValue`) is masked using the placeholder `${KEYCLOAK_<RESOURCE_TYPE>_<RESOURCE_NAME>_<FIELD_NAME>}` and exported to `.secrets`.

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

# 4. Benchmarks (if modifying plan/apply paths)
cargo bench
```

### Testing Layout
*   **Unit Tests**: Located inline in modules (e.g., `src/utils/secrets.rs`).
*   **Integration Tests**: Located in [`tests/`](tests/). Uses local Axum mock servers (in `tests/common/mod.rs`) and mockito.
*   **Real Integration**: Run against a live Keycloak instance. See `tests/real_integration_test.rs`.

---

## 📚 Documentation Integrity Rule

AI agents **MUST** ensure that all project documentation is kept fully up to date. Whenever making changes to codebase features, reconciliation stages, modules, or models:
*   Immediately update relevant developer guides: [AGENTS.md](AGENTS.md), [GEMINI.md](GEMINI.md), [JULES.md](JULES.md), and files under [.jules/](.jules/).
*   Update [README.md](README.md) if user-facing CLI behavior, flags, or configuration options change.
*   Avoid duplicating sections or copy-pasting information across files; instead, reference/link to details in other files where possible.

