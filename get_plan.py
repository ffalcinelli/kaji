def get_plan():
    print("Code Health Improvement Task: Refactor functions in apply module to use a struct for arguments")
    print("1. Update `ApplyContext` in `src/apply/mod.rs` to derive `Clone` and ensure its lifetime bounds are appropriate (they already seem fine). Actually, wait: `workspace_dir` is `PathBuf` inside `ApplyContext`, not `&Path`. Let's change it to `&'a Path` so we can easily create it from a reference, OR leave it as `PathBuf` and `.clone()` it when needed.")
    print("Wait! Let's check `ApplyContext` again. `pub workspace_dir: PathBuf`. We can either change it or leave it. The `apply_resources` functions currently take `workspace_dir: &std::path::Path`. So it makes sense to change `ApplyContext` to use `&'a Path` for `workspace_dir`, so it matches `realm_name: &'a str` and `client: &'a KeycloakClient`.")
    print("2. Modify `ApplyContext<'a>` in `src/apply/mod.rs`:")
    print("   - Change `pub workspace_dir: PathBuf` to `pub workspace_dir: &'a Path`")
    print("3. Modify `apply_resources` in `src/apply/generic.rs` to take `ctx: crate::apply::ApplyContext<'_>` instead of 11 arguments.")
    print("4. Modify `apply_authenticator_configs` in `src/apply/authenticator_config.rs` to take `ctx: crate::apply::ApplyContext<'_>`.")
    print("5. Modify `apply_components_or_keys` in `src/apply/components.rs` to take `ctx: crate::apply::ApplyContext<'_>, dir_name: &str`.")
    print("6. Modify `apply_realm` in `src/apply/realm.rs` to take `ctx: crate::apply::ApplyContext<'_>`.")
    print("7. Modify `apply_single_realm` and `spawn_apply_stage!` in `src/apply/mod.rs` to construct `ApplyContext` and pass it to the child functions.")
    print("8. Run `cargo check` and `cargo test`.")

if __name__ == "__main__":
    get_plan()
