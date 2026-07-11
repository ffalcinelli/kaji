## 2026-07-11 - CLI parsing boundary refactoring
**Observation:** CLI manual fallback parsing with std::env::var in core functions or main loops mixes infrastructure concerns with application setup, violating clean architecture.
**Action:** Let CLI parsing frameworks (like `clap`) handle environment variables declaratively through attributes (e.g. `#[arg(env = "...")]`) to isolate the boundary and improve both architecture and security (via `hide_env_values = true`).
