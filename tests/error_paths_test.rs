//! Mock-server tests for error reporting: every failure names the resource, file or request
//! involved, and safe failures are reported as warnings instead of aborting.
use axum::http::{Method, StatusCode, Uri};
use axum::response::IntoResponse;
use kaji::apply::{self, ApplyContext};
use kaji::client::{KeycloakClient, KeycloakResourceMapping};
use kaji::models::{
    AuthenticationFlowRepresentation, AuthenticatorConfigRepresentation, ClientRepresentation,
    ClientScopeRepresentation, GroupRepresentation, RequiredActionProviderRepresentation,
    RoleRepresentation, UserRepresentation,
};
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::{MockUi, Ui};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

/// Answers from a `(METHOD path) -> (status, body)` table. Unknown GETs return `[]`, other
/// unknown requests `204`. `{"__location": url}` answers with a Location header.
async fn start(responses: Vec<(&str, u16, Value)>) -> String {
    let responses: Arc<Mutex<HashMap<String, (u16, Value)>>> = Arc::new(Mutex::new(
        responses
            .into_iter()
            .map(|(k, s, b)| (k.to_string(), (s, b)))
            .collect(),
    ));
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri| {
        let responses = Arc::clone(&responses);
        async move {
            let key = format!("{} {}", method, uri.path());
            match responses.lock().unwrap().get(&key) {
                Some((status, body)) if body.get("__location").is_some() => (
                    StatusCode::from_u16(*status).unwrap(),
                    [(
                        axum::http::header::LOCATION,
                        body["__location"].as_str().unwrap_or_default().to_string(),
                    )],
                )
                    .into_response(),
                Some((status, body)) => (
                    StatusCode::from_u16(*status).unwrap(),
                    axum::Json(body.clone()),
                )
                    .into_response(),
                None if method == Method::GET => axum::Json(json!([])).into_response(),
                None => StatusCode::NO_CONTENT.into_response(),
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn client(url: String) -> KeycloakClient {
    let mut c = KeycloakClient::new(url);
    c.set_target_realm("r".to_string());
    c.set_token("t".to_string());
    c
}

fn ctx<'a>(client: &'a KeycloakClient, realm_dir: &Path) -> ApplyContext<'a> {
    ApplyContext {
        client,
        workspace_dir: realm_dir.to_path_buf(),
        secrets_path: Arc::new(realm_dir.join(".secrets")),
        resolver: Arc::new(EnvResolver::new(HashMap::new())) as Arc<dyn SecretResolver>,
        planned_files: Arc::new(None),
        realm_name: "r",
        profile: None,
        review: false,
        ui: Arc::new(MockUi::new()) as Arc<dyn Ui>,
        yes: true,
        prune: false,
        prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn chain<T>(result: anyhow::Result<T>) -> String {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(e) => format!("{:#}", e),
    }
}

fn assert_contains(msg: &str, expected: &str) {
    assert!(msg.contains(expected), "expected {expected:?} in:\n{msg}");
}

// --- client ---------------------------------------------------------------------------------

#[tokio::test]
async fn realm_exists_distinguishes_a_missing_realm() {
    let found = client(start(vec![("GET /admin/realms/r", 200, json!({"realm": "r"}))]).await);
    assert!(found.realm_exists().await.unwrap());
    let missing = client(start(vec![("GET /admin/realms/r", 404, json!({}))]).await);
    assert!(!missing.realm_exists().await.unwrap());
    let broken = client(start(vec![("GET /admin/realms/r", 500, json!({}))]).await);
    assert_contains(
        &chain(broken.realm_exists().await),
        "Failed to check whether the realm exists",
    );
}

#[tokio::test]
async fn group_children_failure_names_the_group() {
    let c = client(
        start(vec![(
            "GET /admin/realms/r/groups/g1/children",
            500,
            json!({}),
        )])
        .await,
    );
    let mut group: GroupRepresentation =
        serde_json::from_value(json!({"id": "g1", "name": "eng", "path": "/eng"})).unwrap();
    assert_contains(
        &chain(c.load_group_children(&mut group).await),
        "Failed to list sub-groups of group '/eng'",
    );
}

fn top_flow(executions: Value) -> (&'static str, u16, Value) {
    (
        "GET /admin/realms/r/authentication/flows",
        200,
        json!([{"id": "f1", "alias": "top", "authenticationExecutions": executions}]),
    )
}

#[tokio::test]
async fn sub_flow_fetch_failure_names_the_sub_flow() {
    let c = client(
        start(vec![
            top_flow(json!([])),
            (
                "GET /admin/realms/r/authentication/flows/top/executions",
                200,
                json!([{"id": "e1", "authenticationFlow": true, "flowId": "sf1",
                        "displayName": "sub", "level": 0}]),
            ),
            (
                "GET /admin/realms/r/authentication/flows/sf1",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(c.get_resources::<AuthenticationFlowRepresentation>().await),
        "Failed to fetch sub-flow 'sub' (sf1)",
    );
}

#[tokio::test]
async fn executions_errors_name_the_flow() {
    let c = client(
        start(vec![
            top_flow(json!([])),
            (
                "GET /admin/realms/r/authentication/flows/top/executions",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(c.get_resources::<AuthenticationFlowRepresentation>().await),
        "Failed to list the executions of authentication flow 'top'",
    );

    // 404 while the provider lists cannot be read: the generic explanation is given.
    let c = client(
        start(vec![
            top_flow(json!([{"authenticator": "custom-auth", "requirement": "REQUIRED"}])),
            (
                "GET /admin/realms/r/authentication/flows/top/executions",
                404,
                json!({"error": "Could not find authenticator provider"}),
            ),
            (
                "GET /admin/realms/r/authentication/authenticator-providers",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(c.get_resources::<AuthenticationFlowRepresentation>().await),
        "'top': one of its executions may use an authenticator provider that is not installed",
    );
}

#[tokio::test]
async fn authenticator_config_fetch_failure_names_the_config() {
    let c = client(
        start(vec![
            top_flow(json!([])),
            (
                "GET /admin/realms/r/authentication/flows/top/executions",
                200,
                json!([{"id": "e1", "providerId": "otp", "displayName": "OTP Form",
                        "authenticationConfig": "c1", "level": 0}]),
            ),
            (
                "GET /admin/realms/r/authentication/config/c1",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(c.get_authenticator_configs_internal().await),
        "Failed to fetch authenticator config c1 (of execution 'OTP Form')",
    );
}

#[tokio::test]
async fn authenticator_config_creation_failures() {
    let config: AuthenticatorConfigRepresentation =
        serde_json::from_value(json!({"alias": "otp-config"})).unwrap();
    let failing = client(
        start(vec![(
            "POST /admin/realms/r/authentication/executions/e1/config",
            500,
            json!({}),
        )])
        .await,
    );
    assert_contains(
        &chain(
            failing
                .create_authenticator_config_for_execution("e1", &config)
                .await,
        ),
        "Failed to create authenticator config 'otp-config' for execution e1",
    );
    let no_id = client(
        start(vec![(
            "POST /admin/realms/r/authentication/executions/e1/config",
            201,
            json!({}),
        )])
        .await,
    );
    assert_contains(
        &chain(
            no_id
                .create_authenticator_config_for_execution("e1", &config)
                .await,
        ),
        "returned no config ID",
    );
}

#[tokio::test]
async fn multi_step_creations_name_the_failed_step() {
    let c = client(
        start(vec![
            ("POST /admin/realms/r/roles", 201, json!({})),
            ("GET /admin/realms/r/roles/new-role", 500, json!({})),
            (
                "PUT /admin/realms/r/authentication/required-actions/my-action",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    let role: RoleRepresentation = serde_json::from_value(json!({"name": "new-role"})).unwrap();
    assert_contains(
        &chain(RoleRepresentation::create(&c, &role).await),
        "Role 'new-role' was created, but looking up its ID failed",
    );
    let action: RequiredActionProviderRepresentation =
        serde_json::from_value(json!({"alias": "my-action", "providerId": "my-action"})).unwrap();
    assert_contains(
        &chain(RequiredActionProviderRepresentation::create(&c, &action).await),
        "Required action 'my-action' was registered, but configuring it failed",
    );
}

// --- apply ----------------------------------------------------------------------------------

#[tokio::test]
async fn unknown_linked_client_scope_is_named() {
    let url = start(vec![
        (
            "POST /admin/realms/r/clients",
            201,
            json!({"__location": "http://x/admin/realms/r/clients/c1"}),
        ),
        (
            "GET /admin/realms/r/clients/c1/default-client-scopes",
            200,
            json!([{"id": "o", "name": "old"}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/app.yaml"),
        "clientId: app\ndefaultClientScopes:\n- wanted\n",
    );
    let msg =
        chain(apply::generic::apply_resources::<ClientRepresentation>(ctx(&c, dir.path())).await);
    assert_contains(&msg, "clients/app.yaml");
    assert_contains(
        &msg,
        "Client scope 'old' linked to client 'app' not found on the server",
    );
}

#[tokio::test]
async fn unknown_client_in_client_roles_is_named() {
    let url = start(vec![
        (
            "POST /admin/realms/r/users",
            201,
            json!({"__location": "http://x/admin/realms/r/users/u1"}),
        ),
        ("GET /admin/realms/r/users/u1/role-mappings", 200, json!({})),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("users/u.yaml"),
        "username: u\nclientRoles:\n  missing-client:\n  - reader\n",
    );
    assert_contains(
        &chain(apply::generic::apply_resources::<UserRepresentation>(ctx(&c, dir.path())).await),
        "Client 'missing-client' (in the clientRoles of user u) not found on the server",
    );
}

#[tokio::test]
async fn unknown_client_in_role_composites_is_named() {
    let url = start(vec![
        (
            "GET /admin/realms/r/roles/admin",
            200,
            json!({"id": "rid", "name": "admin"}),
        ),
        (
            "GET /admin/realms/r/roles-by-id/rid/composites",
            200,
            json!([]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("roles/admin.yaml"),
        "name: admin\ncomposites:\n  client:\n    missing-client:\n    - reader\n",
    );
    let msg = chain(apply::client_roles::apply_role_composites(ctx(&c, dir.path())).await);
    assert_contains(&msg, "roles/admin.yaml");
    assert_contains(
        &msg,
        "Client 'missing-client' (in the composites of realm role admin) not found on the server",
    );
}

#[tokio::test]
async fn client_role_lookup_failure_is_named() {
    let url = start(vec![
        (
            "GET /admin/realms/r/clients",
            200,
            json!([{"id": "c1", "clientId": "app"}]),
        ),
        (
            "GET /admin/realms/r/clients/c1/roles/reader",
            500,
            json!({}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/app/roles/reader.yaml"),
        "name: reader\ncomposites:\n  realm:\n  - offline_access\n",
    );
    assert_contains(
        &chain(apply::client_roles::apply_role_composites(ctx(&c, dir.path())).await),
        "Failed to look up role 'reader' of client 'app'",
    );
}

#[tokio::test]
async fn created_resource_missing_from_the_list_is_a_warning() {
    // No Location header and not listed afterwards: applied, but relationships are skipped.
    let url = start(vec![("POST /admin/realms/r/client-scopes", 201, json!({}))]).await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(&dir.path().join("client-scopes/s.yaml"), "name: s\n");
    apply::generic::apply_resources::<ClientScopeRepresentation>(ctx(&c, dir.path()))
        .await
        .unwrap();
}

#[tokio::test]
async fn realm_lookup_failures_are_named() {
    let url = start(vec![("GET /admin/realms/r", 500, json!({}))]).await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(&dir.path().join("realm.yaml"), "realm: r\n");
    let msg = chain(apply::realm::apply_realm(ctx(&c, dir.path())).await);
    assert_contains(&msg, "realm.yaml");
    assert_contains(&msg, "Failed to check whether the realm exists");

    // Components need the realm ID to resolve their parents.
    write(
        &dir.path().join("components/ldap.yaml"),
        "name: ldap\nproviderId: ldap\nproviderType: org.keycloak.storage.UserStorageProvider\n",
    );
    assert_contains(
        &chain(
            apply::components::apply_components_or_keys(ctx(&c, dir.path()), "components").await,
        ),
        "Failed to read the realm ID (needed to resolve components' parentId)",
    );
}

#[tokio::test]
async fn authenticator_config_errors_name_the_config_and_flow() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("authenticator-configs/a.yaml"),
        "alias: a\n",
    );

    // Not referenced by any local execution.
    let c = client(start(vec![]).await);
    let msg =
        chain(apply::authenticator_config::apply_authenticator_configs(ctx(&c, dir.path())).await);
    assert_contains(
        &msg,
        "No local authentication execution references authenticator config 'a'",
    );
    assert_contains(&msg, "Hint: Keycloak ties a config to one execution");

    // Referenced, but the flow's executions cannot be listed.
    write(
        &dir.path().join("authentication-flows/f.yaml"),
        "alias: f\nauthenticationExecutions:\n- authenticator: otp\n  authenticatorConfig: a\n",
    );
    let c = client(
        start(vec![(
            "GET /admin/realms/r/authentication/flows/f/executions",
            500,
            json!({}),
        )])
        .await,
    );
    assert_contains(
        &chain(apply::authenticator_config::apply_authenticator_configs(ctx(&c, dir.path())).await),
        "Failed to list the executions of authentication flow 'f'",
    );

    // The execution exists, but attaching the config fails.
    let c = client(
        start(vec![
            (
                "GET /admin/realms/r/authentication/flows/f/executions",
                200,
                json!([{"id": "e1", "providerId": "otp", "level": 0}]),
            ),
            (
                "POST /admin/realms/r/authentication/executions/e1/config",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::authenticator_config::apply_authenticator_configs(ctx(&c, dir.path())).await),
        "Failed to attach authenticator config 'a' to execution 'otp' of flow 'f'",
    );

    // Remote config without an ID.
    let c = client(
        start(vec![
            top_flow(json!([])),
            (
                "GET /admin/realms/r/authentication/flows/top/executions",
                200,
                json!([{"id": "e1", "providerId": "otp", "authenticationConfig": "c1", "level": 0}]),
            ),
            ("GET /admin/realms/r/authentication/config/c1", 200, json!({"alias": "a"})),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::authenticator_config::apply_authenticator_configs(ctx(&c, dir.path())).await),
        "Remote authenticator config 'a' has no ID",
    );
}

#[tokio::test]
async fn adding_an_execution_to_a_built_in_flow_gives_its_position_and_a_hint() {
    let url = start(vec![
        (
            "GET /admin/realms/r/authentication/flows",
            200,
            json!([{"id": "b1", "alias": "browser", "builtIn": true}]),
        ),
        (
            "GET /admin/realms/r/authentication/flows/b1",
            200,
            json!({"id": "b1", "alias": "browser", "builtIn": true}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("authentication-flows/browser.yaml"),
        "alias: browser\nbuiltIn: true\nauthenticationExecutions:\n- requirement: REQUIRED\n",
    );
    let msg = chain(
        apply::generic::apply_resources::<AuthenticationFlowRepresentation>(ctx(&c, dir.path()))
            .await,
    );
    assert_contains(
        &msg,
        "Cannot add execution #1 (no 'authenticator' or 'flowAlias') to built-in flow 'browser'",
    );
    assert_contains(&msg, "Hint: copy the flow");
}

// --- plan and inspect -----------------------------------------------------------------------

fn plan_args<'a>(client: &'a KeycloakClient, workspace: &Path) -> kaji::plan::PlanArgs<'a> {
    kaji::plan::PlanArgs {
        client,
        workspace_dir: workspace.to_path_buf(),
        changes_only: false,
        interactive: false,
        realms_to_plan: &[],
        ui: Arc::new(MockUi::new()),
        resolver: Arc::new(EnvResolver::new(HashMap::new())),
        profile: None,
        verbose: false,
    }
}

#[tokio::test]
async fn plan_of_a_new_realm_creates_everything() {
    let url = start(vec![
        (
            "GET /admin/realms/r",
            404,
            json!({"error": "Realm not found."}),
        ),
        (
            "GET /admin/realms/r/roles",
            404,
            json!({"error": "Realm not found."}),
        ),
        (
            "GET /admin/realms/r/clients",
            404,
            json!({"error": "Realm not found."}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(&dir.path().join("r/realm.yaml"), "realm: r\n");
    write(&dir.path().join("r/roles/admin.yaml"), "name: admin\n");
    write(
        &dir.path().join("r/clients/app/roles/reader.yaml"),
        "name: reader\n",
    );
    let summary = kaji::plan::run_with_outcome(plan_args(&c, dir.path()), false)
        .await
        .unwrap();
    assert_eq!(summary.created, 3);
}

#[tokio::test]
async fn plan_errors_name_the_realm_and_resource() {
    let url = start(vec![
        ("GET /admin/realms/r", 200, json!({"realm": "r"})),
        (
            "GET /admin/realms/r/clients",
            200,
            json!([{"id": "c1", "clientId": "app"}]),
        ),
        (
            "GET /admin/realms/r/clients/c1/roles",
            200,
            json!([{"id": "rid", "name": "reader"}]),
        ),
        (
            "GET /admin/realms/r/roles-by-id/rid/composites",
            500,
            json!({}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("r/clients/app/roles/reader.yaml"),
        "name: reader\ncomposites:\n  realm:\n  - offline_access\n",
    );
    let msg = chain(kaji::plan::run_with_outcome(plan_args(&c, dir.path()), false).await);
    assert_contains(&msg, "Failed to plan realm 'r'");
    assert_contains(
        &msg,
        "Failed to load the relationships of role reader of client app from Keycloak",
    );
}

#[tokio::test]
async fn plan_reports_an_outdated_plan_it_cannot_remove() {
    // Nothing to plan (no server request needed), but `.kajiplan` is a directory.
    let c = client("http://127.0.0.1:9".to_string());
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("r")).unwrap();
    fs::create_dir_all(dir.path().join(".kajiplan")).unwrap();
    assert_contains(
        &chain(kaji::plan::run_with_outcome(plan_args(&c, dir.path()), true).await),
        "Failed to remove the outdated plan",
    );
}

#[tokio::test]
async fn inspect_errors_name_the_realm_and_client() {
    let url = start(vec![
        ("GET /admin/realms/r", 200, json!({"realm": "r"})),
        (
            "GET /admin/realms/r/clients",
            200,
            json!([{"id": "c1", "clientId": "app"}]),
        ),
        ("GET /admin/realms/r/clients/c1/roles", 500, json!({})),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    let msg = chain(
        kaji::inspect::run_with_ui(
            &c,
            dir.path().to_path_buf(),
            &["r".to_string()],
            true,
            Arc::new(MockUi::new()),
        )
        .await,
    );
    assert_contains(&msg, "Failed to inspect realm 'r'");
    assert_contains(&msg, "Failed to fetch the roles of client 'app'");

    // The workspace cannot be created (its parent is a file).
    let file = dir.path().join("file");
    fs::write(&file, "").unwrap();
    assert_contains(
        &chain(
            kaji::inspect::run_with_ui(
                &c,
                file.join("ws"),
                &["r".to_string()],
                true,
                Arc::new(MockUi::new()),
            )
            .await,
        ),
        "Failed to create the workspace directory",
    );
}

#[tokio::test]
async fn sub_group_errors_name_the_group_path() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("groups/eng.yaml"),
        "name: eng\nsubGroups:\n- name: team\n",
    );
    let created = (
        "POST /admin/realms/r/groups",
        201,
        json!({"__location": "http://x/admin/realms/r/groups/g1"}),
    );

    // An undeclared remote sub-group without ID cannot be removed.
    let c = client(
        start(vec![
            created.clone(),
            (
                "GET /admin/realms/r/groups/g1/children",
                200,
                json!([{"name": "old"}]),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::generic::apply_resources::<GroupRepresentation>(ctx(&c, dir.path())).await),
        "Sub-group 'old' of group /eng has no ID",
    );

    // A created sub-group that Keycloak does not list.
    let c = client(
        start(vec![
            created,
            ("POST /admin/realms/r/groups/g1/children", 201, json!({})),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::generic::apply_resources::<GroupRepresentation>(ctx(&c, dir.path())).await),
        "Created sub-group 'team' of group /eng is not listed by Keycloak",
    );
}

#[tokio::test]
async fn client_role_id_problems_are_named() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/app/roles/reader.yaml"),
        "name: reader\n",
    );
    let app = (
        "GET /admin/realms/r/clients",
        200,
        json!([{"id": "c1", "clientId": "app"}]),
    );

    // Created, but the lookup of its ID fails.
    let c = client(
        start(vec![
            app.clone(),
            ("POST /admin/realms/r/clients/c1/roles", 201, json!({})),
            (
                "GET /admin/realms/r/clients/c1/roles/reader",
                500,
                json!({}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::client_roles::apply_client_roles(ctx(&c, dir.path())).await),
        "Created role reader of client app, but looking up its ID failed",
    );

    // Composites of a role the server returns without ID.
    write(
        &dir.path().join("clients/app/roles/reader.yaml"),
        "name: reader\ncomposites:\n  realm:\n  - offline_access\n",
    );
    let c = client(
        start(vec![
            app,
            (
                "GET /admin/realms/r/clients/c1/roles/reader",
                200,
                json!({"name": "reader"}),
            ),
        ])
        .await,
    );
    assert_contains(
        &chain(apply::client_roles::apply_role_composites(ctx(&c, dir.path())).await),
        "Role 'reader' of client 'app' has no ID",
    );
}

#[tokio::test]
async fn authenticator_configs_outside_the_plan_are_not_applied() {
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("authenticator-configs/a.yaml"),
        "alias: a\n",
    );
    // No server: nothing must be requested.
    let c = client("http://127.0.0.1:9".to_string());
    let mut context = ctx(&c, dir.path());
    context.planned_files = Arc::new(Some(Default::default()));
    apply::authenticator_config::apply_authenticator_configs(context)
        .await
        .unwrap();
}

#[tokio::test]
async fn unreachable_vault_is_named() {
    use kaji::utils::secrets::vault::VaultResolver;
    let resolver = VaultResolver::new("http://127.0.0.1:9", "token").unwrap();
    assert_contains(
        &chain(resolver.resolve("vault:secret/app#password").await),
        "Failed to read secret 'secret/app' from Vault at http://127.0.0.1:9",
    );
}
