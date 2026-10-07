use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Vault secret resolver submodule.
pub mod vault;

/// Interface for resolving masked secret values to actual credentials (e.g. from environment or Vault).
#[async_trait]
#[allow(clippy::double_must_use)]
pub trait SecretResolver: Send + Sync {
    /// Resolves the given secret key.
    ///
    /// Returns `Ok(Some(value))` if found, `Ok(None)` if not managed by this resolver,
    /// or an error if resolution fails.
    async fn resolve(&self, key: &str) -> Result<Option<String>>;
}

/// Resolves secrets from environment variables or a local `.secrets` file.
pub struct EnvResolver {
    vars: HashMap<String, String>,
}

impl EnvResolver {
    /// Creates a new `EnvResolver` with a predefined map of variables.
    pub fn new(vars: HashMap<String, String>) -> Self {
        Self { vars }
    }
}

#[async_trait]
impl SecretResolver for EnvResolver {
    async fn resolve(&self, key: &str) -> Result<Option<String>> {
        if key.starts_with("vault:") {
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

/// Chains multiple `SecretResolver` implementations together in a prioritized list.
pub struct CompositeResolver {
    resolvers: Vec<Box<dyn SecretResolver>>,
}

impl CompositeResolver {
    /// Creates a new `CompositeResolver` with the given list of resolvers.
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
        // Endpoints and durations, e.g. IdP `tokenUrl`, realm `accessTokenLifespan`
        || lower_key.contains("url")
        || lower_key.contains("endpoint")
        || lower_key.contains("lifespan")
        || lower_key.ends_with("uri")
        // Lists of credential types, e.g. realm `requiredCredentials`, conditional credential
        // authenticator config `credentials`
        || lower_key.ends_with("credentials")
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

    if lower_key == "value" || lower_key == "hashedvalue" {
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
fn get_object_identifier(map: &serde_json::Map<String, Value>) -> Option<&str> {
    map.get("clientId")
        .or_else(|| map.get("username"))
        .or_else(|| map.get("alias"))
        .or_else(|| map.get("name"))
        .and_then(|v| v.as_str())
}

/// Helper to format environment variable names
fn format_env_var_name(prefix: &str, key: &str) -> String {
    // Pre-calculate capacity to avoid reallocations: "KEYCLOAK_" + prefix + "_" + key
    let capacity =
        "KEYCLOAK_".len() + prefix.len() + if prefix.is_empty() { 0 } else { 1 } + key.len();
    let mut out = String::with_capacity(capacity);

    out.push_str("KEYCLOAK_");

    if !prefix.is_empty() {
        for c in prefix.chars() {
            out.push(if c.is_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            });
        }
        out.push('_');
    }

    for c in key.chars() {
        out.push(if c.is_alphanumeric() {
            c.to_ascii_uppercase()
        } else {
            '_'
        });
    }

    out
}

/// Recursively extract secrets and replace them with ${ENV_VAR}
pub fn extract_secrets(
    value: &mut Value,
    prefix: &str,
    secrets: &mut std::collections::BTreeMap<String, String>,
) {
    let mut prefix_buf = String::with_capacity(prefix.len() + 64);
    prefix_buf.push_str(prefix);
    extract_secrets_internal(value, &mut prefix_buf, secrets);
}

fn extract_secrets_internal(
    value: &mut Value,
    prefix_buf: &mut String,
    secrets: &mut std::collections::BTreeMap<String, String>,
) {
    match value {
        Value::Object(map) => {
            let original_len = prefix_buf.len();
            if let Some(id_str) = get_object_identifier(map) {
                if !prefix_buf.is_empty() {
                    prefix_buf.push('_');
                }
                prefix_buf.push_str(id_str);
            }

            for (k, v) in map.iter_mut() {
                if let Value::String(s) = v {
                    if is_secret_key(k, prefix_buf)
                        && !is_boolean_string(s)
                        && !contains_placeholder(s)
                    {
                        let env_var_name = format_env_var_name(prefix_buf, k);
                        secrets.insert(env_var_name.clone(), s.clone());
                        let mut replaced = String::with_capacity(env_var_name.len() + 3);
                        replaced.push_str("${");
                        replaced.push_str(&env_var_name);
                        replaced.push('}');
                        *s = replaced;
                    }
                } else if let (Value::Array(arr), true) = (&mut *v, is_secret_key(k, prefix_buf)) {
                    // Component configs store values as string arrays, e.g. `bindCredential: [..]`
                    let single = arr.len() == 1;
                    for (i, item) in arr.iter_mut().enumerate() {
                        if let Value::String(s) = item
                            && !is_boolean_string(s)
                            && !contains_placeholder(s)
                            && s != KEYCLOAK_MASK
                        {
                            let mut env_var_name = format_env_var_name(prefix_buf, k);
                            if !single {
                                use std::fmt::Write;
                                let _ = write!(env_var_name, "_{}", i);
                            }
                            secrets.insert(env_var_name.clone(), s.clone());
                            *s = format!("${{{}}}", env_var_name);
                        }
                    }
                } else if v.is_object() || v.is_array() {
                    let current_prefix_len = prefix_buf.len();
                    if !prefix_buf.is_empty() {
                        prefix_buf.push('_');
                    }
                    prefix_buf.push_str(k);
                    extract_secrets_internal(v, prefix_buf, secrets);
                    prefix_buf.truncate(current_prefix_len);
                }
            }

            prefix_buf.truncate(original_len);
        }
        Value::Array(arr) => {
            let original_len = prefix_buf.len();
            for (i, v) in arr.iter_mut().enumerate() {
                prefix_buf.push('_');
                use std::fmt::Write;
                let _ = write!(prefix_buf, "{}", i);
                extract_secrets_internal(v, prefix_buf, secrets);
                prefix_buf.truncate(original_len);
            }
        }
        _ => {}
    }
}

/// A piece of a string value: literal text or a secret placeholder name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Literal text, already unescaped.
    Literal(String),
    /// A placeholder name (the text between `${` and `}`).
    Placeholder(String),
}

/// Returns true if `name` is a valid secret placeholder name.
///
/// Only `UPPER_SNAKE_CASE` environment-style names and `vault:` references are placeholders.
/// Anything else (e.g. Keycloak localization keys such as `${client_account}` or
/// `${profileScopeConsentText}`) is kept as literal text.
pub fn is_placeholder_name(name: &str) -> bool {
    if let Some(reference) = name.strip_prefix("vault:") {
        return !reference.is_empty();
    }
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Splits a string into literal and placeholder segments.
///
/// `$${...}` escapes a placeholder and yields the literal text `${...}`.
pub fn parse_segments(s: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut rest = s;
    while let Some(pos) = rest.find("${") {
        if pos > 0 && rest.as_bytes()[pos - 1] == b'$' {
            // Escaped: `$${` -> literal `${`
            literal.push_str(&rest[..pos - 1]);
            literal.push_str("${");
            rest = &rest[pos + 2..];
            continue;
        }
        literal.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        match after.find('}') {
            Some(end) if is_placeholder_name(&after[..end]) => {
                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }
                segments.push(Segment::Placeholder(after[..end].to_string()));
                rest = &after[end + 1..];
            }
            _ => {
                literal.push_str("${");
                rest = after;
            }
        }
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    segments
}

/// Returns true if the string contains at least one secret placeholder.
pub fn contains_placeholder(s: &str) -> bool {
    s.contains("${")
        && parse_segments(s)
            .iter()
            .any(|seg| matches!(seg, Segment::Placeholder(_)))
}

/// Substitutes every placeholder in a single string.
///
/// # Errors
/// Returns an error if a placeholder cannot be resolved.
pub async fn substitute_string(s: &str, resolver: &dyn SecretResolver) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    for segment in parse_segments(s) {
        match segment {
            Segment::Literal(text) => result.push_str(&text),
            Segment::Placeholder(name) => match resolver.resolve(&name).await? {
                Some(val) => result.push_str(&val),
                None => {
                    return Err(anyhow::anyhow!(
                        "Missing required secret or environment variable: {}",
                        name
                    ));
                }
            },
        }
    }
    Ok(result)
}

/// Recursively substitute ${ENV_VAR} or ${vault:path#key} with actual values
#[async_recursion::async_recursion]
#[allow(clippy::double_must_use)]
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
        Value::String(s) if s.contains("${") => {
            *s = substitute_string(s, resolver.as_ref()).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Merges secrets into the content of a dotenv-style secrets file.
///
/// Existing keys are updated in place (comments and ordering are preserved) and new keys are
/// appended. A value equal to Keycloak's `**********` mask never overwrites an existing value.
pub fn merge_secrets_content(
    existing: &str,
    secrets: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut remaining = secrets.clone();
    let mut out = String::with_capacity(existing.len());
    for line in existing.lines() {
        if let Some((k, _)) = line.split_once('=') {
            let key = k.trim();
            if let Some(new_val) = remaining.remove(key)
                && new_val != "**********"
            {
                out.push_str(key);
                out.push('=');
                out.push_str(&new_val);
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    for (k, v) in remaining {
        out.push_str(&k);
        out.push('=');
        out.push_str(&v);
        out.push('\n');
    }
    out
}

/// Keycloak's placeholder for stored secrets it does not return (IdP secrets, LDAP credentials).
pub const KEYCLOAK_MASK: &str = "**********";

/// Replaces a secret with a short hash, so diffs show whether it changed without revealing it.
fn obfuscate_string(s: &str) -> String {
    use sha2::{Digest, Sha256};
    if s.is_empty() || s == KEYCLOAK_MASK || contains_placeholder(s) {
        return s.to_string();
    }
    let digest = Sha256::digest(s.as_bytes());
    let short: String = digest[..4].iter().map(|b| format!("{:02x}", b)).collect();
    format!("<redacted:{}>", short)
}

/// Recursively obfuscate known secret fields
pub fn obfuscate_secrets(value: &mut Value, prefix: &str) {
    let mut prefix_buf = String::with_capacity(prefix.len() + 64);
    prefix_buf.push_str(prefix);
    obfuscate_secrets_internal(value, &mut prefix_buf);
}

fn obfuscate_secrets_internal(value: &mut Value, prefix_buf: &mut String) {
    match value {
        Value::Object(map) => {
            let original_len = prefix_buf.len();
            if let Some(id_str) = get_object_identifier(map) {
                if !prefix_buf.is_empty() {
                    prefix_buf.push('_');
                }
                prefix_buf.push_str(id_str);
            }

            for (k, v) in map.iter_mut() {
                if let Value::String(s) = v {
                    if is_secret_key(k, prefix_buf) {
                        *s = obfuscate_string(s);
                    }
                } else if let (Value::Array(arr), true) = (&mut *v, is_secret_key(k, prefix_buf)) {
                    // Component configs store values as string arrays, e.g. `bindCredential: [..]`
                    for item in arr.iter_mut() {
                        if let Value::String(s) = item {
                            *s = obfuscate_string(s);
                        }
                    }
                } else if v.is_object() || v.is_array() {
                    let current_prefix_len = prefix_buf.len();
                    if !prefix_buf.is_empty() {
                        prefix_buf.push('_');
                    }
                    prefix_buf.push_str(k);
                    obfuscate_secrets_internal(v, prefix_buf);
                    prefix_buf.truncate(current_prefix_len);
                }
            }
            prefix_buf.truncate(original_len);
        }
        Value::Array(arr) => {
            let original_len = prefix_buf.len();
            for (i, v) in arr.iter_mut().enumerate() {
                prefix_buf.push('_');
                use std::fmt::Write;
                let _ = write!(prefix_buf, "{}", i);
                obfuscate_secrets_internal(v, prefix_buf);
                prefix_buf.truncate(original_len);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn test_extract_secrets_arrays() {
        let mut val = json!([
            {"secret": "val1", "other": "normal1"},
            {"secret": "val2", "other": "normal2"}
        ]);
        let mut secrets = std::collections::BTreeMap::new();
        extract_secrets(&mut val, "items", &mut secrets);

        assert_eq!(val[0]["secret"], "${KEYCLOAK_ITEMS_0_SECRET}");
        assert_eq!(val[0]["other"], "normal1");
        assert_eq!(val[1]["secret"], "${KEYCLOAK_ITEMS_1_SECRET}");
        assert_eq!(val[1]["other"], "normal2");

        assert_eq!(
            secrets.get("KEYCLOAK_ITEMS_0_SECRET"),
            Some(&"val1".to_string())
        );
        assert_eq!(
            secrets.get("KEYCLOAK_ITEMS_1_SECRET"),
            Some(&"val2".to_string())
        );
    }

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
    fn test_extract_secrets_nested_array() {
        let mut secrets = std::collections::BTreeMap::new();
        let mut val = json!({"clientId": "test", "items": [{"secret": "foo"}]});
        extract_secrets(&mut val, "app", &mut secrets);

        assert_eq!(
            val["items"][0]["secret"],
            "${KEYCLOAK_APP_TEST_ITEMS_0_SECRET}"
        );
        assert_eq!(
            secrets.get("KEYCLOAK_APP_TEST_ITEMS_0_SECRET").unwrap(),
            "foo"
        );
    }

    #[test]
    fn test_obfuscate_secrets_nested_array() {
        let mut val = json!({"clientId": "test", "items": [{"secret": "foo"}]});
        obfuscate_secrets(&mut val, "app");
        assert!(
            val["items"][0]["secret"]
                .as_str()
                .unwrap()
                .starts_with("<redacted:")
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
        vars.insert("CUSTOM_SECRET".to_string(), "val2".to_string());
        let resolver = Arc::new(EnvResolver::new(vars));

        let mut val = json!({
            "secret": "${KEYCLOAK_VAR1}",
            "custom": "${CUSTOM_SECRET}",
            "other": "normal"
        });

        substitute_secrets(&mut val, resolver.clone())
            .await
            .unwrap();
        assert_eq!(val["secret"], "val1");
        assert_eq!(val["custom"], "val2");
        assert_eq!(val["other"], "normal");

        // Missing variable should return error
        let mut missing_val = json!({
            "missing": "${NON_EXISTENT_VAR}"
        });
        let err = substitute_secrets(&mut missing_val, resolver).await;
        assert!(err.is_err());
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("Missing required secret or environment variable: NON_EXISTENT_VAR")
        );
    }

    #[tokio::test]
    async fn test_env_resolver_skips_vault() {
        let mut vars = HashMap::new();
        vars.insert("vault:secret/data#field".to_string(), "val".to_string());
        let resolver = EnvResolver::new(vars);
        assert_eq!(
            resolver.resolve("vault:secret/data#field").await.unwrap(),
            None
        );
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

        // "value" and "hashedValue" special case with prefix case variations
        assert!(is_secret_key("value", "credential"));
        assert!(is_secret_key("hashedValue", "credential"));
        assert!(is_secret_key("hashedvalue", "credential"));
        assert!(is_secret_key("value", "CREDENTIAL"));
        assert!(is_secret_key("hashedValue", "CREDENTIAL"));
        assert!(is_secret_key("VALUE", "credential"));
        assert!(is_secret_key("value", "my_secret_key"));
        assert!(is_secret_key("hashedValue", "my_secret_key"));
        assert!(is_secret_key("value", "password_field"));
        assert!(is_secret_key("value", "some_token"));
        assert!(is_secret_key("value", "TOKEN"));

        // "value" special case failures
        assert!(!is_secret_key("value", "other"));
        assert!(!is_secret_key("hashedValue", "other"));
        assert!(!is_secret_key("value", ""));
        assert!(!is_secret_key("hashedValue", ""));
        assert!(!is_secret_key("VALUE", ""));

        // Keycloak configuration keys that only look like secrets
        assert!(!is_secret_key("tokenUrl", ""));
        assert!(!is_secret_key("userInfoUrl", ""));
        assert!(!is_secret_key("tokenEndpoint", ""));
        assert!(!is_secret_key("accessTokenLifespan", ""));
        assert!(!is_secret_key("tokenIntrospectionUri", ""));
        assert!(!is_secret_key("credentials", ""));
        assert!(!is_secret_key("requiredCredentials", ""));
        assert!(is_secret_key("bindCredential", ""));
        assert!(is_secret_key("clientSecret", ""));

        // General non-secret
        assert!(!is_secret_key("username", ""));
        assert!(!is_secret_key("clientId", ""));
        assert!(!is_secret_key("email", ""));
        assert!(!is_secret_key("foo", ""));
    }

    #[tokio::test]
    async fn test_substitute_compound_and_embedded_secrets() {
        let mut vars = HashMap::new();
        vars.insert("HOST".to_string(), "ldap.example.com".to_string());
        vars.insert("PORT".to_string(), "389".to_string());
        vars.insert("A".to_string(), "part1".to_string());
        vars.insert("B".to_string(), "part2".to_string());
        let resolver = Arc::new(EnvResolver::new(vars));

        let mut val = json!({
            "url": "ldap://${HOST}:${PORT}/dc=example",
            "compound": "${A}_${B}",
            "prefix_only": "prefix_${A}",
            "suffix_only": "${B}_suffix"
        });

        substitute_secrets(&mut val, resolver).await.unwrap();
        assert_eq!(val["url"], "ldap://ldap.example.com:389/dc=example");
        assert_eq!(val["compound"], "part1_part2");
        assert_eq!(val["prefix_only"], "prefix_part1");
        assert_eq!(val["suffix_only"], "part2_suffix");
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
            ],
            "config": { "bindCredential": ["ldap-pw"] },
            "masked": { "clientSecret": "**********" }
        });

        obfuscate_secrets(&mut val, "client");

        let redacted = obfuscate_string("my_super_secret");
        assert_eq!(val["clientSecret"], json!(redacted));
        assert!(redacted.starts_with("<redacted:") && !redacted.contains("my_"));
        assert_eq!(val["normal"], "value");
        assert_ne!(val["nested"]["password"], "pass");
        assert_ne!(val["array"][0]["token"], "secret_token");
        assert_ne!(val["config"]["bindCredential"][0], "ldap-pw");
        assert_eq!(val["masked"]["clientSecret"], "**********");

        // Same value, same hash; different values, different hashes (even with equal ends).
        assert_eq!(obfuscate_string("abc123z"), obfuscate_string("abc123z"));
        assert_ne!(obfuscate_string("a1z"), obfuscate_string("a2z"));
    }

    #[test]
    fn test_obfuscate_string() {
        assert_eq!(obfuscate_string(""), "");
        assert_eq!(obfuscate_string("**********"), "**********");
        assert_eq!(obfuscate_string("${MY_SECRET}"), "${MY_SECRET}");
        assert!(obfuscate_string("abcd").starts_with("<redacted:"));
    }

    #[test]
    fn test_extract_secrets_from_string_arrays() {
        let mut val = json!({"name": "ldap", "config": {
            "bindCredential": ["pw"],
            "otherCredential": ["a", "b"],
            "maskedCredential": ["**********"],
            "connectionUrl": ["ldap://x"]
        }});
        let mut secrets = BTreeMap::new();
        extract_secrets(&mut val, "component", &mut secrets);
        assert_eq!(
            val["config"]["bindCredential"][0],
            "${KEYCLOAK_COMPONENT_LDAP_CONFIG_BINDCREDENTIAL}"
        );
        assert_eq!(
            secrets.get("KEYCLOAK_COMPONENT_LDAP_CONFIG_BINDCREDENTIAL"),
            Some(&"pw".to_string())
        );
        assert_eq!(
            val["config"]["otherCredential"][1],
            "${KEYCLOAK_COMPONENT_LDAP_CONFIG_OTHERCREDENTIAL_1}"
        );
        assert_eq!(val["config"]["maskedCredential"][0], "**********");
        assert_eq!(val["config"]["connectionUrl"][0], "ldap://x");
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

    #[test]
    fn test_extract_secrets_empty_prefix_with_id() {
        let mut val = json!({
            "alias": "my-id",
            "secret": "s3cr3t"
        });
        let mut secrets = BTreeMap::new();
        extract_secrets(&mut val, "", &mut secrets);
        assert_eq!(
            secrets.get("KEYCLOAK_MY_ID_SECRET"),
            Some(&"s3cr3t".to_string())
        );
    }

    #[test]
    fn test_extract_secrets_empty_prefix_nested() {
        let mut val = json!({
            "nested": {
                "secret": "nested-s3cr3t"
            }
        });
        let mut secrets = BTreeMap::new();
        extract_secrets(&mut val, "", &mut secrets);
        assert_eq!(
            secrets.get("KEYCLOAK_NESTED_SECRET"),
            Some(&"nested-s3cr3t".to_string())
        );
    }

    #[tokio::test]
    async fn test_substitute_secrets_unclosed_placeholder() {
        let resolver = Arc::new(EnvResolver::new(HashMap::new()));
        let mut val = json!({
            "key": "prefix_${unclosed_placeholder"
        });
        substitute_secrets(&mut val, resolver).await.unwrap();
        assert_eq!(val["key"], "prefix_${unclosed_placeholder");
    }

    #[tokio::test]
    async fn test_substitute_keeps_keycloak_localization_keys() {
        let mut vars = HashMap::new();
        vars.insert("SECRET".to_string(), "s3cr3t".to_string());
        let resolver = Arc::new(EnvResolver::new(vars));
        let mut val = json!({
            "name": "${client_account}",
            "description": "${role_offline-access}",
            "attributes": { "consent.screen.text": "${profileScopeConsentText}" },
            "mixed": "${client_x} uses ${SECRET}",
            "escaped": "$${SECRET}",
            "escaped_embedded": "a $${SECRET} and ${SECRET}"
        });
        substitute_secrets(&mut val, resolver).await.unwrap();
        assert_eq!(val["name"], "${client_account}");
        assert_eq!(val["description"], "${role_offline-access}");
        assert_eq!(
            val["attributes"]["consent.screen.text"],
            "${profileScopeConsentText}"
        );
        assert_eq!(val["mixed"], "${client_x} uses s3cr3t");
        assert_eq!(val["escaped"], "${SECRET}");
        assert_eq!(val["escaped_embedded"], "a ${SECRET} and s3cr3t");
    }

    #[test]
    fn test_merge_secrets_content() {
        let existing = "# comment\nA=old\nB=keep\nM=real\n";
        let mut new = BTreeMap::new();
        new.insert("A".to_string(), "new".to_string());
        new.insert("C".to_string(), "added".to_string());
        new.insert("M".to_string(), "**********".to_string());
        let merged = merge_secrets_content(existing, &new);
        assert_eq!(merged, "# comment\nA=new\nB=keep\nM=real\nC=added\n");
        // Idempotent: merging again changes nothing and never duplicates keys.
        assert_eq!(merge_secrets_content(&merged, &new), merged);
        assert_eq!(merge_secrets_content("", &new).lines().count(), 3);
    }

    #[test]
    fn test_placeholder_grammar() {
        assert!(is_placeholder_name("KEYCLOAK_CLIENT_SECRET"));
        assert!(is_placeholder_name("_A1"));
        assert!(is_placeholder_name("vault:secret/app#password"));
        assert!(!is_placeholder_name("vault:"));
        assert!(!is_placeholder_name("client_account"));
        assert!(!is_placeholder_name("profileScopeConsentText"));
        assert!(!is_placeholder_name("1ABC"));
        assert!(!is_placeholder_name(""));
        assert!(contains_placeholder("https://${HOST}/cb"));
        assert!(!contains_placeholder("${client_account}"));
        assert!(!contains_placeholder("$${HOST}"));
        assert_eq!(
            parse_segments("x${A}y"),
            vec![
                Segment::Literal("x".into()),
                Segment::Placeholder("A".into()),
                Segment::Literal("y".into())
            ]
        );
    }

    #[test]
    fn test_obfuscate_secrets_empty_prefix_nested() {
        let mut val = json!({
            "nested": {
                "secret": "top-secret"
            }
        });
        obfuscate_secrets(&mut val, "");
        assert_eq!(
            val["nested"]["secret"],
            json!(obfuscate_string("top-secret"))
        );
    }
}
