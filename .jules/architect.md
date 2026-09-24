# 📐 Architect's Journal - kaji

This journal documents critical architectural learnings, boundaries, concepts, and testing improvements for kaji.

## 2026-07-11 - [Tokio Async I/O Boundaries]
**Observation:** Detected synchronous `std::fs::read_to_string` usage inside the asynchronous `load_profile` function in `src/lib.rs`. Mixing synchronous filesystem blocking calls into Tokio async contexts can exhaust worker threads and degrade concurrent performance.
**Action:** Always prefer asynchronous file operations via `tokio::fs` or `async_fs` within async functions. Refactored `load_profile` to use `tokio::fs::read_to_string(&profile_path).await`.

## 2026-07-11 - [Eliminating Fake Tests]
**Observation:** Found that major integration tests (specifically `tests/plan_test.rs` and `tests/apply_test.rs`) were "fake tests" that lacked assert statements to verify the actual output/behavior of planning and application (reconciliation). They merely ensured that the code does not panic.
**Action:** Assertions must strictly validate state or behavior, never just check lack of panic. Refactored `tests/plan_test.rs` to assert the exact files listed in `.kajiplan`, and `tests/apply_test.rs` to assert the deletion of `.kajiplan` upon successful application, as well as mock confirmation queue consumption.

## 2026-09-23 - [Authentication Flow Reconciliation & Shared Sub-flows]
**Observation:** Authentication flows with shared sub-flows exhibited 409 Conflict errors when parent flows auto-created or locked shared sub-flow containers in Keycloak. Additionally, circular subflow references or invalid execution requirement enums could lead to Keycloak API failures midway through reconciliation.
**Action:** Implemented:
1. Pure-local pre-flight validation in `src/validate.rs`: requirement enum validation, subflow alias presence check, and DFS cycle detection (`detect_flow_cycles`) with shared referrer tracking.
2. Topological dependency ordering in `src/apply/generic.rs`: sorting authentication flows into dependency tiers (Tier 0 leaf/shared sub-flows, Tier 1+ dependent parent flows) applied sequentially across tiers and concurrently within each tier.
3. Graceful 409 Conflict auto-adoption: catching 409 Conflict during flow creation, invalidating client cache, discovering the auto-created remote flow ID, and reconciling via PUT update.
