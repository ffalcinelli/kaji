//! Reconciliation of authentication flow executions.
//!
//! Keycloak ignores `authenticationExecutions` on `POST`/`PUT /authentication/flows`, so the
//! executions declared in a flow file (Keycloak export format) are applied through the
//! executions API after the flow itself exists:
//!
//! * declared executions are matched to the flow's direct children (`level` 0) by authenticator
//!   provider, or by sub-flow alias, in declaration order;
//! * unmatched remote executions are removed, missing ones are added (sub-flows declared in their
//!   own file are linked by ID, others are created inline), and requirement/priority are updated;
//! * built-in flows only accept requirement/priority changes.
//!
//! Executions without an explicit `priority` get `10, 20, 30, ...` in declaration order.
//! A flow file without `authenticationExecutions` leaves the remote executions untouched.

use crate::client::KeycloakClient;
use crate::models::{
    AuthenticationExecutionExportRepresentation, AuthenticationExecutionInfoRepresentation,
    AuthenticationFlowRepresentation,
};
use crate::utils::ui::{SUCCESS_CREATE, SUCCESS_UPDATE, WARN};
use anyhow::{Context, Result};
use console::style;

#[derive(Debug, PartialEq, Eq)]
enum ExecutionKey<'a> {
    Authenticator(&'a str),
    SubFlow(&'a str),
}

fn export_key(exec: &AuthenticationExecutionExportRepresentation) -> Option<ExecutionKey<'_>> {
    if exec.is_subflow() {
        exec.flow_alias.as_deref().map(ExecutionKey::SubFlow)
    } else {
        exec.authenticator
            .as_deref()
            .map(ExecutionKey::Authenticator)
    }
}

fn row_key(row: &AuthenticationExecutionInfoRepresentation) -> Option<ExecutionKey<'_>> {
    if row.is_subflow() {
        row.display_name.as_deref().map(ExecutionKey::SubFlow)
    } else {
        row.provider_id.as_deref().map(ExecutionKey::Authenticator)
    }
}

fn describe(key: &Option<ExecutionKey<'_>>) -> String {
    match key {
        Some(ExecutionKey::Authenticator(p)) => format!("execution '{}'", p),
        Some(ExecutionKey::SubFlow(a)) => format!("sub-flow '{}'", a),
        None => "execution".to_string(),
    }
}

/// Target priority of the `index`-th declared execution.
fn target_priority(exec: &AuthenticationExecutionExportRepresentation, index: usize) -> i32 {
    exec.priority
        .unwrap_or_else(|| (index as i32 + 1).saturating_mul(10))
}

/// Direct children (`level` 0) of a flow.
async fn direct_children(
    client: &KeycloakClient,
    flow_alias: &str,
) -> Result<Vec<AuthenticationExecutionInfoRepresentation>> {
    Ok(client
        .get_flow_executions(flow_alias)
        .await?
        .into_iter()
        .filter(|row| row.level.unwrap_or(0) == 0)
        .collect())
}

/// Finds the ID of a flow by alias: flows applied in this run first (standalone sub-flows are not
/// listed by Keycloak until linked), then every flow known to the server.
async fn find_flow_id(client: &KeycloakClient, alias: &str) -> Result<Option<String>> {
    if let Some(id) = client.known_flow_id(alias) {
        return Ok(Some(id));
    }
    Ok(client
        .get_raw_flows_with_executions()
        .await?
        .into_iter()
        .find(|f| f.alias.as_deref() == Some(alias))
        .and_then(|f| f.id))
}

