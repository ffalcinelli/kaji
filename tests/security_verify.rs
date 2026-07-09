use kaji::cli::client::create_client_yaml;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_cli_generated_files_are_secure() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path();

    // Call one of the CLI YAML creation functions
    create_client_yaml(workspace_dir, "master", "testclient", true)
        .await
        .expect("Failed to create client YAML");

    let file_path = workspace_dir
        .join("master")
        .join("clients")
        .join("testclient.yaml");

    assert!(file_path.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&file_path).expect("Failed to get metadata");
        let mode = metadata.permissions().mode();
        // Check that permissions are 0o600 (read/write for owner only)
        assert_eq!(
            mode & 0o777,
            0o600,
            "File permissions should be 0o600, but were {:o}",
            mode & 0o777
        );
    }
}
