use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub mod vault;

#[async_trait]
pub trait SecretResolver: Send + Sync {
    async fn resolve(&self, key: &str) -> Result<Option<String>>;
}

pub struct EnvResolver {
    vars: HashMap<String, String>,
}

impl EnvResolver {
    pub fn new(vars: HashMap<String, String>) -> Self {
        Self { vars }
    }
}

#[async_trait]
impl SecretResolver for EnvResolver {
    async fn resolve(&self, key: &str) -> Result<Option<String>> {
        if !key.starts_with("KEYCLOAK_") {
            return Ok(None);
        }

        if let Some(val) = self.vars.get(key) {
            return Ok(Some(val.clone()));
        }
        if let Ok(val) = std::env::var(key) {
            return Ok(Some(val));
        }
        Ok(None)
    }
}

pub struct CompositeResolver {
    resolvers: Vec<Box<dyn SecretResolver>>,
}

impl CompositeResolver {
    pub fn new(resolvers: Vec<Box<dyn SecretResolver>>) -> Self {
        Self { resolvers }
    }
}

#[async_trait]
impl SecretResolver for CompositeResolver {
    async fn resolve(&self, key: &str) -> Result<Option<String>> {
        for resolver in &self.resolvers {
            if let Some(val) = resolver.resolve(key).await? {
                return Ok(Some(val));
            }
        }
        Ok(None)
    }
}

/// Heuristics to identify a secret key based on its name.
pub fn is_secret_key(key: &str, prefix: &str) -> bool {
    let lower_key = key.to_lowercase();

    // Blacklist common false positives in Keycloak configuration
    if lower_key.contains("policy")
        || lower_key.contains("passwordless")
        || lower_key.contains("creation")
        || lower_key.contains("delivery")
        || lower_key.contains("reset")
    {
        return false;
    }

    if lower_key.contains("secret")
        || lower_key.contains("password")
        || lower_key.contains("token")
        || lower_key.contains("credential")
    {
        return true;
    }

    if lower_key == "value" {
        let lower_prefix = prefix.to_lowercase();
        return lower_prefix.contains("credential")
            || lower_prefix.contains("secret")
            || lower_prefix.contains("password")
            || lower_prefix.contains("token");
    }

    false
}

/// Heuristics to identify if a string looks like a boolean or simple toggle.
fn is_boolean_string(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower == "true" || lower == "false" || lower == "on" || lower == "off"
}

