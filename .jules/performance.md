# ⚡ Performance Guidelines for kaji

This document provides context and guidelines for Google Jules to maintain and improve the performance of `kaji`.

## Core Performance Principles

1.  **Asynchronous by Default**:
    *   All external I/O (network requests to Keycloak, file reading/writing) must be non-blocking.
    *   Use `tokio::fs` for file system operations where appropriate, and `reqwest` for asynchronous HTTP requests.
2.  **Concurrency and Parallelism**:
    *   Use `tokio::task::JoinSet` to run independent actions in parallel.
    *   Reconciliation and planning are parallelized at the realm level, and also within stages of resource reconciliations where dependencies allow.
    *   Always leverage tokio's green threads to prevent sequential blocking calls during deep inspection, planning, or applying changes.
3.  **Preventing Thread Blocking**:
    *   Never use synchronous blocking I/O (like `std::fs` operations or blocking HTTP clients) on the Tokio runtime.
    *   If synchronous blocking logic is absolutely necessary (e.g. some complex parsing/validation libraries), wrap it using `tokio::task::spawn_blocking`.
4.  **YAML Deep-Merging Overhead**:
    *   Overlays are deep-merged using [src/utils/yaml.rs](src/utils/yaml.rs).
    *   Keep merging algorithms efficient. Avoid deep cloning of complex YAML trees where possible. Use references or move values.
5.  **Network Query Efficiency & Location Header Extraction**:
    *   When creating resources via Keycloak's Admin API, Keycloak responds with `HTTP 201 Created` and a `Location: .../{id}` header.
    *   `KeycloakClient::create_resource` extracts this ID directly to avoid subsequent full-list GET queries (`get_resources::<T>()`), preventing $O(N)$ query storms during concurrent reconciliation.

## Performance Benchmarking

`kaji` uses `criterion` for benchmarking key execution flows.

*   **Location**: Benchmark files are located in [benches/](benches).
    *   `bench.rs`: Base planning benchmarks.
    *   `bench_authenticator_config.rs`: Authenticator config reconciliation benchmarks.
    *   `bench_inspect.rs`: Realm deep inspection performance.
    *   `bench_apply.rs`: Reconciliation performance.
    *   `models_bench.rs`: Model parsing/serialization performance.
*   **Running Benchmarks**:
    ```bash
    cargo bench
    ```
*   **Writing Benchmarks**:
    *   If you introduce a new complex resource type or a new custom secret resolver, add matching benchmarking cases to track CPU usage and memory allocations.
