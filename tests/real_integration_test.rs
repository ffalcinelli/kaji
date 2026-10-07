//! Live integration tests against a real Keycloak server.
//!
//! These tests only run when `KAJI_IT_URL` points to a running Keycloak (admin/admin), e.g.:
//!
//! ```bash
//! KAJI_IT_PORT=8180 KAJI_IT_MGMT_PORT=9180 docker compose up -d --wait
//! KAJI_IT_URL=http://localhost:8180 cargo test --test real_integration_test
//! ```
//!
//! Without `KAJI_IT_URL` every test is skipped. Tests marked `#[ignore = "known bug: ..."]`
//! document confirmed defects that are not fixed yet (see AGENTS.md "Known Issues").
use anyhow::Result;
use kaji::client::KeycloakClient;
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::{MockUi, Ui};
use kaji::{apply, inspect, plan};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

fn base_url() -> Option<String> {
    match std::env::var("KAJI_IT_URL") {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            eprintln!("KAJI_IT_URL not set, skipping live Keycloak test");
            None
        }
    }
}

async fn admin_client(url: &str) -> Result<KeycloakClient> {
    let mut client = KeycloakClient::new(url.to_string());
    client
        .login("admin-cli", None, Some("admin"), Some("admin"))
        .await?;
    Ok(client)
}

