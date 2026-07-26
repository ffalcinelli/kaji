#![allow(missing_docs)]
#![allow(clippy::collapsible_if)]
use crate::models::{
    AuthenticationExecutionExportRepresentation, AuthenticationFlowRepresentation,
    AuthenticatorConfigRepresentation, ClientRepresentation, ClientScopeRepresentation,
    ComponentRepresentation, GroupRepresentation, IdentityProviderRepresentation, KeycloakResource,
    RealmRepresentation, RequiredActionProviderRepresentation, RoleRepresentation,
    UserRepresentation,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{debug, info};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// High-level client wrapper for the Keycloak Admin REST API.
#[derive(Clone)]
pub struct KeycloakClient {
    client: Client,
    base_url: String,
    /// The target Keycloak realm being managed.
    pub target_realm: String, // The realm we are managing
    token: Option<String>,
    resource_cache: Arc<RwLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

impl KeycloakClient {
    /// Creates a new `KeycloakClient` instance with the given Keycloak server base URL.
    pub fn new(base_url: String) -> Self {
        let target_realm = "".to_string();
        let base_url = base_url.trim_end_matches('/').to_string();
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            base_url,
            target_realm,
            token: None,
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sets the timeout for the internal HTTP client.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.client = Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| Client::new());
        self
    }

    pub fn set_target_realm(&mut self, target_realm: String) {
        self.target_realm = target_realm;
        if let Ok(mut cache) = self.resource_cache.write() {
            cache.clear();
        }
    }

    pub fn get_base_url(&self) -> &str {
        &self.base_url
    }

    fn realm_admin_url(&self) -> String {
        format!("{}/admin/realms/{}", self.base_url, self.target_realm)
    }

    fn resource_url<T: KeycloakResource>(&self) -> String {
        if T::API_PATH == "realms" {
            format!("{}/admin/realms", self.base_url)
        } else {
            format!("{}/{}", self.realm_admin_url(), T::API_PATH)
        }
    }

    fn object_url<T: KeycloakResource>(&self, id: &str) -> String {
        if T::API_PATH == "realms" {
            format!("{}/admin/realms/{}", self.base_url, id)
        } else {
            format!("{}/{}", self.realm_admin_url(), T::object_path(id))
        }
    }

    pub async fn get_resources<
        T: KeycloakResource + KeycloakResourceMapping + for<'a> Deserialize<'a> + Send,
    >(
        &self,
    ) -> Result<Vec<T>> {
        T::fetch_all(self).await
    }

    pub async fn get_resource<T: KeycloakResource + for<'a> Deserialize<'a>>(
        &self,
        id: &str,
    ) -> Result<T> {
        self.get(&self.object_url::<T>(id)).await
    }

    pub async fn create_resource<
        T: KeycloakResource + KeycloakResourceMapping + Serialize + Clone + Send + 'static,
    >(
        &self,
        res: &T,
    ) -> Result<()> {
        let mapped = res.clone().pre_save(self).await?;
        self.post(&self.resource_url::<T>(), &mapped).await?;
        self.invalidate_resource_cache::<T>();
        Ok(())
    }

    pub async fn update_resource<
        T: KeycloakResource + KeycloakResourceMapping + Serialize + Clone + Send + 'static,
    >(
        &self,
        id: &str,
        res: &T,
    ) -> Result<()> {
        let mapped = res.clone().pre_save(self).await?;
        self.put(&self.object_url::<T>(id), &mapped).await?;
        self.invalidate_resource_cache::<T>();
        Ok(())
    }

    pub async fn delete_resource<T: KeycloakResource + 'static>(&self, id: &str) -> Result<()> {
        self.delete(&self.object_url::<T>(id)).await?;
        self.invalidate_resource_cache::<T>();
        Ok(())
    }

    pub async fn get_realms(&self) -> Result<Vec<RealmRepresentation>> {
        self.get_resources().await
    }

    pub async fn get_realm(&self) -> Result<RealmRepresentation> {
        self.get_resource(&self.target_realm).await
    }

    pub async fn get_clients(&self) -> Result<Vec<ClientRepresentation>> {
        self.get_resources().await
    }

    pub async fn get_roles(&self) -> Result<Vec<RoleRepresentation>> {
        self.get_resources().await
    }

    pub async fn get_identity_providers(&self) -> Result<Vec<IdentityProviderRepresentation>> {
        self.get_resources().await
    }

    /// Updates the target realm representation, passing the realm string by reference to avoid allocations.
    pub async fn update_realm(&self, realm_rep: &RealmRepresentation) -> Result<()> {
        self.update_resource(&self.target_realm, realm_rep).await
    }

    pub async fn create_client(&self, client_rep: &ClientRepresentation) -> Result<()> {
        self.create_resource(client_rep).await
    }

    pub async fn update_client(&self, id: &str, client_rep: &ClientRepresentation) -> Result<()> {
        self.update_resource(id, client_rep).await
    }

    pub async fn delete_client(&self, id: &str) -> Result<()> {
        self.delete_resource::<ClientRepresentation>(id).await
    }

    pub async fn create_role(&self, role_rep: &RoleRepresentation) -> Result<()> {
        self.create_resource(role_rep).await
    }

    pub async fn update_role(&self, id: &str, role_rep: &RoleRepresentation) -> Result<()> {
        self.update_resource(id, role_rep).await
    }

    pub async fn delete_role(&self, id: &str) -> Result<()> {
        self.delete_resource::<RoleRepresentation>(id).await
    }

    pub async fn create_identity_provider(
        &self,
        idp_rep: &IdentityProviderRepresentation,
    ) -> Result<()> {
        self.create_resource(idp_rep).await
    }

    pub async fn update_identity_provider(
        &self,
        alias: &str,
        idp_rep: &IdentityProviderRepresentation,
    ) -> Result<()> {
        self.update_resource(alias, idp_rep).await
    }

    pub async fn delete_identity_provider(&self, alias: &str) -> Result<()> {
        self.delete_resource::<IdentityProviderRepresentation>(alias)
            .await
    }

    pub async fn get_client_scopes(&self) -> Result<Vec<ClientScopeRepresentation>> {
        self.get_resources().await
    }

    pub async fn create_client_scope(&self, scope_rep: &ClientScopeRepresentation) -> Result<()> {
        self.create_resource(scope_rep).await
    }

    pub async fn update_client_scope(
        &self,
        id: &str,
        scope_rep: &ClientScopeRepresentation,
    ) -> Result<()> {
        self.update_resource(id, scope_rep).await
    }

    pub async fn delete_client_scope(&self, id: &str) -> Result<()> {
        self.delete_resource::<ClientScopeRepresentation>(id).await
    }

    pub async fn get_groups(&self) -> Result<Vec<GroupRepresentation>> {
        self.get_resources().await
    }

    pub async fn create_group(&self, group_rep: &GroupRepresentation) -> Result<()> {
        self.create_resource(group_rep).await
    }

    pub async fn update_group(&self, id: &str, group_rep: &GroupRepresentation) -> Result<()> {
        self.update_resource(id, group_rep).await
    }

    pub async fn delete_group(&self, id: &str) -> Result<()> {
        self.delete_resource::<GroupRepresentation>(id).await
    }

    pub async fn get_users(&self) -> Result<Vec<UserRepresentation>> {
        self.get_resources().await
    }

    pub async fn create_user(&self, user_rep: &UserRepresentation) -> Result<()> {
        self.create_resource(user_rep).await
    }

    pub async fn update_user(&self, id: &str, user_rep: &UserRepresentation) -> Result<()> {
        self.update_resource(id, user_rep).await
    }

    pub async fn delete_user(&self, id: &str) -> Result<()> {
        self.delete_resource::<UserRepresentation>(id).await
    }

    pub async fn get_authentication_flows(&self) -> Result<Vec<AuthenticationFlowRepresentation>> {
        self.get_resources().await
    }

    pub async fn create_authentication_flow(
        &self,
        flow_rep: &AuthenticationFlowRepresentation,
    ) -> Result<()> {
        self.create_resource(flow_rep).await
    }

    pub async fn update_authentication_flow(
        &self,
        id: &str,
        flow_rep: &AuthenticationFlowRepresentation,
    ) -> Result<()> {
        self.update_resource(id, flow_rep).await
    }

    pub async fn delete_authentication_flow(&self, id: &str) -> Result<()> {
        self.delete_resource::<AuthenticationFlowRepresentation>(id)
            .await
    }

    pub async fn get_required_actions(&self) -> Result<Vec<RequiredActionProviderRepresentation>> {
        self.get_resources().await
    }

    pub async fn update_required_action(
        &self,
        alias: &str,
        action_rep: &RequiredActionProviderRepresentation,
    ) -> Result<()> {
        self.update_resource(alias, action_rep).await
    }

    pub async fn register_required_action(
        &self,
        action_rep: &RequiredActionProviderRepresentation,
    ) -> Result<()> {
        let url = self.realm_admin_url() + "/authentication/register-required-action";

        #[derive(Serialize)]
        struct RegisterActionBody<'a> {
            #[serde(rename = "providerId")]
            provider_id: &'a str,
            name: &'a str,
        }

        let provider_id = action_rep
            .provider_id
            .as_deref()
            .context("Provider ID required for registration")?;
        let name = action_rep.name.as_deref().unwrap_or(provider_id);

        let body = RegisterActionBody { provider_id, name };
        self.post(&url, &body).await
    }

    pub async fn delete_required_action(&self, alias: &str) -> Result<()> {
        self.delete_resource::<RequiredActionProviderRepresentation>(alias)
            .await
    }

    pub async fn get_components(&self) -> Result<Vec<ComponentRepresentation>> {
        self.get_resources().await
    }

    pub async fn create_component(&self, component_rep: &ComponentRepresentation) -> Result<()> {
        self.create_resource(component_rep).await
    }

    pub async fn update_component(
        &self,
        id: &str,
        component_rep: &ComponentRepresentation,
    ) -> Result<()> {
        self.update_resource(id, component_rep).await
    }

    pub async fn delete_component(&self, id: &str) -> Result<()> {
        self.delete_resource::<ComponentRepresentation>(id).await
    }

    async fn get<T: for<'a> Deserialize<'a>>(&self, url: &str) -> Result<T> {
        let token = self.get_token()?;
        debug!("GET {}", redact_url(url));
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .with_context(|| format!("Failed to send GET request to {}", redact_url(url)))?;

        let response = Self::check_response(response, "GET request failed").await?;

        response.json().await.context("Failed to parse response")
    }

    async fn post<T: Serialize>(&self, url: &str, body: &T) -> Result<()> {
        let token = self.get_token()?;
        debug!("POST {}", redact_url(url));
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("Failed to send POST request to {}", redact_url(url)))?;

        Self::check_response(response, "POST request failed").await?;
        Ok(())
    }

    async fn put<T: Serialize>(&self, url: &str, body: &T) -> Result<()> {
        let token = self.get_token()?;
        debug!("PUT {}", redact_url(url));
        let response = self
            .client
            .put(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("Failed to send PUT request to {}", redact_url(url)))?;

        Self::check_response(response, "PUT request failed").await?;
        Ok(())
    }

    async fn delete(&self, url: &str) -> Result<()> {
        let token = self.get_token()?;
        debug!("DELETE {}", redact_url(url));
        let response = self
            .client
            .delete(url)
            .bearer_auth(token)
            .send()
            .await
            .with_context(|| format!("Failed to send DELETE request to {}", redact_url(url)))?;

        Self::check_response(response, "DELETE request failed").await?;
        Ok(())
    }

    pub async fn login(
        &mut self,
        client_id: &str,
        client_secret: Option<&str>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<()> {
        // We auth against the master realm usually for admin tasks, or the specific realm if using client credentials for a client in that realm.
        // Assuming admin-cli in master realm for now as default.
        let auth_realm = "master";
        let url = format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.base_url, auth_realm
        );

        let mut params = Vec::new();
        params.push(("client_id", client_id));

        if let (Some(u), Some(p)) = (username, password) {
            params.push(("username", u));
            params.push(("password", p));
            params.push(("grant_type", "password"));
        } else if let Some(s) = client_secret {
            params.push(("client_secret", s));
            params.push(("grant_type", "client_credentials"));
        } else {
            anyhow::bail!("Either username/password or client_secret must be provided");
        }

        debug!("Logging in to {}", redact_url(&url));

        let response = self
            .client
            .post(&url)
            .form(&params)
            .send()
            .await
            .context("Failed to send login request")?;

        let response = Self::check_response(response, "Login failed").await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse token response")?;
        self.token = Some(token_response.access_token);

        info!("Successfully logged in to Keycloak");
        Ok(())
    }

    pub fn get_token(&self) -> Result<&str> {
        self.token.as_deref().context("Not authenticated")
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    async fn check_response(response: Response, context_msg: &str) -> Result<Response> {
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            #[derive(Deserialize)]
            struct KeycloakErrorBody {
                error: Option<String>,
                #[serde(rename = "error_description")]
                error_description: Option<String>,
                #[serde(rename = "errorMessage")]
                error_message: Option<String>,
            }

            if let Ok(err_body) = serde_json::from_str::<KeycloakErrorBody>(&text) {
                let detail = err_body
                    .error_description
                    .or(err_body.error_message)
                    .unwrap_or_else(|| err_body.error.unwrap_or_default());
                if !detail.is_empty() {
                    anyhow::bail!("{}: {} - {}", context_msg, status, detail);
                }
            }
            anyhow::bail!("{}: {} - {}", context_msg, status, text);
        }
        Ok(response)
    }
}

