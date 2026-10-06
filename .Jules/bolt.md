## 2024-10-03 - Recursive Path Construction Optimization
**Learning:** In recursive functions that construct paths (like `extract_secrets` appending `_` and keys), creating a new `String` using `format!` or `String::with_capacity` at every depth allocates frequently on the heap. This causes massive memory allocations and slowdowns on deep or large tree structures like complex Keycloak JSON objects.
**Action:** Always use a single mutable `String` buffer (`&mut String`) combined with `.truncate(original_len)` to build paths during recursive traversal instead of allocating at every depth.
