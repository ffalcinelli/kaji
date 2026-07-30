with open("tests/plan_coverage_test.rs", "r") as f:
    content = f.read()

old = """    let res = plan::run(
        &client,
        workspace_dir.clone(),
        false, // changes_only
        false, // interactive
        &["test-realm".to_string()],
        ui.clone(),
        resolver,
        Some("prod".to_string()), // profile = prod (so role-1.prod.yaml is overlay)
    )"""

new = """    let res = plan::run(kaji::plan::PlanArgs {
        client: &client,
        workspace_dir: workspace_dir.clone(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &["test-realm".to_string()],
        ui: ui.clone(),
        resolver,
        profile: Some("prod".to_string()),
    })"""

content = content.replace(old, new)

with open("tests/plan_coverage_test.rs", "w") as f:
    f.write(content)
