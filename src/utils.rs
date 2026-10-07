#![allow(missing_docs)]
pub mod secrets;
pub mod ui;
pub mod yaml;
use anyhow::Context;
use serde::Serialize;
use std::path::Path;
use tokio::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(not(tarpaulin_include))]
#[allow(unused_variables, dead_code)]
async fn write_secure_windows(path: &Path, content: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use tokio::io::AsyncWriteExt;
        use windows_sys::Win32::Foundation::{
            CloseHandle, ERROR_SUCCESS, GENERIC_ALL, HANDLE, LocalFree,
        };
        use windows_sys::Win32::Security::Authorization::{
            EXPLICIT_ACCESS_W, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, SetSecurityInfo,
            TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
        };
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, GetTokenInformation, NO_INHERITANCE,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let mut file = options
            .open(path)
            .await
            .with_context(|| format!("Failed to open {:?}", path))?;

        unsafe {
            let mut token: HANDLE = std::mem::zeroed();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0 {
                let mut return_length = 0;
                GetTokenInformation(
                    token,
                    TokenUser,
                    std::ptr::null_mut(),
                    0,
                    &mut return_length,
                );
                if return_length > 0 {
                    let mut token_user_bytes: Vec<u8> = vec![0; return_length as usize];
                    if GetTokenInformation(
                        token,
                        TokenUser,
                        token_user_bytes.as_mut_ptr() as *mut _,
                        return_length,
                        &mut return_length,
                    ) != 0
                    {
                        let token_user = &*(token_user_bytes.as_ptr() as *const TOKEN_USER);
                        let mut trustee: TRUSTEE_W = std::mem::zeroed();
                        trustee.TrusteeForm = TRUSTEE_IS_SID;
                        trustee.TrusteeType = TRUSTEE_IS_USER;
                        trustee.ptstrName = token_user.User.Sid as *mut _;

                        let mut access: EXPLICIT_ACCESS_W = std::mem::zeroed();
                        access.grfAccessPermissions = GENERIC_ALL;
                        access.grfAccessMode = SET_ACCESS;
                        access.grfInheritance = NO_INHERITANCE;
                        access.Trustee = trustee;

                        let mut pacl = std::ptr::null_mut();
                        let res = SetEntriesInAclW(1, &access, std::ptr::null_mut(), &mut pacl);
                        if res == ERROR_SUCCESS {
                            let handle = file.as_raw_handle();
                            SetSecurityInfo(
                                handle as _,
                                SE_FILE_OBJECT,
                                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                                std::ptr::null_mut(),
                                std::ptr::null_mut(),
                                pacl,
                                std::ptr::null_mut(),
                            );
                            LocalFree(pacl as _);
                        }
                    }
                }
                CloseHandle(token);
            }
        }

        file.write_all(content.as_bytes())
            .await
            .with_context(|| format!("Failed to write to {:?}", path))?;
        file.flush()
            .await
            .with_context(|| format!("Failed to flush {:?}", path))?;

        return Ok(());
    }
    #[cfg(not(windows))]
    {
        unreachable!("write_secure_windows should only be called on Windows")
    }
}

/// Writes `content` to `path` with owner-only permissions, atomically.
///
/// The content is written to a temporary file in the same directory and renamed over the
/// target, so an interrupted run never leaves a truncated configuration or secrets file.
pub async fn write_secure(path: &Path, content: &str) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("Invalid file path {:?}", path))?;
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let tmp_path = path.with_file_name(format!(
        ".{}.kaji-tmp-{}-{}",
        file_name,
        std::process::id(),
        unique
    ));

    let result = async {
        write_secure_in_place(&tmp_path, content).await?;
        fs::rename(&tmp_path, path)
            .await
            .with_context(|| format!("Failed to replace {:?}", path))
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path).await;
    }
    result
}

