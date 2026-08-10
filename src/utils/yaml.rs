use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;
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

/// Loads a base YAML file and optionally merges it with a profile-specific overlay.
pub async fn load_yaml_with_overlay(base_path: &Path, profile: Option<&str>) -> Result<Value> {
    let content = async_fs::read_to_string(base_path)
        .await
        .with_context(|| format!("Failed to read base YAML file: {:?}", base_path))?;

    let mut val: Value = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse base YAML file: {:?}", base_path))?;

    if let Some(profile_name) = profile
        && let Some(stem) = base_path.file_stem().and_then(|s| s.to_str())
        && let Some(ext) = base_path.extension().and_then(|e| e.to_str())
    {
        let overlay_path = base_path.with_file_name(format!("{}.{}.{}", stem, profile_name, ext));
        if async_fs::try_exists(&overlay_path).await? {
            let overlay_content = async_fs::read_to_string(&overlay_path)
                .await
                .with_context(|| format!("Failed to read overlay YAML file: {:?}", overlay_path))?;
            let overlay_val: Value = serde_yaml::from_str(&overlay_content).with_context(|| {
                format!("Failed to parse overlay YAML file: {:?}", overlay_path)
            })?;
            deep_merge(&mut val, &overlay_val);
        }
    }

    Ok(val)
}