fn redact_url(url_str: &str) -> String {
    match reqwest::Url::parse(url_str) {
        Ok(mut url) => {
            if !url.username().is_empty() || url.password().is_some() {
                let _ = url.set_username("");
                let _ = url.set_password(None);
            }
            url.to_string()
        }
        Err(_) => {
            if let Some(pos) = url_str.rfind('@') {
                format!("<redacted>@{}", &url_str[pos + 1..])
            } else {
                url_str.to_string()
            }
        }
    }
}

impl KeycloakClient {
    pub async fn get_cached_resources<T>(&self) -> Result<Vec<T>>
    where
        T: KeycloakResource
            + KeycloakResourceMapping
            + for<'a> Deserialize<'a>
            + Send
            + Sync
            + Clone
            + 'static,
    {
        let type_id = TypeId::of::<T>();
        if let Ok(cache) = self.resource_cache.read() {
            if let Some(cached) = cache.get(&type_id) {
                if let Some(resources) = cached.downcast_ref::<Vec<T>>() {
                    return Ok(resources.clone());
                }
            }
        }

        let resources = T::fetch_all(self).await?;

        if let Ok(mut cache) = self.resource_cache.write() {
            cache.insert(type_id, Box::new(resources.clone()));
        }

        Ok(resources)
    }

