# 🧭 Google Jules Instructions for kaji

Welcome! This directory contains instructions and context to help Google Jules perform high-quality code generation, testing, performance optimizations, and security improvements for `kaji`.

## Project Overview

`kaji` is a declarative configuration management tool for Keycloak (similar to Kubernetes controllers). It manages configurations by:
1.  **Reading Desired State**: Defined in local YAML files. Supports environment-specific overlays (e.g. `realm.yaml` deep-merged with `realm.prod.yaml`).
2.  **Reading Current State**: Pulled from the Keycloak Admin API.
3.  **Generating Diffs**: Done by the diff engine (`src/plan/mod.rs`), producing a list of planned actions in `.kajiplan`.
4.  **Reconciling**: Done by the reconciler (`src/apply/mod.rs`), calling the Keycloak API in a dependency-aware staged pipeline.

---

## 🗂️ Developer Resources

Detailed guidelines for specific aspects of the codebase:

*   **[Testing Context](testing.md)**: Standard testing conventions, Axum mock server, code coverage (tarpaulin), and benchmarking.
*   **[Performance Context](performance.md)**: Asynchronous architecture, parallel execution with `JoinSet`, avoiding blocking, and profiling benchmarks.
*   **[Security Context](security.md)**: Secret resolution, debug log obfuscation, inspection masking heuristics, and safe file permissions.

---

## 🛠️ General Coding Conventions

1.  **Rust Edition**: Use Rust 2024 (as defined in `Cargo.toml`).
2.  **Async/Tokio**: All network and disk operations must be async-first, relying on Tokio.
3.  **Generic Implementations**:
    *   To support new resources, implement `KeycloakResource` and `ResourceMeta` in [src/models.rs](src/models.rs).
    *   Avoid custom API wrappers where generic methods in `KeycloakClient` can be used.
4.  **Error Handling**:
    *   Use `anyhow` for errors.
    *   Decorate actions with descriptive `.context(...)` calls to provide clear debugging trace context.
5.  **Documentation Integrity**: Always update relevant markdown files (`README.md`, `GEMINI.md`, `JULES.md`, `AGENTS.md`, and `.jules/*.md`) whenever codebase features, models, stages, or structures change to prevent document drift.
6.  **Quality Check List**:
    Before completing any task, ensure the following commands run successfully:
    ```bash
    cargo fmt --all -- --check
    cargo clippy -- -D warnings
    cargo test
    ```