/// Try to find an identifier for an object to make secret names better
fn get_object_identifier(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.get("clientId")
        .or_else(|| map.get("username"))
        .or_else(|| map.get("alias"))
        .or_else(|| map.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Helper to format environment variable names
fn format_env_var_name(prefix: &str, key: &str) -> String {
    let env_var_name = if prefix.is_empty() {
        format!("KEYCLOAK_{}", key)
    } else {
        format!("KEYCLOAK_{}_{}", prefix, key)
    };
    env_var_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Recursively extract secrets and replace them with ${ENV_VAR}
pub fn extract_secrets(
    value: &mut Value,
    prefix: &str,
    secrets: &mut std::collections::BTreeMap<String, String>,
) {
    match value {
        Value::Object(map) => {
            let id = get_object_identifier(map);

            let current_prefix = if let Some(id_str) = id {
                if prefix.is_empty() {
                    id_str
                } else {
                    format!("{}_{}", prefix, id_str)
                }
            } else {
                prefix.to_string()
            };

            for (k, v) in map.iter_mut() {
                if let Value::String(s) = v {
                    if is_secret_key(k, &current_prefix) && !is_boolean_string(s) {
                        let env_var_name = format_env_var_name(&current_prefix, k);
                        secrets.insert(env_var_name.clone(), s.clone());
                        *s = format!("${{{}}}", env_var_name);
                    }
                } else if v.is_object() || v.is_array() {
                    let new_prefix = if current_prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}_{}", current_prefix, k)
                    };
                    extract_secrets(v, &new_prefix, secrets);
                }
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter_mut().enumerate() {
                let new_prefix = format!("{}_{}", prefix, i);
                extract_secrets(v, &new_prefix, secrets);
            }
        }
        _ => {}
    }
}

/// Recursively substitute ${ENV_VAR} or ${vault:path#key} with actual values
#[async_recursion::async_recursion]
pub async fn substitute_secrets(
    value: &mut Value,
    resolver: Arc<dyn SecretResolver>,
) -> Result<()> {
    match value {
        Value::Object(map) => {
            let futures: Vec<_> = map
                .values_mut()
                .map(|v| Box::pin(substitute_secrets(v, Arc::clone(&resolver))))
                .collect();
            futures::future::try_join_all(futures).await?;
        }
        Value::Array(arr) => {
            let futures: Vec<_> = arr
                .iter_mut()
                .map(|v| Box::pin(substitute_secrets(v, Arc::clone(&resolver))))
                .collect();
            futures::future::try_join_all(futures).await?;
        }
        Value::String(s) if s.starts_with("${") && s.ends_with("}") => {
            let var_name = &s[2..s.len() - 1];
            if let Some(val) = resolver.resolve(var_name).await? {
                *s = val;
            } else if var_name.starts_with("KEYCLOAK_") {
                return Err(anyhow::anyhow!(
                    "Missing required secret or environment variable: {}",
                    var_name
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Helper to obfuscate a single string
fn obfuscate_string(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }

    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 3 {
        return "***".to_string();
    }
    let first = chars[0];
    let last = chars[chars.len() - 1];
    format!("{}***{}", first, last)
}

/// Recursively obfuscate known secret fields
pub fn obfuscate_secrets(value: &mut Value, prefix: &str) {
    match value {
        Value::Object(map) => {
            let id = get_object_identifier(map);

            let current_prefix = if let Some(id_str) = id {
                if prefix.is_empty() {
                    id_str
                } else {
                    format!("{}_{}", prefix, id_str)
                }
            } else {
                prefix.to_string()
            };

            for (k, v) in map.iter_mut() {
                if let Value::String(s) = v {
                    if is_secret_key(k, &current_prefix) {
                        *s = obfuscate_string(s);
                    }
                } else if v.is_object() || v.is_array() {
                    let new_prefix = if current_prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{}_{}", current_prefix, k)
                    };
                    obfuscate_secrets(v, &new_prefix);
                }
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter_mut().enumerate() {
                let new_prefix = format!("{}_{}", prefix, i);
                obfuscate_secrets(v, &new_prefix);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_secrets() {
        let mut val = json!({
            "clientId": "my_client",
            "clientSecret": "my_super_secret",
            "storeToken": "true"
        });
        let mut secrets = std::collections::BTreeMap::new();
        extract_secrets(&mut val, "client", &mut secrets);

        assert_eq!(
            val["clientSecret"],
            "${KEYCLOAK_CLIENT_MY_CLIENT_CLIENTSECRET}"
        );
        assert_eq!(val["storeToken"], "true");
        assert_eq!(
            secrets.get("KEYCLOAK_CLIENT_MY_CLIENT_CLIENTSECRET"),
            Some(&"my_super_secret".to_string())
        );

        let mut val2 = json!({"clientSecret": "secret_value_2"});
        let mut secrets2 = std::collections::BTreeMap::new();
        extract_secrets(&mut val2, "", &mut secrets2);
        assert_eq!(val2["clientSecret"], "${KEYCLOAK_CLIENTSECRET}");
        assert_eq!(
            secrets2.get("KEYCLOAK_CLIENTSECRET").unwrap(),
            "secret_value_2"
        );

        let mut val3 = json!({"clientSecret-special": "secret_value_3"});
        let mut secrets3 = std::collections::BTreeMap::new();
        extract_secrets(&mut val3, "prefix", &mut secrets3);
        assert_eq!(
            val3["clientSecret-special"],
            "${KEYCLOAK_PREFIX_CLIENTSECRET_SPECIAL}"
        );
        assert_eq!(
            secrets3
                .get("KEYCLOAK_PREFIX_CLIENTSECRET_SPECIAL")
                .unwrap(),
            "secret_value_3"
        );
    }

    #[test]
    fn test_extract_secrets_edge_cases() {
        let mut secrets = std::collections::BTreeMap::new();

        // Null
        let mut val = json!(null);
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!(null));
        assert!(secrets.is_empty());

        // Bool
        let mut val = json!(true);
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!(true));
        assert!(secrets.is_empty());

        // Number
        let mut val = json!(42);
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!(42));
        assert!(secrets.is_empty());

        // String
        let mut val = json!("just a string");
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!("just a string"));
        assert!(secrets.is_empty());

        // Empty object
        let mut val = json!({});
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!({}));
        assert!(secrets.is_empty());

        // Empty array
        let mut val = json!([]);
        extract_secrets(&mut val, "prefix", &mut secrets);
        assert_eq!(val, json!([]));
        assert!(secrets.is_empty());
    }

    #[tokio::test]
    async fn test_substitute_secrets() {
        let mut vars = HashMap::new();
        vars.insert("KEYCLOAK_VAR1".to_string(), "val1".to_string());
        let resolver = Arc::new(EnvResolver::new(vars));

        let mut val = json!({
            "secret": "${KEYCLOAK_VAR1}",
            "other": "normal"
        });

        substitute_secrets(&mut val, resolver).await.unwrap();
        assert_eq!(val["secret"], "val1");
        assert_eq!(val["other"], "normal");
    }

    #[tokio::test]
    async fn test_composite_resolver() {
        let mut vars1 = HashMap::new();
        vars1.insert("KEYCLOAK_KEY1".to_string(), "VAL1".to_string());
        let res1 = Box::new(EnvResolver::new(vars1));

        let mut vars2 = HashMap::new();
        vars2.insert("KEYCLOAK_KEY2".to_string(), "VAL2".to_string());
        let res2 = Box::new(EnvResolver::new(vars2));

        let composite = CompositeResolver::new(vec![res1, res2]);

        assert_eq!(
            composite.resolve("KEYCLOAK_KEY1").await.unwrap(),
            Some("VAL1".to_string())
        );
        assert_eq!(
            composite.resolve("KEYCLOAK_KEY2").await.unwrap(),
            Some("VAL2".to_string())
        );
        assert_eq!(composite.resolve("KEYCLOAK_KEY3").await.unwrap(), None);
    }

    #[test]
    fn test_is_secret_key() {
        // Whitelist exact matches (case variations)
        assert!(is_secret_key("secret", ""));
        assert!(is_secret_key("SECRET", ""));
        assert!(is_secret_key("password", ""));
        assert!(is_secret_key("PASSWORD", ""));
        assert!(is_secret_key("token", ""));
        assert!(is_secret_key("TOKEN", ""));
        assert!(is_secret_key("credential", ""));
        assert!(is_secret_key("CREDENTIAL", ""));

        // Whitelist substring matches
        assert!(is_secret_key("myToken", ""));
        assert!(is_secret_key("user_credential", ""));
        assert!(is_secret_key("clientSecret", ""));

        // Blacklist exact and substring matches (override whitelist)
        assert!(!is_secret_key("passwordPolicy", ""));
        assert!(!is_secret_key("isPasswordless", ""));
        assert!(!is_secret_key("creationDate", ""));
        assert!(!is_secret_key("deliveryMethod", ""));
        assert!(!is_secret_key("resetCredentials", ""));

        // Additional blacklist cases testing case insensitivity and combinations
        assert!(!is_secret_key("SECRET_POLICY", ""));
        assert!(!is_secret_key("passwordless_auth", ""));
        assert!(!is_secret_key("CREATION_secret", ""));
        assert!(!is_secret_key("delivery_token", ""));
        assert!(!is_secret_key("RESET_password", ""));
        assert!(!is_secret_key("policy", ""));

        // "value" special case with prefix case variations
        assert!(is_secret_key("value", "credential"));
        assert!(is_secret_key("value", "CREDENTIAL"));
        assert!(is_secret_key("VALUE", "credential"));
        assert!(is_secret_key("value", "my_secret_key"));
        assert!(is_secret_key("value", "password_field"));
        assert!(is_secret_key("value", "some_token"));
        assert!(is_secret_key("value", "TOKEN"));

        // "value" special case failures
        assert!(!is_secret_key("value", "other"));
        assert!(!is_secret_key("value", ""));
        assert!(!is_secret_key("VALUE", ""));

        // General non-secret
        assert!(!is_secret_key("username", ""));
        assert!(!is_secret_key("clientId", ""));
        assert!(!is_secret_key("email", ""));
        assert!(!is_secret_key("foo", ""));
    }

    #[test]
    fn test_obfuscate_secrets() {
        let mut val = json!({
            "clientId": "my_client",
            "clientSecret": "my_super_secret",
            "normal": "value",
            "nested": {
                "password": "pass"
            },
            "array": [
                {"token": "secret_token"}
            ]
        });

        obfuscate_secrets(&mut val, "client");

        assert_eq!(val["clientSecret"], "m***t");
        assert_eq!(val["normal"], "value");
        assert_eq!(val["nested"]["password"], "p***s");
        assert_eq!(val["array"][0]["token"], "s***n");

        let mut val2 = json!({"secret": "secret_value"});
        obfuscate_secrets(&mut val2, "");
        assert_eq!(val2["secret"], "s***e");
    }

    #[test]
    fn test_obfuscate_string() {
        assert_eq!(obfuscate_string(""), "");
        assert_eq!(obfuscate_string("abc"), "***");
        assert_eq!(obfuscate_string("abcd"), "a***d");
    }

    #[test]
    fn test_is_boolean_string() {
        assert!(is_boolean_string("true"));
        assert!(is_boolean_string("false"));
        assert!(is_boolean_string("on"));
        assert!(is_boolean_string("off"));
        assert!(is_boolean_string("TRUE"));
        assert!(is_boolean_string("False"));
        assert!(is_boolean_string("On"));
        assert!(is_boolean_string("OFF"));

        assert!(!is_boolean_string("yes"));
        assert!(!is_boolean_string("no"));
        assert!(!is_boolean_string("1"));
        assert!(!is_boolean_string("0"));
        assert!(!is_boolean_string("random"));
        assert!(!is_boolean_string(""));
        assert!(!is_boolean_string(" true "));
    }
}
