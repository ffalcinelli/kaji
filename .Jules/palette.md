## 2024-07-11 - Route UI print methods to stderr
**Learning:** Routing standard informational/success/error/warning messages to `stderr` allows users to pipe CLI data from `stdout` cleanly without mixing it with UI feedback. This is a common pattern in Unix tools.
**Action:** Ensure `ui.print_*` methods use `eprintln!` instead of `println!`.
