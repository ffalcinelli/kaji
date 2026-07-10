# 🧪 Testing Guidelines for kaji

This document provides context and guidelines for Google Jules when writing, refactoring, or running tests in the `kaji` codebase.

## Test Directory Structure

*   **Unit & Model Tests**: Located within specific source modules or in `tests/models_coverage_test.rs`, `tests/validate_test.rs`, and `tests/ui_coverage_test.rs`.
*   **Mocked Integration Tests**: Located in `tests/`. These tests simulate Keycloak API interactions using a local mock server.
    *   Example: [plan_test.rs](tests/plan_test.rs), [apply_test.rs](tests/apply_test.rs), [coverage_improvement_test.rs](tests/coverage_improvement_test.rs).
*   **Real Integration Tests**: [real_integration_test.rs](tests/real_integration_test.rs) requires a live Keycloak server (configured via environment variables or profiles).
*   **Benchmarks**: Located in the [benches/](benches) folder.

## Key Testing Tools & Frameworks

1.  **Axum-based Mock Server**:
    *   Instead of calling a live Keycloak instance, integration tests spin up a mock server using `start_mock_server()` defined in [tests/common/mod.rs](tests/common/mod.rs).
    *   This server listens on an ephemeral port on `127.0.0.1` and mocks the Keycloak Admin HTTP endpoints (OIDC tokens, realms, clients, roles, groups, users, authentication flows, components, etc.).
    *   When writing new tests that involve API endpoints, extend the mock server in `tests/common/mod.rs` to support any additional HTTP routes/methods.
2.  **Tokio for Async Tests**:
    *   Use `#[tokio::test]` for asynchronous test cases.
3.  **Mockito**:
    *   Optionally used in older tests to mock external HTTP responses (though Axum is preferred for Keycloak API mocking).
4.  **Tempfile**:
    *   Use `tempfile::tempdir()` to create temporary workspaces for reading/writing configuration files during tests to avoid polluting the host system.
5.  **Cargo Tarpaulin**:
    *   Used to measure test coverage. Run coverage checks with:
        ```bash
        cargo tarpaulin --out Xml
        ```

## Guidelines for Writing Tests

*   **Mock Behavior**: When adding support for a new resource or Keycloak API endpoint, you **must** update the Axum mock server in [tests/common/mod.rs](tests/common/mod.rs) to handle the new routes and return expected mock responses.
*   **Isolate Filesystem Side Effects**: Always use `tempdir()` for testing file I/O operations (e.g. testing `plan`, `apply`, `inspect`, profile configuration parsing).
*   **Validate Model Debug Obfuscation**: If you introduce sensitive fields to models, write a test in [coverage_improvement_test.rs](tests/coverage_improvement_test.rs) (specifically under `test_models_debug_obfuscation`) to verify that `format!("{:?}", model)` obfuscates the sensitive values with `********`.
*   **Coverage Rules**: Every new feature or resource support should come with:
    1.  A validation test in [validate_test.rs](tests/validate_test.rs).
    2.  An integration test verifying planning and reconciliation in [plan_test.rs](tests/plan_test.rs) or [apply_test.rs](tests/apply_test.rs).
    3.  A model coverage test in [models_coverage_test.rs](tests/models_coverage_test.rs).
