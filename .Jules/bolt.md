## 2026-07-18 - Zero-Cost Abstractions in String Manipulation
**Learning:** Chaining `format!()` macros and `.collect::<Vec<char>>` on short configuration strings in hot loops causes massive heap allocation overhead. `Vec<char>` allocates `4 * N` bytes plus structural overhead just to look at the first and last element of a string.
**Action:** When working with strings where capacity can be pre-calculated (like concatenating known lengths) or where parsing bounds is needed (like finding the first and last char), always use `String::with_capacity()` or iterator mapping (`chars()`) directly. This reduces allocations to a single, sized heap structure, making the hot path substantially faster.
## 2024-04-12 - Avoid format! inside recursive parsing loops
**Learning:** Using `format!()` inside deep recursive traversals over JSON objects (like config masking or processing) incurs massive continuous heap allocation overhead which is unnecessary when predicting string sizes.
**Action:** Use `String::with_capacity()` to allocate string sizes exactly once for the maximum predicted byte count, and combine strings using `.push_str()` or `.push()` in recursive hot loops.
## 2026-08-03 - Concurrent Network Requests
**Learning:** Sequential network I/O in loops can create significant bottlenecks, especially when fetching related resources (like flow executions for multiple flows).
**Action:** Extract network calls into a vector of futures and execute them concurrently using `futures::future::join_all` (or similar primitives) to parallelize network I/O and reduce latency.
## 2024-06-25 - Redundant fs reads in append_secrets
**Learning:** Checking for file existence before reading, and then reading again later, introduces unnecessary latency and TOCTOU vulnerabilities.
**Action:** Use `.unwrap_or_default()` directly on `tokio::fs::read_to_string` to avoid the `try_exists` check and cache the read contents in memory instead of re-reading from disk on the same execution path.
## 2026-11-20 - Redundant fs reads when checking file existence
**Learning:** Checking for file existence with `fs::try_exists()` and then calling `fs::read_to_string()` requires two system calls (stat and read/open). This causes unnecessary overhead, and creates a Time-of-Check to Time-of-Use (TOCTOU) vulnerability where the file can change between operations.
**Action:** Remove `try_exists()` checks before file reads. Read the file directly with `fs::read_to_string()` and handle the resulting `Result`. Use `.unwrap_or_default()` if it's acceptable to swallow all errors (like missing optional configs), or match explicitly on `std::io::ErrorKind::NotFound` to safely ignore missing files while retaining context on real I/O errors.
## 2024-05-24 - Async IO TOCTOU and concurrent tasks
**Learning:** Checking for file existence before reading with `fs::try_exists()` -> `fs::read()` is a common anti-pattern in async Rust. It causes an extra I/O operation and is vulnerable to TOCTOU.
**Action:** Instead, just attempt to read the file directly and handle the `std::io::ErrorKind::NotFound` error. Also, `tokio::fs::read_dir` sequential metadata fetches with `file_type().await` can be painfully slow for large directories. Using `tokio::task::JoinSet::spawn` to check file types in parallel offers a massive speedup when scanning a workspace directory.
