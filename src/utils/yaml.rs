use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::fs as async_fs;

/// Deep merges two JSON values. `b` is merged into `a`.
pub fn deep_merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(a_map), Value::Object(b_map)) => {
            for (key, val) in b_map {
                deep_merge(a_map.entry(key.clone()).or_insert(Value::Null), val);
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// Returns true if the path has a YAML extension (`.yaml` or `.yml`).
pub fn is_yaml_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
}

/// Returns the profile overlay file (`name.{profile}.yaml|yml`) for a base file, if it exists.
pub async fn find_overlay_path(base_path: &Path, profile: Option<&str>) -> Option<PathBuf> {
    let profile_name = profile?;
    let stem = base_path.file_stem()?.to_str()?;
    let ext = base_path.extension()?.to_str()?;
    let alt_ext = if ext == "yaml" { "yml" } else { "yaml" };
    for candidate_ext in [ext, alt_ext] {
        let candidate =
            base_path.with_file_name(format!("{}.{}.{}", stem, profile_name, candidate_ext));
        if async_fs::try_exists(&candidate).await.unwrap_or(false) {
            return Some(candidate);
        }
    }
    None
}

/// Loads a base YAML file and optionally merges it with a profile-specific overlay.
pub async fn load_yaml_with_overlay(base_path: &Path, profile: Option<&str>) -> Result<Value> {
    let content = async_fs::read_to_string(base_path)
        .await
        .with_context(|| format!("Failed to read base YAML file: {:?}", base_path))?;

    let mut val: Value = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse base YAML file: {:?}", base_path))?;

    if let Some(path) = find_overlay_path(base_path, profile).await {
        let overlay_content = async_fs::read_to_string(&path)
            .await
            .with_context(|| format!("Failed to read overlay YAML file: {:?}", path))?;
        let overlay_val: Value = serde_yaml::from_str(&overlay_content)
            .with_context(|| format!("Failed to parse overlay YAML file: {:?}", path))?;
        deep_merge(&mut val, &overlay_val);
    }

    Ok(val)
}

/// Profile names treated as overlays even without a matching `profiles/<name>.yaml` file.
const COMMON_PROFILES: &[&str] = &[
    "prod",
    "production",
    "dev",
    "development",
    "stage",
    "staging",
    "test",
    "local",
    "default",
];

/// Returns true if the file is a profile-specific overlay (`<stem>.<profile>.yaml|yml`).
///
/// A file is only an overlay when its base file (`<stem>.yaml` or `<stem>.yml`) exists next to
/// it, so resources whose names contain dots (e.g. a client `app.test`) are never skipped.
pub fn is_overlay_file(path: &Path, profile: Option<&str>) -> bool {
    if !is_yaml_file(path) {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    // Avoid matching ".hidden.yaml" which splits to ["", "hidden", "yaml"]
    let parts: Vec<&str> = file_name.split('.').collect();
    if parts.len() < 3 || parts[0].is_empty() {
        return false;
    }
    let potential_profile = parts[parts.len() - 2];
    if potential_profile.is_empty() {
        return false;
    }

    let stem = parts[..parts.len() - 2].join(".");
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let base_exists = ["yaml", "yml"]
        .iter()
        .any(|ext| dir.join(format!("{}.{}", stem, ext)).is_file());
    if !base_exists {
        return false;
    }

    Some(potential_profile) == profile
        || is_defined_profile(path, potential_profile)
        || COMMON_PROFILES.contains(&potential_profile)
}

/// Returns true if `profiles/<name>.yaml|yml` exists in a parent directory of `path` or the CWD.
fn is_defined_profile(path: &Path, name: &str) -> bool {
    let mut current = path.parent();
    let mut found_profiles_dir = None;
    while let Some(dir) = current {
        if dir.as_os_str().is_empty() {
            break;
        }
        let p_dir = dir.join("profiles");
        if p_dir.is_dir() {
            found_profiles_dir = Some(p_dir);
            break;
        }
        current = dir.parent();
    }

    let profiles_dir = found_profiles_dir.or_else(|| {
        let p_dir = std::env::current_dir().ok()?.join("profiles");
        p_dir.is_dir().then_some(p_dir)
    });

    profiles_dir.is_some_and(|p_dir| {
        p_dir.join(format!("{}.yaml", name)).is_file()
            || p_dir.join(format!("{}.yml", name)).is_file()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_deep_merge() {
        let mut a = json!({
            "key1": "val1",
            "nested": {
                "sub1": 1
            }
        });
        let b = json!({
            "key2": "val2",
            "nested": {
                "sub2": 2
            }
        });
        deep_merge(&mut a, &b);
        assert_eq!(
            a,
            json!({
                "key1": "val1",
                "key2": "val2",
                "nested": {
                    "sub1": 1,
                    "sub2": 2
                }
            })
        );

        let mut c = json!({ "a": 1 });
        let d = json!({ "a": 2 });
        deep_merge(&mut c, &d);
        assert_eq!(c, json!({ "a": 2 }));
    }

    #[tokio::test]
    async fn test_load_yaml_with_overlay() {
        let dir = tempdir().unwrap();
        let base_path = dir.path().join("resource.yaml");
        fs::write(&base_path, "name: base\nenabled: true\nconfig:\n  k1: v1").unwrap();

        // 1. Load without profile
        let val = load_yaml_with_overlay(&base_path, None).await.unwrap();
        assert_eq!(val["name"], "base");
        assert_eq!(val["enabled"], true);

        // 2. Load with non-existent profile
        let val = load_yaml_with_overlay(&base_path, Some("prod"))
            .await
            .unwrap();
        assert_eq!(val["name"], "base");

        // 3. Load with overlay
        let overlay_path = dir.path().join("resource.prod.yaml");
        fs::write(&overlay_path, "name: prod-override\nconfig:\n  k2: v2").unwrap();

        let val = load_yaml_with_overlay(&base_path, Some("prod"))
            .await
            .unwrap();
        assert_eq!(val["name"], "prod-override");
        assert_eq!(val["enabled"], true);
        assert_eq!(val["config"]["k1"], "v1");
        assert_eq!(val["config"]["k2"], "v2");
    }

    /// Creates the given files (empty) in a fresh temp dir.
    fn files(names: &[&str]) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        for name in names {
            fs::write(dir.path().join(name), "").unwrap();
        }
        dir
    }

    #[test]
    fn test_is_overlay_file_exact_profile() {
        let d = files(&[
            "role.yaml",
            "role.prod.yaml",
            "role.prod.yml",
            "client.yml",
            "client.test.yaml",
        ]);
        assert!(is_overlay_file(
            &d.path().join("role.prod.yaml"),
            Some("prod")
        ));
        assert!(is_overlay_file(
            &d.path().join("role.prod.yml"),
            Some("prod")
        ));
        assert!(is_overlay_file(
            &d.path().join("client.test.yaml"),
            Some("test")
        ));
    }

    #[test]
    fn test_is_overlay_file_generic_profile() {
        // Overlays of other profiles are still recognized (and skipped during Apply)
        let d = files(&["client.yaml"]);
        assert!(is_overlay_file(
            &d.path().join("client.test.yaml"),
            Some("prod")
        ));
        assert!(is_overlay_file(
            &d.path().join("client.test.yml"),
            Some("prod")
        ));
    }

    #[test]
    fn test_is_overlay_file_no_profile() {
        let d = files(&["role.yaml", "my.yaml"]);
        assert!(is_overlay_file(&d.path().join("role.prod.yaml"), None));
        assert!(!is_overlay_file(&d.path().join("my.resource.yaml"), None));
    }

    #[test]
    fn test_is_overlay_file_requires_base_file() {
        // Resources whose names contain profile-like tokens are not overlays without a base.
        let d = files(&["app.test.yaml", "john.dev.yaml"]);
        assert!(!is_overlay_file(&d.path().join("app.test.yaml"), None));
        assert!(!is_overlay_file(
            &d.path().join("app.test.yaml"),
            Some("test")
        ));
        assert!(!is_overlay_file(
            &d.path().join("john.dev.yaml"),
            Some("prod")
        ));
    }

    #[test]
    fn test_is_overlay_file_non_overlays() {
        assert!(!is_overlay_file(Path::new("role.yaml"), Some("prod")));
        assert!(!is_overlay_file(Path::new("role.yml"), Some("prod")));
        assert!(!is_overlay_file(Path::new("role.yaml"), None));
        assert!(!is_overlay_file(Path::new("some.txt"), Some("prod")));
        assert!(!is_overlay_file(Path::new("some.txt"), None));
        assert!(!is_overlay_file(Path::new("no_extension"), Some("prod")));
        assert!(!is_overlay_file(Path::new(".hidden.yaml"), Some("prod")));
    }

    #[test]
    fn test_is_overlay_file_invalid_path() {
        // These paths return None for file_name() and should be gracefully handled
        assert!(!is_overlay_file(Path::new("/"), Some("prod")));
        assert!(!is_overlay_file(Path::new(".."), Some("prod")));
        assert!(!is_overlay_file(Path::new(""), Some("prod")));
    }

    #[tokio::test]
    async fn test_load_yaml_with_invalid_overlay() {
        let dir = tempdir().unwrap();
        let base_path = dir.path().join("resource.yaml");
        fs::write(&base_path, "name: base").unwrap();

        let overlay_path = dir.path().join("resource.prod.yaml");
        fs::write(&overlay_path, "invalid yaml: { :").unwrap();

        let res = load_yaml_with_overlay(&base_path, Some("prod")).await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Failed to parse overlay YAML file")
        );
    }

    #[test]
    fn test_is_overlay_file_matching_profile() {
        let d = files(&["my.yaml"]);
        assert!(is_overlay_file(
            &d.path().join("my.customprofile.yaml"),
            Some("customprofile")
        ));
        assert!(!is_overlay_file(
            &d.path().join("my.customprofile.yaml"),
            None
        ));
    }

    #[tokio::test]
    async fn test_is_overlay_file_finding_profiles_dir() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let profiles_dir = workspace_dir.join("profiles");
        fs::create_dir(&profiles_dir).unwrap();
        fs::write(profiles_dir.join("custom.yaml"), "").unwrap();

        let clients_dir = workspace_dir.join("realm").join("clients");
        fs::create_dir_all(&clients_dir).unwrap();
        fs::write(clients_dir.join("client.yaml"), "").unwrap();

        assert!(is_overlay_file(
            &clients_dir.join("client.custom.yaml"),
            None
        ));
        assert!(!is_overlay_file(
            &clients_dir.join("client.other.yaml"),
            None
        ));
    }

    #[test]
    fn test_is_yaml_file() {
        assert!(is_yaml_file(Path::new("client.yaml")));
        assert!(is_yaml_file(Path::new("client.yml")));
        assert!(is_yaml_file(Path::new("/path/to/flow.dev.yaml")));
        assert!(is_yaml_file(Path::new("/path/to/flow.dev.yml")));
        assert!(!is_yaml_file(Path::new("client.json")));
        assert!(!is_yaml_file(Path::new("client.toml")));
        assert!(!is_yaml_file(Path::new("client")));
    }

    #[tokio::test]
    async fn test_load_yaml_with_cross_extension_overlay() {
        let temp = tempdir().unwrap();
        let base_path = temp.path().join("service.yaml");
        let overlay_path = temp.path().join("service.prod.yml");

        tokio::fs::write(&base_path, "name: base-service\nenabled: false\n")
            .await
            .unwrap();
        tokio::fs::write(&overlay_path, "enabled: true\nport: 8080\n")
            .await
            .unwrap();

        let val = load_yaml_with_overlay(&base_path, Some("prod"))
            .await
            .unwrap();
        assert_eq!(val["name"], "base-service");
        assert_eq!(val["enabled"], true);
        assert_eq!(val["port"], 8080);
    }
}
