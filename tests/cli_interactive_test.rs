mod common;
use kaji::cli;
use kaji::utils::ui::MockUi;
use std::sync::Mutex;
use tempfile::tempdir;

#[tokio::test]
async fn test_cli_run_exit() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![8]), // Exit is option 8
        passwords: Mutex::new(vec![]),
    };

    cli::run(workspace_dir, &ui).await.unwrap();
}

#[tokio::test]
async fn test_cli_run_invalid_selection() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![999, 8]), // Invalid option followed by Exit
        passwords: Mutex::new(vec![]),
    };

    // The function should not panic and eventually exit when selecting 8
    cli::run(workspace_dir, &ui).await.unwrap();
}

#[tokio::test]
async fn test_create_user_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "newuser".to_string(),
            "user@example.com".to_string(),
            "John".to_string(),
            "Doe".to_string(),
        ]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::user::create_user_interactive(&workspace_dir, &ui)
        .await
        .unwrap();

    let user_path = workspace_dir
        .join("master")
        .join("users")
        .join("newuser.yaml");
    assert!(user_path.exists());
}

#[tokio::test]
async fn test_change_password_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // Pre-create user
    cli::user::create_user_yaml(&workspace_dir, "master", "testuser", None, None, None)
        .await
        .unwrap();

    let ui = MockUi {
        inputs: Mutex::new(vec!["testuser".to_string()]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![0]), // Select master realm
        passwords: Mutex::new(vec!["newpassword".to_string()]),
    };

    cli::user::change_user_password_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_create_client_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec!["master".to_string(), "newclient".to_string()]),
        confirms: Mutex::new(vec![true]), // public client
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::client::create_client_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("clients")
            .join("newclient.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_create_role_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "newrole".to_string(),
            "desc".to_string(),
        ]),
        confirms: Mutex::new(vec![false]), // not a client role
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::role::create_role_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("roles")
            .join("newrole.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_create_group_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec!["master".to_string(), "newgroup".to_string()]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::group::create_group_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("groups")
            .join("newgroup.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_create_idp_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "google".to_string(),
            "google".to_string(),
        ]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::idp::create_idp_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("identity-providers")
            .join("google.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_create_client_scope_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "myscope".to_string(),
            "openid-connect".to_string(),
        ]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::client::create_client_scope_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("client-scopes")
            .join("myscope.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_rotate_keys_interactive() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // Create a key file
    let keys_dir = workspace_dir.join("master").join("components");
    std::fs::create_dir_all(&keys_dir).unwrap();
    let key = kaji::models::ComponentRepresentation {
        id: Some("k1".to_string()),
        name: Some("rsa-generated".to_string()),
        provider_id: Some("rsa-generated".to_string()),
        provider_type: Some("org.keycloak.keys.KeyProvider".to_string()),
        config: Some(std::collections::HashMap::from([(
            "priority".to_string(),
            serde_json::json!(["100"]),
        )])),
        parent_id: None,
        sub_type: None,
        extra: std::collections::HashMap::new(),
    };
    std::fs::write(
        keys_dir.join("key1.yaml"),
        serde_yaml::to_string(&key).unwrap(),
    )
    .unwrap();

    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![0]), // Select master realm
        passwords: Mutex::new(vec![]),
    };

    cli::keys::rotate_keys_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_rotate_keys_interactive_no_keys() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec!["master".to_string()]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::keys::rotate_keys_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_create_role_interactive_empty_desc_and_realm_role() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "realm-role".to_string(),
            "".to_string(), // Empty description
        ]),
        confirms: Mutex::new(vec![false]), // Not a client role
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::role::create_role_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("roles")
            .join("realm-role.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_create_user_interactive_empty_names() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            "master".to_string(),
            "nonameuser".to_string(),
            "".to_string(), // Empty email
            "".to_string(), // Empty first name
            "".to_string(), // Empty last name
        ]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };

    cli::user::create_user_interactive(&workspace_dir, &ui)
        .await
        .unwrap();
    assert!(
        workspace_dir
            .join("master")
            .join("users")
            .join("nonameuser.yaml")
            .exists()
    );
}