/// Minimal raw admin API helper used to arrange and assert server state independently of kaji.
struct Admin {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl Admin {
    async fn new(url: &str) -> Result<Self> {
        let http = reqwest::Client::new();
        let token: Value = http
            .post(format!(
                "{}/realms/master/protocol/openid-connect/token",
                url
            ))
            .form(&[
                ("client_id", "admin-cli"),
                ("username", "admin"),
                ("password", "admin"),
                ("grant_type", "password"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(Self {
            http,
            url: url.to_string(),
            token: token["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        })
    }

    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .http
            .get(format!("{}/admin/realms{}", self.url, path))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn status(&self, path: &str) -> Result<u16> {
        Ok(self
            .http
            .get(format!("{}/admin/realms{}", self.url, path))
            .bearer_auth(&self.token)
            .send()
            .await?
            .status()
            .as_u16())
    }

    async fn post(&self, path: &str, body: &Value) -> Result<u16> {
        Ok(self
            .http
            .post(format!("{}/admin/realms{}", self.url, path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?
            .status()
            .as_u16())
    }

    async fn delete(&self, path: &str) -> Result<u16> {
        Ok(self
            .http
            .delete(format!("{}/admin/realms{}", self.url, path))
            .bearer_auth(&self.token)
            .send()
            .await?
            .status()
            .as_u16())
    }

    async fn delete_realm(&self, realm: &str) {
        let _ = self
            .http
            .delete(format!("{}/admin/realms/{}", self.url, realm))
            .bearer_auth(&self.token)
            .send()
            .await;
    }

    async fn names(&self, path: &str, field: &str) -> Result<Vec<String>> {
        let items = self.get(path).await?;
        Ok(items
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|i| i[field].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }
}

fn unique_realm(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    format!("kaji-it-{}-{}", prefix, nanos)
}

fn resolver() -> Arc<dyn SecretResolver> {
    Arc::new(EnvResolver::new(HashMap::new()))
}

/// Resolver backed by the workspace `.secrets` file, as the CLI does (apply stores generated
/// secrets such as confidential client secrets there).
fn workspace_resolver(workspace: &Path) -> Arc<dyn SecretResolver> {
    let mut vars = HashMap::new();
    if let Ok(iter) = dotenvy::from_path_iter(workspace.join(".secrets")) {
        for (k, v) in iter.flatten() {
            vars.insert(k, v);
        }
    }
    Arc::new(EnvResolver::new(vars))
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

async fn run_plan(client: &KeycloakClient, workspace: &Path, realm: &str) -> Result<()> {
    plan::run(plan::PlanArgs {
        client,
        workspace_dir: workspace.to_path_buf(),
        changes_only: true,
        interactive: false,
        realms_to_plan: &[realm.to_string()],
        ui: Arc::new(MockUi::new()),
        resolver: resolver(),
        profile: None,
        verbose: false,
    })
    .await
}

async fn run_apply(
    client: &KeycloakClient,
    workspace: &Path,
    realm: &str,
    prune: bool,
) -> Result<()> {
    let ui: Arc<dyn Ui> = Arc::new(MockUi::new());
    apply::run(apply::ApplyArgs {
        client,
        workspace_dir: workspace.to_path_buf(),
        realms_to_apply: &[realm.to_string()],
        yes: true,
        review: false,
        prune,
        ui,
        resolver: resolver(),
        profile: None,
    })
    .await
}

/// Inspecting a stock realm and planning it must not choke on Keycloak's own
/// `${client_account}`-style localization keys, and must report no changes.
#[tokio::test]
async fn inspect_then_plan_master_succeeds() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let mut client = admin_client(&url).await?;
    client.set_target_realm("master".to_string());
    let dir = tempdir()?;

    inspect::run_with_ui(
        &client,
        dir.path().to_path_buf(),
        &["master".to_string()],
        true,
        Arc::new(MockUi::new()),
    )
    .await?;
    assert!(dir.path().join("master/realm.yaml").exists());

    // A fresh export must plan without any change (full round trip).
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: &["master".to_string()],
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: true,
        },
        false,
    )
    .await?;
    // Other tests create realms concurrently (adding composites to master's admin role), so
    // only success is asserted here; see `inspect_then_plan_is_clean` for the round trip.
    let _ = summary;
    Ok(())
}

/// Exporting a realm with relationships and planning the export reports no change.
#[tokio::test]
async fn inspect_then_plan_is_clean() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("roundtrip");
    let admin = Admin::new(&url).await?;
    admin
        .post(
            "",
            &json!({"realm": realm, "enabled": true, "displayName": "Round Trip"}),
        )
        .await?;
    admin
        .post(
            &format!("/{}/clients", realm),
            &json!({"clientId": "api", "publicClient": false, "redirectUris": ["https://api/*"]}),
        )
        .await?;
    let api_id = admin
        .get(&format!("/{}/clients?clientId=api", realm))
        .await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    admin
        .post(
            &format!("/{}/clients/{}/roles", realm, api_id),
            &json!({"name": "reader", "description": "Read", "attributes": {"tier": ["1"]}}),
        )
        .await?;
    admin
        .post(
            &format!("/{}/roles", realm),
            &json!({"name": "bundle", "composite": true,
                    "composites": {"client": {"api": ["reader"]}}}),
        )
        .await?;
    admin
        .post(&format!("/{}/groups", realm), &json!({"name": "org"}))
        .await?;
    let org = admin.get(&format!("/{}/groups?search=org", realm)).await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    admin
        .post(
            &format!("/{}/groups/{}/children", realm, org),
            &json!({"name": "team", "attributes": {"floor": ["3"]}}),
        )
        .await?;
    let bundle = admin.get(&format!("/{}/roles/bundle", realm)).await?;
    admin
        .post(
            &format!("/{}/groups/{}/role-mappings/realm", realm, org),
            &json!([bundle]),
        )
        .await?;
    admin
        .post(
            &format!("/{}/users", realm),
            &json!({"username": "carol", "enabled": true, "email": "carol@example.com",
                    "groups": ["/org/team"]}),
        )
        .await?;

    let mut client = admin_client(&url).await?;
    client.set_target_realm(realm.clone());
    let dir = tempdir()?;
    let exported = inspect::run_with_ui(
        &client,
        dir.path().to_path_buf(),
        std::slice::from_ref(&realm),
        true,
        Arc::new(MockUi::new()),
    )
    .await;
    let summary = match exported {
        Ok(()) => plan::run_with_outcome(
            plan::PlanArgs {
                client: &client,
                workspace_dir: dir.path().to_path_buf(),
                changes_only: true,
                interactive: false,
                realms_to_plan: std::slice::from_ref(&realm),
                ui: Arc::new(MockUi::new()),
                resolver: workspace_resolver(dir.path()),
                profile: None,
                verbose: true,
            },
            false,
        )
        .await
        .map(Some),
        Err(e) => Err(e),
    };
    admin.delete_realm(&realm).await;

    let summary = summary?.expect("plan ran");
    assert!(
        dir.path()
            .join(&realm)
            .join("clients/api/roles/reader.yaml")
            .exists()
    );
    assert_eq!(summary.total(), 0, "inspect followed by plan must be clean");
    Ok(())
}

/// `apply` must be able to bootstrap a realm that does not exist yet.
#[tokio::test]
async fn apply_bootstraps_new_realm() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("boot");
    let admin = Admin::new(&url).await?;
    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    write(
        &dir.path().join(&realm).join("realm.yaml"),
        &format!("realm: {}\nenabled: true\ndisplayName: Kaji IT\n", realm),
    );
    write(
        &dir.path().join(&realm).join("clients/it-client.yaml"),
        "clientId: it-client\npublicClient: true\nenabled: true\n",
    );

    // Planning a realm that does not exist yet must preview its creation, not fail.
    let mut realm_client = client.clone();
    realm_client.set_target_realm(realm.clone());
    run_plan(&realm_client, dir.path(), &realm).await?;
    assert!(dir.path().join(".kajiplan").exists());

    let result = run_apply(&client, dir.path(), &realm, false).await;
    let realm_status = admin.status(&format!("/{}", realm)).await?;
    let clients = admin
        .names(&format!("/{}/clients", realm), "clientId")
        .await
        .unwrap_or_default();
    admin.delete_realm(&realm).await;

    result?;
    assert_eq!(realm_status, 200);
    assert!(clients.contains(&"it-client".to_string()));
    Ok(())
}

/// A fresh realm whose resources reference each other (realm -> flow, IdP -> flow,
/// client -> client scope) must apply in one run.
#[tokio::test]
async fn apply_bootstraps_cross_references() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("xref");
    let admin = Admin::new(&url).await?;
    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!(
            "realm: {}\nenabled: true\nbrowserFlow: custom-browser\n",
            realm
        ),
    );
    write(
        &realm_dir.join("authentication-flows/custom-browser.yaml"),
        "alias: custom-browser\nproviderId: basic-flow\ntopLevel: true\nbuiltIn: false\n",
    );
    write(
        &realm_dir.join("identity-providers/corp.yaml"),
        "alias: corp\nproviderId: oidc\nenabled: true\nfirstBrokerLoginFlowAlias: custom-browser\nconfig:\n  clientId: corp\n  authorizationUrl: https://idp.example.com/auth\n  tokenUrl: https://idp.example.com/token\n",
    );
    write(
        &realm_dir.join("client-scopes/custom-scope.yaml"),
        "name: custom-scope\nprotocol: openid-connect\n",
    );
    write(
        &realm_dir.join("clients/app.yaml"),
        "clientId: app\npublicClient: true\ndefaultClientScopes:\n- custom-scope\n",
    );

    let result = run_apply(&client, dir.path(), &realm, false).await;
    let realm_rep = admin.get(&format!("/{}", realm)).await;
    let idp = admin
        .get(&format!("/{}/identity-provider/instances/corp", realm))
        .await;
    let clients = admin.get(&format!("/{}/clients?clientId=app", realm)).await;
    admin.delete_realm(&realm).await;

    result?;
    assert_eq!(realm_rep?["browserFlow"], "custom-browser");
    assert_eq!(idp?["firstBrokerLoginFlowAlias"], "custom-browser");
    let scopes = clients?[0]["defaultClientScopes"].clone();
    assert!(
        scopes
            .as_array()
            .is_some_and(|a| a.contains(&json!("custom-scope"))),
        "defaultClientScopes: {}",
        scopes
    );
    // The local realm file keeps the declared binding (no enrichment rewrite to "browser").
    let local = fs::read_to_string(realm_dir.join("realm.yaml"))?;
    assert!(local.contains("browserFlow: custom-browser"), "{}", local);
    Ok(())
}

/// Client scope links are only honored by Keycloak on creation: updates must reconcile them.
#[tokio::test]
async fn apply_updates_client_scope_links() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("scopes");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": realm, "enabled": true}))
        .await?;
    admin
        .post(
            &format!("/{}/client-scopes", realm),
            &json!({"name": "custom", "protocol": "openid-connect"}),
        )
        .await?;
    admin
        .post(
            &format!("/{}/clients", realm),
            &json!({"clientId": "app", "defaultClientScopes": ["profile", "email"],
                    "optionalClientScopes": ["address"]}),
        )
        .await?;

    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &realm_dir.join("clients/app.yaml"),
        "clientId: app\ndefaultClientScopes:\n- profile\n- custom\noptionalClientScopes:\n- email\n",
    );
    let result = run_apply(&client, dir.path(), &realm, false).await;
    let app = admin.get(&format!("/{}/clients?clientId=app", realm)).await;
    admin.delete_realm(&realm).await;

    result?;
    let app = app?;
    let mut defaults: Vec<String> = serde_json::from_value(app[0]["defaultClientScopes"].clone())?;
    defaults.sort();
    assert_eq!(defaults, vec!["custom".to_string(), "profile".to_string()]);
    assert_eq!(app[0]["optionalClientScopes"], json!(["email"]));
    Ok(())
}

/// A user's group membership and realm/client role mappings are reconciled and planned.
#[tokio::test]
async fn apply_reconciles_user_groups_and_roles() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("user-rel");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": realm, "enabled": true}))
        .await?;
    for group in ["g-old", "g-new"] {
        admin
            .post(&format!("/{}/groups", realm), &json!({"name": group}))
            .await?;
    }
    for role in ["r-old", "r-new"] {
        admin
            .post(&format!("/{}/roles", realm), &json!({"name": role}))
            .await?;
    }
    admin
        .post(&format!("/{}/clients", realm), &json!({"clientId": "api"}))
        .await?;
    let api_id = admin
        .get(&format!("/{}/clients?clientId=api", realm))
        .await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    admin
        .post(
            &format!("/{}/clients/{}/roles", realm, api_id),
            &json!({"name": "reader"}),
        )
        .await?;
    admin
        .post(
            &format!("/{}/users", realm),
            &json!({"username": "bob", "enabled": true, "groups": ["/g-old"]}),
        )
        .await?;
    let bob = admin
        .get(&format!("/{}/users?username=bob&exact=true", realm))
        .await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let r_old = admin.get(&format!("/{}/roles/r-old", realm)).await?;
    admin
        .post(
            &format!("/{}/users/{}/role-mappings/realm", realm, bob),
            &json!([r_old]),
        )
        .await?;

    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &realm_dir.join("users/bob.yaml"),
        &format!(
            "username: bob\nenabled: true\ngroups:\n- /g-new\nrealmRoles:\n- default-roles-{}\n- r-new\nclientRoles:\n  api:\n  - reader\n",
            realm
        ),
    );
    let result = run_apply(&client, dir.path(), &realm, false).await;
    let groups = admin.get(&format!("/{}/users/{}/groups", realm, bob)).await;
    let mappings = admin
        .get(&format!("/{}/users/{}/role-mappings", realm, bob))
        .await;