    pub fn invalidate_resource_cache<T: 'static>(&self) {
        if let Ok(mut cache) = self.resource_cache.write() {
            cache.remove(&TypeId::of::<T>());
        }
    }

    pub async fn get_keys(&self) -> Result<crate::models::KeysMetadataRepresentation> {
        let url = self.realm_admin_url() + "/keys";
        self.get(&url).await
    }

    pub async fn get_authenticator_configs_internal(
        &self,
    ) -> Result<Vec<AuthenticatorConfigRepresentation>> {
        let flows = self.get_authentication_flows_raw().await?;
        let mut configs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for flow in &flows {
            if let Some(alias) = &flow.alias {
                if let Ok(executions) = self.get_flow_executions(alias).await {
                    for exec in executions {
                        if let Some(config_id) = exec.authenticator_config {
                            if seen.insert(config_id.clone()) {
                                if let Ok(config) =
                                    self.get_authenticator_config_raw(&config_id).await
                                {
                                    configs.push(config);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(configs)
    }

    pub async fn get_authenticator_config_map(&self) -> Result<HashMap<String, String>> {
        let configs = self
            .get_cached_resources::<AuthenticatorConfigRepresentation>()
            .await?;
        let mut map = HashMap::new();
        for config in configs {
            if let (Some(alias), Some(id)) = (config.alias, config.id) {
                map.insert(alias, id);
            }
        }
        Ok(map)
    }

    pub async fn get_authentication_flows_raw(
        &self,
    ) -> Result<Vec<AuthenticationFlowRepresentation>> {
        self.get(&self.resource_url::<AuthenticationFlowRepresentation>())
            .await
    }

    pub async fn get_flow_executions(
        &self,
        flow_alias: &str,
    ) -> Result<Vec<AuthenticationExecutionExportRepresentation>> {
        let url = format!(
            "{}/authentication/flows/{}/executions",
            self.realm_admin_url(),
            flow_alias
        );
        self.get(&url).await
    }

    pub async fn get_authenticator_config_raw(
        &self,
        id: &str,
    ) -> Result<AuthenticatorConfigRepresentation> {
        let url = format!("{}/authentication/config/{}", self.realm_admin_url(), id);
        self.get(&url).await
    }

    pub async fn update_flow_execution(
        &self,
        flow_alias: &str,
        exec: &AuthenticationExecutionExportRepresentation,
    ) -> Result<()> {
        let url = format!(
            "{}/authentication/flows/{}/executions",
            self.realm_admin_url(),
            flow_alias
        );
        self.put(&url, exec).await
    }

    pub async fn create_authenticator_config_for_execution(
        &self,
        execution_id: &str,
        config: &AuthenticatorConfigRepresentation,
    ) -> Result<AuthenticatorConfigRepresentation> {
        let url = format!(
            "{}/authentication/executions/{}/config",
            self.realm_admin_url(),
            execution_id
        );
        let token = self.get_token()?;
        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(config)
            .send()
            .await
            .with_context(|| format!("Failed to send POST request to {}", url))?;
        let response = Self::check_response(response, "POST authenticator config failed").await?;
        self.invalidate_resource_cache::<AuthenticatorConfigRepresentation>();
        response
            .json()
            .await
            .context("Failed to parse created authenticator config response")
    }

    pub async fn map_flow_executions(
        &self,
        mut flow: AuthenticationFlowRepresentation,
    ) -> AuthenticationFlowRepresentation {
        if let Ok(config_map) = self.get_authenticator_config_map().await {
            let id_map: HashMap<String, String> =
                config_map.into_iter().map(|(k, v)| (v, k)).collect();
            if let Some(ref mut executions) = flow.authentication_executions {
                for exec in executions {
                    if let Some(ref config_id) = exec.authenticator_config {
                        if let Some(alias) = id_map.get(config_id) {
                            exec.authenticator_config = Some(alias.clone());
                        }
                    }
                }
            }
        }
        flow
    }

    pub async fn unmap_flow_executions(
        &self,
        mut flow: AuthenticationFlowRepresentation,
    ) -> AuthenticationFlowRepresentation {
        if let Ok(config_map) = self.get_authenticator_config_map().await {
            if let Some(ref mut executions) = flow.authentication_executions {
                for exec in executions {
                    if let Some(ref alias) = exec.authenticator_config {
                        if let Some(config_id) = config_map.get(alias) {
                            exec.authenticator_config = Some(config_id.clone());
                        } else {
                            if !is_uuid(alias) {
                                exec.authenticator_config = None;
                            }
                        }
                    }
                }
            }
        }
        flow
    }
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Trait for defining specialized resource-mapping behaviors for generic client operations.
#[async_trait]
pub trait KeycloakResourceMapping: Sized {
    /// Fetches all remote resources of this type.
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>>
    where
        Self: for<'a> Deserialize<'a> + KeycloakResource,
    {
        client.get(&client.resource_url::<Self>()).await
    }

    /// Pre-processes the resource before saving (creating or updating) it.
    async fn pre_save(self, _client: &KeycloakClient) -> Result<Self> {
        Ok(self)
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for RealmRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for RoleRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for ClientRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for ClientScopeRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for UserRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for GroupRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for IdentityProviderRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for RequiredActionProviderRepresentation {}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for ComponentRepresentation {}

#[async_trait]
impl KeycloakResourceMapping for AuthenticatorConfigRepresentation {
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        client.get_authenticator_configs_internal().await
    }
}

#[async_trait]
impl KeycloakResourceMapping for AuthenticationFlowRepresentation {
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        let flows: Vec<AuthenticationFlowRepresentation> =
            client.get(&client.resource_url::<Self>()).await?;
        let futures = flows.into_iter().map(|mut flow| async move {
            if let Some(alias) = &flow.alias {
                if let Ok(executions) = client.get_flow_executions(alias).await {
                    flow.authentication_executions = Some(executions);
                }
            }
            client.map_flow_executions(flow).await
        });
        Ok(futures::future::join_all(futures).await)
    }

    async fn pre_save(self, client: &KeycloakClient) -> Result<Self> {
        Ok(client.unmap_flow_executions(self).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_target_realm() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        assert_eq!(client.target_realm, "");

        client.set_target_realm("new_realm".to_string());
        assert_eq!(client.target_realm, "new_realm");
    }

    #[test]
    fn test_get_token_missing() {
        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());

        // Initially, there's no token
        let result = client.get_token();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Not authenticated");
    }

    #[test]
    fn test_get_token_present() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());

        // Set token
        client.set_token("mock_token".to_string());

        // After setting token, we can get it
        let result = client.get_token();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "mock_token");
    }

    #[test]
    fn test_redact_url() {
        assert_eq!(
            redact_url("http://localhost:8080"),
            "http://localhost:8080/"
        );
        assert_eq!(
            redact_url("http://user:pass@localhost:8080/path"),
            "http://localhost:8080/path"
        );
        assert_eq!(
            redact_url("http://user@localhost:8080/path"),
            "http://localhost:8080/path"
        );
        assert_eq!(redact_url("invalid-url"), "invalid-url");
        assert_eq!(
            redact_url("https://user:password@example.com:99999"),
            "<redacted>@example.com:99999"
        );
    }

    #[tokio::test]
    async fn test_post_send_failure() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        client.token = Some("mock_token".to_string());
        let result = client.post("http://127.0.0.1:1", &"body").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to send POST request")
        );
    }

    #[tokio::test]
    async fn test_delete_send_failure() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        client.token = Some("mock_token".to_string());
        let result = client.delete("http://127.0.0.1:1").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to send DELETE request")
        );
    }

    #[tokio::test]
    async fn test_get_send_failure() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        client.token = Some("mock_token".to_string());
        let result = client.get::<serde_json::Value>("http://127.0.0.1:1").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to send GET request")
        );
    }

    #[tokio::test]
    async fn test_put_send_failure() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        client.token = Some("mock_token".to_string());
        let result = client.put("http://127.0.0.1:1", &"body").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to send PUT request")
        );
    }

    #[tokio::test]
    async fn test_check_response_structured_error() {
        use mockito::Server;
        use serde_json::json;

        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/test-err")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "error": "invalid_request",
                    "error_description": "Custom error detail message"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut client = KeycloakClient::new(server.url());
        client.token = Some("mock_token".to_string());
        let result = client
            .get::<serde_json::Value>(&format!("{}/test-err", server.url()))
            .await;

        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("Custom error detail message"),
            "Error message was: {}",
            err_str
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_check_response_error_message() {
        use mockito::Server;
        use serde_json::json;

        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/test-err2")
            .with_status(409)
            .with_header("content-type", "application/json")
            .with_body(
                json!({
                    "errorMessage": "User already exists"
                })
                .to_string(),
            )
            .create_async()
            .await;

        let mut client = KeycloakClient::new(server.url());
        client.token = Some("mock_token".to_string());
        let result = client
            .get::<serde_json::Value>(&format!("{}/test-err2", server.url()))
            .await;

        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("User already exists"),
            "Error message was: {}",
            err_str
        );
        mock.assert_async().await;
    }
}
