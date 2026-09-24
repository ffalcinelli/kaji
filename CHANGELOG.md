# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.4] - 2026-09-24
### Security
- **TOCTOU Vulnerability Fixes & Filesystem Hardening**: Removed redundant `fs::try_exists` checks across all file read, directory creation, and file deletion operations in favor of direct operations with explicit `ErrorKind::NotFound` matching, eliminating race conditions.
- **Data Race Elimination in Tests**: Replaced `unsafe { std::env::set_var/remove_var }` calls with isolated subprocess test runners to eliminate multithreaded environment mutation races.
- **Safe Secret Loading**: Loaded environment secrets in `EnvResolver` using scoped `dotenvy::from_path_iter` instead of mutating the process environment.
- **Inspect Secret Masking**: Added `hashedValue` to the secret field masking heuristic during Keycloak inspection.
- **Vault Path Sanitization**: Stripped leading slashes in Vault secret paths to prevent false-positive path traversal rejections.
- **Dependency Security Update**: Updated `rustls` dependency to resolve security advisory.

### Added
- **Authentication Flow Topological Staging & Auto-Adoption**:
  - Implemented topological dependency ordering (Tier 0 leaf/shared flows, Tier 1+ parent flows) to ensure correct creation sequence.
  - Added graceful HTTP 409 Conflict auto-adoption for auto-generated or shared sub-flows in Keycloak.
  - Added cycle detection (DFS), execution reference integrity checks (`flowAlias`), and requirement enum validation before API execution.
  - Added sub-flow check in `plan` to visualize topological dependencies and shared flows.
- **Compound & Embedded Secret Placeholders**: Added support for embedded and multi-variable placeholders (e.g. `"${HOST}:${PORT}"`) in secret resolution.
- **Cross-Extension Profile Overlays**: Unified `.yaml` and `.yml` extension handling across resource loaders, overlays, and pruning, allowing cross-extension overlay deep-merging (`name.{profile}.yaml` on `name.yml` and vice versa).
- **Interactive Review Mode for Components & Authenticator Configs**: Added interactive review mode support and prompt mutex synchronization across concurrent tasks during component and authenticator configuration reconciliation.
- **Key Rotation in `components/`**: Added key rotation support for both `keys/` and `components/` directories.
- **Workspace Pre-Validation in `plan` & `drift`**: Added automated workspace pre-validation with credential checks prior to initiating Keycloak API calls.
- **Actionable CLI Hints**: Added contextual guidance with recommended resolution steps when Keycloak authentication credentials or workspace directories are missing (suggesting `kaji init`).
- **Global CLI Flag Ergonomics**: Added `global = true` to top-level CLI flags, allowing options like `--server` or credentials to be placed after subcommands (e.g. `kaji plan --server ...`) and surfacing them in subcommand help menus.

### Changed
- **Workspace Scan & File I/O Optimization**: Parallelized directory metadata scanning using `tokio::task::JoinSet`, reducing workspace scan latency on large directories.
- **Memory & Allocation Efficiency**: Eliminated intermediate string allocations in `append_secrets` and switched from eager `format!` to lazy `.with_context(|| ...)` throughout error paths.

## [0.0.3] - 2026-08-11
### Security
- **Strict File Permissions**: Guaranteed restrictive file permissions on exported configuration files by removing `fs::write` fallbacks in `write_if_changed_with_mutex`.
- **Vault Path Traversal Prevention**: Fixed path traversal vulnerability in `VaultResolver` by implementing a chroot-style base URL containment check.

### Changed
- **N+1 API Call Optimization**: Eliminated N+1 request spikes when retrieving authenticator configurations by using a cached, stream-buffered (`buffered(10)`) approach for `AuthenticationFlow` execution fetching.
- **Redundant Disk I/O Reduction**: Optimized `append_secrets` performance by eliminating redundant file existence checks and disk re-reads.
- **Code Health & Refactoring**: Streamlined `authenticator_config` execution linking with idiomatic Rust iterators (`.is_some_and`, `.any`), replaced raw `unwrap()` calls in `generic.rs` with descriptive `expect()` messages, and cleaned up macro hygiene.