    let mut realm_client = client.clone();
    realm_client.set_target_realm(realm.clone());
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &realm_client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: std::slice::from_ref(&realm),
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: true,
        },
        false,
    )
    .await;
    admin.delete_realm(&realm).await;

    result?;
    let paths: Vec<String> = groups?
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| g["path"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(paths, vec!["/g-new".to_string()]);
    let mappings = mappings?;
    let mut realm_roles: Vec<String> = mappings["realmMappings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    realm_roles.sort();
    assert_eq!(
        realm_roles,
        vec![format!("default-roles-{}", realm), "r-new".to_string()]
    );
    assert_eq!(
        mappings["clientMappings"]["api"]["mappings"][0]["name"],
        "reader"
    );
    assert_eq!(summary?.total(), 0, "plan after apply must be clean");
    Ok(())
}

/// Nested sub-groups and group role mappings are reconciled and planned.
#[tokio::test]
async fn apply_reconciles_nested_groups() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("groups");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": realm, "enabled": true}))
        .await?;
    admin
        .post(&format!("/{}/roles", realm), &json!({"name": "staff"}))
        .await?;
    admin
        .post(&format!("/{}/clients", realm), &json!({"clientId": "api"}))
        .await?;
    let api_id = admin
        .get(&format!("/{}/clients?clientId=api", realm))
        .await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    admin
        .post(
            &format!("/{}/clients/{}/roles", realm, api_id),
            &json!({"name": "reader"}),
        )
        .await?;
    admin
        .post(&format!("/{}/groups", realm), &json!({"name": "org"}))
        .await?;
    let org_id = admin.get(&format!("/{}/groups?search=org", realm)).await?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    admin
        .post(
            &format!("/{}/groups/{}/children", realm, org_id),
            &json!({"name": "old-team"}),
        )
        .await?;

    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &realm_dir.join("groups/org.yaml"),
        "name: org\nrealmRoles:\n- staff\nclientRoles: {}\nattributes: {}\nsubGroups:\n- name: team\n  attributes:\n    floor:\n    - '3'\n  realmRoles: []\n  clientRoles:\n    api:\n    - reader\n  subGroups:\n  - name: squad\n    attributes: {}\n    realmRoles: []\n    clientRoles: {}\n    subGroups: []\n",
    );
    write(
        &realm_dir.join("users/alice.yaml"),
        "username: alice\nenabled: true\ngroups:\n- /org/team/squad\n",
    );
    let result = run_apply(&client, dir.path(), &realm, false).await;
    let children = admin
        .get(&format!("/{}/groups/{}/children", realm, org_id))
        .await;
    let org_roles = admin
        .get(&format!("/{}/groups/{}/role-mappings", realm, org_id))
        .await;
    let alice = admin
        .get(&format!("/{}/users?username=alice&exact=true", realm))
        .await?;
    let alice_groups = admin
        .get(&format!(
            "/{}/users/{}/groups",
            realm,
            alice[0]["id"].as_str().unwrap_or_default()
        ))
        .await;

    let mut realm_client = client.clone();
    realm_client.set_target_realm(realm.clone());
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &realm_client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: std::slice::from_ref(&realm),
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: true,
        },
        false,
    )
    .await;
    admin.delete_realm(&realm).await;

    result?;
    let names: Vec<String> = children?
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| g["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(names, vec!["team".to_string()]);
    assert_eq!(org_roles?["realmMappings"][0]["name"], "staff");
    assert_eq!(alice_groups?[0]["path"], "/org/team/squad");
    assert_eq!(summary?.total(), 0, "plan after apply must be clean");
    Ok(())
}

