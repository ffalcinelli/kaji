## 2026-07-18 - Zero-Cost Abstractions in String Manipulation
**Learning:** Chaining `format!()` macros and `.collect::<Vec<char>>` on short configuration strings in hot loops causes massive heap allocation overhead. `Vec<char>` allocates `4 * N` bytes plus structural overhead just to look at the first and last element of a string.
**Action:** When working with strings where capacity can be pre-calculated (like concatenating known lengths) or where parsing bounds is needed (like finding the first and last char), always use `String::with_capacity()` or iterator mapping (`chars()`) directly. This reduces allocations to a single, sized heap structure, making the hot path substantially faster.
## 2024-04-12 - Avoid format! inside recursive parsing loops
**Learning:** Using `format!()` inside deep recursive traversals over JSON objects (like config masking or processing) incurs massive continuous heap allocation overhead which is unnecessary when predicting string sizes.
**Action:** Use `String::with_capacity()` to allocate string sizes exactly once for the maximum predicted byte count, and combine strings using `.push_str()` or `.push()` in recursive hot loops.
