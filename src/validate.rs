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
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::task::JoinSet;

async fn read_yaml_files<T: DeserializeOwned + Send + 'static>(
    dir: &Path,
    file_type: &str,
    profile: Option<&str>,
) -> Result<Vec<(PathBuf, T)>> {
    let mut results = Vec::new();
    if fs::try_exists(dir).await? {
        let mut entries = fs::read_dir(dir).await?;
        let mut join_set = JoinSet::new();
        let file_type_str = file_type.to_string();
        let profile_owned = profile.map(|s| s.to_string());

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                if crate::utils::yaml::is_overlay_file(&path, profile_owned.as_deref()) {
                    continue;
                }
                let ft = file_type_str.clone();
                let prof = profile_owned.clone();
                join_set.spawn(async move {
                    let val =
                        crate::utils::yaml::load_yaml_with_overlay(&path, prof.as_deref()).await?;
                    let item: T = serde_json::from_value(val)
                        .with_context(|| format!("Failed to parse {} file {:?}", ft, path))?;
                    Ok::<(PathBuf, T), anyhow::Error>((path, item))
                });
            }
        }

        while let Some(res) = join_set.join_next().await {
            results.push(res??);
        }
    }
    Ok(results)
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
/// # Errors
/// Returns an error if validation fails or a file cannot be parsed.
pub async fn run_with_profile(
    workspace_dir: PathBuf,
    realms_to_validate: &[String],
    profile: Option<&str>,
) -> Result<()> {
    if !fs::try_exists(&workspace_dir).await? {
        return Err(anyhow::anyhow!(
            "Hint: Create the workspace directory first or use `kaji init`."
        ))
        .with_context(|| format!("Input directory {:?} does not exist", workspace_dir));
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

    for realm_name in &realms {
        eprintln!(
            "\n{} {}",
            SEARCH,
            style(format!("Validating realm: {}", realm_name))
                .cyan()
                .bold()
        );
        let realm_dir = workspace_dir.join(realm_name);
        validate_realm(realm_dir, profile).await?;
        eprintln!(
            "  {} {}",
            SUCCESS,
            style(format!("Successfully validated realm: {}", realm_name))
                .green()
                .bold()
        );
    }
    Ok(())
}

async fn validate_realm_config(workspace_dir: &Path, profile: Option<&str>) -> Result<()> {
    let realm_path = workspace_dir.join("realm.yaml");
    let val = crate::utils::yaml::load_yaml_with_overlay(&realm_path, profile)
        .await
        .with_context(|| {
            format!(
                "realm.yaml not found or failed to read in {:?}",
                workspace_dir
            )
        })?;
    let realm: RealmRepresentation =
        serde_json::from_value(val).context("Failed to parse realm.yaml")?;

    if realm.realm.is_empty() {
        anyhow::bail!("Realm name is empty in realm.yaml");
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
    let mut role_names = HashSet::new();
    for (path, role) in roles {
        if role.name.is_empty() {
            anyhow::bail!("Role name is empty in {:?}", path);
        }
        if role_names.contains(&role.name) {
            anyhow::bail!("Duplicate role name: {}", role.name);
        }
        role_names.insert(role.name.clone());
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated roles:").dim(),
        style(roles.len()).green()
    );
    Ok(())
}

fn validate_clients(clients: &[(PathBuf, ClientRepresentation)]) -> Result<()> {
    for (path, client) in clients {
        if client.client_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Client ID is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated clients:").dim(),
        style(clients.len()).green()
    );
    Ok(())
}

fn validate_idps(idps: &[(PathBuf, IdentityProviderRepresentation)]) -> Result<()> {
    for (path, idp) in idps {
        if idp.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Identity Provider alias is missing or empty in {:?}", path);
        }
        if idp.provider_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Identity Provider providerId is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated Identity Providers:").dim(),
        style(idps.len()).green()
    );
    Ok(())
}

fn validate_client_scopes(scopes: &[(PathBuf, ClientScopeRepresentation)]) -> Result<()> {
    for (path, scope) in scopes {
        if scope.name.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Client Scope name is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated client scopes:").dim(),
        style(scopes.len()).green()
    );
    Ok(())
}

fn validate_groups(groups: &[(PathBuf, GroupRepresentation)]) -> Result<()> {
    for (path, group) in groups {
        if group.name.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Group name is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated groups:").dim(),
        style(groups.len()).green()
    );
    Ok(())
}

fn validate_users(users: &[(PathBuf, UserRepresentation)]) -> Result<()> {
    for (path, user) in users {
        if user.username.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("User username is missing or empty in {:?}", path);
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated users:").dim(),
        style(users.len()).green()
    );
    Ok(())
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

fn detect_flow_cycles(adj: &HashMap<String, Vec<String>>) -> Result<()> {
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
    ) -> Result<()> {
        visited.insert(node.to_string(), State::Visiting);
        path.push(node.to_string());

        if let Some(neighbors) = adj.get(node) {
            for neighbor in neighbors {
                match visited.get(neighbor) {
                    Some(State::Visiting) => {
                        path.push(neighbor.clone());
                        let cycle_start = path.iter().position(|n| n == neighbor).unwrap_or(0);
                        let cycle_str = path[cycle_start..].join(" -> ");
                        anyhow::bail!(
                            "Circular subflow dependency detected in authentication flows: {}",
                            cycle_str
                        );
                    }
                    Some(State::Visited) => {}
                    None => {
                        dfs(neighbor, adj, visited, path)?;
                    }
                }
            }
        }

        path.pop();
        visited.insert(node.to_string(), State::Visited);
        Ok(())
    }

    let mut visited = HashMap::new();
    let mut path = Vec::new();

    let mut nodes: Vec<&String> = adj.keys().collect();
    nodes.sort();
    for node in nodes {
        if !visited.contains_key(node) {
            dfs(node, adj, &mut visited, &mut path)?;
        }
    }

    Ok(())
}

fn validate_authentication_flows(
    flows: &[(PathBuf, AuthenticationFlowRepresentation)],
) -> Result<()> {
    let mut seen_aliases = HashSet::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut subflow_referrers: HashMap<String, Vec<String>> = HashMap::new();

    for (path, flow) in flows {
        let alias = flow.alias.as_deref().unwrap_or_default();
        if alias.is_empty() {
            anyhow::bail!(
                "Authentication Flow alias is missing or empty in {:?}",
                path
            );
        }
        if let Some(bad_char) = alias
            .chars()
            .find(|c| FLOW_ALIAS_FORBIDDEN_CHARS.contains(c))
        {
            anyhow::bail!(
                "Authentication Flow alias '{}' contains forbidden character '{}' \
                 (not allowed by Keycloak) in {:?}",
                alias,
                bad_char,
                path
            );
        }
        if seen_aliases.contains(alias) {
            anyhow::bail!(
                "Duplicate Authentication Flow alias '{}' found in {:?}",
                alias,
                path
            );
        }
        seen_aliases.insert(alias.to_string());

        // Validate child executions
        if let Some(execs) = &flow.authentication_executions {
            for exec in execs {
                if let Some(req) = &exec.requirement {
                    let req_upper = req.to_uppercase();
                    if !VALID_FLOW_REQUIREMENTS.contains(&req_upper.as_str()) {
                        anyhow::bail!(
                            "Invalid requirement '{}' in authentication flow execution in {:?}. Expected one of: {:?}",
                            req,
                            path,
                            VALID_FLOW_REQUIREMENTS
                        );
                    }
                }

                if exec.authenticator_flow == Some(true)
                    && exec.flow_alias.as_deref().unwrap_or_default().is_empty()
                {
                    anyhow::bail!(
                        "Authentication flow execution marked as sub-flow ('authenticatorFlow: true') is missing 'flowAlias' in {:?}",
                        path
                    );
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

    // Cycle detection across flow and sub-flow dependencies
    detect_flow_cycles(&adj)?;

    let shared_count = subflow_referrers
        .values()
        .filter(|parents| parents.len() > 1)
        .count();

    let count_label = if shared_count > 0 {
        format!("{} ({} shared)", flows.len(), shared_count)
    } else {
        flows.len().to_string()
    };

    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated authentication flows:").dim(),
        style(count_label).green()
    );
    Ok(())
}

fn validate_required_actions(
    actions: &[(PathBuf, RequiredActionProviderRepresentation)],
) -> Result<()> {
    for (path, action) in actions {
        if action.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!("Required Action alias is missing or empty in {:?}", path);
        }
        if action.provider_id.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Required Action providerId is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated required actions:").dim(),
        style(actions.len()).green()
    );
    Ok(())
}

fn validate_authenticator_configs(
    configs: &[(PathBuf, AuthenticatorConfigRepresentation)],
) -> Result<()> {
    for (path, config) in configs {
        if config.alias.as_deref().unwrap_or_default().is_empty() {
            anyhow::bail!(
                "Authenticator Config alias is missing or empty in {:?}",
                path
            );
        }
    }
    eprintln!(
        "  {} {} {}",
        CHECK,
        style("Validated authenticator configs:").dim(),
        style(configs.len()).green()
    );
    Ok(())
}

async fn validate_realm(workspace_dir: PathBuf, profile: Option<&str>) -> Result<()> {
    // 1. Validate Realm
    validate_realm_config(&workspace_dir, profile).await?;

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
        read_yaml_files::<RoleRepresentation>(&roles_dir, "role", profile),
        read_yaml_files::<ClientRepresentation>(&clients_dir, "client", profile),
        read_yaml_files::<IdentityProviderRepresentation>(&idps_dir, "idp", profile),
        read_yaml_files::<ClientScopeRepresentation>(&scopes_dir, "client-scope", profile),
        read_yaml_files::<GroupRepresentation>(&groups_dir, "group", profile),
        read_yaml_files::<UserRepresentation>(&users_dir, "user", profile),
        read_yaml_files::<AuthenticationFlowRepresentation>(
            &flows_dir,
            "authentication-flow",
            profile
        ),
        read_yaml_files::<RequiredActionProviderRepresentation>(
            &actions_dir,
            "required-action",
            profile
        ),
        read_yaml_files::<AuthenticatorConfigRepresentation>(
            &configs_dir,
            "authenticator-config",
            profile
        ),
    )?;

    // Validate resources
    validate_roles(&roles)?;
    validate_clients(&clients)?;
    validate_idps(&idps)?;
    validate_client_scopes(&scopes)?;
    validate_groups(&groups)?;
    validate_users(&users)?;
    validate_authentication_flows(&flows)?;
    validate_required_actions(&actions)?;
    validate_authenticator_configs(&configs)?;

    // Validate Components and Keys
    tokio::try_join!(
        validate_components_in_dir(&workspace_dir, "components", profile),
        validate_components_in_dir(&workspace_dir, "keys", profile)
    )?;

    Ok(())
}

async fn validate_components_in_dir(
    workspace_dir: &Path,
    dir_name: &str,
    profile: Option<&str>,
) -> Result<()> {
    let dir = workspace_dir.join(dir_name);
    if fs::try_exists(&dir).await? {
        let components: Vec<(PathBuf, ComponentRepresentation)> =
            read_yaml_files(&dir, dir_name, profile).await?;
        for (path, component) in &components {
            if let Some(name) = &component.name
                && name.is_empty()
            {
                anyhow::bail!("Component name is empty in {:?}", path);
            }
            if component
                .provider_id
                .as_deref()
                .unwrap_or_default()
                .is_empty()
            {
                anyhow::bail!("Component providerId is missing or empty in {:?}", path);
            }
        }
        eprintln!(
            "  {} {} {}",
            CHECK,
            style(format!("Validated {}:", dir_name)).dim(),
            style(components.len()).green()
        );
    }
    Ok(())
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
        tokio::fs::write(ws.join("realm.yaml"), "realm: test-realm\n")
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