async fn write_secure_in_place(path: &Path, content: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::io::AsyncWriteExt;

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);

        let mut file = options
            .open(path)
            .await
            .with_context(|| format!("Failed to open {:?}", path))?;

        let mut perms = file
            .metadata()
            .await
            .with_context(|| format!("Failed to get metadata for {:?}", path))?
            .permissions();
        perms.set_mode(0o600);
        file.set_permissions(perms)
            .await
            .with_context(|| format!("Failed to set permissions for {:?}", path))?;

        file.write_all(content.as_bytes())
            .await
            .with_context(|| format!("Failed to write to {:?}", path))?;
        file.flush()
            .await
            .with_context(|| format!("Failed to flush {:?}", path))?;
        file.sync_all()
            .await
            .with_context(|| format!("Failed to sync {:?}", path))?;
    }
    #[cfg(windows)]
    {
        write_secure_windows(path, content).await?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::write(path, content)
            .await
            .with_context(|| format!("Failed to write {:?}", path))?;
    }
    Ok(())
}

/// Keys whose arrays are order-sensitive and must never be reordered.
const ORDERED_ARRAY_KEYS: &[&str] = &["authenticationExecutions"];

/// Sorts object keys (via `serde_json`'s sorted map) and arrays for stable, diffable output.
///
/// Arrays of primitives are sorted (numbers numerically); arrays of objects are sorted by the
/// first identity key shared by all elements (`id`, `clientId`, `username`, `alias`, `name`).
/// Arrays under order-sensitive keys (e.g. `authenticationExecutions`) keep their order.
pub fn recursive_sort(value: &mut serde_json::Value) {
    sort_value(value, true);
}

fn sort_value(value: &mut serde_json::Value, sort_arrays: bool) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                sort_value(v, !ORDERED_ARRAY_KEYS.contains(&k.as_str()));
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                sort_value(v, true);
            }
            if sort_arrays && !arr.is_empty() {
                sort_array(arr);
            }
        }
        _ => {}
    }
}

fn sort_array(arr: &mut [serde_json::Value]) {
    if arr.iter().all(|v| v.is_number()) {
        arr.sort_by(|a, b| {
            let (a, b) = (
                a.as_f64().unwrap_or_default(),
                b.as_f64().unwrap_or_default(),
            );
            a.total_cmp(&b)
        });
    } else if arr
        .iter()
        .all(|v| v.is_string() || v.is_number() || v.is_boolean())
    {
        arr.sort_by_cached_key(|a| a.to_string());
    } else if arr.iter().all(|v| v.is_object()) {
        // Try to find a common sorting key: id, clientId, username, alias, or name
        let keys = ["id", "clientId", "username", "alias", "name"];
        for key in keys {
            if arr.iter().all(|v| v.get(key).is_some()) {
                if arr
                    .iter()
                    .all(|v| v.get(key).is_some_and(|k| k.is_number()))
                {
                    arr.sort_by(|a, b| {
                        let a = a.get(key).and_then(|v| v.as_f64()).unwrap_or_default();
                        let b = b.get(key).and_then(|v| v.as_f64()).unwrap_or_default();
                        a.total_cmp(&b)
                    });
                } else {
                    arr.sort_by_cached_key(|a| a.get(key).map_or(String::new(), |v| v.to_string()));
                }
                break;
            }
        }
    }
}

pub fn to_sorted_yaml_with_secrets<T: Serialize>(
    value: &T,
    prefix: &str,
    secrets: &mut std::collections::BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let mut json_value =
        serde_json::to_value(value).context("Failed to serialize to JSON value")?;
    crate::utils::secrets::extract_secrets(&mut json_value, prefix, secrets);
    recursive_sort(&mut json_value);
    serde_yaml::to_string(&json_value).context("Failed to serialize to sorted YAML")
}

pub fn to_sorted_yaml<T: Serialize>(value: &T) -> anyhow::Result<String> {
    let mut json_value =
        serde_json::to_value(value).context("Failed to serialize to JSON value")?;
    recursive_sort(&mut json_value);
    serde_yaml::to_string(&json_value).context("Failed to serialize to sorted YAML")
}

