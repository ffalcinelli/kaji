use kcd::client::KeycloakClient;
use kcd::plan;
use kcd::utils::secrets::{EnvResolver, SecretResolver};
use kcd::utils::ui::DialoguerUi;
use std::collections::HashMap;
use std::path::PathBuf;
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
        // For password grant: login(client_id, client_secret, username, password)
        client
            .login("admin-cli", None, Some("admin"), Some("admin"))
            .await
            .unwrap();

        let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));

        // Create some temp directories with resources inside to actually have something to plan
        std::fs::create_dir_all("/tmp/perf_test/test-realm").unwrap();
        std::fs::write(
            "/tmp/perf_test/test-realm/realm.yaml",
            "realm: test-realm\nenabled: true\n",
        )
        .unwrap();

        let start = std::time::Instant::now();
        let ui = Arc::new(DialoguerUi::new());
        for _ in 0..500 {
            plan::run(
                &client,
                PathBuf::from("/tmp/perf_test"),
                true,
                false,
                &[],
                ui.clone(),
                resolver.clone(),
                None,
            )
            .await
            .unwrap();
        }
        let elapsed = start.elapsed();
        println!("Elapsed time: {:?}", elapsed);

        std::fs::remove_dir_all("/tmp/perf_test").unwrap();
    });
}
