use crate::error::MultiError;
use crate::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, ComponentRepresentation, GroupRepresentation,
    IdentityProviderRepresentation, RealmRepresentation, RequiredActionProviderRepresentation,
    RoleRepresentation, UserRepresentation,
};
use crate::utils::ui::{CHECK, SEARCH, SUCCESS, WARN};
use anyhow::{Context, Result};
use console::style;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::task::JoinSet;

/// Summary noun of the problems found in a realm.
const PROBLEMS: &str = "problems found";

/// A validation problem in the file at `path`.
fn problem(path: &Path, message: impl Display) -> anyhow::Error {
    anyhow::anyhow!("{}: {}", path.display(), message)
}

/// Records `key` as declared in `path`, returning a problem if another file declared it first.
fn check_unique(
    seen: &mut HashMap<String, PathBuf>,
    key: &str,
    path: &Path,
    what: &str,
) -> Option<anyhow::Error> {
    match seen.get(key) {
        Some(first) => Some(problem(
            path,
            format!(
                "Duplicate {} '{}' (also declared in {})",
                what,
                key,
                first.display()
            ),
        )),
        None => {
            seen.insert(key.to_string(), path.to_path_buf());
            None
        }
    }
}

/// Prints the count of validated resources when there are no problems, or returns them.
fn report(label: &str, count: impl Display, problems: Vec<anyhow::Error>) -> Result<()> {
    if problems.is_empty() {
        eprintln!(
            "  {} {} {}",
            CHECK,
            style(format!("Validated {}:", label)).dim(),
            style(count).green()
        );
    }
    MultiError::into_result_with_noun(problems, PROBLEMS)
}

