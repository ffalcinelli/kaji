use kaji::args::Cli;

#[test]
fn test_cli_debug_obfuscation() {
    let cli = Cli {
        command: kaji::args::Commands::Clean {
            workspace: Some(std::path::PathBuf::from("workspace")),
            yes: false,
        },
        server: Some("http://localhost:8080".to_string()),
        realms: vec![],
        user: Some("admin".to_string()),
        password: Some("secret123".to_string()),
        client_id: Some("admin-cli".to_string()),
        client_secret: Some("secret_client".to_string()),
        profile: None,
        timeout: None,
        vault_addr: None,
        vault_token: Some("secret_vault".to_string()),
        config: None,
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
