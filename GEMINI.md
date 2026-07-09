# GEMINI.md - kaji (舵 — Helm/Rudder)

This document serves as the internal developer guide for `kaji`. It explains the architecture, design decisions, and workflows for extending the tool.

## 🏛️ Architecture Overview

`kaji` follows a **Reconciliation Loop** pattern, similar to Kubernetes controllers.

1.  **Desired State**: Defined in local YAML files within the workspace. Support for **Environment Profiles & Overlays** allows for multi-environment configurations (e.g., `realm.yaml` + `realm.prod.yaml`).
2.  **Current State**: Fetched from the Keycloak Admin API.
3.  **Diff Engine (`plan.rs`)**: Compares the two states to identify what needs to be Created, Updated, or Deleted. It generates a `.kajiplan` file in the workspace containing the list of files that have pending changes.
4.  **Reconciler (`apply.rs`)**: Executes the necessary API calls to bring the Current State in line with the Desired State. It uses **Dependency-Aware (Staged) Reconciliation** to ensure resources are applied in the correct order (e.g., Realms before Roles, Roles before Users).

### Core Modules

-   `src/client.rs`: Low-level wrapper for the Keycloak Admin REST API. Handles authentication and provides a **generic CRUD interface** for Keycloak resources.
-   `src/models.rs`: Serde-based representations of Keycloak resources. Defines the `KeycloakResource` and `ResourceMeta` traits for generic resource management.
-   `src/inspect.rs`: Deep-scans the remote Keycloak server and serializes resources into local files using a **generic, parallelized inspection pipeline**.
-   `src/plan/`: Contains the logic for calculating diffs. Uses a **generic planning engine** (`generic.rs`) for most resource types.
-   `src/apply/`: Contains the logic for applying changes. Uses a **generic reconciliation engine** (`generic.rs`) and a **staged application pipeline** to ensure reliability.
-   `src/utils/secrets/`: Manages secret resolution (Env, Vault, etc.).
-   `src/utils/yaml.rs`: Handles YAML deep-merging and profile-specific overlays.
-   `src/utils/ui.rs`: Centralized module for CLI output formatting, emoji management, and **indicatif progress bars**.

---

## 🛠️ Staged Reconciliation Pipeline

To prevent race conditions and ensure correct dependency handling, `apply` is executed in stages:

1.  **Stage 0**: Realms (Foundation).
2.  **Stage 1**: Identity Providers, Roles (Infrastructure).
3.  **Stage 2**: Clients, Client Scopes, Authentication Flows, Required Actions, Groups (Structure).
4.  **Stage 3**: Users, Components, Keys (Data & Final Config).

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
For any resource `resource.yaml`, `kaji` looks for `resource.{profile}.yaml` and deep-merges it onto the base configuration if that profile is active. This allows for environment-specific tweaks without duplicating entire files.

---

## 🔐 Secret Management Logic

Secret handling is managed via the `SecretResolver` trait, which allows for multiple resolution strategies:

- **EnvResolver**: Resolves `${VAR_NAME}` from the environment or a `.secrets` file.
- **VaultResolver**: Resolves `${vault:mount/path#field}` from a HashiCorp Vault KV2 engine.
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

---

## 🚀 Future Roadmap

-   [x] Parallel reconciliation (apply changes concurrently for resources within a realm).
-   [x] Generic refactor for `inspect.rs`.
-   [x] Integration with HashiCorp Vault for secret resolution.
-   [ ] Support for custom SPIs and provider configurations.
-   [ ] Support for multiple environment profiles (e.g., `prod.yaml`, `staging.yaml`).
-   [ ] Generic refactor for `plan.rs` and `apply.rs` (similar to `inspect.rs`).
