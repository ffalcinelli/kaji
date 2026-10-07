//! Mock-server tests for relationship reconciliation (scope links, memberships, role
//! mappings, sub-groups, composites and client roles).
use axum::http::{Method, StatusCode, Uri};
use axum::response::IntoResponse;
use kaji::apply::{self, ApplyContext, relations};
use kaji::client::KeycloakClient;
use kaji::models::{
    ClientRepresentation, GroupRepresentation, RoleRepresentation, UserRepresentation,
};
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::{MockUi, Ui};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Default)]
struct Recorder {
    responses: HashMap<String, (u16, Value)>,
    calls: Vec<(String, Value)>,
}

/// Answers from a `(METHOD path) -> (status, body)` table; unknown GETs return `[]`, others 204.
async fn start(responses: Vec<(&str, u16, Value)>) -> (String, Arc<Mutex<Recorder>>) {
    let state = Arc::new(Mutex::new(Recorder {
        responses: responses
            .into_iter()
            .map(|(k, s, b)| (k.to_string(), (s, b)))
            .collect(),
        calls: Vec::new(),
    }));
    let shared = Arc::clone(&state);
    let app =
        axum::Router::new().fallback(move |method: Method, uri: Uri, body: axum::body::Bytes| {
            let state = Arc::clone(&shared);
            async move {
                let key = format!("{} {}", method, uri.path());
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let mut st = state.lock().unwrap();
                st.calls.push((key.clone(), body));
                match st.responses.get(&key) {
                    // `{"__location": url}` answers with a Location header
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
    (url, state)
}

fn client(url: String) -> KeycloakClient {
    let mut c = KeycloakClient::new(url);
    c.set_target_realm("r".to_string());
    c.set_token("t".to_string());
    c
}

fn calls(state: &Arc<Mutex<Recorder>>, key: &str) -> Vec<Value> {
    state
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, b)| b.clone())
        .collect()
}

fn names(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn ctx<'a>(
    client: &'a KeycloakClient,
    realm_dir: &Path,
    ui: Arc<dyn Ui>,
    prune: bool,
) -> ApplyContext<'a> {
    ApplyContext {
        client,
        workspace_dir: realm_dir.to_path_buf(),
        secrets_path: Arc::new(realm_dir.join(".secrets")),
        resolver: Arc::new(EnvResolver::new(HashMap::new())) as Arc<dyn SecretResolver>,
        planned_files: Arc::new(None),
        realm_name: "r",
        profile: None,
        review: false,
        ui,
        yes: true,
        prune,
        prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
    }
}

const CLIENTS: &str = "GET /admin/realms/r/clients";

#[tokio::test]
async fn client_scope_links_move_between_lists() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r/client-scopes",
            200,
            json!([{"id": "p", "name": "profile"}, {"id": "e", "name": "email"}, {"id": "c", "name": "custom"}]),
        ),
        (
            "GET /admin/realms/r/clients/c1/default-client-scopes",
            200,
            json!([{"id": "p", "name": "profile"}, {"id": "e", "name": "email"}]),
        ),
    ])
    .await;
    let c = client(url);
    let rep: ClientRepresentation = serde_json::from_value(json!({
        "clientId": "app",
        "defaultClientScopes": ["profile", "custom"],
        "optionalClientScopes": ["email"]
    }))
    .unwrap();
    relations::reconcile_client_scopes(&c, &rep, "c1")
        .await
        .unwrap();
    assert_eq!(
        calls(
            &state,
            "DELETE /admin/realms/r/clients/c1/default-client-scopes/e"
        )
        .len(),
        1
    );
    assert_eq!(
        calls(
            &state,
            "PUT /admin/realms/r/clients/c1/default-client-scopes/c"
        )
        .len(),
        1
    );
    assert_eq!(
        calls(
            &state,
            "PUT /admin/realms/r/clients/c1/optional-client-scopes/e"
        )
        .len(),
        1
    );

    // Unknown scope names are reported
    let bad: ClientRepresentation =
        serde_json::from_value(json!({"clientId": "app", "defaultClientScopes": ["missing"]}))
            .unwrap();
    let err = relations::reconcile_client_scopes(&c, &bad, "c1")
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("missing"));

    // Nothing declared: no request at all
    let before = state.lock().unwrap().calls.len();
    let plain: ClientRepresentation = serde_json::from_value(json!({"clientId": "app"})).unwrap();
    relations::reconcile_client_scopes(&c, &plain, "c1")
        .await
        .unwrap();
    assert_eq!(state.lock().unwrap().calls.len(), before);
}