/// Reads every YAML file of `dir`: the parsed resources, plus one problem per file that cannot
/// be read or parsed.
async fn read_yaml_files<T: DeserializeOwned + Send + 'static>(
    dir: &Path,
    profile: Option<&str>,
) -> Result<(Vec<(PathBuf, T)>, Vec<anyhow::Error>)> {
    let Some(files) = crate::utils::yaml::list_yaml_files(dir, profile).await? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut join_set = JoinSet::new();
    for path in files {
        let profile = profile.map(str::to_string);
        join_set.spawn(async move {
            let val = crate::utils::yaml::load_yaml_with_overlay(&path, profile.as_deref()).await?;
            let item: T = crate::utils::yaml::from_value_at(val, &path)?;
            Ok::<(PathBuf, T), anyhow::Error>((path, item))
        });
    }
    let mut items = Vec::new();
    let mut problems = Vec::new();
    for result in join_set.join_all().await {
        match result {
            Ok(item) => items.push(item),
            Err(e) => problems.push(e),
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((items, problems))
}

/// Validates the structure and syntax of local YAML configuration files.
///
/// # Errors
/// Returns an error if validation fails or a file cannot be parsed.
pub async fn run(workspace_dir: PathBuf, realms_to_validate: &[String]) -> Result<()> {
    run_with_profile(workspace_dir, realms_to_validate, None).await
}

/// Validates the structure and syntax of local YAML configuration files with an active profile.
///
/// Every realm is validated and every problem is reported (each with its file), not only the
/// first one.
///
/// # Errors
/// Returns an error listing the problems found, per realm.
pub async fn run_with_profile(
    workspace_dir: PathBuf,
    realms_to_validate: &[String],
    profile: Option<&str>,
) -> Result<()> {
    if let Err(e) = fs::metadata(&workspace_dir).await {
        if e.kind() == std::io::ErrorKind::NotFound {
            return Err(anyhow::anyhow!(
                "Hint: Create the workspace directory first or use `kaji init`."
            )
            .context(format!(
                "Input directory {:?} does not exist",
                workspace_dir
            )));
        }
        return Err(e).with_context(|| format!("Failed to read {}", workspace_dir.display()));
    }

    let realms = if realms_to_validate.is_empty() {
        crate::utils::discover_realms(&workspace_dir).await?
    } else {
        realms_to_validate.to_vec()
    };

    if realms.is_empty() {
        eprintln!(
            "{} {}",
            WARN,
            style(format!(
                "No realms found to validate in {:?}",
                workspace_dir
            ))
            .yellow()
        );
        return Ok(());
    }

    let mut invalid = Vec::new();
    for realm_name in &realms {
        eprintln!(
            "\n{} {}",
            SEARCH,
            style(format!("Validating realm: {}", realm_name))
                .cyan()
                .bold()
        );
        let realm_dir = workspace_dir.join(realm_name);
        match validate_realm(realm_dir, profile).await {
            Ok(()) => eprintln!(
                "  {} {}",
                SUCCESS,
                style(format!("Successfully validated realm: {}", realm_name))
                    .green()
                    .bold()
            ),
            Err(e) => invalid.push(e.context(format!("Realm '{}' is invalid", realm_name))),
        }
    }
    MultiError::into_result_with_noun(invalid, "realms are invalid")
}

async fn validate_realm_config(workspace_dir: &Path, profile: Option<&str>) -> Result<()> {
    let realm_path = crate::utils::yaml::find_realm_file(workspace_dir).await;
    let exists = fs::try_exists(&realm_path)
        .await
        .with_context(|| format!("Failed to check {}", realm_path.display()))?;
    if !exists {
        return Err(problem(workspace_dir, "realm.yaml not found"));
    }
    let val = crate::utils::yaml::load_yaml_with_overlay(&realm_path, profile).await?;
    let realm: RealmRepresentation = crate::utils::yaml::from_value_at(val, &realm_path)?;

    if realm.realm.is_empty() {
        return Err(problem(&realm_path, "Realm name is empty"));
    }
    if let Some(dir_name) = workspace_dir.file_name().and_then(|n| n.to_str())
        && realm.realm != dir_name
    {
        return Err(problem(
            &realm_path,
            format!(
                "Realm name '{}' does not match its directory '{}' \
                 (applying it would rename realm '{}')",
                realm.realm, dir_name, dir_name
            ),
        ));
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Realm configuration is valid:").dim(),
        style(&realm.realm).green()
    );
    Ok(())
}

fn validate_roles(roles: &[(PathBuf, RoleRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, role) in roles {
        if role.name.is_empty() {
            problems.push(problem(path, "Role name is empty"));
            continue;
        }
        problems.extend(check_unique(&mut seen, &role.name, path, "role name"));
    }
    report("roles", roles.len(), problems)
}

async fn validate_client_roles(workspace_dir: &Path, profile: Option<&str>) -> Result<()> {
    let dirs = crate::apply::client_roles::client_role_dirs(workspace_dir).await?;
    let mut problems = Vec::new();
    for dir in dirs {
        let (roles, read_problems) =
            read_yaml_files::<RoleRepresentation>(&dir.roles_dir, profile).await?;
        problems.extend(read_problems);
        let mut seen = HashMap::new();
        for (path, role) in &roles {
            if role.name.is_empty() {
                problems.push(problem(path, "Role name is empty"));
                continue;
            }
            problems.extend(check_unique(
                &mut seen,
                &role.name,
                path,
                "client role name",
            ));
        }
    }
    MultiError::into_result_with_noun(problems, PROBLEMS)
}

fn validate_clients(clients: &[(PathBuf, ClientRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, client) in clients {
        let client_id = client.client_id.as_deref().unwrap_or_default();
        if client_id.is_empty() {
            problems.push(problem(path, "Client ID is missing or empty"));
            continue;
        }
        problems.extend(check_unique(&mut seen, client_id, path, "clientId"));
    }
    report("clients", clients.len(), problems)
}

fn validate_idps(idps: &[(PathBuf, IdentityProviderRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, idp) in idps {
        let alias = idp.alias.as_deref().unwrap_or_default();
        if alias.is_empty() {
            problems.push(problem(path, "Identity Provider alias is missing or empty"));
        }
        if idp.provider_id.as_deref().unwrap_or_default().is_empty() {
            problems.push(problem(
                path,
                "Identity Provider providerId is missing or empty",
            ));
        }
        if !alias.is_empty() {
            problems.extend(check_unique(
                &mut seen,
                alias,
                path,
                "Identity Provider alias",
            ));
        }
    }
    report("Identity Providers", idps.len(), problems)
}

fn validate_client_scopes(scopes: &[(PathBuf, ClientScopeRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, scope) in scopes {
        let name = scope.name.as_deref().unwrap_or_default();
        if name.is_empty() {
            problems.push(problem(path, "Client Scope name is missing or empty"));
            continue;
        }
        problems.extend(check_unique(&mut seen, name, path, "Client Scope name"));
    }
    report("client scopes", scopes.len(), problems)
}

/// Checks a group and its sub-groups (`group_path` is e.g. `/parent/child`).
fn validate_group_tree(
    group: &GroupRepresentation,
    group_path: &str,
    path: &Path,
    problems: &mut Vec<anyhow::Error>,
) {
    let name = group.name.as_deref().unwrap_or_default();
    if name.is_empty() {
        problems.push(problem(
            path,
            format!("Group name is missing or empty (at '{}')", group_path),
        ));
    }
    if let Some(sub_groups) = &group.sub_groups {
        let mut seen_sub = HashSet::new();
        for sub in sub_groups {
            let sub_name = sub.name.as_deref().unwrap_or_default();
            let sub_path = format!("{}/{}", group_path.trim_end_matches('/'), sub_name);
            validate_group_tree(sub, &sub_path, path, problems);
            if !sub_name.is_empty() && !seen_sub.insert(sub_name.to_string()) {
                problems.push(problem(
                    path,
                    format!(
                        "Duplicate subGroup name '{}' under group '{}'",
                        sub_name, group_path
                    ),
                ));
            }
        }
    }
}

fn validate_groups(groups: &[(PathBuf, GroupRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, group) in groups {
        let identity = group
            .path
            .as_deref()
            .map(|p| p.trim_start_matches('/').to_string())
            .filter(|p| !p.is_empty())
            .or_else(|| group.name.clone())
            .unwrap_or_default();
        validate_group_tree(group, &format!("/{}", identity), path, &mut problems);
        if !identity.is_empty() {
            problems.extend(check_unique(&mut seen, &identity, path, "Group"));
        }
    }
    report("groups", groups.len(), problems)
}

fn validate_users(users: &[(PathBuf, UserRepresentation)]) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, user) in users {
        let username = user.username.as_deref().unwrap_or_default();
        if username.is_empty() {
            problems.push(problem(path, "User username is missing or empty"));
            continue;
        }
        problems.extend(check_unique(&mut seen, username, path, "User username"));
    }
    report("users", users.len(), problems)
}

/// Characters forbidden by Keycloak in authentication flow alias names.
const FLOW_ALIAS_FORBIDDEN_CHARS: &[char] = &['(', ')', '[', ']', '{', '}', '/', '\\'];

/// Valid Keycloak requirement settings for executions.
const VALID_FLOW_REQUIREMENTS: &[&str] = &[
    "REQUIRED",
    "ALTERNATIVE",
    "OPTIONAL",
    "CONDITIONAL",
    "DISABLED",
];

/// Finds every cycle of the sub-flow dependency graph, as alias paths (`a -> b -> a`).
fn find_flow_cycles(adj: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Visiting,
        Visited,
    }

    fn dfs(
        node: &str,
        adj: &HashMap<String, Vec<String>>,
        visited: &mut HashMap<String, State>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string(), State::Visiting);
        path.push(node.to_string());

        if let Some(neighbors) = adj.get(node) {
            for neighbor in neighbors {
                match visited.get(neighbor) {
                    Some(State::Visiting) => {
                        let cycle_start = path.iter().position(|n| n == neighbor).unwrap_or(0);
                        let mut cycle = path[cycle_start..].to_vec();
                        cycle.push(neighbor.clone());
                        cycles.push(cycle);
                    }
                    Some(State::Visited) => {}
                    None => dfs(neighbor, adj, visited, path, cycles),
                }
            }
        }

        path.pop();
        visited.insert(node.to_string(), State::Visited);
    }

    let mut visited = HashMap::new();
    let mut path = Vec::new();
    let mut cycles = Vec::new();

    let mut nodes: Vec<&String> = adj.keys().collect();
    nodes.sort();
    for node in nodes {
        if !visited.contains_key(node) {
            dfs(node, adj, &mut visited, &mut path, &mut cycles);
        }
    }
    cycles
}

fn validate_authentication_flows(
    flows: &[(PathBuf, AuthenticationFlowRepresentation)],
) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut subflow_referrers: HashMap<String, Vec<String>> = HashMap::new();

    for (path, flow) in flows {
        let alias = flow.alias.as_deref().unwrap_or_default();
        if alias.is_empty() {
            problems.push(problem(
                path,
                "Authentication Flow alias is missing or empty",
            ));
        }
        if let Some(bad_char) = alias
            .chars()
            .find(|c| FLOW_ALIAS_FORBIDDEN_CHARS.contains(c))
        {
            problems.push(problem(
                path,
                format!(
                    "Authentication Flow alias '{}' contains forbidden character '{}' \
                     (not allowed by Keycloak)",
                    alias, bad_char
                ),
            ));
        }
        if !alias.is_empty() {
            problems.extend(check_unique(
                &mut seen,
                alias,
                path,
                "Authentication Flow alias",
            ));
        }

        // Validate child executions
        if let Some(execs) = &flow.authentication_executions {
            for (index, exec) in execs.iter().enumerate() {
                let execution = format!(
                    "execution #{} ({}) of flow '{}'",
                    index + 1,
                    exec.authenticator
                        .as_deref()
                        .or(exec.flow_alias.as_deref())
                        .unwrap_or("no authenticator or flowAlias"),
                    alias
                );
                if let Some(req) = &exec.requirement {
                    let req_upper = req.to_uppercase();
                    if !VALID_FLOW_REQUIREMENTS.contains(&req_upper.as_str()) {
                        problems.push(problem(
                            path,
                            format!(
                                "Invalid requirement '{}' in {}. Expected one of: {}",
                                req,
                                execution,
                                VALID_FLOW_REQUIREMENTS.join(", ")
                            ),
                        ));
                    }
                }

                if exec.authenticator_flow == Some(true)
                    && exec.flow_alias.as_deref().unwrap_or_default().is_empty()
                {
                    problems.push(problem(
                        path,
                        format!(
                            "{} is marked as sub-flow ('authenticatorFlow: true') but is missing 'flowAlias'",
                            execution
                        ),
                    ));
                }

                if let Some(sub_alias) = &exec.flow_alias {
                    adj.entry(alias.to_string())
                        .or_default()
                        .push(sub_alias.clone());
                    subflow_referrers
                        .entry(sub_alias.clone())
                        .or_default()
                        .push(alias.to_string());
                }
            }
        }
    }

    // Cycle detection across flow and sub-flow dependencies (every flow of a cycle has
    // executions, hence a file)
    for cycle in find_flow_cycles(&adj) {
        let files: Vec<String> = cycle[..cycle.len() - 1]
            .iter()
            .filter_map(|alias| {
                let file = seen.get(alias)?;
                Some(format!("'{}' in {}", alias, file.display()))
            })
            .collect();
        problems.push(anyhow::anyhow!(
            "Circular subflow dependency detected in authentication flows: {} ({})",
            cycle.join(" -> "),
            files.join(", ")
        ));
    }

    let shared_count = subflow_referrers
        .values()
        .filter(|parents| parents.len() > 1)
        .count();

    let count_label = if shared_count > 0 {
        format!("{} ({} shared)", flows.len(), shared_count)
    } else {
        flows.len().to_string()
    };
    report("authentication flows", count_label, problems)
}

fn validate_required_actions(
    actions: &[(PathBuf, RequiredActionProviderRepresentation)],
) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, action) in actions {
        let alias = action.alias.as_deref().unwrap_or_default();
        if alias.is_empty() {
            problems.push(problem(path, "Required Action alias is missing or empty"));
        }
        if action.provider_id.as_deref().unwrap_or_default().is_empty() {
            problems.push(problem(
                path,
                "Required Action providerId is missing or empty",
            ));
        }
        if !alias.is_empty() {
            problems.extend(check_unique(
                &mut seen,
                alias,
                path,
                "Required Action alias",
            ));
        }
    }
    report("required actions", actions.len(), problems)
}

