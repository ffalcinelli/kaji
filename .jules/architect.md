# 📐 Architect's Journal - kaji

This journal documents critical architectural learnings, boundaries, concepts, and testing improvements for kaji.

## 2026-07-11 - [Tokio Async I/O Boundaries]
**Observation:** Detected synchronous `std::fs::read_to_string` usage inside the asynchronous `load_profile` function in `src/lib.rs`. Mixing synchronous filesystem blocking calls into Tokio async contexts can exhaust worker threads and degrade concurrent performance.
**Action:** Always prefer asynchronous file operations via `tokio::fs` or `async_fs` within async functions. Refactored `load_profile` to use `tokio::fs::read_to_string(&profile_path).await`.

## 2026-07-11 - [Eliminating Fake Tests]
**Observation:** Found that major integration tests (specifically `tests/plan_test.rs` and `tests/apply_test.rs`) were "fake tests" that lacked assert statements to verify the actual output/behavior of planning and application (reconciliation). They merely ensured that the code does not panic.
**Action:** Assertions must strictly validate state or behavior, never just check lack of panic. Refactored `tests/plan_test.rs` to assert the exact files listed in `.kajiplan`, and `tests/apply_test.rs` to assert the deletion of `.kajiplan` upon successful application, as well as mock confirmation queue consumption.
