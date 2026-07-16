use super::SecretResolver;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use std::collections::HashMap;
use tokio::sync::Mutex;

/// Resolves secrets from a HashiCorp Vault server.
pub struct VaultResolver {
    address: String,
    token: String,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, serde_json::Value>>,
}

impl VaultResolver {
    /// Creates a new `VaultResolver` targeting the Vault address and authenticated with the given token.
    ///
    /// # Errors
    /// Returns an error if the address is not a valid URL.
    pub fn new(address: &str, token: &str) -> Result<Self> {
        reqwest::Url::parse(address)?;
        Ok(Self {
            address: address.trim_end_matches('/').to_string(),
            token: token.to_string(),
            client: reqwest::Client::new(),
            cache: Mutex::new(HashMap::new()),
        })
    }
}

#[derive(Deserialize)]
struct VaultResponse {
    data: VaultData,
}

#[derive(Deserialize)]
struct VaultData {
    data: serde_json::Value,
}

#[async_trait]
impl SecretResolver for VaultResolver {
    async fn resolve(&self, key: &str) -> Result<Option<String>> {
        if !key.starts_with("vault:") {
            return Ok(None);
        }

        // vault:mount/path/to/secret#field
        let parts: Vec<&str> = key[6..].split('#').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid vault secret format. Expected 'vault:mount/path#field', got '{}'",
                key
            ));
        }

        let full_path = parts[0];
        let field = parts[1];

        if full_path.contains("..") {
            return Err(anyhow::anyhow!(
                "Invalid vault path: path traversal detected"
            ));
        }

        // Check cache first
        {
            let cache_lock = self.cache.lock().await;
            if let Some(secret_data) = cache_lock.get(full_path) {
                if let Some(val) = secret_data.get(field) {
                    if let Some(s) = val.as_str() {
                        return Ok(Some(s.to_string()));
                    }
                    return Ok(Some(val.to_string()));
                }
                return Err(anyhow::anyhow!(
                    "Field '{}' not found in cached vault secret '{}'",
                    field,
                    full_path
                ));
            }
        }

        // Split mount and path
        let path_parts: Vec<&str> = full_path.splitn(2, '/').collect();
        if path_parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid vault path format. Expected 'mount/path', got '{}'",
                full_path
            ));
        }
        let mount = path_parts[0];
        let path = path_parts[1];

        let url = format!("{}/v1/{}/data/{}", self.address, mount, path);
        let resp = self
            .client
            .get(&url)
            .header("X-Vault-Token", &self.token)
            .send()
            .await?;

        if resp.status().is_success() {
            let body: VaultResponse = resp.json().await?;
            let secret_data = body.data.data;

            // Insert into cache
            {
                let mut cache_lock = self.cache.lock().await;
                cache_lock.insert(full_path.to_string(), secret_data.clone());
            }

            if let Some(val) = secret_data.get(field) {
                if let Some(s) = val.as_str() {
                    return Ok(Some(s.to_string()));
                }
                return Ok(Some(val.to_string()));
            }
            Err(anyhow::anyhow!(
                "Field '{}' not found in vault secret '{}'",
                field,
                full_path
            ))
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            Err(anyhow::anyhow!("Vault secret not found: {}", full_path))
        } else {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("Vault error ({}): {}", status, text))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use serde_json::json;

    #[tokio::test]
    async fn test_vault_resolver_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "password": "supersecret"
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();
        let res = resolver
            .resolve("vault:secret/mysecret#password")
            .await
            .unwrap();

        assert_eq!(res, Some("supersecret".to_string()));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_vault_resolver_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/secret/data/missing")
            .with_status(404)
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();
        let res = resolver.resolve("vault:secret/missing#key").await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Vault secret not found")
        );
    }

    #[tokio::test]
    async fn test_vault_resolver_invalid_format() {
        let resolver = VaultResolver::new("http://localhost", "token").unwrap();

        let res = resolver.resolve("vault:noparts").await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Invalid vault secret format")
        );

        let res = resolver.resolve("vault:no_slash#field").await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Invalid vault path format")
        );

        let res = resolver.resolve("not-vault").await.unwrap();
        assert_eq!(res, None);
    }

    #[tokio::test]
    async fn test_vault_resolver_path_traversal() {
        let resolver = VaultResolver::new("http://localhost", "token").unwrap();

        let res = resolver.resolve("vault:secret/../mysecret#field").await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("path traversal detected")
        );
    }

    #[tokio::test]
    async fn test_vault_resolver_error_status() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "token").unwrap();
        let res = resolver.resolve("vault:secret/mysecret#field").await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Vault error (500 Internal Server Error)")
        );
    }

    #[tokio::test]
    async fn test_vault_resolver_invalid_address() {
        let res = VaultResolver::new("invalid_addr", "token");
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains("relative URL without a base"));
        }
    }

    #[tokio::test]
    async fn test_vault_resolver_non_string_field() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "port": 8080
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();
        let res = resolver
            .resolve("vault:secret/mysecret#port")
            .await
            .unwrap();

        assert_eq!(res, Some("8080".to_string()));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_vault_resolver_missing_field() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "password": "supersecret"
                        }
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();
        let res = resolver
            .resolve("vault:secret/mysecret#missing_field")
            .await;

        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Field 'missing_field' not found")
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_vault_resolver_caching() {
        let mut server = Server::new_async().await;
        // Mock only allows exactly one call
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "username": "user1",
                            "password": "pass1"
                        }
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();

        let res_user = resolver
            .resolve("vault:secret/mysecret#username")
            .await
            .unwrap();
        assert_eq!(res_user, Some("user1".to_string()));

        let res_pass = resolver
            .resolve("vault:secret/mysecret#password")
            .await
            .unwrap();
        assert_eq!(res_pass, Some("pass1".to_string()));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_vault_resolver_cached_non_string_field() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "port": 8080
                        }
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();

        // 1st call gets value, caches it
        let res1 = resolver
            .resolve("vault:secret/mysecret#port")
            .await
            .unwrap();
        assert_eq!(res1, Some("8080".to_string()));

        // 2nd call should hit the cache (port is a number, so hits the non-string val.to_string() line)
        let res2 = resolver
            .resolve("vault:secret/mysecret#port")
            .await
            .unwrap();
        assert_eq!(res2, Some("8080".to_string()));

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_vault_resolver_cached_missing_field() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/secret/data/mysecret")
            .match_header("X-Vault-Token", "mock-token")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "data": {
                        "data": {
                            "username": "user1"
                        }
                    }
                })
                .to_string(),
            )
            .expect(1)
            .create_async()
            .await;

        let resolver = VaultResolver::new(&server.url(), "mock-token").unwrap();

        // 1st call caches the data
        let res1 = resolver
            .resolve("vault:secret/mysecret#username")
            .await
            .unwrap();
        assert_eq!(res1, Some("user1".to_string()));

        // 2nd call asks for missing field on cached secret data, hits cache-missing-field error path
        let res2 = resolver
            .resolve("vault:secret/mysecret#missing_field")
            .await;
        assert!(res2.is_err());
        assert!(
            res2.unwrap_err()
                .to_string()
                .contains("Field 'missing_field' not found in cached vault secret")
        );

        mock.assert_async().await;
    }
}