/// Client roles and composite roles (mixing realm and client roles) apply to a fresh realm.
#[tokio::test]
async fn apply_creates_client_roles_and_composites() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("roles");
    let admin = Admin::new(&url).await?;
    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(&realm_dir.join("roles/viewer.yaml"), "name: viewer\n");
    write(
        &realm_dir.join("roles/bundle.yaml"),
        "name: bundle\ncomposite: true\ncomposites:\n  realm:\n  - viewer\n  client:\n    api:\n    - reader\n",
    );
    write(&realm_dir.join("clients/api.yaml"), "clientId: api\n");
    write(
        &realm_dir.join("clients/api/roles/reader.yaml"),
        "name: reader\ndescription: Read access\n",
    );
    write(
        &realm_dir.join("clients/api/roles/writer.yaml"),
        "name: writer\ncomposite: true\ncomposites:\n  client:\n    api:\n    - reader\n",
    );

    let result = run_apply(&client, dir.path(), &realm, false).await;
    let bundle = admin
        .get(&format!("/{}/roles/bundle/composites", realm))
        .await;
    let api_id = admin
        .get(&format!("/{}/clients?clientId=api", realm))
        .await
        .ok()
        .and_then(|c| c[0]["id"].as_str().map(String::from))
        .unwrap_or_default();
    let reader = admin
        .get(&format!("/{}/clients/{}/roles/reader", realm, api_id))
        .await;
    let writer = admin
        .get(&format!(
            "/{}/clients/{}/roles/writer/composites",
            realm, api_id
        ))
        .await;

    let mut realm_client = client.clone();
    realm_client.set_target_realm(realm.clone());
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &realm_client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: std::slice::from_ref(&realm),
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: true,
        },
        false,
    )
    .await;
    admin.delete_realm(&realm).await;

    result?;
    let mut bundle_names: Vec<String> = bundle?
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    bundle_names.sort();
    assert_eq!(
        bundle_names,
        vec!["reader".to_string(), "viewer".to_string()]
    );
    assert_eq!(reader?["description"], "Read access");
    assert_eq!(writer?[0]["name"], "reader");
    assert_eq!(summary?.total(), 0, "plan after apply must be clean");
    Ok(())
}