/// Waits for every task of the set and returns their results.
///
/// All tasks are always awaited, even after a failure: aborting siblings could interrupt
/// a resource between its API call and the local file update. When several tasks fail, the
/// returned error lists every failure.
pub async fn join_all_tasks<T: 'static>(
    mut set: tokio::task::JoinSet<anyhow::Result<T>>,
    context_msg: Option<&str>,
) -> anyhow::Result<Vec<T>> {
    let mut results = Vec::new();
    let mut errors: Vec<anyhow::Error> = Vec::new();
    while let Some(res) = set.join_next().await {
        let outcome = match res {
            Ok(result) => result,
            Err(join_err) => Err(anyhow::Error::new(join_err)
                .context(context_msg.unwrap_or("Task panicked").to_string())),
        };
        match outcome {
            Ok(value) => results.push(value),
            Err(e) => errors.push(e),
        }
    }
    match errors.len() {
        0 => Ok(results),
        1 => Err(errors.remove(0)),
        n => {
            let details: Vec<String> = errors.iter().map(|e| format!("- {:#}", e)).collect();
            Err(anyhow::anyhow!("{}", details.join("\n")))
                .with_context(|| format!("{} tasks failed", n))
        }
    }
}

/// Validates that a realm name is safe to use as a workspace directory name.
///
/// # Errors
/// Returns an error for empty names, names starting with `.`, or names containing path
/// separators (which would escape the workspace directory).
pub fn validate_realm_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) || name.contains('\0')
    {
        anyhow::bail!(
            "Invalid realm name '{}': realm names used as directories must not be empty, start with '.', or contain path separators",
            name
        );
    }
    Ok(())
}