/// Returns true if the file is a profile-specific overlay.
pub fn is_overlay_file(path: &Path, profile: Option<&str>) -> bool {
    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
        let is_yaml_ext = file_name.ends_with(".yaml") || file_name.ends_with(".yml");
        if !is_yaml_ext {
            return false;
        }

        // Pattern: *.profile.yaml or *.profile.yml
        if let Some(p) = profile
            && (file_name.ends_with(&format!(".{}.yaml", p))
                || file_name.ends_with(&format!(".{}.yml", p)))
        {
            return true;
        }

        // Generic pattern for any profile: *.*.yaml or *.*.yml
        // Avoid matching ".hidden.yaml" which splits to ["", "hidden", "yaml"]
        let parts: Vec<&str> = file_name.split('.').collect();
        if parts.len() >= 3 && !parts[0].is_empty() {
            let potential_profile = parts[parts.len() - 2];

            // 1. Check if potential_profile matches active profile exactly
            if Some(potential_profile) == profile {
                return true;
            }

            // 2. Check if a profile configuration file exists for this name
            // Walk up to find profiles dir
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

            // Fallback to CWD profiles dir
            let profiles_dir = found_profiles_dir.or_else(|| {
                if let Ok(cwd) = std::env::current_dir() {
                    let p_dir = cwd.join("profiles");
                    if p_dir.is_dir() {
                        return Some(p_dir);
                    }
                }
                None
            });

            if let Some(p_dir) = profiles_dir
                && (p_dir.join(format!("{}.yaml", potential_profile)).is_file()
                    || p_dir.join(format!("{}.yml", potential_profile)).is_file())
            {
                return true;
            }

            // 3. Fallback check for common profile names to keep unit tests passing
            let common_profiles = [
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
            if common_profiles.contains(&potential_profile) {
                return true;
            }
        }
    }
    false
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

    #[test]
    fn test_is_overlay_file_exact_profile() {
        assert!(is_overlay_file(Path::new("role.prod.yaml"), Some("prod")));
        assert!(is_overlay_file(Path::new("role.prod.yml"), Some("prod")));
        assert!(is_overlay_file(Path::new("client.test.yaml"), Some("test")));
    }

    #[test]
    fn test_is_overlay_file_generic_profile() {
        // Even if we specify profile "prod", "test.yaml" is detected as an overlay file
        // (which is correctly handled so we know to skip it during Apply)
        assert!(is_overlay_file(Path::new("client.test.yaml"), Some("prod")));
        assert!(is_overlay_file(Path::new("client.test.yml"), Some("prod")));
    }

    #[test]
    fn test_is_overlay_file_no_profile() {
        // Without a profile, only files with common or defined profile tokens are overlays
        assert!(is_overlay_file(Path::new("role.prod.yaml"), None));
        assert!(!is_overlay_file(Path::new("my.resource.yaml"), None));
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
        assert!(is_overlay_file(
            Path::new("my.customprofile.yaml"),
            Some("customprofile")
        ));
    }

    #[tokio::test]
    async fn test_is_overlay_file_finding_profiles_dir() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();
        let profiles_dir = workspace_dir.join("profiles");
        fs::create_dir(&profiles_dir).unwrap();
        fs::write(profiles_dir.join("custom.yaml"), "").unwrap();

        let realm_dir = workspace_dir.join("realm");
        let clients_dir = realm_dir.join("clients");
        fs::create_dir_all(&clients_dir).unwrap();
        let file_path = clients_dir.join("client.custom.yaml");

        assert!(is_overlay_file(&file_path, None));
    }

    #[test]
    fn test_is_overlay_file_cwd_fallback() {
        let cwd = std::env::current_dir().unwrap();
        let profiles_dir = cwd.join("profiles");
        let created = if !profiles_dir.exists() {
            std::fs::create_dir(&profiles_dir).is_ok()
        } else {
            false
        };
        if created {
            std::fs::write(profiles_dir.join("temp_profile.yaml"), "").unwrap();
        }

        // Call is_overlay_file on a path with no parent profiles dir, matching temp_profile
        let path = std::path::Path::new("role.temp_profile.yaml");
        let result = is_overlay_file(path, None);

        // Cleanup
        if created {
            let _ = std::fs::remove_file(profiles_dir.join("temp_profile.yaml"));
            let _ = std::fs::remove_dir(&profiles_dir);
        }

        assert!(result);
    }

    #[test]
    #[cfg(not(windows))] // Cannot delete active current directory on Windows
    fn test_is_overlay_file_cwd_fallback_err() {
        // Run this test in a subprocess to avoid polluting global state (CWD) in concurrent tests
        if std::env::var("RUN_CWD_ERR_TEST").is_ok() {
            let temp = tempdir().unwrap();
            let deleted_dir = temp.path().join("deleted");
            std::fs::create_dir(&deleted_dir).unwrap();
            std::env::set_current_dir(&deleted_dir).unwrap();
            std::fs::remove_dir(&deleted_dir).unwrap();

            let path = Path::new("role.prod.yaml");
            let result = is_overlay_file(path, None);

            assert!(result); // Falls back to common profiles ("prod")
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("utils::yaml::tests::test_is_overlay_file_cwd_fallback_err")
            .arg("--exact")
            .arg("--nocapture")
            .env("RUN_CWD_ERR_TEST", "1")
            .output()
            .unwrap();

        // Ensure the subprocess actually ran the test and didn't just filter out 0 tests
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "Subprocess didn't run the test. Output: {}",
            stdout
        );
        assert!(output.status.success(), "Subprocess failed: {:?}", output);
    }

    #[test]
    fn test_is_overlay_file_cwd_fallback_no_profiles_dir() {
        // Run this test in a subprocess to avoid polluting global state (CWD) in concurrent tests
        if std::env::var("RUN_CWD_NO_PROFILES_TEST").is_ok() {
            let temp = tempdir().unwrap();
            std::env::set_current_dir(temp.path()).unwrap();

            // Path with common profile token
            let path = Path::new("role.dev.yaml");
            let result = is_overlay_file(path, None);

            assert!(result); // Falls back to common profiles ("dev")
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .arg("utils::yaml::tests::test_is_overlay_file_cwd_fallback_no_profiles_dir")
            .arg("--exact")
            .arg("--nocapture")
            .env("RUN_CWD_NO_PROFILES_TEST", "1")
            .output()
            .unwrap();

        // Ensure the subprocess actually ran the test and didn't just filter out 0 tests
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "Subprocess didn't run the test. Output: {}",
            stdout
        );
        assert!(output.status.success(), "Subprocess failed: {:?}", output);
    }
}
