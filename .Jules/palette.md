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