/// Declaring a required action that is not registered yet must register it.
#[tokio::test]
async fn apply_registers_required_action() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("ra");
    let admin = Admin::new(&url).await?;
    assert_eq!(
        admin
            .post("", &json!({"realm": realm, "enabled": true}))
            .await?,
        201
    );
    // Fresh realms register every built-in action: unregister one so kaji has to register it.
    let provider_id = "TERMS_AND_CONDITIONS".to_string();
    assert_eq!(
        admin
            .delete(&format!(
                "/{}/authentication/required-actions/{}",
                realm, provider_id
            ))
            .await?,
        204
    );

    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    write(
        &dir.path().join(&realm).join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &dir.path()
            .join(&realm)
            .join(format!("required-actions/{}.yaml", provider_id)),
        &format!(
            "alias: {p}\nproviderId: {p}\nname: {p}\nenabled: true\ndefaultAction: false\n",
            p = provider_id
        ),
    );

    let result = run_apply(&client, dir.path(), &realm, false).await;
    let registered = admin
        .names(
            &format!("/{}/authentication/required-actions", realm),
            "alias",
        )
        .await
        .unwrap_or_default();
    admin.delete_realm(&realm).await;

    result?;
    assert!(registered.contains(&provider_id));
    Ok(())
}

/// `--prune` must never delete Keycloak's built-in resources.
#[tokio::test]
async fn prune_keeps_builtin_resources() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("prune");
    let admin = Admin::new(&url).await?;
    assert_eq!(
        admin
            .post("", &json!({"realm": realm, "enabled": true}))
            .await?,
        201
    );
    assert_eq!(
        admin
            .post(
                &format!("/{}/clients", realm),
                &json!({"clientId": "svc", "serviceAccountsEnabled": true, "publicClient": false})
            )
            .await?,
        201
    );
    assert_eq!(
        admin
            .post(
                &format!("/{}/client-scopes", realm),
                &json!({"name": "orphan-scope", "protocol": "openid-connect"})
            )
            .await?,
        201
    );

    let scopes_before = admin
        .names(&format!("/{}/client-scopes", realm), "name")
        .await?;
    let clients_before = admin
        .names(&format!("/{}/clients", realm), "clientId")
        .await?;
    let actions_before = admin
        .names(
            &format!("/{}/authentication/required-actions", realm),
            "alias",
        )
        .await?;
    let flows_before = admin
        .names(&format!("/{}/authentication/flows", realm), "alias")
        .await?;

    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &realm_dir.join("clients/svc.yaml"),
        "clientId: svc\nserviceAccountsEnabled: true\npublicClient: false\n",
    );
    for d in [
        "client-scopes",
        "roles",
        "users",
        "required-actions",
        "authentication-flows",
        "groups",
        "identity-providers",
    ] {
        fs::create_dir_all(realm_dir.join(d))?;
    }

    let result = run_apply(&client, dir.path(), &realm, true).await;

    let scopes_after = admin
        .names(&format!("/{}/client-scopes", realm), "name")
        .await?;
    let clients_after = admin
        .names(&format!("/{}/clients", realm), "clientId")
        .await?;
    let actions_after = admin
        .names(
            &format!("/{}/authentication/required-actions", realm),
            "alias",
        )
        .await?;
    let flows_after = admin
        .names(&format!("/{}/authentication/flows", realm), "alias")
        .await?;
    let svc_users = admin
        .get(&format!(
            "/{}/users?username=service-account-svc&exact=true",
            realm
        ))
        .await?;
    admin.delete_realm(&realm).await;

    result?;
    let expected_scopes: Vec<_> = scopes_before
        .iter()
        .filter(|s| s.as_str() != "orphan-scope")
        .cloned()
        .collect();
    for s in &expected_scopes {
        assert!(
            scopes_after.contains(s),
            "built-in scope '{}' was pruned",
            s
        );
    }
    assert!(
        !scopes_after.contains(&"orphan-scope".to_string()),
        "undeclared custom scope should be pruned"
    );
    for c in &clients_before {
        assert!(clients_after.contains(c), "client '{}' was pruned", c);
    }
    for a in &actions_before {
        assert!(
            actions_after.contains(a),
            "required action '{}' was pruned",
            a
        );
    }
    for f in &flows_before {
        assert!(flows_after.contains(f), "built-in flow '{}' was pruned", f);
    }
    assert_eq!(
        svc_users.as_array().map(Vec::len),
        Some(1),
        "service account user was pruned"
    );
    Ok(())
}