#[tokio::test]
async fn user_groups_and_role_mappings_are_reconciled() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r/users/u1/groups",
            200,
            json!([{"id": "g-old", "path": "/old"}]),
        ),
        (
            "GET /admin/realms/r/group-by-path/old",
            200,
            json!({"id": "g-old", "path": "/old"}),
        ),
        (
            "GET /admin/realms/r/group-by-path/org/team",
            200,
            json!({"id": "g-team", "path": "/org/team"}),
        ),
        (
            "GET /admin/realms/r/users/u1/role-mappings",
            200,
            json!({
                "realmMappings": [{"id": "r-old", "name": "old"}],
                "clientMappings": {"legacy": {"id": "cl", "client": "legacy",
                    "mappings": [{"id": "lr", "name": "legacy-role", "clientRole": true}]}}
            }),
        ),
        (
            "GET /admin/realms/r/roles/new",
            200,
            json!({"id": "r-new", "name": "new"}),
        ),
        (
            CLIENTS,
            200,
            json!([{"id": "ca", "clientId": "api"}, {"id": "cl", "clientId": "legacy"}]),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles/reader",
            200,
            json!({"id": "cr", "name": "reader", "clientRole": true}),
        ),
    ])
    .await;
    let c = client(url);
    let user: UserRepresentation = serde_json::from_value(json!({
        "username": "bob",
        "groups": ["org/team"],
        "realmRoles": ["new"],
        "clientRoles": {"api": ["reader"]}
    }))
    .unwrap();
    relations::reconcile_user(&c, &user, "u1").await.unwrap();

    assert_eq!(
        calls(&state, "DELETE /admin/realms/r/users/u1/groups/g-old").len(),
        1
    );
    assert_eq!(
        calls(&state, "PUT /admin/realms/r/users/u1/groups/g-team").len(),
        1
    );
    let removed = calls(
        &state,
        "DELETE /admin/realms/r/users/u1/role-mappings/realm",
    );
    assert_eq!(names(&removed[0]), vec!["old"]);
    let added = calls(&state, "POST /admin/realms/r/users/u1/role-mappings/realm");
    assert_eq!(names(&added[0]), vec!["new"]);
    let client_added = calls(
        &state,
        "POST /admin/realms/r/users/u1/role-mappings/clients/ca",
    );
    assert_eq!(names(&client_added[0]), vec!["reader"]);
    let client_removed = calls(
        &state,
        "DELETE /admin/realms/r/users/u1/role-mappings/clients/cl",
    );
    assert_eq!(names(&client_removed[0]), vec!["legacy-role"]);

    // Relations are loaded only for declared keys
    let mut remote: UserRepresentation =
        serde_json::from_value(json!({"id": "u1", "username": "bob"})).unwrap();
    let declared: UserRepresentation =
        serde_json::from_value(json!({"username": "bob", "groups": []})).unwrap();
    relations::load_user_relations(&c, &mut remote, Some(&declared.extra))
        .await
        .unwrap();
    assert_eq!(remote.extra["groups"], json!(["/old"]));
    assert!(!remote.extra.contains_key("realmRoles"));
    relations::load_user_relations(&c, &mut remote, None)
        .await
        .unwrap();
    assert_eq!(remote.extra["realmRoles"], json!(["old"]));
    assert_eq!(
        remote.extra["clientRoles"],
        json!({"legacy": ["legacy-role"]})
    );
}