/// Reconciles the executions of `flow` (already created/updated with ID `flow_id`).
pub async fn reconcile(
    client: &KeycloakClient,
    flow: &AuthenticationFlowRepresentation,
    flow_id: &str,
) -> Result<()> {
    let Some(alias) = flow.alias.as_deref() else {
        return Ok(());
    };
    client.remember_flow_id(alias, flow_id);
    let Some(desired) = flow.authentication_executions.as_ref() else {
        return Ok(());
    };

    let built_in = client
        .get_flow(flow_id)
        .await
        .with_context(|| format!("Failed to get authentication flow '{}'", alias))?
        .built_in
        == Some(true);
    let rows = direct_children(client, alias).await?;

    // Match declared executions to remote rows, in declaration order.
    let mut used = vec![false; rows.len()];
    let mut matches: Vec<Option<usize>> = Vec::with_capacity(desired.len());
    for exec in desired {
        let key = export_key(exec);
        let found = rows
            .iter()
            .enumerate()
            .position(|(j, row)| !used[j] && key.is_some() && row_key(row) == key);
        if let Some(j) = found {
            used[j] = true;
        }
        matches.push(found);
    }

    // Remove executions that are no longer declared.
    for (row, _) in rows.iter().zip(&used).filter(|(_, used)| !**used) {
        let key = row_key(row);
        if built_in {
            crate::utils::ui::log_line(format!(
                "  {} {}",
                WARN,
                style(format!(
                    "Cannot remove {} from built-in flow '{}': skipped",
                    describe(&key),
                    alias
                ))
                .yellow()
            ));
            continue;
        }
        let id = row
            .id
            .as_deref()
            .context("Remote execution is missing 'id'")?;
        client.delete_execution(id).await.with_context(|| {
            format!("Failed to remove {} from flow '{}'", describe(&key), alias)
        })?;
        crate::utils::ui::log_line(format!(
            "    {} Removed {} from flow {}",
            SUCCESS_UPDATE,
            describe(&key),
            alias
        ));
    }

    // Add missing executions and update requirement/priority of existing ones.
    for (index, (exec, matched)) in desired.iter().zip(&matches).enumerate() {
        let priority = target_priority(exec, index);
        let key = export_key(exec);
        match matched {
            Some(j) => {
                let row = &rows[*j];
                let requirement = exec.requirement.clone().or_else(|| row.requirement.clone());
                if row.requirement != requirement || row.priority != Some(priority) {
                    let mut updated = row.clone();
                    updated.requirement = requirement;
                    updated.priority = Some(priority);
                    client
                        .update_flow_execution(alias, &updated)
                        .await
                        .with_context(|| {
                            format!("Failed to update {} in flow '{}'", describe(&key), alias)
                        })?;
                    crate::utils::ui::log_line(format!(
                        "    {} Updated {} in flow {}",
                        SUCCESS_UPDATE,
                        describe(&key),
                        alias
                    ));
                }
            }
            None => {
                if built_in {
                    anyhow::bail!(
                        "Cannot add {} to built-in flow '{}': copy the flow and bind the copy instead",
                        describe(&key),
                        alias
                    );
                }
                add_execution(client, alias, flow_id, exec, priority)
                    .await
                    .with_context(|| {
                        format!("Failed to add {} to flow '{}'", describe(&key), alias)
                    })?;
                crate::utils::ui::log_line(format!(
                    "    {} Added {} to flow {}",
                    SUCCESS_CREATE,
                    describe(&key),
                    alias
                ));
            }
        }
    }
    Ok(())
}

