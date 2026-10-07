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
    if !fs::try_exists(&workspace_dir).await.unwrap_or(false) {
        eprintln!(
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

    // Only realm directories (and the plan) are ever removed: profiles, secrets files and
    // anything else in the workspace (possibly the project root) are left alone.
    let realms = if realms_to_clean.is_empty() {
        crate::utils::discover_realms(&workspace_dir).await?
    } else {
        realms_to_clean.to_vec()
    };
    let mut targets = Vec::new();
    for r in &realms {
        crate::utils::validate_realm_name(r)?;
        let p = workspace_dir.join(r);
        if fs::try_exists(&p).await.unwrap_or(false) {
            targets.push(p);
        }
    }
    let plan_path = workspace_dir.join(crate::plan::plan_file::PLAN_FILE_NAME);
    if realms_to_clean.is_empty() && fs::try_exists(&plan_path).await.unwrap_or(false) {
        targets.push(plan_path);
    }

    if targets.is_empty() {
        eprintln!("{} {}", WARN, style("No targets found to clean.").yellow());
        return Ok(());
    }

    if !yes {
        let msg = format!(
            "Are you sure you want to delete the following realms in {:?}: {}?",
            workspace_dir,
            realms.join(", ")
        );

        if !ui.confirm(&msg, false)? {
            eprintln!("{} {}", ERROR, style("Aborted.").red());
            return Ok(());
        }
    }

    let mut set = tokio::task::JoinSet::new();

    for target in targets {
        eprintln!(
            "{} {}",
            ACTION,
            style(format!("Removing {:?}", target)).cyan()
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

    crate::utils::join_all_tasks(set, Some("Join error")).await?;

    eprintln!(
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

        fs::create_dir(workspace_dir.join("profiles"))
            .await
            .unwrap();
        fs::write(workspace_dir.join(".kajiplan"), "{}")
            .await
            .unwrap();

        run(workspace_dir.clone(), true, &[], &ui).await.unwrap();

        assert!(workspace_dir.exists());
        assert!(!workspace_dir.join("realm1").exists());
        assert!(!workspace_dir.join(".kajiplan").exists());
        // Secrets and profiles are never deleted.
        assert!(workspace_dir.join(".secrets").exists());
        assert!(workspace_dir.join("profiles").exists());
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
    async fn test_clean_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().join("ws");
        fs::create_dir(&workspace_dir).await.unwrap();
        fs::write(dir.path().join("outside.txt"), "keep")
            .await
            .unwrap();
        let ui = MockUi::new();
        let res = run(workspace_dir, true, &["../".to_string()], &ui).await;
        assert!(res.is_err());
        assert!(dir.path().join("outside.txt").exists());
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

    #[tokio::test]
    async fn test_clean_confirm_subset_yes() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path().to_path_buf();
        fs::create_dir(workspace_dir.join("realm1")).await.unwrap();
        fs::create_dir(workspace_dir.join("realm2")).await.unwrap();

        let ui = MockUi {
            inputs: Mutex::new(vec![]),
            confirms: Mutex::new(vec![true]),
            selects: Mutex::new(vec![]),
            passwords: Mutex::new(vec![]),
        };

        run(workspace_dir.clone(), false, &["realm1".to_string()], &ui)
            .await
            .unwrap();

        assert!(!workspace_dir.join("realm1").exists());
        assert!(workspace_dir.join("realm2").exists());
    }
}