fn validate_authenticator_configs(
    configs: &[(PathBuf, AuthenticatorConfigRepresentation)],
) -> Result<()> {
    let mut problems = Vec::new();
    let mut seen = HashMap::new();
    for (path, config) in configs {
        let alias = config.alias.as_deref().unwrap_or_default();
        if alias.is_empty() {
            problems.push(problem(
                path,
                "Authenticator Config alias is missing or empty",
            ));
            continue;
        }
        problems.extend(check_unique(
            &mut seen,
            alias,
            path,
            "Authenticator Config alias",
        ));
    }
    report("authenticator configs", configs.len(), problems)
}

async fn validate_realm(workspace_dir: PathBuf, profile: Option<&str>) -> Result<()> {
    let mut problems = Vec::new();
    let mut collect = |result: Result<()>| {
        if let Err(e) = result {
            problems.extend(MultiError::flatten(vec![e]));
        }
    };

    // 1. Validate Realm
    collect(validate_realm_config(&workspace_dir, profile).await);

    // Read all resource directories concurrently
    let roles_dir = workspace_dir.join("roles");
    let clients_dir = workspace_dir.join("clients");
    let idps_dir = workspace_dir.join("identity-providers");
    let scopes_dir = workspace_dir.join("client-scopes");
    let groups_dir = workspace_dir.join("groups");
    let users_dir = workspace_dir.join("users");
    let flows_dir = workspace_dir.join("authentication-flows");
    let actions_dir = workspace_dir.join("required-actions");
    let configs_dir = workspace_dir.join("authenticator-configs");

    let (roles, clients, idps, scopes, groups, users, flows, actions, configs) = tokio::try_join!(
        read_yaml_files::<RoleRepresentation>(&roles_dir, profile),
        read_yaml_files::<ClientRepresentation>(&clients_dir, profile),
        read_yaml_files::<IdentityProviderRepresentation>(&idps_dir, profile),
        read_yaml_files::<ClientScopeRepresentation>(&scopes_dir, profile),
        read_yaml_files::<GroupRepresentation>(&groups_dir, profile),
        read_yaml_files::<UserRepresentation>(&users_dir, profile),
        read_yaml_files::<AuthenticationFlowRepresentation>(&flows_dir, profile),
        read_yaml_files::<RequiredActionProviderRepresentation>(&actions_dir, profile),
        read_yaml_files::<AuthenticatorConfigRepresentation>(&configs_dir, profile),
    )?;

    // Validate resources (files that could not be parsed are reported once, not checked)
    collect(validate_parsed(roles, validate_roles));
    collect(validate_parsed(clients, validate_clients));
    collect(validate_client_roles(&workspace_dir, profile).await);
    collect(validate_parsed(idps, validate_idps));
    collect(validate_parsed(scopes, validate_client_scopes));
    collect(validate_parsed(groups, validate_groups));
    collect(validate_parsed(users, validate_users));
    collect(validate_parsed(flows, validate_authentication_flows));
    collect(validate_parsed(actions, validate_required_actions));
    collect(validate_parsed(configs, validate_authenticator_configs));

    // Validate Components and Keys
    collect(validate_components_in_dir(&workspace_dir, "components", profile).await);
    collect(validate_components_in_dir(&workspace_dir, "keys", profile).await);

    MultiError::into_result_with_noun(problems, PROBLEMS)
}

