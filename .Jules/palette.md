## 2024-07-11 - Route UI print methods to stderr
**Learning:** Routing standard informational/success/error/warning messages to `stderr` allows users to pipe CLI data from `stdout` cleanly without mixing it with UI feedback. This is a common pattern in Unix tools.
**Action:** Ensure `ui.print_*` methods use `eprintln!` instead of `println!`.
## 2024-07-18 - Better nested errors with anyhow's chain

**Learning:** anyhow's default `Debug` format displays errors as a raw numbered list like:
```
Caused by:
    0: Failed to send login request
    1: client error
```
Which is not very pleasing to the eye or easy to parse for users.

**Action:** Instead of returning `anyhow::Result<()>` from `main()`, we can return `std::process::ExitCode`. This allows us to manually iterate through `anyhow::Error::chain().enumerate()`, displaying the root cause in bold, and cleanly indenting nested causes with a `↳` symbol.
## 2026-08-01 - Enhance CLI error hints
**Learning:** Added visual styling for actionable hints using an 'anyhow' context prefix.
**Action:** Use `.context("Hint: ...")` for UX guidance in error chains, as the main CLI error handler now parses this prefix to display the text in blue with an INFO emoji.
## 2023-10-27 - Actionable Hints via Anyhow Context
**Learning:** `anyhow::Error`'s `context()` method is highly effective for appending actionable hints to generic errors in the CLI. When combined with a global error handler that iterates over `Error::chain()`, a context like `Hint: Run kaji init` is styled cleanly as a nested, causal message without changing the underlying error struct. Also, when embedding backticks in these hints (like `kaji init`), Rust requires raw string literals (`r#"..."#`) to avoid escape character compilation errors.
**Action:** When adding hints to existing `bail!` or `Result` types, wrap the hint in `anyhow!("Hint: ...")` and attach the original message via `.context()`. Always use `r#""#` for hints containing terminal commands.
