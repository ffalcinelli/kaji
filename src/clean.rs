//! Clean module for cleaning local workspace directories.

use crate::utils::ui::{ACTION, ERROR, SUCCESS, Ui, WARN};
use anyhow::{Context, Result};
use console::style;
use std::path::PathBuf;
use tokio::fs;

/// Cleans the local configuration files in the workspace directory.
///
/// # Errors
/// Returns an error if file deletion fails.
pub async fn run(
    workspace_dir: PathBuf,
    yes: bool,
    realms_to_clean: &[String],
    ui: &dyn Ui,
) -> Result<()> {
    if !workspace_dir.exists() {
        println!(
            "{} {}",
            WARN,
            style(format!(
                "Output directory {:?} does not exist, nothing to clean.",
                workspace_dir
            ))
            .yellow()
        );
        return Ok(());
    }

    let targets = if realms_to_clean.is_empty() {
        vec![workspace_dir.clone()]
    } else {
        realms_to_clean
            .iter()
            .map(|r| workspace_dir.join(r))
            .filter(|p| p.exists())
            .collect()
    };

    if targets.is_empty() {
        println!("{} {}", WARN, style("No targets found to clean.").yellow());
        return Ok(());
    }

    if !yes {
        let msg = if realms_to_clean.is_empty() {
            format!(
                "Are you sure you want to delete everything in {:?}?",
                workspace_dir
            )
        } else {
            format!(
                "Are you sure you want to delete the following realms in {:?}: {}?",
                workspace_dir,
                realms_to_clean.join(", ")
            )
        };

        if !ui.confirm(&msg, false)? {
            println!("{} {}", ERROR, style("Aborted.").red());
            return Ok(());
        }
    }

    let mut set = tokio::task::JoinSet::new();

    for target in targets {
        if target == workspace_dir && realms_to_clean.is_empty() {
            println!(
                "{} {}",
                ACTION,
                style(format!("Cleaning all configuration in {:?}", workspace_dir)).cyan()
            );
            let mut entries = fs::read_dir(&workspace_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                set.spawn(async move {
                    if file_type.is_dir() {
                        fs::remove_dir_all(&path)
                            .await
                            .with_context(|| format!("Failed to remove dir {:?}", path))
                    } else {
                        fs::remove_file(&path)
                            .await
                            .with_context(|| format!("Failed to remove file {:?}", path))
                    }
                });
            }
        } else {
            println!(
                "{} {}",
                ACTION,
                style(format!("Cleaning realm directory {:?}", target)).cyan()
            );
            set.spawn(async move {
                let metadata = fs::metadata(&target)
                    .await
                    .with_context(|| format!("Failed to get metadata for {:?}", target))?;
                if metadata.is_dir() {
                    fs::remove_dir_all(&target)
                        .await
                        .with_context(|| format!("Failed to remove dir {:?}", target))
                } else {
                    fs::remove_file(&target)
                        .await
                        .with_context(|| format!("Failed to remove file {:?}", target))
                }
            });
        }
    }

    crate::utils::join_all_tasks(set, Some("Join error")).await?;

    println!(
        "{} {}",
        SUCCESS,
        style("Clean completed successfully.").green().bold()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::ui::MockUi;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_clean_all() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();

        fs::create_dir(workspace_dir.join("realm1")).await.unwrap();
        fs::write(workspace_dir.join("realm1").join("realm.yaml"), "test")
            .await
            .unwrap();
        fs::write(workspace_dir.join(".secrets"), "test")
            .await
            .unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(workspace_dir.clone(), true, &[], &ui).await.unwrap();

        assert!(workspace_dir.exists());
        let mut entries = fs::read_dir(&workspace_dir).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_clean_subset() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();

        fs::create_dir(workspace_dir.join("realm1")).await.unwrap();
        fs::create_dir(workspace_dir.join("realm2")).await.unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(workspace_dir.clone(), true, &["realm1".to_string()], &ui)
            .await
            .unwrap();

        assert!(workspace_dir.exists());
        assert!(!workspace_dir.join("realm1").exists());
        assert!(workspace_dir.join("realm2").exists());
    }

    #[tokio::test]
    async fn test_clean_non_existent_workspace() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().join("non-existent");
        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };
        // Should not fail, just print a warning
        run(workspace_dir, true, &[], &ui).await.unwrap();
    }

    #[tokio::test]
    async fn test_clean_empty_targets() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();
        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };
        // workspace exists but we specify a realm that doesn't exist
        run(
            workspace_dir,
            true,
            &["non-existent-realm".to_string()],
            &ui,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_clean_confirm_yes() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();
        fs::create_dir(workspace_dir.join("realm1")).await.unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![true]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(workspace_dir.clone(), false, &[], &ui).await.unwrap();
        assert!(!workspace_dir.join("realm1").exists());
    }

    #[tokio::test]
    async fn test_clean_confirm_abort() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();
        fs::create_dir(workspace_dir.join("realm1")).await.unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![false]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(workspace_dir.clone(), false, &[], &ui).await.unwrap();
        assert!(workspace_dir.join("realm1").exists());
    }
}
