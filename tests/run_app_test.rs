mod common;
use anyhow::Result;
use kaji::args::{Cli, Commands};
use kaji::run_app;
use tempfile::tempdir;

static RUN_APP_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_run_app_validate() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Validate {
            workspace: Some(workspace),
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_inspect() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Inspect {
            workspace: Some(workspace),
            yes: true,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_apply() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let realm_dir = workspace.join("test-realm");
    std::fs::create_dir_all(&realm_dir).unwrap();
    std::fs::write(realm_dir.join("realm.yaml"), "realm: test-realm\n").unwrap();

    let cli = Cli {
        command: Commands::Apply {
            workspace: Some(workspace),
            yes: true,
            review: false,
            prune: false,
        },
        server: Some(mock_url),
        realms: vec!["test-realm".to_string()],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_plan() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Plan {
            workspace: Some(workspace),
            changes_only: false,
            interactive: false,
            verbose: false,
        },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_clean() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Clean {
            workspace: Some(workspace),
            yes: true,
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: None,
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_drift() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    // We need a mock server for drift because it calls init_client
    use common::start_mock_server;
    let mock_url = start_mock_server().await;

    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let cli = Cli {
        command: Commands::Drift {
            workspace: Some(workspace),
            verbose: false,
        },
        server: Some(mock_url),
        realms: vec![],
        user: None,
        password: None,
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret".to_string()),
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: None,
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_with_config_toml() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    let dir = tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    // Write a custom config toml file
    let config_path = dir.path().join("my_config.toml");
    let toml_content = format!(
        r#"
workspace = {:?}
server = "http://localhost:8080"
realms = ["master"]
client_id = "toml-client-id"
"#,
        workspace.to_string_lossy()
    );
    std::fs::write(&config_path, toml_content)?;

    // We pass None for workspace, server, and client_id,
    // and let them resolve from the configuration file.
    let cli = Cli {
        command: Commands::Clean {
            workspace: None,
            yes: true,
        },
        server: None,
        realms: vec![],
        user: None,
        password: None,
        client_id: None,
        client_secret: None,
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: None,
        config: Some(config_path),
    };

    run_app(cli).await?;
    Ok(())
}

#[tokio::test]
async fn test_run_app_init() -> Result<()> {
    if std::env::var("RUN_TEST_RUN_APP_INIT").is_ok() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("my_scaffolded_kaji.toml");

        let cli = Cli {
            command: Commands::Init {
                interactive: false,
                output: Some(config_path.clone()),
            },
            server: None,
            realms: vec![],
            user: None,
            password: None,
            client_id: None,
            client_secret: None,
            profile: None,
            timeout: None,
            vault_addr: None,
            vault_token: None,
            config: None,
        };

        run_app(cli).await.unwrap();

        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path)?;
        assert!(content.contains("server = \"http://myhost:8080\""));
        std::process::exit(0);
    }

    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(exe)
        .arg("test_run_app_init")
        .arg("--exact")
        .arg("--nocapture")
        .env("RUN_TEST_RUN_APP_INIT", "1")
        .env("KEYCLOAK_URL", "http://myhost:8080")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "Subprocess didn't run the test. Output: {}",
        stdout
    );
    assert!(output.status.success(), "Subprocess failed: {:?}", output);
    Ok(())
}

#[tokio::test]
async fn test_run_app_cli_errors_non_tty() -> Result<()> {
    if std::env::var("RUN_TEST_RUN_APP_CLI_ERRORS_NON_TTY").is_ok() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        let cli = Cli {
            command: Commands::Cli {
                workspace: Some(workspace),
            },
            server: None,
            realms: vec![],
            user: None,
            password: None,
            client_id: None,
            client_secret: None,
            profile: None,
            timeout: None,
            vault_addr: None,
            vault_token: None,
            config: None,
        };

        let result = run_app(cli).await;
        assert!(result.is_ok());
        std::process::exit(0);
    }

    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(exe)
        .arg("test_run_app_cli_errors_non_tty")
        .arg("--exact")
        .arg("--nocapture")
        .env("RUN_TEST_RUN_APP_CLI_ERRORS_NON_TTY", "1")
        .env("KAJI_TEST", "true")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "Subprocess didn't run the test. Output: {}",
        stdout
    );
    assert!(output.status.success(), "Subprocess failed: {:?}", output);
    Ok(())
}

#[tokio::test]
async fn test_run_app_clean_interactive_abort() -> Result<()> {
    if std::env::var("RUN_TEST_RUN_APP_CLEAN_INTERACTIVE_ABORT").is_ok() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().to_path_buf();

        let cli = Cli {
            command: Commands::Clean {
                workspace: Some(workspace),
                yes: false,
            },
            server: Some("http://localhost:8080".to_string()),
            realms: vec![],
            user: None,
            password: None,
            client_id: Some("admin-cli".to_string()),
            client_secret: None,
            profile: None,
            timeout: None,
            vault_addr: None,
            vault_token: None,
            config: None,
        };

        let result = run_app(cli).await;
        assert!(result.is_err());
        std::process::exit(0);
    }

    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(exe)
        .arg("test_run_app_clean_interactive_abort")
        .arg("--exact")
        .arg("--nocapture")
        .env("RUN_TEST_RUN_APP_CLEAN_INTERACTIVE_ABORT", "1")
        .env("KAJI_TEST", "true")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("running 1 test"),
        "Subprocess didn't run the test. Output: {}",
        stdout
    );
    assert!(output.status.success(), "Subprocess failed: {:?}", output);
    Ok(())
}

#[tokio::test]
async fn test_load_config_file_from_cwd_kaji() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("kaji.toml");
    let toml_content = r#"
server = "http://localhost:8080"
"#;
    std::fs::write(&config_path, toml_content)?;

    let original_cwd = std::env::current_dir()?;
    std::env::set_current_dir(dir.path())?;

    let config = kaji::load_config_file(None).await?;
    assert_eq!(config.server, Some("http://localhost:8080".to_string()));

    std::env::set_current_dir(original_cwd)?;
    Ok(())
}

#[tokio::test]
async fn test_load_config_file_from_cwd_dot_kaji() -> Result<()> {
    let _lock = RUN_APP_TEST_MUTEX.lock().await;
    let dir = tempdir().unwrap();
    let config_path = dir.path().join(".kaji.toml");
    let toml_content = r#"
server = "http://localhost:9090"
"#;
    std::fs::write(&config_path, toml_content)?;

    let original_cwd = std::env::current_dir()?;
    std::env::set_current_dir(dir.path())?;

    let config = kaji::load_config_file(None).await?;
    assert_eq!(config.server, Some("http://localhost:9090".to_string()));

    std::env::set_current_dir(original_cwd)?;
    Ok(())
}
