#!/bin/bash
set -e

# Change `ApplyContext` to take `workspace_dir: PathBuf`? Or `&'a Path`?
# In `apply_single_realm`, it owns `workspace_dir` as `PathBuf`.
# But `ApplyContext` is already defined as having `workspace_dir: PathBuf` and it works!
