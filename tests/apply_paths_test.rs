//! Mock-server tests for less common apply/plan paths: review and prune prompts, error
//! reporting, deferred realm flow bindings, child components and shared sub-flow reporting.
use axum::http::{Method, StatusCode, Uri};
use axum::response::IntoResponse;
use kaji::apply::{self, ApplyContext};
use kaji::client::KeycloakClient;
use kaji::models::RoleRepresentation;
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::{MockUi, Ui};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

/// Records every request and answers from a `(METHOD path) -> (status, body)` table.
/// Unknown GETs return `[]`, other unknown requests `204`.
#[derive(Default)]
struct Recorder {
    responses: HashMap<String, (u16, Value)>,
    calls: Vec<(String, Value)>,
}

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

fn ui(confirms: Vec<bool>) -> Arc<MockUi> {
    let ui = MockUi::new();
    *ui.confirms.lock().unwrap() = confirms;
    Arc::new(ui)
}

fn ctx<'a>(
    client: &'a KeycloakClient,
    realm_dir: &Path,
    ui: Arc<dyn Ui>,
    review: bool,
    yes: bool,
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
        review,
        ui,
        yes,
        prune,
        prompt_mutex: Arc::new(tokio::sync::Mutex::new(())),
    }
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn calls_matching(state: &Arc<Mutex<Recorder>>, prefix: &str) -> Vec<Value> {
    state
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|(k, _)| k.starts_with(prefix))
        .map(|(_, b)| b.clone())
        .collect()
}

#[tokio::test]
async fn review_and_prune_prompts_drive_updates_and_deletions() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r/roles",
            200,
            json!([{"id": "r1", "name": "existing"}, {"id": "r2", "name": "orphan"},
                   {"id": "r3", "name": "kept-orphan"}, {"id": "r4", "name": "offline_access"}]),
        ),
        (
            "GET /admin/realms/r/roles-by-id/r1",
            200,
            json!({"id": "r1", "name": "existing", "description": "server"}),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(&dir.path().join("roles/existing.yaml"), "name: existing\n");

    // review: update existing; enrichment: decline; prune: delete orphan, keep kept-orphan
    let ui = ui(vec![true, false, true, false]);
    apply::generic::apply_resources::<RoleRepresentation>(ctx(
        &c,
        dir.path(),
        ui.clone(),
        true,
        false,
        true,
    ))
    .await
    .unwrap();
    assert!(ui.confirms.lock().unwrap().is_empty());
    assert_eq!(
        calls_matching(&state, "PUT /admin/realms/r/roles-by-id/r1").len(),
        1
    );
    assert_eq!(
        calls_matching(&state, "DELETE /admin/realms/r/roles-by-id/r2").len(),
        1
    );
    assert!(calls_matching(&state, "DELETE /admin/realms/r/roles-by-id/r3").is_empty());
    assert!(calls_matching(&state, "DELETE /admin/realms/r/roles-by-id/r4").is_empty());
    // Declined enrichment: the local file is untouched
    assert_eq!(
        fs::read_to_string(dir.path().join("roles/existing.yaml")).unwrap(),
        "name: existing\n"
    );

    // Review declined: nothing is sent
    let ui = self::ui(vec![false]);
    apply::generic::apply_resources::<RoleRepresentation>(ctx(
        &c,
        dir.path(),
        ui,
        true,
        false,
        false,
    ))
    .await
    .unwrap();
    assert_eq!(
        calls_matching(&state, "PUT /admin/realms/r/roles-by-id/r1").len(),
        1
    );
}

#[tokio::test]
async fn update_and_create_failures_are_reported_with_context() {
    let (url, _state) = start(vec![
        (
            "GET /admin/realms/r/roles",
            200,
            json!([{"id": "r1", "name": "existing"}]),
        ),
        (
            "PUT /admin/realms/r/roles-by-id/r1",
            400,
            json!({"errorMessage": "bad update"}),
        ),
        (
            "POST /admin/realms/r/roles",
            400,
            json!({"errorMessage": "bad create"}),
        ),
    ])
    .await;
    let c = client(url);

    let dir = tempdir().unwrap();
    write(&dir.path().join("roles/existing.yaml"), "name: existing\n");
    let err = apply::generic::apply_resources::<RoleRepresentation>(ctx(
        &c,
        dir.path(),
        ui(vec![]),
        false,
        true,
        false,
    ))
    .await
    .unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("Failed to update roles 'existing'") && msg.contains("bad update"),
        "{msg}"
    );

    let dir = tempdir().unwrap();
    write(&dir.path().join("roles/new.yaml"), "name: new\n");
    let err = apply::generic::apply_resources::<RoleRepresentation>(ctx(
        &c,
        dir.path(),
        ui(vec![]),
        false,
        true,
        false,
    ))
    .await
    .unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("Failed to create roles 'new'") && msg.contains("bad create"),
        "{msg}"
    );
}