#[tokio::test]
async fn test_cli_run_full_loop() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    let ui = MockUi {
        inputs: Mutex::new(vec![
            // 0. Create User (workspace empty -> input realm)
            "master".to_string(),
            "newuser".to_string(),
            "user@example.com".to_string(),
            "John".to_string(),
            "Doe".to_string(),
            // 1. Change Password (workspace has master -> select master (0))
            "newuser".to_string(),
            // 2. Create Client (workspace has master -> select master (0))
            "newclient".to_string(),
            // 3. Create Role (workspace has master -> select master (0))
            "newrole".to_string(),
            "desc".to_string(),
            "newclient".to_string(),
            // 4. Create Group (workspace has master -> select master (0))
            "newgroup".to_string(),
            // 5. Create Identity Provider (workspace has master -> select master (0))
            "google".to_string(),
            "google".to_string(),
            // 6. Create Client Scope (workspace has master -> select master (0))
            "myscope".to_string(),
            "openid-connect".to_string(),
            // 7. Rotate Keys (workspace has master -> select master (0))
        ]),
        confirms: Mutex::new(vec![
            true, // Create Client: is public?
            true, // Create Role: is client role?
        ]),
        selects: Mutex::new(vec![
            0, // Menu: Create User
            1, // Menu: Change Password
            0, // Sub: Select master realm
            2, // Menu: Create Client
            0, // Sub: Select master realm
            3, // Menu: Create Role
            0, // Sub: Select master realm
            4, // Menu: Create Group
            0, // Sub: Select master realm
            5, // Menu: Create Identity Provider
            0, // Sub: Select master realm
            6, // Menu: Create Client Scope
            0, // Sub: Select master realm
            7, // Menu: Rotate Keys
            0, // Sub: Select master realm
            8, // Menu: Exit
        ]),
        passwords: Mutex::new(vec!["secret123".to_string()]),
    };

    cli::run(workspace_dir, &ui).await.unwrap();
}

#[tokio::test]
async fn test_cli_errors_hit() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().to_path_buf();

    // We want the interactive sub-functions to fail.
    // They usually fail if prompt_realm fails, which calls ui.input() or ui.select()
    // For prompt_realm to call input, the realm directory needs to be empty, which it is.

    struct MockFailingUi {
        selects: std::sync::Mutex<Vec<usize>>,
    }

    impl kaji::utils::ui::Ui for MockFailingUi {
        fn print_info(&self, _msg: &str) {}
        fn print_success(&self, _msg: &str) {}
        fn print_warn(&self, _msg: &str) {}
        fn print_error(&self, _msg: &str) {}
        fn input(&self, _prompt: &str, _default: Option<String>, _allow_empty: bool) -> anyhow::Result<String> {
            anyhow::bail!("mock input error")
        }
        fn confirm(&self, _prompt: &str, _default: bool) -> anyhow::Result<bool> {
            anyhow::bail!("mock confirm error")
        }
        fn select(&self, _prompt: &str, _items: &[&str], _default: usize) -> anyhow::Result<usize> {
            let mut selects = self.selects.lock().unwrap();
            if !selects.is_empty() {
                Ok(selects.remove(0))
            } else {
                anyhow::bail!("mock select error")
            }
        }
        fn password(&self, _prompt: &str, _confirm: Option<&str>) -> anyhow::Result<String> {
            anyhow::bail!("mock password error")
        }
    }

    for menu_option in 0..8 {
        let ui = MockFailingUi {
            selects: std::sync::Mutex::new(vec![menu_option, 8]), // execute option, then exit
        };
        cli::run(workspace_dir.clone(), &ui).await.unwrap();
    }
}