#[tokio::test]
async fn groups_reconcile_sub_groups_and_role_mappings() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r/groups/g1/children",
            200,
            json!([{"id": "old", "name": "old-team"}, {"id": "keep", "name": "team"}]),
        ),
        (
            "GET /admin/realms/r/roles/staff",
            200,
            json!({"id": "rs", "name": "staff"}),
        ),
        (
            "POST /admin/realms/r/groups/g1/children",
            201,
            json!({"__location": "http://x/admin/realms/r/groups/new"}),
        ),
    ])
    .await;
    let c = client(url);
    let group: GroupRepresentation = serde_json::from_value(json!({
        "name": "org",
        "realmRoles": ["staff"],
        "subGroups": [
            {"name": "team", "attributes": {"floor": ["3"]}, "path": "/org/team", "id": "x"},
            {"name": "new-team"}
        ]
    }))
    .unwrap();
    relations::reconcile_group(&c, &group, "g1").await.unwrap();

    assert_eq!(calls(&state, "DELETE /admin/realms/r/groups/old").len(), 1);
    let updated = calls(&state, "PUT /admin/realms/r/groups/keep");
    assert_eq!(updated[0]["attributes"]["floor"], json!(["3"]));
    assert!(updated[0].get("path").is_none(), "server fields stripped");
    let created = calls(&state, "POST /admin/realms/r/groups/g1/children");
    assert_eq!(created[0]["name"], "new-team");
    let mapped = calls(&state, "POST /admin/realms/r/groups/g1/role-mappings/realm");
    assert_eq!(names(&mapped[0]), vec!["staff"]);

    // Children are loaded recursively for plan/inspect
    let mut remote: GroupRepresentation =
        serde_json::from_value(json!({"id": "g1", "name": "org", "subGroupCount": 1})).unwrap();
    c.load_group_children(&mut remote).await.unwrap();
    assert_eq!(remote.sub_groups.as_ref().map(Vec::len), Some(2));
}

#[tokio::test]
async fn role_composites_are_reconciled_and_loaded() {
    let (url, state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/roles-by-id/rb/composites",
            200,
            json!([{"id": "o", "name": "obsolete", "clientRole": false},
                   {"id": "w", "name": "writer", "clientRole": true, "containerId": "ca"}]),
        ),
        (
            "GET /admin/realms/r/roles/viewer",
            200,
            json!({"id": "v", "name": "viewer"}),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles/reader",
            200,
            json!({"id": "cr", "name": "reader", "clientRole": true}),
        ),
    ])
    .await;
    let c = client(url);
    let role: RoleRepresentation = serde_json::from_value(json!({
        "name": "bundle",
        "composite": true,
        "composites": {"realm": ["viewer"], "client": {"api": ["reader"]}}
    }))
    .unwrap();
    relations::reconcile_role_composites(&c, &role, "rb", "realm role bundle")
        .await
        .unwrap();
    let removed = calls(&state, "DELETE /admin/realms/r/roles-by-id/rb/composites");
    assert_eq!(names(&removed[0]), vec!["obsolete", "writer"]);
    let added = calls(&state, "POST /admin/realms/r/roles-by-id/rb/composites");
    assert_eq!(names(&added[0]), vec!["reader", "viewer"]);

    let mut remote: RoleRepresentation =
        serde_json::from_value(json!({"id": "rb", "name": "bundle", "composite": true})).unwrap();
    relations::load_role_composites(&c, &mut remote)
        .await
        .unwrap();
    assert_eq!(
        remote.extra["composites"],
        json!({"realm": ["obsolete"], "client": {"api": ["writer"]}})
    );
}

