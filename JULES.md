# 🧭 Google Jules Context for kaji

This repository uses a declarative reconciliation pattern to manage Keycloak resources. 

To help Google Jules make high-quality changes, run tests, ensure security, and optimize performance, the following specific guidelines have been established:

## 📂 Jules Guidelines Index

Detailed guidelines are located under the `.jules/` directory:

*   **[Testing & Validation](.jules/testing.md)**: Details the testing strategy (Axum-based mock servers, unit/integration test rules, and cargo-tarpaulin coverage).
*   **[Performance Guidelines](.jules/performance.md)**: Rules for writing async-first code, preventing thread blocking, and working with Criterion benchmarks.
*   **[Security Guidelines](.jules/security.md)**: Explains the secret resolution flow, custom debug logs obfuscation, masking rules, and safe file permissions.

## 🏗️ Architecture & Operations Summary

*   **Diff Engine & Reconciler**: The workflow calculates differences between the desired local YAML state and the Keycloak Admin API state, generating a `.kajiplan` file, and applying it sequentially in stages (Stage 0: Realms; Stage 1: IDPs/Roles; Stage 2: Clients/Scopes/Flows/Groups; Stage 3: Users/Components/Keys).
*   **Overlay Deep Merging**: Keycloak resources can be customized per-environment by applying overlays (e.g. `roles/admin.prod.yaml` over `roles/admin.yaml`).
*   **Quality Commands**:
    *   **Format Check**: `cargo fmt --all -- --check`
    *   **Linter Check**: `cargo clippy -- -D warnings`
    *   **Test Suite**: `cargo test`
    *   **Dependency Audit**: `cargo audit`
    *   **Coverage**: `cargo tarpaulin --out Xml`
    *   **Benchmarks**: `cargo bench`
