use criterion::{Criterion, criterion_group, criterion_main};
use kaji::apply::authenticator_config::apply_authenticator_configs;
use kaji::client::KeycloakClient;
use kaji::utils::secrets::{EnvResolver, SecretResolver};
use kaji::utils::ui::DialoguerUi;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[path = "../tests/common/mod.rs"]
mod common;

fn bench_apply_auth_configs(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let server_url = rt.block_on(async { common::start_mock_server().await });
    let mut client = KeycloakClient::new(server_url);
    client.set_target_realm("test-realm".to_string());
    rt.block_on(async {
        client
            .login("admin-cli", None, Some("admin"), Some("admin"))
            .await
            .unwrap();
    });

    let resolver: Arc<dyn SecretResolver> = Arc::new(EnvResolver::new(HashMap::new()));
    let ui = Arc::new(DialoguerUi::new());

    let dir = tempfile::tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();
    let auth_configs_dir = workspace_dir.join("authenticator-configs");
    std::fs::create_dir_all(&auth_configs_dir).unwrap();
    let local_flows_dir = workspace_dir.join("authentication-flows");
    std::fs::create_dir_all(&local_flows_dir).unwrap();

    // Create 10 local authenticator configs
    for i in 0..10 {
        let config = kaji::models::AuthenticatorConfigRepresentation {
            alias: Some(format!("config-{}", i)),
            config: Some(HashMap::new()),
            id: None,
            extra: HashMap::new(),
        };
        std::fs::write(
            auth_configs_dir.join(format!("config-{}.yaml", i)),
            serde_yaml::to_string(&config).unwrap(),
        )
        .unwrap();
    }

    // Create 5 local flows
    for i in 0..5 {
        let mut flow = kaji::models::AuthenticationFlowRepresentation {
            alias: Some(format!("flow-{}", i)),
            authentication_executions: Some(vec![]),
            built_in: None,
            description: None,
            id: None,
            provider_id: None,
            top_level: None,
            extra: HashMap::new(),
        };
        for j in 0..10 {
            flow.authentication_executions.as_mut().unwrap().push(
                kaji::models::AuthenticationExecutionExportRepresentation {
                    authenticator: Some(format!("provider-{}", j)),
                    authenticator_config: Some(format!("config-{}", j)),
                    authenticator_flow: None,
                    flow_alias: None,
                    priority: None,
                    requirement: None,
                    user_setup_allowed: None,
                    id: None,
                    extra: HashMap::new(),
                },
            );
        }
        std::fs::write(
            local_flows_dir.join(format!("flow-{}.yaml", i)),
            serde_yaml::to_string(&flow).unwrap(),
        )
        .unwrap();
    }

    c.bench_function("apply_authenticator_configs", |b| {
        b.to_async(&rt).iter(|| async {
            apply_authenticator_configs(
                &client,
                &workspace_dir,
                Arc::new(workspace_dir.join(".secrets")),
                resolver.clone(),
                Arc::new(None),
                "test-realm",
                None,
                false,
                ui.clone(),
                false,
            )
            .await
            .unwrap();
        });
    });
}

criterion_group!(benches, bench_apply_auth_configs);
criterion_main!(benches);