#[tokio::test]
async fn client_roles_are_applied_pruned_and_planned() {
    let (url, state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/clients/ca/roles",
            200,
            json!([{"id": "r1", "name": "reader", "description": "old"},
                   {"id": "r2", "name": "orphan"}]),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles/writer",
            200,
            json!({"id": "r3", "name": "writer"}),
        ),
        (
            "GET /admin/realms/r/roles-by-id/r1",
            200,
            json!({"id": "r1", "name": "reader", "description": "Read"}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/api/roles/reader.yaml"),
        "name: reader\ndescription: Read\n",
    );
    write(
        &dir.path().join("clients/api/roles/writer.yaml"),
        "name: writer\n",
    );

    // Plan: reader differs, writer is new, orphan is reported
    let ui = Arc::new(MockUi::new());
    let plan_ctx = kaji::plan::PlanContext {
        client: &c,
        workspace_dir: dir.path(),
        options: kaji::plan::PlanOptions {
            changes_only: true,
            interactive: false,
            verbose: false,
        },
        resolver: Arc::new(EnvResolver::new(HashMap::new())),
        realm_name: "r",
        ui: &*ui,
        profile: None,
    };
    let (files, summary) = kaji::plan::client_roles::plan_client_roles(&plan_ctx)
        .await
        .unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        (summary.created, summary.updated, summary.orphaned),
        (1, 1, 1)
    );

    apply::client_roles::apply_client_roles(ctx(&c, dir.path(), Arc::new(MockUi::new()), true))
        .await
        .unwrap();
    let updated = calls(&state, "PUT /admin/realms/r/roles-by-id/r1");
    assert_eq!(updated[0]["description"], "Read");
    assert_eq!(updated[0]["clientRole"], true);
    let created = calls(&state, "POST /admin/realms/r/clients/ca/roles");
    assert_eq!(created[0]["name"], "writer");
    assert_eq!(
        calls(&state, "DELETE /admin/realms/r/roles-by-id/r2").len(),
        1
    );

    // Roles directory for an unknown client is an error
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/missing/roles/x.yaml"),
        "name: x\n",
    );
    let err = apply::client_roles::apply_client_roles(ctx(
        &c,
        dir.path(),
        Arc::new(MockUi::new()),
        false,
    ))
    .await
    .unwrap_err();
    assert!(format!("{:#}", err).contains("missing"));
}

