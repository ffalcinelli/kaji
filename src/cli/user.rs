use crate::models::{CredentialRepresentation, UserRepresentation};
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn change_user_password_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let username = ui.input("Username", None, false)?;
    let new_password = ui.password("New Password", Some("Confirm Password"))?;

    change_user_password_yaml(workspace_dir, &realm, &username, &new_password).await?;

    ui.print_success("Successfully updated user password YAML.");
    Ok(())
}

pub async fn change_user_password_yaml(
    workspace_dir: &Path,
    realm: &str,
    username: &str,
    new_password: &str,
) -> Result<()> {
    let file_path = workspace_dir
        .join(sanitize(realm))
        .join("users")
        .join(format!("{}.yaml", sanitize(username)));

    if !tokio::fs::try_exists(&file_path).await.unwrap_or(false) {
        println!(
            "{} Warning: User file does not exist. Creating a new one.",
            crate::utils::ui::WARN
        );
        create_user_yaml(workspace_dir, realm, username, None, None, None).await?;
    }

    let yaml_content = fs::read_to_string(&file_path)
        .await
        .context("Failed to read user YAML file")?;
    let mut user: UserRepresentation =
        serde_yaml::from_str(&yaml_content).context("Failed to parse user YAML file")?;

    let new_cred = CredentialRepresentation {
        id: None,
        type_: Some("password".to_string()),
        value: Some(new_password.to_string()),
        temporary: Some(false),
        extra: HashMap::new(),
    };

    if let Some(credentials) = &mut user.credentials {
        if let Some(existing) = credentials
            .iter_mut()
            .find(|c| c.type_.as_deref() == Some("password"))
        {
            existing.value = Some(new_password.to_string());
        } else {
            credentials.push(new_cred);
        }
    } else {
        user.credentials = Some(vec![new_cred]);
    }

    let yaml = serde_yaml::to_string(&user).context("Failed to serialize user to YAML")?;
    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write updated user YAML file")?;

    Ok(())
}

pub async fn create_user_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let username = ui.input("Username", None, false)?;
    let email = ui.input("Email", None, true)?;
    let first_name = ui.input("First Name", None, true)?;
    let last_name = ui.input("Last Name", None, true)?;

    let email_opt = if email.is_empty() { None } else { Some(email) };
    let first_name_opt = if first_name.is_empty() {
        None
    } else {
        Some(first_name)
    };
    let last_name_opt = if last_name.is_empty() {
        None
    } else {
        Some(last_name)
    };

    create_user_yaml(
        workspace_dir,
        &realm,
        &username,
        email_opt,
        first_name_opt,
        last_name_opt,
    )
    .await?;

    ui.print_success("Successfully generated user YAML.");
    Ok(())
}

