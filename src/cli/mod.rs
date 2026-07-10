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
            0 => {
                if let Err(e) = user::create_user_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating user: {}", e));
                }
            }
            1 => {
                if let Err(e) = user::change_user_password_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error changing password: {}", e));
                }
            }
            2 => {
                if let Err(e) = client::create_client_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating client: {}", e));
                }
            }
            3 => {
                if let Err(e) = role::create_role_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating role: {}", e));
                }
            }
            4 => {
                if let Err(e) = group::create_group_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating group: {}", e));
                }
            }
            5 => {
                if let Err(e) = idp::create_idp_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating IDP: {}", e));
                }
            }
            6 => {
                if let Err(e) = client::create_client_scope_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error creating client scope: {}", e));
                }
            }
            7 => {
                if let Err(e) = keys::rotate_keys_interactive(&workspace_dir, ui).await {
                    ui.print_error(&format!("Error rotating keys: {}", e));
                }
            }
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