#[tokio::test]
async fn role_composites_pass_covers_realm_and_client_roles() {
    let (url, state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/roles/bundle",
            200,
            json!({"id": "rb", "name": "bundle"}),
        ),
        (
            "GET /admin/realms/r/roles/plain",
            200,
            json!({"id": "rp", "name": "plain"}),
        ),
        (
            "GET /admin/realms/r/roles/viewer",
            200,
            json!({"id": "v", "name": "viewer"}),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles/writer",
            200,
            json!({"id": "w", "name": "writer"}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("roles/bundle.yaml"),
        "name: bundle\ncomposites:\n  realm:\n  - viewer\n",
    );
    write(&dir.path().join("roles/plain.yaml"), "name: plain\n");
    write(
        &dir.path().join("clients/api/roles/writer.yaml"),
        "name: writer\ncomposites:\n  realm:\n  - viewer\n",
    );
    apply::client_roles::apply_role_composites(ctx(&c, dir.path(), Arc::new(MockUi::new()), false))
        .await
        .unwrap();
    assert_eq!(
        calls(&state, "POST /admin/realms/r/roles-by-id/rb/composites").len(),
        1
    );
    assert_eq!(
        calls(&state, "POST /admin/realms/r/roles-by-id/w/composites").len(),
        1
    );
    assert!(calls(&state, "GET /admin/realms/r/roles-by-id/rp/composites").is_empty());
}

#[tokio::test]
async fn inspect_exports_client_roles_with_composites() {
    let (url, _state) = start(vec![
        (
            "GET /admin/realms/r",
            200,
            json!({"realm": "r", "id": "rid"}),
        ),
        (
            CLIENTS,
            200,
            json!([{"id": "ca", "clientId": "api"}, {"id": "cb", "clientId": "gone"}]),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles",
            200,
            json!([{"id": "r1", "name": "writer", "composite": true, "clientRole": true,
                    "containerId": "ca", "attributes": {}}]),
        ),
        (
            "GET /admin/realms/r/clients/cb/roles",
            404,
            json!({"error": "Could not find client"}),
        ),
        (
            "GET /admin/realms/r/roles-by-id/r1/composites",
            200,
            json!([{"id": "v", "name": "viewer", "clientRole": false}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    kaji::inspect::run_with_ui(
        &c,
        dir.path().to_path_buf(),
        &["r".to_string()],
        true,
        Arc::new(MockUi::new()),
    )
    .await
    .unwrap();
    let exported = fs::read_to_string(dir.path().join("r/clients/api/roles/writer.yaml")).unwrap();
    let role: Value = serde_yaml::from_str(&exported).unwrap();
    assert_eq!(role["name"], "writer");
    assert_eq!(role["composites"], json!({"realm": ["viewer"]}));
    assert!(role.get("id").is_none() && role.get("containerId").is_none());
    // A client deleted while inspecting is skipped
    assert!(!dir.path().join("r/clients/gone").exists());
}

#[tokio::test]
async fn interactive_client_role_plan_can_exclude_changes() {
    let (url, _state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/clients/ca/roles",
            200,
            json!([{"id": "r1", "name": "reader", "description": "old"}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/api/roles/reader.yaml"),
        "name: reader\ndescription: new\n",
    );
    write(
        &dir.path().join("clients/api/roles/writer.yaml"),
        "name: writer\n",
    );
    let ui = MockUi::new();
    // Exclude the update (No), include the creation (Yes)
    *ui.selects.lock().unwrap() = vec![1, 0];
    let plan_ctx = kaji::plan::PlanContext {
        client: &c,
        workspace_dir: dir.path(),
        options: kaji::plan::PlanOptions {
            changes_only: false,
            interactive: true,
            verbose: true,
        },
        resolver: Arc::new(EnvResolver::new(HashMap::new())),
        realm_name: "r",
        ui: &ui,
        profile: None,
    };
    let (files, summary) = kaji::plan::client_roles::plan_client_roles(&plan_ctx)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("writer.yaml"));
    assert_eq!((summary.created, summary.updated), (1, 0));
}

#[tokio::test]
async fn client_role_review_prompts_and_group_child_shortcuts() {
    let (url, state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/clients/ca/roles",
            200,
            json!([{"id": "r1", "name": "reader"}]),
        ),
        (
            "GET /admin/realms/r/clients/ca/roles/writer",
            200,
            json!({"id": "r3", "name": "writer"}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/api/roles/reader.yaml"),
        "name: reader\n",
    );
    write(
        &dir.path().join("clients/api/roles/writer.yaml"),
        "name: writer\n",
    );
    let ui = MockUi::new();
    // Decline the update of reader, accept the creation of writer
    *ui.confirms.lock().unwrap() = vec![false, true];
    let ui = Arc::new(ui);
    let mut review_ctx = ctx(&c, dir.path(), ui.clone(), false);
    review_ctx.review = true;
    apply::client_roles::apply_client_roles(review_ctx)
        .await
        .unwrap();
    assert!(ui.confirms.lock().unwrap().is_empty());
    assert!(calls(&state, "PUT /admin/realms/r/roles-by-id/r1").is_empty());
    assert_eq!(
        calls(&state, "POST /admin/realms/r/clients/ca/roles").len(),
        1
    );

    // Groups without children or without an ID never query /children
    let before = state.lock().unwrap().calls.len();
    let mut leaf: GroupRepresentation =
        serde_json::from_value(json!({"id": "g", "name": "leaf", "subGroupCount": 0})).unwrap();
    c.load_group_children(&mut leaf).await.unwrap();
    assert_eq!(leaf.sub_groups.as_ref().map(Vec::len), Some(0));
    let mut local_only: GroupRepresentation =
        serde_json::from_value(json!({"name": "local"})).unwrap();
    c.load_group_children(&mut local_only).await.unwrap();
    assert!(local_only.sub_groups.is_none());
    assert_eq!(state.lock().unwrap().calls.len(), before);
}

#[tokio::test]
async fn relationship_errors_name_the_missing_reference() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r/client-scopes",
            200,
            json!([{"id": "c", "name": "custom"}]),
        ),
        (
            "PUT /admin/realms/r/clients/c1/default-client-scopes/c",
            400,
            json!({"errorMessage": "cannot link"}),
        ),
        (
            "GET /admin/realms/r/users/u1/role-mappings",
            200,
            json!({"realmMappings": []}),
        ),
        (
            "GET /admin/realms/r/roles/ghost",
            404,
            json!({"error": "Role not found"}),
        ),
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/clients/ca/roles/ghost",
            404,
            json!({"error": "Role not found"}),
        ),
    ])
    .await;
    let c = client(url);

    let rep: ClientRepresentation =
        serde_json::from_value(json!({"clientId": "app", "defaultClientScopes": ["custom"]}))
            .unwrap();
    let err = relations::reconcile_client_scopes(&c, &rep, "c1")
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Failed to add default-client-scopes 'custom'"));

    let err = relations::reconcile_role_mappings(
        &c,
        &["users", "u1"],
        "user bob",
        Some(vec!["ghost".to_string()]),
        None,
    )
    .await
    .unwrap_err();
    assert!(format!("{:#}", err).contains("Realm role 'ghost' assigned to user bob not found"));

    let realm_composite: RoleRepresentation =
        serde_json::from_value(json!({"name": "b", "composites": {"realm": ["ghost"]}})).unwrap();
    let err = relations::reconcile_role_composites(&c, &realm_composite, "rb", "realm role b")
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Composite realm role 'ghost'"));
    let client_composite: RoleRepresentation =
        serde_json::from_value(json!({"name": "b", "composites": {"client": {"api": ["ghost"]}}}))
            .unwrap();
    let err = relations::reconcile_role_composites(&c, &client_composite, "rb", "realm role b")
        .await
        .unwrap_err();
    assert!(format!("{:#}", err).contains("Composite role 'ghost' of client 'api'"));

    // Nothing to do without IDs or declarations: no requests
    let before = state.lock().unwrap().calls.len();
    let mut local_user: UserRepresentation =
        serde_json::from_value(json!({"username": "x"})).unwrap();
    relations::load_user_relations(&c, &mut local_user, None)
        .await
        .unwrap();
    let mut local_role: RoleRepresentation = serde_json::from_value(json!({"name": "x"})).unwrap();
    relations::load_role_composites(&c, &mut local_role)
        .await
        .unwrap();
    relations::reconcile_role_composites(&c, &local_role, "rx", "role x")
        .await
        .unwrap();
    relations::reconcile_role_mappings(&c, &["users", "u1"], "user x", None, None)
        .await
        .unwrap();
    assert_eq!(state.lock().unwrap().calls.len(), before);
}

#[tokio::test]
async fn client_role_prune_asks_without_yes() {
    let (url, state) = start(vec![
        (CLIENTS, 200, json!([{"id": "ca", "clientId": "api"}])),
        (
            "GET /admin/realms/r/clients/ca/roles",
            200,
            json!([{"id": "r1", "name": "reader"}, {"id": "r2", "name": "orphan"}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("clients/api/roles/reader.yaml"),
        "name: reader\n",
    );
    let ui = MockUi::new();
    *ui.confirms.lock().unwrap() = vec![false];
    let ui = Arc::new(ui);
    let mut prune_ctx = ctx(&c, dir.path(), ui.clone(), true);
    prune_ctx.yes = false;
    apply::client_roles::apply_client_roles(prune_ctx)
        .await
        .unwrap();
    assert!(ui.confirms.lock().unwrap().is_empty(), "prune prompt shown");
    assert!(calls(&state, "DELETE /admin/realms/r/roles-by-id/r2").is_empty());
}

#[tokio::test]
async fn sub_group_created_without_location_is_found_by_name() {
    // Stateful server: the child appears in /children once it has been created.
    let created = Arc::new(Mutex::new(false));
    let shared = Arc::clone(&created);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri| {
        let created = Arc::clone(&shared);
        async move {
            let path = uri.path().to_string();
            match (method.as_str(), path.as_str()) {
                ("POST", "/admin/realms/r/groups/g1/children") => {
                    *created.lock().unwrap() = true;
                    StatusCode::CREATED.into_response()
                }
                ("GET", "/admin/realms/r/groups/g1/children") => {
                    if *created.lock().unwrap() {
                        axum::Json(json!([{"id": "new-id", "name": "new-team"}])).into_response()
                    } else {
                        axum::Json(json!([])).into_response()
                    }
                }
                ("GET", _) => axum::Json(json!([])).into_response(),
                _ => StatusCode::NO_CONTENT.into_response(),
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let c = client(url);
    let group: GroupRepresentation = serde_json::from_value(json!({
        "name": "org",
        "subGroups": [{"name": "new-team", "subGroups": []}]
    }))
    .unwrap();
    relations::reconcile_group(&c, &group, "g1").await.unwrap();
    assert!(*created.lock().unwrap());
}