#[tokio::test]
async fn realm_flow_bindings_are_deferred_until_flows_exist() {
    let (url, state) = start(vec![
        (
            "GET /admin/realms/r",
            200,
            json!({"realm": "r", "id": "rid", "browserFlow": "browser"}),
        ),
        (
            "GET /admin/realms/r/authentication/flows",
            200,
            json!([{"id": "b", "alias": "browser"}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("realm.yaml"),
        "realm: r\nbrowserFlow: custom-browser\ndirectGrantFlow: browser\n",
    );

    let pending = apply::realm::apply_realm(ctx(&c, dir.path(), ui(vec![]), false, true, false))
        .await
        .unwrap();
    let first_put = calls_matching(&state, "PUT /admin/realms/r");
    assert_eq!(first_put.len(), 1);
    assert!(
        first_put[0].get("browserFlow").is_none(),
        "{}",
        first_put[0]
    );
    assert_eq!(first_put[0]["directGrantFlow"], "browser");
    let deferred = pending.as_ref().unwrap().deferred_bindings();
    assert_eq!(deferred.get("browserFlow"), Some(&json!("custom-browser")));

    apply::realm::finish_realm(ctx(&c, dir.path(), ui(vec![]), false, true, false), pending)
        .await
        .unwrap();
    let puts = calls_matching(&state, "PUT /admin/realms/r");
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[1]["browserFlow"], "custom-browser");

    // Review declined: the realm is skipped
    let pending =
        apply::realm::apply_realm(ctx(&c, dir.path(), ui(vec![false]), true, false, false))
            .await
            .unwrap();
    assert!(pending.is_none());
}

#[tokio::test]
async fn child_components_get_the_target_parent_id() {
    let (url, state) = start(vec![
        ("GET /admin/realms/r", 200, json!({"realm": "r", "id": "rid"})),
        (
            "GET /admin/realms/r/components",
            200,
            json!([{"id": "ldap-target", "name": "ldap", "providerId": "ldap",
                    "providerType": "org.keycloak.storage.UserStorageProvider", "parentId": "rid"}]),
        ),
    ])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    write(
        &dir.path().join("components/ldap.yaml"),
        "id: ldap-source\nname: ldap\nproviderId: ldap\nproviderType: org.keycloak.storage.UserStorageProvider\nparentId: source-realm-id\n",
    );
    write(
        &dir.path().join("components/mapper.yaml"),
        "name: title\nproviderId: user-attribute-ldap-mapper\nproviderType: org.keycloak.storage.ldap.mappers.LDAPStorageMapper\nparentId: ldap-source\n",
    );

    apply::components::apply_components_or_keys(
        ctx(&c, dir.path(), ui(vec![true, true]), true, true, false),
        "components",
    )
    .await
    .unwrap();

    let updates = calls_matching(&state, "PUT /admin/realms/r/components/ldap-target");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["parentId"], "rid");
    let creates = calls_matching(&state, "POST /admin/realms/r/components");
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0]["name"], "title");
    assert_eq!(creates[0]["parentId"], "ldap-target");
}

#[tokio::test]
async fn plan_reports_shared_and_new_sub_flows() {
    let (url, _state) = start(vec![(
        "GET /admin/realms/r/authentication/flows",
        200,
        json!([{"id": "p1", "alias": "parent-a", "providerId": "basic-flow", "topLevel": true,
                "authenticationExecutions": []}]),
    )])
    .await;
    let c = client(url);
    let dir = tempdir().unwrap();
    let flows = dir.path().join("r/authentication-flows");
    write(
        &flows.join("shared.yaml"),
        "alias: shared\nproviderId: basic-flow\ntopLevel: false\nauthenticationExecutions: []\n",
    );
    write(
        &flows.join("only-child.yaml"),
        "alias: only-child\nproviderId: basic-flow\ntopLevel: false\nauthenticationExecutions: []\n",
    );
    for parent in ["parent-a", "parent-b"] {
        write(
            &flows.join(format!("{parent}.yaml")),
            &format!(
                "alias: {parent}\nproviderId: basic-flow\ntopLevel: true\nauthenticationExecutions:\n- authenticatorFlow: true\n  flowAlias: shared\n  requirement: REQUIRED\n"
            ),
        );
    }
    write(
        &flows.join("parent-c.yaml"),
        "alias: parent-c\nproviderId: basic-flow\ntopLevel: true\nauthenticationExecutions:\n- authenticatorFlow: true\n  flowAlias: only-child\n  requirement: REQUIRED\n",
    );

    let summary = kaji::plan::run_with_outcome(
        kaji::plan::PlanArgs {
            client: &c,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: &["r".to_string()],
            ui: Arc::new(MockUi::new()),
            resolver: Arc::new(EnvResolver::new(HashMap::new())),
            profile: None,
            verbose: true,
        },
        false,
    )
    .await
    .unwrap();
    // parent-b, parent-c, shared, only-child are new; parent-a differs (executions)
    assert_eq!(summary.created, 4);
    assert_eq!(summary.updated, 1);
}