pub async fn create_user_yaml(
    workspace_dir: &Path,
    realm: &str,
    username: &str,
    email: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
) -> Result<()> {
    let user = UserRepresentation {
        id: None,
        username: Some(username.to_string()),
        enabled: Some(true),
        first_name,
        last_name,
        email,
        email_verified: Some(false),
        credentials: None,
        extra: HashMap::new(),
    };

    let realm_dir = workspace_dir.join(sanitize(realm)).join("users");
    fs::create_dir_all(&realm_dir)
        .await
        .context("Failed to create users directory")?;

    let file_path = realm_dir.join(format!("{}.yaml", sanitize(username)));
    let yaml = serde_yaml::to_string(&user).context("Failed to serialize user to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write user YAML file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_user_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_user_yaml(
            workspace_dir,
            "master",
            "testuser",
            Some("test@example.com".to_string()),
            Some("Test".to_string()),
            Some("User".to_string()),
        )
        .await
        .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("users")
            .join("testuser.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let user: UserRepresentation = serde_yaml::from_str(&content).unwrap();

        assert_eq!(user.username.as_deref(), Some("testuser"));
        assert_eq!(user.email.as_deref(), Some("test@example.com"));
        assert_eq!(user.first_name.as_deref(), Some("Test"));
        assert_eq!(user.last_name.as_deref(), Some("User"));
    }

    #[tokio::test]
    async fn test_create_user_yaml_partial() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_user_yaml(workspace_dir, "master", "user2", None, None, None)
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("users")
            .join("user2.yaml");
        let content = fs::read_to_string(&file_path).await.unwrap();
        let user: UserRepresentation = serde_yaml::from_str(&content).unwrap();

        assert_eq!(user.username.as_deref(), Some("user2"));
        assert_eq!(user.email, None);
        assert_eq!(user.first_name, None);
        assert_eq!(user.last_name, None);
    }

    #[tokio::test]
    async fn test_change_user_password_yaml_existing_password() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_user_yaml(workspace_dir, "master", "testuser", None, None, None)
            .await
            .unwrap();

        // Add first password
        change_user_password_yaml(workspace_dir, "master", "testuser", "pass1")
            .await
            .unwrap();

        // Change password (should update existing)
        change_user_password_yaml(workspace_dir, "master", "testuser", "pass2")
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("users")
            .join("testuser.yaml");
        let content = fs::read_to_string(&file_path).await.unwrap();
        let user: UserRepresentation = serde_yaml::from_str(&content).unwrap();

        let credentials = user.credentials.unwrap();
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].value.as_deref(), Some("pass2"));
    }

    #[tokio::test]
    async fn test_change_user_password_yaml_with_other_credentials() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        let user = UserRepresentation {
            id: None,
            username: Some("testuser".to_string()),
            enabled: Some(true),
            first_name: None,
            last_name: None,
            email: None,
            email_verified: None,
            credentials: Some(vec![CredentialRepresentation {
                id: None,
                type_: Some("otp".to_string()),
                value: Some("secret".to_string()),
                temporary: Some(false),
                extra: HashMap::new(),
            }]),
            extra: HashMap::new(),
        };

        let realm_dir = workspace_dir.join("master").join("users");
        fs::create_dir_all(&realm_dir).await.unwrap();
        let file_path = realm_dir.join("testuser.yaml");
        let yaml = serde_yaml::to_string(&user).unwrap();
        fs::write(&file_path, yaml).await.unwrap();

        change_user_password_yaml(workspace_dir, "master", "testuser", "newpass")
            .await
            .unwrap();

        let content = fs::read_to_string(&file_path).await.unwrap();
        let updated_user: UserRepresentation = serde_yaml::from_str(&content).unwrap();
        let credentials = updated_user.credentials.unwrap();
        assert_eq!(credentials.len(), 2);
        assert!(
            credentials
                .iter()
                .any(|c| c.type_.as_deref() == Some("otp"))
        );
        assert!(credentials.iter().any(
            |c| c.type_.as_deref() == Some("password") && c.value.as_deref() == Some("newpass")
        ));
    }

    #[tokio::test]
    async fn test_change_user_password_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_user_yaml(workspace_dir, "master", "testuser", None, None, None)
            .await
            .unwrap();

        change_user_password_yaml(workspace_dir, "master", "testuser", "newpass123")
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("users")
            .join("testuser.yaml");
        let content = fs::read_to_string(&file_path).await.unwrap();
        let user: UserRepresentation = serde_yaml::from_str(&content).unwrap();

        let credentials = user.credentials.expect("Credentials should not be None");
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].type_.as_deref(), Some("password"));
        assert_eq!(credentials[0].value.as_deref(), Some("newpass123"));
    }

    #[tokio::test]
    async fn test_change_user_password_yaml_new_user() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        // Change password for a user that doesn't exist yet
        change_user_password_yaml(workspace_dir, "master", "newuser", "pass123")
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("users")
            .join("newuser.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let user: UserRepresentation = serde_yaml::from_str(&content).unwrap();
        let credentials = user.credentials.unwrap();
        assert_eq!(credentials[0].value.as_deref(), Some("pass123"));
    }

    #[tokio::test]
    async fn test_change_user_password_yaml_invalid_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let user_path = workspace_dir
            .join("master")
            .join("users")
            .join("baduser.yaml");
        fs::create_dir_all(user_path.parent().unwrap())
            .await
            .unwrap();
        fs::write(&user_path, "not a yaml : [ :").await.unwrap();

        let res = change_user_password_yaml(workspace_dir, "master", "baduser", "newpass").await;
        assert!(res.is_err());
    }
}
