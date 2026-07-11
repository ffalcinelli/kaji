use kaji::args::Cli;

#[test]
fn test_cli_debug_obfuscation() {
    let cli = Cli {
        command: kaji::args::Commands::Clean {
            workspace: std::path::PathBuf::from("workspace"),
            yes: false,
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: Some("admin".to_string()),
        password: Some("secret123".to_string()),
        client_id: "admin-cli".to_string(),
        client_secret: Some("secret_client".to_string()),
        profile: None,
        vault_addr: None,
        vault_token: Some("secret_vault".to_string()),
    };

    let debug_str = format!("{:?}", cli);
    assert!(!debug_str.contains("secret123"), "Password was exposed");
    assert!(
        !debug_str.contains("secret_client"),
        "Client secret was exposed"
    );
    assert!(
        !debug_str.contains("secret_vault"),
        "Vault token was exposed"
    );
    assert!(debug_str.contains("********"), "Redaction missing");
    assert!(
        debug_str.contains("http://localhost:8080"),
        "Server was exposed"
    );
}

#[test]
fn test_cli_env_parsing() {
    use clap::Parser;

    unsafe {
        // Set environment variables for testing
        std::env::set_var("KEYCLOAK_PASSWORD", "test-env-password");
        std::env::set_var("KEYCLOAK_CLIENT_SECRET", "test-env-client-secret");
        std::env::set_var("VAULT_TOKEN", "test-env-vault-token");
    }

    // Parse from a dummy command line
    let cli = Cli::parse_from(&[
        "kaji",
        "--server",
        "http://localhost:8080",
        "clean",
        "-w",
        "workspace",
    ]);

    assert_eq!(
        cli.password.as_deref(),
        Some("test-env-password"),
        "Password was not parsed from environment"
    );
    assert_eq!(
        cli.client_secret.as_deref(),
        Some("test-env-client-secret"),
        "Client secret was not parsed from environment"
    );
    assert_eq!(
        cli.vault_token.as_deref(),
        Some("test-env-vault-token"),
        "Vault token was not parsed from environment"
    );

    unsafe {
        // Clean up environment variables
        std::env::remove_var("KEYCLOAK_PASSWORD");
        std::env::remove_var("KEYCLOAK_CLIENT_SECRET");
        std::env::remove_var("VAULT_TOKEN");
    }
}