/// Runs `check` on the parsed resources and adds the files that could not be parsed.
fn validate_parsed<T>(
    (items, mut problems): (Vec<(PathBuf, T)>, Vec<anyhow::Error>),
    check: fn(&[(PathBuf, T)]) -> Result<()>,
) -> Result<()> {
    if let Err(e) = check(&items) {
        problems.extend(MultiError::flatten(vec![e]));
    }
    MultiError::into_result_with_noun(problems, PROBLEMS)
}

async fn validate_components_in_dir(
    workspace_dir: &Path,
    dir_name: &str,
    profile: Option<&str>,
) -> Result<()> {
    let dir = workspace_dir.join(dir_name);
    let (components, mut problems) =
        read_yaml_files::<ComponentRepresentation>(&dir, profile).await?;
    if components.is_empty() && problems.is_empty() {
        return Ok(());
    }
    for (path, component) in &components {
        if component
            .name
            .as_deref()
            .is_some_and(|name| name.is_empty())
        {
            problems.push(problem(path, "Component name is empty"));
        }
        if component
            .provider_id
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            problems.push(problem(path, "Component providerId is missing or empty"));
        }
    }
    report(dir_name, components.len(), problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AuthenticationFlowRepresentation;
    use std::collections::HashMap;

    fn make_flow(alias: &str) -> (PathBuf, AuthenticationFlowRepresentation) {
        (
            PathBuf::from(format!("{}.yaml", alias)),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some(alias.to_string()),
                description: None,
                provider_id: None,
                top_level: None,
                built_in: None,
                authentication_executions: None,
                extra: HashMap::new(),
            },
        )
    }

    #[tokio::test]
    async fn test_realm_name_must_match_directory() {
        let temp = tempfile::tempdir().unwrap();
        let realm_dir = temp.path().join("prod");
        tokio::fs::create_dir_all(&realm_dir).await.unwrap();
        tokio::fs::write(realm_dir.join("realm.yaml"), "realm: staging\n")
            .await
            .unwrap();
        let err = run(temp.path().to_path_buf(), &["prod".to_string()])
            .await
            .unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("'staging'") && msg.contains("'prod'"), "{msg}");
    }

    #[test]
    fn test_validate_flow_alias_forbidden_char_parens() {
        let flows = vec![make_flow("Step Up (combined) Context Selection")];
        let result = validate_authentication_flows(&flows);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("forbidden character"),
            "Expected 'forbidden character' in: {err}"
        );
        assert!(err.contains('('), "Expected the bad char in: {err}");
    }

    #[test]
    fn test_validate_flow_alias_forbidden_char_slash() {
        let flows = vec![make_flow("My/Flow")];
        let result = validate_authentication_flows(&flows);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("forbidden character")
        );
    }

    #[test]
    fn test_validate_flow_alias_duplicate() {
        let flows = vec![
            make_flow("MyFlow"),
            make_flow("OtherFlow"),
            make_flow("MyFlow"),
        ];
        let result = validate_authentication_flows(&flows);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Duplicate"), "Expected 'Duplicate' in: {err}");
        assert!(err.contains("MyFlow"), "Expected alias name in: {err}");
    }

    #[test]
    fn test_validate_flow_alias_empty() {
        let flows = vec![make_flow("")];
        let result = validate_authentication_flows(&flows);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing or empty"));
    }

    #[test]
    fn test_validate_flow_alias_valid() {
        let flows = vec![
            make_flow("browser"),
            make_flow("Step Up MFA L4"),
            make_flow("My-Flow_v2.0"),
        ];
        assert!(
            validate_authentication_flows(&flows).is_ok(),
            "Valid aliases should pass without error"
        );
    }

    #[test]
    fn test_validate_flow_cycle_detection() {
        use crate::models::AuthenticationExecutionExportRepresentation;

        let flow_a = (
            PathBuf::from("flow-a.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("flow-a".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(true),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: None,
                        authenticator_config: None,
                        requirement: Some("REQUIRED".to_string()),
                        priority: None,
                        authenticator_flow: Some(true),
                        flow_alias: Some("flow-b".to_string()),
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let flow_b = (
            PathBuf::from("flow-b.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("flow-b".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(false),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: None,
                        authenticator_config: None,
                        requirement: Some("REQUIRED".to_string()),
                        priority: None,
                        authenticator_flow: Some(true),
                        flow_alias: Some("flow-a".to_string()),
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let flows = vec![flow_a, flow_b];
        let result = validate_authentication_flows(&flows);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Circular subflow dependency"));
        assert!(err.contains("flow-a -> flow-b -> flow-a"));
    }

    #[test]
    fn test_validate_flow_invalid_requirement() {
        use crate::models::AuthenticationExecutionExportRepresentation;

        let flow = (
            PathBuf::from("flow.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("my-flow".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(true),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: Some("auth-cookie".to_string()),
                        authenticator_config: None,
                        requirement: Some("NON_EXISTENT_REQ".to_string()),
                        priority: None,
                        authenticator_flow: None,
                        flow_alias: None,
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let result = validate_authentication_flows(&[flow]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid requirement")
        );
    }

    #[test]
    fn test_validate_flow_subflow_missing_alias() {
        use crate::models::AuthenticationExecutionExportRepresentation;

        let flow = (
            PathBuf::from("flow.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("my-flow".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(true),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: None,
                        authenticator_config: None,
                        requirement: Some("REQUIRED".to_string()),
                        priority: None,
                        authenticator_flow: Some(true),
                        flow_alias: None,
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let result = validate_authentication_flows(&[flow]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing 'flowAlias'")
        );
    }

    #[test]
    fn test_validate_shared_flow_success() {
        use crate::models::AuthenticationExecutionExportRepresentation;

        let shared_subflow = (
            PathBuf::from("shared-subflow.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("shared-mfa".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(false),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: Some("auth-otp-form".to_string()),
                        authenticator_config: None,
                        requirement: Some("REQUIRED".to_string()),
                        priority: None,
                        authenticator_flow: None,
                        flow_alias: None,
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let parent_1 = (
            PathBuf::from("parent-1.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("browser-flow".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(true),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: None,
                        authenticator_config: None,
                        requirement: Some("ALTERNATIVE".to_string()),
                        priority: None,
                        authenticator_flow: Some(true),
                        flow_alias: Some("shared-mfa".to_string()),
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let parent_2 = (
            PathBuf::from("parent-2.yaml"),
            AuthenticationFlowRepresentation {
                id: None,
                alias: Some("direct-grant-flow".to_string()),
                description: None,
                provider_id: None,
                top_level: Some(true),
                built_in: None,
                authentication_executions: Some(vec![
                    AuthenticationExecutionExportRepresentation {
                        id: None,
                        authenticator: None,
                        authenticator_config: None,
                        requirement: Some("REQUIRED".to_string()),
                        priority: None,
                        authenticator_flow: Some(true),
                        flow_alias: Some("shared-mfa".to_string()),
                        user_setup_allowed: None,
                        extra: HashMap::new(),
                    },
                ]),
                extra: HashMap::new(),
            },
        );

        let flows = vec![shared_subflow, parent_1, parent_2];
        assert!(validate_authentication_flows(&flows).is_ok());
    }

    #[tokio::test]
    async fn test_validate_with_profile_overlay() {
        use tempfile::tempdir;
        let temp = tempdir().unwrap();
        let ws = temp.path().join("realm");
        let clients_dir = ws.join("clients");
        tokio::fs::create_dir_all(&clients_dir).await.unwrap();

        // Write realm.yaml
        tokio::fs::write(ws.join("realm.yaml"), "realm: realm\n")
            .await
            .unwrap();

        // Write base client
        tokio::fs::write(
            clients_dir.join("my-client.yaml"),
            "clientId: my-client\nname: Base Client\n",
        )
        .await
        .unwrap();

        // Write partial overlay client (missing clientId, which would fail without overlay skip)
        tokio::fs::write(
            clients_dir.join("my-client.prod.yaml"),
            "name: Prod Override Client\n",
        )
        .await
        .unwrap();

        // 1. Without profile: overlay is skipped, base client validates successfully
        let res_no_profile = run(temp.path().to_path_buf(), &["realm".to_string()]).await;
        assert!(
            res_no_profile.is_ok(),
            "Validation should succeed by skipping partial overlay: {:?}",
            res_no_profile.err()
        );

        // 2. With profile: overlay is deep merged, validation succeeds
        let res_with_profile = run_with_profile(
            temp.path().to_path_buf(),
            &["realm".to_string()],
            Some("prod"),
        )
        .await;
        assert!(
            res_with_profile.is_ok(),
            "Validation with profile should succeed: {:?}",
            res_with_profile.err()
        );
    }
}
