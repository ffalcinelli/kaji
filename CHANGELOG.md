# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
