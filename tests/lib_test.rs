use kaji::args::{Cli, Commands};
use kaji::init_client;
use kaji::run_app;
use std::path::PathBuf;

#[tokio::test]
async fn test_init_client_fail() {
    let cli = Cli {
        server: Some("http://invalid".to_string()),
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        user: Some("admin".to_string()),
        password: Some("password".to_string()),
        realms: vec![],
        profile: None,
        timeout: None,
        command: Commands::Validate {
            workspace: Some(PathBuf::from(".")),
        },
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let res = init_client(&cli, None).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_init_client_missing_server_url() {
    let cli = Cli {
        server: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        user: None,
        password: None,
        realms: vec![],
        profile: None,
        timeout: None,
        command: Commands::Validate {
            workspace: Some(PathBuf::from(".")),
        },
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let res = init_client(&cli, None).await;
    match res {
        Err(err) => {
            assert!(err.to_string().contains("Keycloak server URL not provided"));
            let chain: Vec<_> = err.chain().map(|c| c.to_string()).collect();
            assert!(chain.iter().any(|c| c.starts_with("Hint:")));
        }
        Ok(_) => panic!("Expected init_client to fail without server URL"),
    }
}

#[tokio::test]
async fn test_init_client_missing_credentials() {
    let cli = Cli {
        server: Some("http://localhost:8080".to_string()),
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        user: None,
        password: None,
        realms: vec![],
        profile: None,
        timeout: None,
        command: Commands::Validate {
            workspace: Some(PathBuf::from(".")),
        },
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let res = init_client(&cli, None).await;
    match res {
        Err(err) => {
            let err_str = format!("{:?}", err);
            assert!(err_str.contains("Missing authentication credentials"));
        }
        Ok(_) => panic!("Expected init_client to fail without credentials"),
    }
}

#[tokio::test]
async fn test_run_app_plan_non_existent() {
    let cli = Cli {
        server: Some("http://localhost:8080".to_string()),
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        user: None,
        password: None,
        realms: vec![],
        profile: None,
        timeout: None,
        command: Commands::Plan {
            workspace: Some(PathBuf::from("non-existent-dir-123")),
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let res = run_app(cli).await;
    assert!(res.is_err());
}

#[tokio::test]
async fn test_run_app_apply_non_existent() {
    let cli = Cli {
        server: Some("http://localhost:8080".to_string()),
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        user: None,
        password: None,
        realms: vec![],
        profile: None,
        timeout: None,
        command: Commands::Apply {
            workspace: Some(PathBuf::from("non-existent-dir-123")),
            yes: true,
            review: false,
            prune: false,
        },
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    let res = run_app(cli).await;
    assert!(res.is_err());
}