/// Discovers valid realm directories in the given workspace.
/// Ignores hidden directories (`.*`), `profiles`, `target`, and non-directories.
pub async fn discover_realms(workspace_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut realms = Vec::new();
    match fs::read_dir(workspace_dir).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.') && name != "profiles" && name != "target" {
                        realms.push(name);
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(realms);
        }
        Err(e) => return Err(e).context("Failed to read workspace directory"),
    }
    realms.sort();
    Ok(realms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sorting_with_value() -> anyhow::Result<()> {
        let val = serde_json::json!({
            "z": "val_z",
            "a": "val_a",
            "m": ["item3", "item1", "item2"],
            "b": [true, false, true],
            "n": [3, 1, 2]
        });

        let yaml = to_sorted_yaml(&val)?;
        eprintln!("Generated YAML:\n{}", yaml);

        let lines: Vec<&str> = yaml.lines().collect();
        assert_eq!(lines[0], "a: val_a");
        assert_eq!(lines[1], "b:");
        assert_eq!(lines[2], "- false");
        assert_eq!(lines[3], "- true");
        assert_eq!(lines[4], "- true");
        assert_eq!(lines[5], "m:");
        assert_eq!(lines[6], "- item1");
        assert_eq!(lines[7], "- item2");
        assert_eq!(lines[8], "- item3");
        assert_eq!(lines[9], "n:");
        assert_eq!(lines[10], "- 1");
        assert_eq!(lines[11], "- 2");
        assert_eq!(lines[12], "- 3");
        assert_eq!(lines[13], "z: val_z");
        Ok(())
    }

    #[test]
    fn test_sorting_arrays_of_objects() -> anyhow::Result<()> {
        let val = serde_json::json!({
            "list": [
                { "name": "c", "v": 3 },
                { "name": "a", "v": 1 },
                { "name": "b", "v": 2 }
            ],
            "aliases": [
                { "alias": "z" },
                { "alias": "x" }
            ],
            "ids": [
                { "id": "2" },
                { "id": "1" }
            ]
        });

        let yaml = to_sorted_yaml(&val)?;
        eprintln!("Generated YAML:\n{}", yaml);

        let lines: Vec<&str> = yaml.lines().collect();
        // aliases sorted by alias
        assert_eq!(lines[0], "aliases:");
        assert_eq!(lines[1], "- alias: x");
        assert_eq!(lines[2], "- alias: z");
        // ids sorted by id
        assert_eq!(lines[3], "ids:");
        assert_eq!(lines[4], "- id: '1'");
        assert_eq!(lines[5], "- id: '2'");
        // list sorted by name
        assert_eq!(lines[6], "list:");
        assert_eq!(lines[7], "- name: a");
        assert_eq!(lines[8], "  v: 1");
        assert_eq!(lines[9], "- name: b");
        assert_eq!(lines[10], "  v: 2");
        assert_eq!(lines[11], "- name: c");
        assert_eq!(lines[12], "  v: 3");
        Ok(())
    }

    #[test]
    fn test_recursive_sort_keeps_ordered_arrays_and_sorts_numbers() {
        let mut val = serde_json::json!({
            "authenticationExecutions": [
                {"authenticator": "z", "requirement": "REQUIRED"},
                {"authenticator": "a", "requirement": "ALTERNATIVE"}
            ],
            "numbers": [10, 2, 1],
            "other": [{"name": "z"}, {"name": "a"}]
        });
        recursive_sort(&mut val);
        assert_eq!(val["authenticationExecutions"][0]["authenticator"], "z");
        assert_eq!(val["numbers"], serde_json::json!([1, 2, 10]));
        assert_eq!(val["other"][0]["name"], "a");
    }

    #[test]
    fn test_recursive_sort_empty_array() {
        let mut val = serde_json::json!([]);
        recursive_sort(&mut val);
        assert_eq!(val, serde_json::json!([]));

        let mut val_obj = serde_json::json!({ "empty_arr": [] });
        recursive_sort(&mut val_obj);
        assert_eq!(val_obj, serde_json::json!({ "empty_arr": [] }));
    }

    #[test]
    fn test_recursive_sort_simple_arrays() {
        let mut val = serde_json::json!([3, 1, 2]);
        recursive_sort(&mut val);
        assert_eq!(val, serde_json::json!([1, 2, 3]));

        let mut val_bool = serde_json::json!([true, false, true]);
        recursive_sort(&mut val_bool);
        assert_eq!(val_bool, serde_json::json!([false, true, true]));

        let mut val_mixed = serde_json::json!(["b", 1, true, false, "a"]);
        recursive_sort(&mut val_mixed);
        // string representation: "b", "1", "true", "false", "a"
        // sorted by exact output of .to_string() for json Values:
        // "b" -> "\"b\"", 1 -> "1", true -> "true", false -> "false", "a" -> "\"a\""
        // sorted: "\"a\"", "\"b\"", "1", "false", "true"
        // meaning String("a"), String("b"), Number(1), Bool(false), Bool(true)
        // Note: Number strings ("1") come after String strings starting with double quotes ("\"a\"") in lexicographical order.
        assert_eq!(val_mixed, serde_json::json!(["a", "b", 1, false, true]));
    }

    #[test]
    fn test_recursive_sort_mixed_and_no_keys() {
        let mut val_mixed = serde_json::json!([{"a": 1}, 1, "string"]);
        recursive_sort(&mut val_mixed);
        assert_eq!(val_mixed, serde_json::json!([{"a": 1}, 1, "string"]));

        let mut val_no_keys = serde_json::json!([
            {"other": 2},
            {"other": 1}
        ]);
        recursive_sort(&mut val_no_keys);
        assert_eq!(
            val_no_keys,
            serde_json::json!([
                {"other": 2},
                {"other": 1}
            ])
        );
    }

    #[test]
    fn test_recursive_sort_nested_arrays() {
        let mut val = serde_json::json!({
            "nested": [[2, 1], [4, 3]]
        });
        recursive_sort(&mut val);
        // Elements in array might not be sortable (they are arrays), but inner should be
        assert_eq!(
            val,
            serde_json::json!({
                "nested": [[1, 2], [3, 4]]
            })
        );
    }

    #[test]
    fn test_recursive_sort_primitive() {
        let mut val_str = serde_json::json!("test");
        recursive_sort(&mut val_str);
        assert_eq!(val_str, serde_json::json!("test"));

        let mut val_num = serde_json::json!(42);
        recursive_sort(&mut val_num);
        assert_eq!(val_num, serde_json::json!(42));

        let mut val_null = serde_json::Value::Null;
        recursive_sort(&mut val_null);
        assert_eq!(val_null, serde_json::Value::Null);
    }

    #[test]
    fn test_recursive_sort_mixed_identify_keys() {
        // Scenario 1: Both "id" and "name" are present in all elements. Should sort by "id".
        let mut val1 = serde_json::json!([
            { "id": "2", "name": "a" },
            { "id": "1", "name": "b" }
        ]);
        recursive_sort(&mut val1);
        assert_eq!(
            val1,
            serde_json::json!([
                { "id": "1", "name": "b" },
                { "id": "2", "name": "a" }
            ])
        );

        // Scenario 2: "id" is only present in some elements, but "name" is present in all. Should sort by "name".
        let mut val2 = serde_json::json!([
            { "id": "1", "name": "b" },
            { "name": "a" }
        ]);
        recursive_sort(&mut val2);
        assert_eq!(
            val2,
            serde_json::json!([
                { "name": "a" },
                { "id": "1", "name": "b" }
            ])
        );

        // Scenario 3: "alias" is present in all, but "id" and "name" are missing or partially present. Should sort by "alias".
        let mut val3 = serde_json::json!([
            { "alias": "z", "name": "a" },
            { "alias": "x", "id": "1" }
        ]);
        recursive_sort(&mut val3);
        assert_eq!(
            val3,
            serde_json::json!([
                { "alias": "x", "id": "1" },
                { "alias": "z", "name": "a" }
            ])
        );
    }

    #[test]
    fn test_to_sorted_yaml_with_secrets() -> anyhow::Result<()> {
        let mut secrets = std::collections::BTreeMap::new();
        let val = serde_json::json!({
            "clientId": "myclient",
            "secret": "very-secret",
            "nested": {
                "password": "pass"
            }
        });

        let yaml = to_sorted_yaml_with_secrets(&val, "CLIENT", &mut secrets)?;
        // current_prefix should be "CLIENT_myclient"
        // secret env var should be "KEYCLOAK_CLIENT_MYCLIENT_SECRET"
        // nested password env var should be "KEYCLOAK_CLIENT_MYCLIENT_NESTED_PASSWORD"
        assert!(yaml.contains("secret: ${KEYCLOAK_CLIENT_MYCLIENT_SECRET}"));
        assert!(yaml.contains("password: ${KEYCLOAK_CLIENT_MYCLIENT_NESTED_PASSWORD}"));
        assert_eq!(
            secrets.get("KEYCLOAK_CLIENT_MYCLIENT_SECRET"),
            Some(&"very-secret".to_string())
        );
        assert_eq!(
            secrets.get("KEYCLOAK_CLIENT_MYCLIENT_NESTED_PASSWORD"),
            Some(&"pass".to_string())
        );
        Ok(())
    }

    #[test]
    fn test_to_sorted_yaml_with_secrets_struct() -> anyhow::Result<()> {
        #[derive(serde::Serialize)]
        struct TestConfig {
            #[serde(rename = "clientId")]
            client_id: String,
            name: String,
            password: Option<String>,
            value: i32,
        }

        let mut secrets = std::collections::BTreeMap::new();
        let val = TestConfig {
            client_id: "test-client".to_string(),
            name: "test-name".to_string(),
            password: Some("super-secret".to_string()),
            value: 42,
        };

        let yaml = to_sorted_yaml_with_secrets(&val, "APP", &mut secrets)?;

        assert!(yaml.contains("password: ${KEYCLOAK_APP_TEST_CLIENT_PASSWORD}"));
        assert!(yaml.contains("clientId: test-client"));
        assert_eq!(
            secrets.get("KEYCLOAK_APP_TEST_CLIENT_PASSWORD"),
            Some(&"super-secret".to_string())
        );
        Ok(())
    }

    #[test]
    fn test_to_sorted_yaml_with_secrets_error() {
        struct FailingStruct;
        impl serde::Serialize for FailingStruct {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("Simulated serialization error"))
            }
        }
        let mut secrets = std::collections::BTreeMap::new();
        let result = to_sorted_yaml_with_secrets(&FailingStruct, "FAIL", &mut secrets);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_sorted_yaml_error() {
        struct FailingStruct;
        impl serde::Serialize for FailingStruct {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("Simulated serialization error"))
            }
        }
        let result = to_sorted_yaml(&FailingStruct);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_sorted_yaml_simple() -> anyhow::Result<()> {
        let val = serde_json::json!({ "b": 2, "a": 1 });
        let yaml = to_sorted_yaml(&val)?;
        assert_eq!(yaml.trim(), "a: 1\nb: 2");
        Ok(())
    }

    #[tokio::test]
    async fn test_write_secure_permissions() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.path().join("secure.txt");
        let content = "sensitive data";

        // Test creating a new file
        write_secure(&file_path, content).await?;
        let read_content = std::fs::read_to_string(&file_path)?;
        assert_eq!(read_content, content);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&file_path)?;
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Test updating an existing file with insecure permissions
        let existing_path = temp_dir.path().join("existing.txt");
        std::fs::write(&existing_path, "old content")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&existing_path, std::fs::Permissions::from_mode(0o644))?;
            let metadata = std::fs::metadata(&existing_path)?;
            assert_eq!(metadata.permissions().mode() & 0o777, 0o644);
        }

        write_secure(&existing_path, "new content").await?;
        let read_content = std::fs::read_to_string(&existing_path)?;
        assert_eq!(read_content, "new content");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&existing_path)?;
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_write_secure_is_atomic_and_leaves_no_temp_files() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("realm.yaml");
        std::fs::write(&path, "old")?;
        write_secure(&path, "new").await?;
        assert_eq!(std::fs::read_to_string(&path)?, "new");
        let leftovers: Vec<_> = std::fs::read_dir(temp_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("kaji-tmp"))
            .collect();
        assert!(leftovers.is_empty());

        // A failed write leaves the original file untouched.
        let missing_dir = temp_dir.path().join("missing").join("file.yaml");
        assert!(write_secure(&missing_dir, "x").await.is_err());
        Ok(())
    }

    #[test]
    fn test_recursive_sort_keycloak_identifiers() {
        // Test clientId sorting
        let mut val_client = serde_json::json!([
            { "clientId": "z", "name": "app1" },
            { "clientId": "a", "name": "app2" }
        ]);
        recursive_sort(&mut val_client);
        assert_eq!(
            val_client,
            serde_json::json!([
                { "clientId": "a", "name": "app2" },
                { "clientId": "z", "name": "app1" }
            ])
        );

        // Test username sorting (with id omitted to ensure username is the sort key)
        let mut val_user = serde_json::json!([
            { "username": "user2" },
            { "username": "user1" }
        ]);
        recursive_sort(&mut val_user);
        assert_eq!(
            val_user,
            serde_json::json!([
                { "username": "user1" },
                { "username": "user2" }
            ])
        );

        // Test nested recursive sorting
        let mut val_nested = serde_json::json!({
            "users": [
                { "username": "user2" },
                { "username": "user1" }
            ],
            "config": {
                "clients": [
                    { "clientId": "z" },
                    { "clientId": "a" }
                ]
            }
        });
        recursive_sort(&mut val_nested);
        assert_eq!(
            val_nested,
            serde_json::json!({
                "users": [
                    { "username": "user1" },
                    { "username": "user2" }
                ],
                "config": {
                    "clients": [
                        { "clientId": "a" },
                        { "clientId": "z" }
                    ]
                }
            })
        );
    }

    #[test]
    fn test_recursive_sort_edge_cases() {
        // 1. Array of empty objects
        let mut val_empty_objs = serde_json::json!([{}, {}]);
        recursive_sort(&mut val_empty_objs);
        assert_eq!(val_empty_objs, serde_json::json!([{}, {}]));

        // 2. Array of arrays of arrays
        let mut val_deep_arrays = serde_json::json!([[[2, 1]], [[4, 3]]]);
        recursive_sort(&mut val_deep_arrays);
        assert_eq!(val_deep_arrays, serde_json::json!([[[1, 2]], [[3, 4]]]));

        // 3. Array with Null mixed with primitives
        let mut val_mixed_null = serde_json::json!([1, null, "a"]);
        recursive_sort(&mut val_mixed_null);
        // Does not sort because not all are string/number/boolean
        assert_eq!(val_mixed_null, serde_json::json!([1, null, "a"]));

        // 4. Integer sort keys
        let mut val_int_keys = serde_json::json!([
            { "id": 2, "val": "b" },
            { "id": 1, "val": "a" },
            { "id": 10, "val": "c" }
        ]);
        recursive_sort(&mut val_int_keys);
        // Numeric keys sort numerically
        assert_eq!(
            val_int_keys,
            serde_json::json!([
                { "id": 1, "val": "a" },
                { "id": 2, "val": "b" },
                { "id": 10, "val": "c" }
            ])
        );

        // 5. Null sort keys
        let mut val_null_keys = serde_json::json!([
            { "id": null, "val": "b" },
            { "id": 1, "val": "a" }
        ]);
        recursive_sort(&mut val_null_keys);
        // string representations: "null" vs "1"
        assert_eq!(
            val_null_keys,
            serde_json::json!([
                { "id": 1, "val": "a" },
                { "id": null, "val": "b" }
            ])
        );
    }

    #[tokio::test]
    async fn test_join_all_tasks_reports_panics() {
        let mut set = tokio::task::JoinSet::new();
        set.spawn(async {
            if std::hint::black_box(true) {
                panic!("boom");
            }
            Ok::<(), anyhow::Error>(())
        });
        let err = join_all_tasks(set, Some("Realm task panicked"))
            .await
            .unwrap_err();
        assert!(format!("{:#}", err).contains("Realm task panicked"));
    }

    #[tokio::test]
    async fn test_join_all_tasks_waits_for_all_and_aggregates_errors() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let finished = Arc::new(AtomicUsize::new(0));
        let mut set = tokio::task::JoinSet::new();
        set.spawn(async { Err::<(), _>(anyhow::anyhow!("first failure")) });
        set.spawn(async { Err::<(), _>(anyhow::anyhow!("second failure")) });
        let counter = Arc::clone(&finished);
        set.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let err = join_all_tasks(set, None).await.unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains("2 tasks failed"), "{msg}");
        assert!(msg.contains("first failure") && msg.contains("second failure"));
        // The slow sibling was not aborted.
        assert_eq!(finished.load(Ordering::SeqCst), 1);

        let mut ok_set = tokio::task::JoinSet::new();
        ok_set.spawn(async { Ok::<_, anyhow::Error>(1) });
        assert_eq!(join_all_tasks(ok_set, None).await.unwrap(), vec![1]);
    }

    #[test]
    fn test_validate_realm_name() {
        assert!(validate_realm_name("my-realm").is_ok());
        assert!(validate_realm_name("Realm With Spaces").is_ok());
        for bad in ["", ".", "..", "../etc", "a/b", "a\\b", ".hidden"] {
            assert!(validate_realm_name(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn test_discover_realms() {
        use tempfile::tempdir;
        let temp = tempdir().unwrap();
        let ws = temp.path();

        // Create valid realms
        tokio::fs::create_dir_all(ws.join("realm1")).await.unwrap();
        tokio::fs::create_dir_all(ws.join("realm2")).await.unwrap();

        // Create directories that must be ignored
        tokio::fs::create_dir_all(ws.join(".git")).await.unwrap();
        tokio::fs::create_dir_all(ws.join(".hidden")).await.unwrap();
        tokio::fs::create_dir_all(ws.join("profiles"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(ws.join("target")).await.unwrap();

        // Create a regular file that should be ignored
        tokio::fs::write(ws.join("kaji.toml"), "").await.unwrap();

        let realms = discover_realms(ws).await.unwrap();
        assert_eq!(realms, vec!["realm1".to_string(), "realm2".to_string()]);
    }

    #[tokio::test]
    async fn test_discover_realms_not_found() {
        use tempfile::tempdir;
        let temp = tempdir().unwrap();
        let nonexistent = temp.path().join("does_not_exist");
        let realms = discover_realms(&nonexistent).await.unwrap();
        assert!(realms.is_empty());
    }

    #[tokio::test]
    async fn test_discover_realms_not_dir_error() {
        use tempfile::tempdir;
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("a_file.txt");
        tokio::fs::write(&file_path, "content").await.unwrap();
        let result = discover_realms(&file_path).await;
        assert!(result.is_err());
    }
}
