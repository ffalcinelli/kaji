//! The `.kajiplan` file produced by `kaji plan` and consumed by `kaji apply`.
//!
//! A plan records which files have pending changes, relative to the workspace, together with the
//! profile it was computed for and a content hash of every planned file (base file plus profile
//! overlay). `apply` refuses plans made for another profile or whose files changed since planning.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs as async_fs;

/// File name of the plan inside the workspace directory.
pub const PLAN_FILE_NAME: &str = ".kajiplan";

const PLAN_VERSION: u32 = 1;

/// A planned file and the hash of its content at planning time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedFile {
    /// Path relative to the workspace directory.
    pub path: PathBuf,
    /// Hex sha256 of the base file followed by its active profile overlay (if any).
    pub sha256: String,
}

/// Contents of a `.kajiplan` file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanFile {
    /// Plan format version.
    pub version: u32,
    /// Profile the plan was computed with.
    pub profile: Option<String>,
    /// Files with pending changes.
    pub files: Vec<PlannedFile>,
}

/// Hashes a base file and its profile overlay, if one exists.
async fn content_hash(path: &Path, profile: Option<&str>) -> Result<String> {
    let mut hasher = Sha256::new();
    let base = async_fs::read(path)
        .await
        .with_context(|| format!("Failed to read planned file {:?}", path))?;
    hasher.update(&base);
    if let Some(overlay) = crate::utils::yaml::find_overlay_path(path, profile).await {
        let overlay_content = async_fs::read(&overlay)
            .await
            .with_context(|| format!("Failed to read overlay {:?}", overlay))?;
        hasher.update(b"\0overlay\0");
        hasher.update(&overlay_content);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

impl PlanFile {
    /// Builds a plan from changed file paths located inside `workspace_dir`.
    ///
    /// # Errors
    /// Returns an error if a file is outside the workspace or cannot be read.
    pub async fn build(
        workspace_dir: &Path,
        profile: Option<&str>,
        changed_files: &[PathBuf],
    ) -> Result<Self> {
        let mut files = Vec::with_capacity(changed_files.len());
        for path in changed_files {
            let relative = path
                .strip_prefix(workspace_dir)
                .with_context(|| format!("Planned file {:?} is outside {:?}", path, workspace_dir))?
                .to_path_buf();
            files.push(PlannedFile {
                path: relative,
                sha256: content_hash(path, profile).await?,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            version: PLAN_VERSION,
            profile: profile.map(String::from),
            files,
        })
    }

    /// Writes the plan to `<workspace_dir>/.kajiplan`.
    ///
    /// # Errors
    /// Returns an error if the file cannot be written.
    pub async fn write(&self, workspace_dir: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        crate::utils::write_secure(&workspace_dir.join(PLAN_FILE_NAME), &content).await
    }

    /// Reads `<workspace_dir>/.kajiplan`, returning `None` if there is no plan.
    ///
    /// # Errors
    /// Returns an error if the plan cannot be read or uses an unsupported format.
    pub async fn read(workspace_dir: &Path) -> Result<Option<Self>> {
        let path = workspace_dir.join(PLAN_FILE_NAME);
        let content = match async_fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("Failed to read {:?}", path)),
        };
        let plan: Self = serde_json::from_str(&content)
            .map_err(|_| anyhow::anyhow!("Hint: Run `kaji plan` again to regenerate it."))
            .with_context(|| format!("Unsupported or corrupt plan file {:?}", path))?;
        if plan.version != PLAN_VERSION {
            return Err(anyhow::anyhow!(
                "Hint: Run `kaji plan` again to regenerate it."
            ))
            .with_context(|| format!("Unsupported plan version {}", plan.version));
        }
        Ok(Some(plan))
    }

    /// Verifies the plan still matches the workspace and returns the absolute planned paths.
    ///
    /// # Errors
    /// Returns an error if the plan was made for another profile or a planned file changed.
    pub async fn verify(
        &self,
        workspace_dir: &Path,
        profile: Option<&str>,
    ) -> Result<HashSet<PathBuf>> {
        if self.profile.as_deref() != profile {
            return Err(anyhow::anyhow!(
                "Hint: Run `kaji plan` with the same profile, or apply with `--profile {}`.",
                self.profile.as_deref().unwrap_or("<none>")
            ))
            .with_context(|| {
                format!(
                    "Plan was created for profile '{}' but apply runs with profile '{}'",
                    self.profile.as_deref().unwrap_or("<none>"),
                    profile.unwrap_or("<none>")
                )
            });
        }

        let mut planned = HashSet::with_capacity(self.files.len());
        let mut stale = Vec::new();
        for file in &self.files {
            let absolute = workspace_dir.join(&file.path);
            match content_hash(&absolute, profile).await {
                Ok(hash) if hash == file.sha256 => {}
                _ => stale.push(file.path.display().to_string()),
            }
            planned.insert(absolute);
        }
        if !stale.is_empty() {
            return Err(anyhow::anyhow!(
                "Hint: Run `kaji plan` again to review the current changes."
            ))
            .with_context(|| {
                format!(
                    "Plan is stale: files changed since it was created: {}",
                    stale.join(", ")
                )
            });
        }
        Ok(planned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_plan_roundtrip_and_relative_paths() -> Result<()> {
        let dir = tempdir()?;
        let ws = dir.path();
        std::fs::create_dir_all(ws.join("r/clients"))?;
        let file = ws.join("r/clients/c.yaml");
        std::fs::write(&file, "clientId: c\n")?;

        let plan = PlanFile::build(ws, None, std::slice::from_ref(&file)).await?;
        assert_eq!(plan.files[0].path, PathBuf::from("r/clients/c.yaml"));
        plan.write(ws).await?;

        let read = PlanFile::read(ws).await?.expect("plan exists");
        assert_eq!(read, plan);
        // The same workspace spelled differently resolves to the same planned set.
        let planned = read.verify(&ws.join("r/.."), None).await?;
        assert!(planned.contains(&ws.join("r/..").join("r/clients/c.yaml")));
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_rejects_stale_files_and_profile_mismatch() -> Result<()> {
        let dir = tempdir()?;
        let ws = dir.path();
        let file = ws.join("c.yaml");
        std::fs::write(&file, "clientId: c\n")?;
        let plan = PlanFile::build(ws, Some("prod"), std::slice::from_ref(&file)).await?;

        let err = plan.verify(ws, None).await.unwrap_err();
        assert!(format!("{:#}", err).contains("profile 'prod'"));

        assert!(plan.verify(ws, Some("prod")).await.is_ok());
        // Overlay changes make the plan stale too.
        std::fs::write(ws.join("c.prod.yaml"), "enabled: false\n")?;
        let err = plan.verify(ws, Some("prod")).await.unwrap_err();
        assert!(format!("{:#}", err).contains("stale"));
        std::fs::remove_file(ws.join("c.prod.yaml"))?;
        std::fs::write(&file, "clientId: changed\n")?;
        assert!(plan.verify(ws, Some("prod")).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_read_errors() -> Result<()> {
        let dir = tempdir()?;
        // Unreadable plan (a directory)
        std::fs::create_dir(dir.path().join(PLAN_FILE_NAME))?;
        let err = PlanFile::read(dir.path()).await.unwrap_err();
        assert!(format!("{:#}", err).contains("Failed to read"));
        std::fs::remove_dir(dir.path().join(PLAN_FILE_NAME))?;
        // Unsupported version
        std::fs::write(
            dir.path().join(PLAN_FILE_NAME),
            r#"{"version": 99, "profile": null, "files": []}"#,
        )?;
        let err = PlanFile::read(dir.path()).await.unwrap_err();
        assert!(format!("{:#}", err).contains("Unsupported plan version 99"));
        // A planned file outside the workspace cannot be recorded
        let outside = tempdir()?;
        let file = outside.path().join("x.yaml");
        std::fs::write(&file, "a: 1")?;
        assert!(PlanFile::build(dir.path(), None, &[file]).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_read_missing_and_legacy() -> Result<()> {
        let dir = tempdir()?;
        assert!(PlanFile::read(dir.path()).await?.is_none());
        std::fs::write(dir.path().join(PLAN_FILE_NAME), "[\"a.yaml\"]")?;
        let err = PlanFile::read(dir.path()).await.unwrap_err();
        assert!(format!("{:#}", err).contains("kaji plan"));
        Ok(())
    }
}
