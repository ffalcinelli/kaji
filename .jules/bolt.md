## 2024-07-11 - Lazy Error Formatting Avoids Hot Path Allocations
**Learning:** Using `.context(format!(...))` forces the `format!` macro and its underlying string allocation to evaluate eagerly every time the code path executes, even if no error occurs.
**Action:** Replace `.context(format!(...))` with `.with_context(|| format!(...))` to defer execution and allocation exclusively to the error path, keeping the successful hot path zero-allocation.