### Added
- **Test Suite Expansion**: Added comprehensive unit tests for `is_overlay_file` edge cases, `print_diff`, component indexing, primitive array sorting, and secret extraction within arrays.

## [0.0.2] - 2026-08-09
### Security
- **Secure File Writes**: Fixed TOCTOU race condition vulnerability on Unix systems in `write_secure` by setting permissions directly on open file descriptors.
- **Cross-Platform Security**: Enforced restricted file permissions on Windows during secure file writing.
- **TLS Enforcement**: Enforced HTTPS for all `KeycloakClient` and `VaultResolver` connections by default (except for `localhost` and `127.0.0.1`).
- **Credential Protection**: Redacted sensitive fields in `UserRepresentation` and `ClientRepresentation` `Debug` implementations to prevent accidental secret exposure in logs.
- **CLI Secret Safety**: Masked secret values in `clap` CLI environment variable displays.
- **Path Traversal Protection**: Sanitized path parsing in `VaultResolver`.

### Added
- **Project-Level Configuration (`kaji.toml` / `.kaji.toml`)**: Added support for local configuration files in project root or via `--config` / `KAJI_CONFIG`.
- **Collapsed & Interactive Diffs**: Unified diffs default to collapsed view (3 context lines) in `kaji plan` / `kaji drift`, with a `--verbose` flag for full diffs and interactive expansion choices during confirmation.
- **Graceful Timeout Handling**: Added configurable timeouts for unreachable Keycloak server instances.
- **CLI Visual Enhancements**: Standardized error display with `anyhow` context chains, routed progress/UI logs to `stderr`, and added visual hints.

### Changed
- **Performance Optimizations**: Added concurrent fetching of authentication flows and authenticator configs, generic resource caching in `KeycloakClient`, and optimized string/heap allocations in secret placeholder replacement.
- **Code Health & Refactoring**: Consolidated `plan::run` and `apply::run` parameters into structured context structs (`PlanArgs`, `ApplyArgs`, `ApplyContext`).
- **Test Coverage Expansion**: Added isolated unit and integration tests for client methods, identity providers, authentication flows, secrets appending, and edge cases.

## [0.0.1] - 2026-07-10
### Changed
- **Project Rebrand**: Renamed from `kcd` (Keycloak Configuration Drive) to `kaji` (舵, Japanese for *helm/rudder*). The new name reflects the tool's purpose — steering Keycloak configuration to a stable, declared state.
- Binary renamed from `kcd` to `kaji`.
- Plan artifact renamed from `.kcdplan` to `.kajiplan`.

## [2606.1.0] - 2026-06-05
### Added
- **Generic Reconciliation Engine**: Consolidated reconciliation logic for all resource types into a single, maintainable generic engine.
- **Environment Profiles**: Support for multiple environments (Dev, Staging, Prod) via `--profile` flag and `profiles/` directory.
- **Resource Overlays**: Support for `resource.{profile}.yaml` overlays with deep-merging.
- **Dependency-Aware (Staged) Application**: Ensured correct resource application order (Stages 0-3) to prevent race conditions.
- **Interactive Review Mode**: Added `--review` flag to `apply` command for granular change confirmation.
- **Enhanced UX**: Integrated `indicatif` for high-quality progress bars and spinners.
- **Plan Summary**: Added a concise summary of actions to the `plan` command.

### Changed
- Refactored `src/apply/` to remove hundreds of lines of redundant boilerplate code.
- Enhanced `KeycloakResource` trait to support generic ID management.
- Updated `plan` and `apply` command signatures to support profiles and enhanced UX.

## [2603.1.0] - 2026-03-22
### Added
- Adopted Calendar Versioning (CalVer).
- Added pre-built binary installation scripts (`install.sh`, `install.ps1`).
