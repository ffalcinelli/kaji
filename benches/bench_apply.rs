use kaji::apply;
use kaji::client::KeycloakClient;
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::DialoguerUi;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[path = "../tests/common/mod.rs"]
mod common;

fn main() {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let server_url = common::start_mock_server().await;
        let mut client = KeycloakClient::new(server_url);
        client.set_target_realm("test-realm".to_string());
        client
            .login("admin-cli", None, Some("admin"), Some("admin"))
            .await
            .unwrap();

        let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

        let dir = tempfile::tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();
        let mut realms = Vec::new();
        for i in 0..100 {
            let realm_name = format!("realm-{}", i);
            let realm_dir = workspace_dir.join(&realm_name);
            std::fs::create_dir_all(&realm_dir).unwrap();

            let realm = kaji::models::RealmRepresentation {
                realm: realm_name.clone(),
                enabled: Some(true),
                display_name: Some(format!("Updated Realm {}", i)),
                extra: HashMap::new(),
            };
            std::fs::write(
                realm_dir.join("realm.yaml"),
                serde_yaml::to_string(&realm).unwrap(),
            )
            .unwrap();

            realms.push(realm_name);
        }

        let mut total_time = std::time::Duration::new(0, 0);
        let iters = 10;
        let ui = Arc::new(DialoguerUi::new());

        for _ in 0..iters {
            let start = std::time::Instant::now();
            apply::run(
                &client,
                workspace_dir.clone(),
                &realms,
                true,
                false,
                ui.clone(),
                resolver.clone(),
                None,
            )
            .await
            .unwrap();
            total_time += start.elapsed();
        }

        println!(
            "Average elapsed time (100 realms, 10 iterations): {:?}",
            total_time / iters
        );
    });
}