async fn add_execution(
    client: &KeycloakClient,
    flow_alias: &str,
    flow_id: &str,
    exec: &AuthenticationExecutionExportRepresentation,
    priority: i32,
) -> Result<()> {
    let mut body = serde_json::json!({
        "parentFlow": flow_id,
        "priority": priority,
    });
    if let Some(requirement) = &exec.requirement {
        body["requirement"] = serde_json::json!(requirement);
    }

    if !exec.is_subflow() {
        let provider = exec
            .authenticator
            .as_deref()
            .context("Execution is missing 'authenticator'")?;
        body["authenticator"] = serde_json::json!(provider);
        body["authenticatorFlow"] = serde_json::json!(false);
        client.add_execution(&body).await?;
        return Ok(());
    }

    let subflow_alias = exec
        .flow_alias
        .as_deref()
        .context("Sub-flow execution is missing 'flowAlias'")?;
    if let Some(subflow_id) = find_flow_id(client, subflow_alias).await? {
        // The sub-flow has its own file (applied in an earlier tier): link it.
        body["authenticatorFlow"] = serde_json::json!(true);
        body["flowId"] = serde_json::json!(subflow_id);
        client.add_execution(&body).await?;
        return Ok(());
    }

    // No such flow yet: create an empty sub-flow inline, then set requirement and priority.
    client
        .add_subflow_execution(flow_alias, subflow_alias, "basic-flow", None)
        .await?;
    let row = direct_children(client, flow_alias)
        .await?
        .into_iter()
        .find(|row| row.is_subflow() && row.display_name.as_deref() == Some(subflow_alias))
        .with_context(|| format!("Created sub-flow '{}' not found", subflow_alias))?;
    let mut updated = row;
    if exec.requirement.is_some() {
        updated.requirement = exec.requirement.clone();
    }
    updated.priority = Some(priority);
    client.update_flow_execution(flow_alias, &updated).await?;
    if let Some(id) = updated.flow_id.as_deref() {
        client.remember_flow_id(subflow_alias, id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn export(v: serde_json::Value) -> AuthenticationExecutionExportRepresentation {
        serde_json::from_value(v).unwrap()
    }

    fn info(v: serde_json::Value) -> AuthenticationExecutionInfoRepresentation {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn test_execution_keys() {
        let auth = export(json!({"authenticator": "auth-cookie"}));
        let sub = export(json!({"authenticatorFlow": true, "flowAlias": "forms"}));
        assert_eq!(
            export_key(&auth),
            Some(ExecutionKey::Authenticator("auth-cookie"))
        );
        assert_eq!(export_key(&sub), Some(ExecutionKey::SubFlow("forms")));

        let row_auth = info(json!({"providerId": "auth-cookie", "displayName": "Cookie"}));
        let row_sub = info(json!({"authenticationFlow": true, "displayName": "forms"}));
        assert_eq!(row_key(&row_auth), export_key(&auth));
        assert_eq!(row_key(&row_sub), export_key(&sub));
    }

    /// Minimal stateful Keycloak mock for the executions API of realm `r`.
    struct MockKeycloak {
        built_in: bool,
        rows: Vec<serde_json::Value>,
        calls: Vec<String>,
    }

    async fn start(state: MockKeycloak) -> (String, Arc<Mutex<MockKeycloak>>) {
        use axum::http::{Method, StatusCode, Uri, header};
        use axum::response::IntoResponse;
        let state = Arc::new(Mutex::new(state));
        let shared = Arc::clone(&state);
        let app = axum::Router::new().fallback(
            move |method: Method, uri: Uri, body: axum::body::Bytes| {
                let state = Arc::clone(&shared);
                async move {
                    let path = uri
                        .path()
                        .trim_start_matches("/admin/realms/r/authentication");
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let mut st = state.lock().unwrap();
                    st.calls.push(format!("{} {}", method, path));
                    match (method.as_str(), path) {
                        ("GET", "/flows") => axum::Json(json!([
                            {"id": "sub-id", "alias": "existing-sub", "topLevel": true}
                        ]))
                        .into_response(),
                        ("GET", "/flows/flow-id") => axum::Json(
                            json!({"id": "flow-id", "alias": "f", "builtIn": st.built_in}),
                        )
                        .into_response(),
                        ("GET", p) if p.ends_with("/executions") => {
                            axum::Json(serde_json::Value::Array(st.rows.clone())).into_response()
                        }
                        ("DELETE", p) if p.starts_with("/executions/") => {
                            let id = p.trim_start_matches("/executions/").to_string();
                            st.rows.retain(|r| r["id"] != json!(id));
                            StatusCode::NO_CONTENT.into_response()
                        }
                        ("PUT", p) if p.ends_with("/executions") => {
                            if let Some(row) = st.rows.iter_mut().find(|r| r["id"] == body["id"]) {
                                *row = body;
                            }
                            StatusCode::NO_CONTENT.into_response()
                        }
                        ("POST", "/executions") => {
                            let id = format!("new-{}", st.rows.len());
                            let mut row = json!({
                                "id": id, "level": 0,
                                "requirement": body["requirement"],
                                "priority": body["priority"],
                            });
                            if body["authenticatorFlow"] == json!(true) {
                                row["authenticationFlow"] = json!(true);
                                row["flowId"] = body["flowId"].clone();
                                row["displayName"] = json!("existing-sub");
                            } else {
                                row["providerId"] = body["authenticator"].clone();
                            }
                            st.rows.push(row);
                            (
                                StatusCode::CREATED,
                                [(header::LOCATION, format!("http://x/{}", id))],
                            )
                                .into_response()
                        }
                        ("POST", p) if p.ends_with("/executions/flow") => {
                            st.rows.push(json!({
                                "id": "inline-row", "level": 0, "authenticationFlow": true,
                                "displayName": body["alias"], "flowId": "inline-flow",
                                "requirement": "DISABLED", "priority": 99
                            }));
                            StatusCode::CREATED.into_response()
                        }
                        _ => StatusCode::NOT_IMPLEMENTED.into_response(),
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, state)
    }

    fn flow(execs: serde_json::Value) -> AuthenticationFlowRepresentation {
        serde_json::from_value(json!({"alias": "f", "authenticationExecutions": execs})).unwrap()
    }

    fn client(url: String) -> KeycloakClient {
        let mut c = KeycloakClient::new(url);
        c.set_target_realm("r".to_string());
        c.set_token("t".to_string());
        c
    }

    #[tokio::test]
    async fn test_reconcile_adds_updates_removes_and_links() {
        let (url, state) = start(MockKeycloak {
            built_in: false,
            rows: vec![
                json!({"id": "cookie", "providerId": "auth-cookie", "requirement": "ALTERNATIVE", "priority": 10, "level": 0}),
                json!({"id": "otp", "providerId": "auth-otp-form", "requirement": "REQUIRED", "priority": 20, "level": 0}),
                json!({"id": "nested", "providerId": "deep", "requirement": "REQUIRED", "priority": 10, "level": 1}),
            ],
            calls: vec![],
        })
        .await;
        let c = client(url);
        let desired = flow(json!([
            {"authenticator": "auth-otp-form", "requirement": "ALTERNATIVE"},
            {"authenticator": "auth-username-password-form", "requirement": "REQUIRED"},
            {"authenticatorFlow": true, "flowAlias": "existing-sub", "requirement": "CONDITIONAL"},
            {"authenticatorFlow": true, "flowAlias": "brand-new-sub", "requirement": "REQUIRED", "priority": 70}
        ]));
        reconcile(&c, &desired, "flow-id").await.unwrap();

        let st = state.lock().unwrap();
        let level0: Vec<_> = st.rows.iter().filter(|r| r["level"] == 0).collect();
        // cookie removed; otp updated; password form and both sub-flows added
        assert!(!level0.iter().any(|r| r["id"] == "cookie"));
        let otp = level0.iter().find(|r| r["id"] == "otp").unwrap();
        assert_eq!(otp["requirement"], "ALTERNATIVE");
        assert_eq!(otp["priority"], 10);
        let pw = level0
            .iter()
            .find(|r| r["providerId"] == "auth-username-password-form")
            .unwrap();
        assert_eq!(pw["priority"], 20);
        let linked = level0.iter().find(|r| r["flowId"] == "sub-id").unwrap();
        assert_eq!(linked["requirement"], "CONDITIONAL");
        let inline = level0.iter().find(|r| r["id"] == "inline-row").unwrap();
        assert_eq!(inline["requirement"], "REQUIRED");
        assert_eq!(inline["priority"], 70);
        // Nested rows belong to sub-flows and are never touched
        assert!(st.rows.iter().any(|r| r["id"] == "nested"));
        drop(st);
        assert_eq!(
            c.known_flow_id("brand-new-sub").as_deref(),
            Some("inline-flow")
        );
        assert_eq!(c.known_flow_id("f").as_deref(), Some("flow-id"));
    }

    #[tokio::test]
    async fn test_reconcile_built_in_flow_only_updates_requirements() {
        let (url, state) = start(MockKeycloak {
            built_in: true,
            rows: vec![
                json!({"id": "cookie", "providerId": "auth-cookie", "requirement": "ALTERNATIVE", "priority": 10, "level": 0}),
                json!({"id": "kerberos", "providerId": "auth-spnego", "requirement": "DISABLED", "priority": 20, "level": 0}),
            ],
            calls: vec![],
        })
        .await;
        let c = client(url.clone());
        // Removing kerberos is skipped with a warning; cookie requirement is updated.
        reconcile(
            &c,
            &flow(json!([{"authenticator": "auth-cookie", "requirement": "DISABLED"}])),
            "flow-id",
        )
        .await
        .unwrap();
        {
            let st = state.lock().unwrap();
            assert_eq!(st.rows.len(), 2);
            assert_eq!(st.rows[0]["requirement"], "DISABLED");
            assert!(!st.calls.iter().any(|c| c.starts_with("DELETE")));
        }
        // Adding an execution to a built-in flow is an error.
        let err = reconcile(
            &c,
            &flow(json!([{"authenticator": "auth-otp-form"}])),
            "flow-id",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("built-in flow"), "{err}");
    }

    #[tokio::test]
    async fn test_reconcile_without_executions_is_a_no_op() {
        let (url, state) = start(MockKeycloak {
            built_in: false,
            rows: vec![],
            calls: vec![],
        })
        .await;
        let c = client(url);
        let unmanaged: AuthenticationFlowRepresentation =
            serde_json::from_value(json!({"alias": "f"})).unwrap();
        reconcile(&c, &unmanaged, "flow-id").await.unwrap();
        assert!(state.lock().unwrap().calls.is_empty());
        let nameless: AuthenticationFlowRepresentation =
            serde_json::from_value(json!({"authenticationExecutions": []})).unwrap();
        reconcile(&c, &nameless, "flow-id").await.unwrap();
    }

    #[test]
    fn test_target_priority() {
        let explicit = export(json!({"authenticator": "a", "priority": 7}));
        let implicit = export(json!({"authenticator": "b"}));
        assert_eq!(target_priority(&explicit, 3), 7);
        assert_eq!(target_priority(&implicit, 0), 10);
        assert_eq!(target_priority(&implicit, 2), 30);
    }
}
