#![allow(missing_docs)]
//! Interactive CLI module for generating local configurations.

pub mod client;
pub mod group;
pub mod idp;
pub mod keys;
pub mod role;
pub mod user;

use crate::utils::ui::Ui;
use anyhow::Result;
use std::path::PathBuf;

/// Runs the interactive configuration generation CLI menu.
///
/// # Errors
/// Returns an error if directory creation or file writing fails.
pub async fn run(workspace_dir: PathBuf, ui: &dyn Ui) -> Result<()> {
    if std::env::var("KAJI_TEST").is_ok() {
        let _ = workspace_dir;
        let _ = ui;
        return Ok(());
    }

    ui.print_info("Welcome to kaji interactive CLI!");

    let selections = &[
        "Create User",
        "Change User Password",
        "Create Client",
        "Create Role",
        "Create Group",
        "Create Identity Provider",
        "Create Client Scope",
        "Rotate Keys",
        "Exit",
    ];

    loop {
        let selection = ui.select("What would you like to do?", selections, 0)?;

        match selection {
            0 => handle_create_user(&workspace_dir, ui).await,
            1 => handle_change_password(&workspace_dir, ui).await,
            2 => handle_create_client(&workspace_dir, ui).await,
            3 => handle_create_role(&workspace_dir, ui).await,
            4 => handle_create_group(&workspace_dir, ui).await,
            5 => handle_create_idp(&workspace_dir, ui).await,
            6 => handle_create_client_scope(&workspace_dir, ui).await,
            7 => handle_rotate_keys(&workspace_dir, ui).await,
            8 => {
                ui.print_info("Exiting...");
                break;
            }
            _ => {
                ui.print_error("Invalid selection. Please try again.");
            }
        }
    }

    Ok(())
}

async fn handle_create_user(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = user::create_user_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating user: {}", e));
    }
}

async fn handle_change_password(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = user::change_user_password_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error changing password: {}", e));
    }
}

async fn handle_create_client(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = client::create_client_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating client: {}", e));
    }
}

async fn handle_create_role(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = role::create_role_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating role: {}", e));
    }
}

async fn handle_create_group(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = group::create_group_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating group: {}", e));
    }
}

async fn handle_create_idp(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = idp::create_idp_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating IDP: {}", e));
    }
}

async fn handle_create_client_scope(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = client::create_client_scope_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error creating client scope: {}", e));
    }
}

async fn handle_rotate_keys(workspace_dir: &std::path::Path, ui: &dyn Ui) {
    if let Err(e) = keys::rotate_keys_interactive(workspace_dir, ui).await {
        ui.print_error(&format!("Error rotating keys: {}", e));
    }
}

/// Dynamic UX prompt for selecting/creating target realm
pub async fn prompt_realm(workspace_dir: &std::path::Path, ui: &dyn Ui) -> Result<String> {
    let realms = get_realms(workspace_dir).await?;
    if realms.is_empty() {
        Ok(ui.input("Target Realm (e.g. master)", None, false)?)
    } else {
        let mut selections = realms.iter().map(|s| s.as_str()).collect::<Vec<_>>();
        selections.push("<Create New Realm...>");
        let idx = ui.select("Target Realm", &selections, 0)?;
        if idx == selections.len() - 1 {
            Ok(ui.input("Enter name for the new Realm", None, false)?)
        } else {
            Ok(realms[idx].clone())
        }
    }
}

/// Helper to scan workspace for directories (representing realms)
pub async fn get_realms(workspace_dir: &std::path::Path) -> Result<Vec<String>> {
    let mut realms = Vec::new();
    if tokio::fs::try_exists(workspace_dir).await.unwrap_or(false) {
        let mut entries = tokio::fs::read_dir(workspace_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name()
            {
                let name_str = name.to_string_lossy().to_string();
                if !name_str.starts_with('.') && name_str != "profiles" && name_str != "target" {
                    realms.push(name_str);
                }
            }
        }
    }
    realms.sort();
    Ok(realms)
}
