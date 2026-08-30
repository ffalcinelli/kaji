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
## 2024-05-18 - Nested Error Hint Formatting with anyhow

**Learning:** When adding actionable hints using `anyhow`, a naive `Err(anyhow::anyhow!("Main Error")).context("Hint: ...")` makes `to_string()` (which `main.rs` uses) hide the main error because `to_string` only prints the outermost context.
**Action:** To display both the main error and properly style the hint in `kaji`, place the hint inside the inner error (e.g., `anyhow::anyhow!("Hint: ...")`) and wrap the main message in the outer context (`.context("Main error")`). This allows `main.rs`'s global error handler to style the outer error appropriately and detect the nested `"Hint: "` prefix when iterating through the `.chain()`.
