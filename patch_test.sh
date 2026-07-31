#!/bin/bash
set -e

# ApplyContext is passed to `apply_single_realm`.
# Where is `apply_single_realm` called?
grep -rn "apply_single_realm" src/apply/mod.rs