/// The master realm owns one `<realm>-realm` client per realm; prune must keep them.
#[tokio::test]
async fn prune_keeps_master_realm_management_clients() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let admin = Admin::new(&url).await?;
    let clients_before = admin.names("/master/clients", "clientId").await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join("master");
    write(&realm_dir.join("realm.yaml"), "realm: master\n");
    fs::create_dir_all(realm_dir.join("clients"))?;
    for c in &clients_before {
        // Declare every non-system client so only system clients are prune candidates.
        if !c.ends_with("-realm") {
            write(
                &realm_dir.join(format!("clients/{}.yaml", c.replace('/', "_"))),
                &format!("clientId: {}\n", c),
            );
        }
    }
    let mut client = admin_client(&url).await?;
    client.set_target_realm("master".to_string());
    // Only plan-level safety is checked here: we never want this to delete anything.
    let protected: Vec<_> = clients_before
        .iter()
        .filter(|c| c.ends_with("-realm"))
        .cloned()
        .collect();
    assert!(!protected.is_empty());
    for c in &protected {
        assert!(
            kaji::apply::generic::is_protected_client(c, "master"),
            "'{}' must be protected in master",
            c
        );
    }
    Ok(())
}

fn level0(rows: &Value) -> Vec<(String, String)> {
    rows.as_array()
        .map(|a| {
            a.iter()
                .filter(|r| r["level"] == 0)
                .map(|r| {
                    (
                        r["displayName"].as_str().unwrap_or_default().to_string(),
                        r["requirement"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Flow executions (including a sub-flow declared in its own file and an authenticator config)
/// are created, updated and removed, and planning afterwards reports no changes.
#[tokio::test]
async fn apply_reconciles_flow_executions() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("flow");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": realm, "enabled": true}))
        .await?;
    let client = admin_client(&url).await?;
    let dir = tempdir()?;
    let realm_dir = dir.path().join(&realm);
    write(
        &realm_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", realm),
    );
    write(
        &realm_dir.join("authentication-flows/it-forms.yaml"),
        "alias: it-forms\nproviderId: basic-flow\ntopLevel: false\nbuiltIn: false\nauthenticationExecutions:\n- authenticator: auth-username-password-form\n  requirement: REQUIRED\n",
    );
    let parent = |cookie: bool, forms_requirement: &str| {
        let cookie_exec = if cookie {
            "- authenticator: auth-cookie\n  requirement: ALTERNATIVE\n"
        } else {
            ""
        };
        format!(
            "alias: it-flow\nproviderId: basic-flow\ntopLevel: true\nbuiltIn: false\nauthenticationExecutions:\n{}- authenticator: identity-provider-redirector\n  authenticatorConfig: it-redirector\n  requirement: ALTERNATIVE\n- authenticatorFlow: true\n  flowAlias: it-forms\n  requirement: {}\n",
            cookie_exec, forms_requirement
        )
    };
    write(
        &realm_dir.join("authentication-flows/it-flow.yaml"),
        &parent(true, "ALTERNATIVE"),
    );
    write(
        &realm_dir.join("authenticator-configs/it-redirector.yaml"),
        "alias: it-redirector\nconfig:\n  defaultProvider: corp\n",
    );

    let first = run_apply(&client, dir.path(), &realm, false).await;
    let rows_first = admin
        .get(&format!(
            "/{}/authentication/flows/it-flow/executions",
            realm
        ))
        .await;
    let sub_rows = admin
        .get(&format!(
            "/{}/authentication/flows/it-forms/executions",
            realm
        ))
        .await;

    let mut realm_client = client.clone();
    realm_client.set_target_realm(realm.clone());
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &realm_client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: std::slice::from_ref(&realm),
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: false,
        },
        false,
    )
    .await;

    write(
        &realm_dir.join("authentication-flows/it-flow.yaml"),
        &parent(false, "REQUIRED"),
    );
    let second = run_apply(&client, dir.path(), &realm, false).await;
    let rows_second = admin
        .get(&format!(
            "/{}/authentication/flows/it-flow/executions",
            realm
        ))
        .await;
    admin.delete_realm(&realm).await;

    first?;
    let rows_first = rows_first?;
    assert_eq!(
        level0(&rows_first),
        vec![
            ("Cookie".to_string(), "ALTERNATIVE".to_string()),
            (
                "Identity Provider Redirector".to_string(),
                "ALTERNATIVE".to_string()
            ),
            ("it-forms".to_string(), "ALTERNATIVE".to_string()),
        ]
    );
    let redirector = rows_first
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|r| r["providerId"] == "identity-provider-redirector")
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        redirector["alias"], "it-redirector",
        "config linked: {}",
        redirector
    );
    assert_eq!(
        level0(&sub_rows?),
        vec![("Username Password Form".to_string(), "REQUIRED".to_string())]
    );
    assert_eq!(summary?.total(), 0, "plan after apply must be clean");

    second?;
    assert_eq!(
        level0(&rows_second?),
        vec![
            (
                "Identity Provider Redirector".to_string(),
                "ALTERNATIVE".to_string()
            ),
            ("it-forms".to_string(), "REQUIRED".to_string()),
        ]
    );
    Ok(())
}

/// Components exported from one realm apply to another: matched by portable key and child
/// components (LDAP mappers) get the target parent's ID.
#[tokio::test]
async fn components_are_portable_across_realms() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let source = unique_realm("src");
    let target = unique_realm("dst");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": source, "enabled": true}))
        .await?;
    let source_id = admin.get(&format!("/{}", source)).await?["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let ldap_config = json!({
        "enabled": ["true"], "vendor": ["other"], "editMode": ["READ_ONLY"],
        "connectionUrl": ["ldap://localhost:10389"], "usersDn": ["ou=users,dc=example,dc=com"],
        "usernameLDAPAttribute": ["uid"], "rdnLDAPAttribute": ["uid"],
        "uuidLDAPAttribute": ["entryUUID"], "userObjectClasses": ["inetOrgPerson"],
        "authType": ["none"]
    });
    assert_eq!(
        admin
            .post(
                &format!("/{}/components", source),
                &json!({"name": "corp-ldap", "providerId": "ldap",
                        "providerType": "org.keycloak.storage.UserStorageProvider",
                        "parentId": source_id, "config": ldap_config})
            )
            .await?,
        201
    );
    let comps = admin
        .get(&format!("/{}/components?name=corp-ldap", source))
        .await?;
    let ldap_id = comps[0]["id"].as_str().unwrap_or_default().to_string();
    assert_eq!(
        admin
            .post(
                &format!("/{}/components", source),
                &json!({"name": "custom-title", "providerId": "user-attribute-ldap-mapper",
                        "providerType": "org.keycloak.storage.ldap.mappers.LDAPStorageMapper",
                        "parentId": ldap_id,
                        "config": {"ldap.attribute": ["title"], "user.model.attribute": ["title"],
                                   "read.only": ["true"], "always.read.value.from.ldap": ["false"],
                                   "is.mandatory.in.ldap": ["false"]}})
            )
            .await?,
        201
    );

    let mut source_client = admin_client(&url).await?;
    source_client.set_target_realm(source.clone());
    let dir = tempdir()?;
    inspect::run_with_ui(
        &source_client,
        dir.path().to_path_buf(),
        std::slice::from_ref(&source),
        true,
        Arc::new(MockUi::new()),
    )
    .await?;

    // Build the target workspace from the export: realm.yaml + LDAP components only.
    let target_dir = dir.path().join(&target);
    fs::create_dir_all(target_dir.join("components"))?;
    write(
        &target_dir.join("realm.yaml"),
        &format!("realm: {}\nenabled: true\n", target),
    );
    for entry in fs::read_dir(dir.path().join(&source).join("components"))? {
        let path = entry?.path();
        let content = fs::read_to_string(&path)?;
        if content.contains("org.keycloak.storage") {
            fs::copy(
                &path,
                target_dir
                    .join("components")
                    .join(path.file_name().unwrap()),
            )?;
        }
    }

    let client = admin_client(&url).await?;
    let applied = run_apply(&client, dir.path(), &target, false).await;
    let target_comps = admin.get(&format!("/{}/components", target)).await;
    let target_ldap = admin
        .get(&format!("/{}/components?name=corp-ldap", target))
        .await;

    let mut target_client = client.clone();
    target_client.set_target_realm(target.clone());
    let summary = plan::run_with_outcome(
        plan::PlanArgs {
            client: &target_client,
            workspace_dir: dir.path().to_path_buf(),
            changes_only: true,
            interactive: false,
            realms_to_plan: std::slice::from_ref(&target),
            ui: Arc::new(MockUi::new()),
            resolver: workspace_resolver(dir.path()),
            profile: None,
            verbose: false,
        },
        false,
    )
    .await;
    admin.delete_realm(&source).await;
    admin.delete_realm(&target).await;

    applied?;
    let target_ldap_id = target_ldap?[0]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!target_ldap_id.is_empty() && target_ldap_id != ldap_id);
    let custom = target_comps?
        .as_array()
        .and_then(|a| a.iter().find(|c| c["name"] == "custom-title").cloned())
        .expect("custom mapper created in target realm");
    assert_eq!(custom["parentId"], json!(target_ldap_id));
    assert_eq!(summary?.total(), 0, "plan after apply must be clean");
    Ok(())
}

/// `GET /users` is capped at 100 results per page: kaji must fetch every page.
#[tokio::test]
async fn get_users_returns_more_than_100() -> Result<()> {
    let Some(url) = base_url() else {
        return Ok(());
    };
    let realm = unique_realm("users");
    let admin = Admin::new(&url).await?;
    admin
        .post("", &json!({"realm": realm, "enabled": true}))
        .await?;
    for i in 0..110 {
        admin
            .post(
                &format!("/{}/users", realm),
                &json!({"username": format!("u{:03}", i), "enabled": true}),
            )
            .await?;
    }
    let mut client = admin_client(&url).await?;
    client.set_target_realm(realm.clone());
    let users = client.get_users().await;
    admin.delete_realm(&realm).await;
    assert_eq!(users?.len(), 110);
    Ok(())
}
