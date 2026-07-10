# 🔒 Security Guidelines for kaji

This document outlines the security architecture and guidelines that Google Jules must adhere to when modifying `kaji`.

## Security Philosophy

`kaji` manages infrastructure as code (IaC) configuration for Keycloak. Keycloak databases contain sensitive credential information (client secrets, LDAP passwords, DB user passwords, certificates, private keys). Therefore, **confidentiality and integrity** of configuration states and logs are critical.

## 1. Least Privilege
*   Ensure that any generated configurations or setup instructions guide users toward using the minimum necessary permissions/roles for the Keycloak client/user (e.g., restricted realm-management roles).
*   Avoid adding code that requires admin credentials when realm-level permissions are sufficient.

## 2. Secrets Management & Obfuscation

To avoid leaking secrets into source control or diagnostic logs, `kaji` employs two primary mechanisms:

### A. Secret Resolvers
*   All sensitive values in desired YAML states are represented by placeholders (e.g., `${MY_SECRET}` or `${vault:mount/path#field}`).
*   `kaji` uses the `SecretResolver` trait (implementations in [src/utils/secrets/](src/utils/secrets)) to interpolate secrets at runtime:
    *   `EnvResolver`: Resolves environment variables or reads from a local `.secrets` file (which is in `.gitignore` and must **never** be committed).
    *   `VaultResolver`: Resolves secrets directly from a HashiCorp Vault server.
    *   `CompositeResolver`: Chains multiple resolvers.
*   *Jules Guidelines*: If you add configuration properties that could contain secrets, always resolve them using the active `SecretResolver` instance.

### B. Custom Debug Obfuscation
*   To prevent sensitive values from being accidentally logged during debugging or panic traces, structs representing Keycloak resources that house secrets implement custom `Debug` traits.
*   Specifically, `IdentityProviderRepresentation`, `CredentialRepresentation`, and `ComponentRepresentation` in [src/models.rs](src/models.rs) override the standard `Debug` formatting to map sensitive config or value fields to `"********"`.
*   *Jules Guidelines*: If you add a new model representing sensitive resources, you **must** implement a custom `Debug` formatter that redacts these fields. Always write a corresponding test in [tests/coverage_improvement_test.rs](tests/coverage_improvement_test.rs) or [src/models.rs](src/models.rs) unit tests.

### C. Inspection Masking Heuristic
*   During realm inspection ([src/inspect.rs](src/inspect.rs)), `kaji` automatically masks configuration values whose keys match the following patterns:
    *   Contains `secret` (case-insensitive)
    *   Contains `password` (case-insensitive)
    *   Is exactly `value` or `hashedValue` (typical in Keycloak credentials/components)
*   These are replaced with `${KEYCLOAK_<RESOURCE_TYPE>_<RESOURCE_NAME>_<FIELD_NAME>}` to construct secure YAML files.
*   *Jules Guidelines*: If a new resource type is added with different secret field names, ensure they are captured by the masking heuristic.

## 3. Safe File Permissions
*   When `kaji` creates files locally (e.g., YAML configurations, state files), they must have restricted permissions.
*   On Unix-like systems, files containing or referencing potential secret structures must be created with `0o600` permissions (read/write for owner only).
*   Verify this behavior using tests like `test_cli_generated_files_are_secure` in [tests/security_verify.rs](tests/security_verify.rs).

## 4. Dependencies and Audit
*   GitHub actions run `cargo audit` in CI to detect vulnerabilities in dependencies.
*   Before adding or upgrading crates, run `cargo audit` locally to ensure no vulnerabilities are introduced.
