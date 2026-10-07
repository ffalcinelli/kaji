#![allow(missing_docs)]
#![allow(clippy::collapsible_if)]
use crate::models::{
    AuthenticationExecutionInfoRepresentation, AuthenticationFlowRepresentation,
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
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// High-level client wrapper for the Keycloak Admin REST API.
#[derive(Clone)]
pub struct KeycloakClient {
    client: Client,
    base_url: String,
    /// The target Keycloak realm being managed.
    pub target_realm: String, // The realm we are managing
    auth_realm: String,
    http_timeout: Duration,
    allow_insecure_http: bool,
    session: Arc<Session>,
    limiter: Arc<tokio::sync::Semaphore>,
    /// Flow IDs by alias recorded while applying (standalone sub-flows are not listed by Keycloak).
    flow_ids: Arc<RwLock<HashMap<String, String>>>,
    resource_cache: Arc<RwLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>>,
}

impl KeycloakClient {
    /// Creates a new `KeycloakClient` instance with the given Keycloak server base URL.
    ///
    /// HTTPS is required unless the server is local (`localhost`, `127.0.0.1`, `::1`) or
    /// [`Self::with_allow_insecure_http`] is enabled.
    pub fn new(base_url: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let http_timeout = Duration::from_secs(10);
        let client = build_http_client(&base_url, http_timeout, false);
        Self {
            client,
            base_url,
            target_realm: String::new(),
            auth_realm: AUTH_REALM.to_string(),
            http_timeout,
            allow_insecure_http: false,
            session: Arc::new(Session::default()),
            limiter: Arc::new(tokio::sync::Semaphore::new(DEFAULT_CONCURRENCY)),
            flow_ids: Arc::new(RwLock::new(HashMap::new())),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sets the timeout for the internal HTTP client.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.http_timeout = timeout;
        self.client = build_http_client(&self.base_url, timeout, self.allow_insecure_http);
        self
    }

    /// Allows plain HTTP connections to non-local servers (e.g. `http://keycloak:8080` inside a
    /// cluster). Credentials and tokens are then sent unencrypted.
    pub fn with_allow_insecure_http(mut self, allow: bool) -> Self {
        self.allow_insecure_http = allow;
        self.client = build_http_client(&self.base_url, self.http_timeout, allow);
        self
    }

    /// Sets the realm used to obtain admin tokens (default `master`).
    pub fn with_auth_realm(mut self, auth_realm: impl Into<String>) -> Self {
        self.auth_realm = auth_realm.into();
        self
    }

    /// Limits the number of concurrent HTTP requests sent to Keycloak (minimum 1).
    pub fn with_concurrency(mut self, max_concurrent_requests: usize) -> Self {
        self.limiter = Arc::new(tokio::sync::Semaphore::new(max_concurrent_requests.max(1)));
        self
    }

    /// Records the ID of an authentication flow applied in this run.
    pub fn remember_flow_id(&self, alias: &str, id: &str) {
        if let Ok(mut ids) = self.flow_ids.write() {
            ids.insert(alias.to_string(), id.to_string());
        }
    }

    /// Returns the ID of a flow recorded with [`Self::remember_flow_id`].
    pub fn known_flow_id(&self, alias: &str) -> Option<String> {
        self.flow_ids.read().ok()?.get(alias).cloned()
    }

    pub fn set_target_realm(&mut self, target_realm: String) {
        self.target_realm = target_realm;
        // Flow IDs are realm specific: never share them with another realm's client.
        self.flow_ids = Arc::new(RwLock::new(HashMap::new()));
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
        T: KeycloakResource + KeycloakResourceMapping + Serialize + Clone + Send + Sync + 'static,
    >(
        &self,
        res: &T,
    ) -> Result<Option<String>> {
        let mapped = res.clone().pre_save(self).await?;
        let maybe_id = T::create(self, &mapped).await?;
        self.invalidate_resource_cache::<T>();
        Ok(maybe_id)
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

    /// Creates a new realm (`POST /admin/realms`).
    pub async fn create_realm(&self, realm_rep: &RealmRepresentation) -> Result<()> {
        self.create_resource(realm_rep).await?;
        Ok(())
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
        self.create_resource(client_rep).await?;
        Ok(())
    }

    pub async fn update_client(&self, id: &str, client_rep: &ClientRepresentation) -> Result<()> {
        self.update_resource(id, client_rep).await
    }

    /// Builds `{realm}/{segments...}` with every segment percent-encoded.
    fn realm_url(&self, segments: &[&str]) -> Result<String> {
        let mut url =
            reqwest::Url::parse(&self.realm_admin_url()).context("Invalid Keycloak base URL")?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid Keycloak base URL"))?
            .extend(segments);
        Ok(url.to_string())
    }

    /// Lists the direct children of a group (with role mappings and attributes).
    pub async fn get_group_children(&self, group_id: &str) -> Result<Vec<GroupRepresentation>> {
        let url = format!(
            "{}?briefRepresentation=false",
            self.realm_url(&["groups", group_id, "children"])?
        );
        self.get_paginated(&url).await
    }

    /// Recursively loads the sub-groups of a group into `sub_groups`.
    #[async_recursion::async_recursion]
    #[allow(clippy::double_must_use)]
    pub async fn load_group_children(&self, group: &mut GroupRepresentation) -> Result<()> {
        let has_children = group
            .extra
            .get("subGroupCount")
            .and_then(|c| c.as_u64())
            .is_none_or(|c| c > 0);
        let Some(id) = group.id.clone() else {
            return Ok(());
        };
        if !has_children {
            group.sub_groups = Some(Vec::new());
            return Ok(());
        }
        let mut children = self.get_group_children(&id).await?;
        for child in &mut children {
            self.load_group_children(child).await?;
        }
        group.sub_groups = Some(children);
        Ok(())
    }

    /// Creates a sub-group, returning its ID when Keycloak reports it.
    pub async fn create_child_group(
        &self,
        parent_id: &str,
        child: &GroupRepresentation,
    ) -> Result<Option<String>> {
        let url = self.realm_url(&["groups", parent_id, "children"])?;
        let id = self.post_with_location(&url, child).await?;
        self.invalidate_resource_cache::<GroupRepresentation>();
        Ok(id)
    }

    /// Lists the roles of a client (including attributes).
    pub async fn get_client_roles(&self, client_uuid: &str) -> Result<Vec<RoleRepresentation>> {
        let url = format!(
            "{}?briefRepresentation=false",
            self.realm_url(&["clients", client_uuid, "roles"])?
        );
        self.get(&url).await
    }

    /// Creates a client role.
    pub async fn create_client_role(
        &self,
        client_uuid: &str,
        role: &RoleRepresentation,
    ) -> Result<()> {
        let role = role.clone().pre_save(self).await?;
        self.post(&self.realm_url(&["clients", client_uuid, "roles"])?, &role)
            .await
    }

    /// Lists the composites of a role.
    pub async fn get_role_composites(&self, role_id: &str) -> Result<Vec<RoleRepresentation>> {
        self.get(&self.realm_url(&["roles-by-id", role_id, "composites"])?)
            .await
    }

    /// Adds or removes composites of a role.
    pub async fn change_role_composites(
        &self,
        role_id: &str,
        roles: &[RoleRepresentation],
        add: bool,
    ) -> Result<()> {
        let url = self.realm_url(&["roles-by-id", role_id, "composites"])?;
        let method = if add {
            reqwest::Method::POST
        } else {
            reqwest::Method::DELETE
        };
        self.execute(method, &url, |rb| rb.json(roles)).await?;
        Ok(())
    }

    /// Lists the paths of the groups a user belongs to.
    pub async fn get_user_group_paths(&self, user_id: &str) -> Result<Vec<String>> {
        let groups: Vec<GroupRepresentation> = self
            .get_paginated(&self.realm_url(&["users", user_id, "groups"])?)
            .await?;
        Ok(groups.into_iter().filter_map(|g| g.path).collect())
    }

    /// Adds (`member = true`) or removes a user from a group.
    pub async fn set_user_group(&self, user_id: &str, group_id: &str, member: bool) -> Result<()> {
        let url = self.realm_url(&["users", user_id, "groups", group_id])?;
        if member {
            self.put(&url, &serde_json::json!({})).await
        } else {
            self.delete(&url).await
        }
    }

    /// Finds a group by its path (e.g. `/parent/child`).
    pub async fn get_group_by_path(&self, path: &str) -> Result<GroupRepresentation> {
        let mut segments = vec!["group-by-path"];
        segments.extend(path.split('/').filter(|s| !s.is_empty()));
        self.get(&self.realm_url(&segments)?).await
    }

    /// Returns the direct role mappings of `owner` (`["users", id]` or `["groups", id]`).
    pub async fn get_role_mappings(
        &self,
        owner: &[&str],
    ) -> Result<crate::models::MappingsRepresentation> {
        let mut segments = owner.to_vec();
        segments.push("role-mappings");
        self.get(&self.realm_url(&segments)?).await
    }

    /// Adds or removes role mappings; `target` is `["realm"]` or `["clients", client_uuid]`.
    pub async fn change_role_mappings(
        &self,
        owner: &[&str],
        target: &[&str],
        roles: &[RoleRepresentation],
        add: bool,
    ) -> Result<()> {
        let mut segments = owner.to_vec();
        segments.push("role-mappings");
        segments.extend(target);
        let url = self.realm_url(&segments)?;
        let method = if add {
            reqwest::Method::POST
        } else {
            reqwest::Method::DELETE
        };
        self.execute(method, &url, |rb| rb.json(roles)).await?;
        Ok(())
    }

    /// Fetches a realm role by name.
    pub async fn get_realm_role(&self, name: &str) -> Result<RoleRepresentation> {
        self.get(&self.realm_url(&["roles", name])?).await
    }

    /// Fetches a client role by name.
    pub async fn get_client_role(
        &self,
        client_uuid: &str,
        name: &str,
    ) -> Result<RoleRepresentation> {
        self.get(&self.realm_url(&["clients", client_uuid, "roles", name])?)
            .await
    }

    /// Lists the names of the client scopes linked to a client (`kind` is
    /// `default-client-scopes` or `optional-client-scopes`).
    pub async fn get_client_scope_links(
        &self,
        client_uuid: &str,
        kind: &str,
    ) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Link {
            name: Option<String>,
        }
        let url = format!(
            "{}/clients/{}/{}",
            self.realm_admin_url(),
            client_uuid,
            kind
        );
        let links: Vec<Link> = self.get(&url).await?;
        Ok(links.into_iter().filter_map(|l| l.name).collect())
    }

    /// Links a client scope to a client.
    pub async fn link_client_scope(
        &self,
        client_uuid: &str,
        kind: &str,
        scope_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/clients/{}/{}/{}",
            self.realm_admin_url(),
            client_uuid,
            kind,
            scope_id
        );
        self.put(&url, &serde_json::json!({})).await
    }

    /// Unlinks a client scope from a client.
    pub async fn unlink_client_scope(
        &self,
        client_uuid: &str,
        kind: &str,
        scope_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/clients/{}/{}/{}",
            self.realm_admin_url(),
            client_uuid,
            kind,
            scope_id
        );
        self.delete(&url).await
    }

    pub async fn delete_client(&self, id: &str) -> Result<()> {
        self.delete_resource::<ClientRepresentation>(id).await
    }

    pub async fn create_role(&self, role_rep: &RoleRepresentation) -> Result<()> {
        self.create_resource(role_rep).await?;
        Ok(())
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
        self.create_resource(idp_rep).await?;
        Ok(())
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
        self.create_resource(scope_rep).await?;
        Ok(())
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
        self.create_resource(group_rep).await?;
        Ok(())
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
        self.create_resource(user_rep).await?;
        Ok(())
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
        self.create_resource(flow_rep).await?;
        Ok(())
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
        self.create_resource(component_rep).await?;
        Ok(())
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
        let response = self.execute(reqwest::Method::GET, url, |rb| rb).await?;
        response.json().await.context("Failed to parse response")
    }

    /// Fetches every page of a paginated list endpoint (`first`/`max` query parameters).
    async fn get_paginated<T: for<'a> Deserialize<'a>>(&self, url: &str) -> Result<Vec<T>> {
        const PAGE_SIZE: usize = 500;
        let separator = if url.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        loop {
            let page: Vec<T> = self
                .get(&format!(
                    "{}{}first={}&max={}",
                    url,
                    separator,
                    all.len(),
                    PAGE_SIZE
                ))
                .await?;
            let len = page.len();
            all.extend(page);
            if len < PAGE_SIZE {
                return Ok(all);
            }
        }
    }

    async fn post<T: Serialize>(&self, url: &str, body: &T) -> Result<()> {
        let _ = self.post_with_location(url, body).await?;
        Ok(())
    }

    async fn post_with_location<T: Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<Option<String>> {
        let response = self
            .execute(reqwest::Method::POST, url, |rb| rb.json(body))
            .await?;
        Ok(location_id(&response))
    }

    async fn put<T: Serialize>(&self, url: &str, body: &T) -> Result<()> {
        self.execute(reqwest::Method::PUT, url, |rb| rb.json(body))
            .await?;
        Ok(())
    }

    async fn delete(&self, url: &str) -> Result<()> {
        self.execute(reqwest::Method::DELETE, url, |rb| rb).await?;
        Ok(())
    }

    /// Sends an authenticated request.
    ///
    /// - Refreshes the access token shortly before it expires, and once more on HTTP 401.
    /// - Retries HTTP 429/502/503/504 with exponential backoff (honoring `Retry-After`).
    /// - Retries connection failures for GET requests only (other methods may not be idempotent).
    /// - Limits the number of in-flight requests with the client's semaphore.
    async fn execute<F>(&self, method: reqwest::Method, url: &str, build: F) -> Result<Response>
    where
        F: Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    {
        let label = method.as_str().to_string();
        let mut token = self.current_token().await?;
        let mut refreshed = false;
        let mut attempt: u32 = 0;
        loop {
            debug!("{} {}", label, redact_url(url));
            let result = {
                let _permit = self
                    .limiter
                    .acquire()
                    .await
                    .context("HTTP request limiter closed")?;
                build(self.client.request(method.clone(), url).bearer_auth(&token))
                    .send()
                    .await
            };
            match result {
                Err(e) => {
                    if method == reqwest::Method::GET && attempt < MAX_RETRIES {
                        attempt += 1;
                        tokio::time::sleep(backoff_delay(attempt)).await;
                        continue;
                    }
                    return Err(e).with_context(|| {
                        format!("Failed to send {} request to {}", label, redact_url(url))
                    });
                }
                Ok(response) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED && !refreshed {
                        refreshed = true;
                        match self.refresh_access_token(&token).await {
                            Ok(new_token) => {
                                token = new_token;
                                continue;
                            }
                            Err(e) => debug!("Token refresh after 401 failed: {:#}", e),
                        }
                    }
                    if matches!(status.as_u16(), 429 | 502 | 503 | 504) && attempt < MAX_RETRIES {
                        attempt += 1;
                        let delay =
                            retry_after(&response).unwrap_or_else(|| backoff_delay(attempt));
                        debug!(
                            "{} {} returned {}, retrying in {:?}",
                            label,
                            redact_url(url),
                            status,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Self::check_response(response, &format!("{} request failed", label))
                        .await;
                }
            }
        }
    }

    fn token_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.base_url, self.auth_realm
        )
    }

    /// Requests a token from the token endpoint with the given form parameters.
    async fn request_token(&self, params: &[(&str, &str)]) -> Result<TokenState> {
        let response = self
            .client
            .post(self.token_url())
            .form(params)
            .send()
            .await
            .context("Failed to send login request")?;
        let response = Self::check_response(response, "Login failed").await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: Option<u64>,
            refresh_token: Option<String>,
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .context("Failed to parse token response")?;
        Ok(TokenState {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: token_response
                .expires_in
                .map(|secs| Instant::now() + Duration::from_secs(secs)),
        })
    }

    async fn authenticate(&self, credentials: &Credentials) -> Result<TokenState> {
        let mut params = vec![("client_id", credentials.client_id.as_str())];
        if let (Some(u), Some(p)) = (&credentials.username, &credentials.password) {
            params.push(("username", u));
            params.push(("password", p));
            params.push(("grant_type", "password"));
        } else if let Some(s) = &credentials.client_secret {
            params.push(("client_secret", s));
            params.push(("grant_type", "client_credentials"));
        } else {
            return Err(anyhow::anyhow!(
                "Hint: Provide credentials via --user/--password flags, KEYCLOAK_USER/KEYCLOAK_PASSWORD env vars, or in your config file."
            )).context("Missing authentication credentials");
        }
        debug!("Logging in to {}", redact_url(&self.token_url()));
        self.request_token(&params).await
    }

    /// Returns a valid access token, refreshing it first when it is about to expire.
    async fn current_token(&self) -> Result<String> {
        let (token, expiring) = {
            let state = self
                .session
                .token
                .read()
                .map_err(|_| anyhow::anyhow!("Token lock poisoned"))?;
            let state = state.as_ref().context("Not authenticated")?;
            (state.access_token.clone(), state.is_expiring())
        };
        if expiring {
            self.refresh_access_token(&token).await
        } else {
            Ok(token)
        }
    }

    /// Replaces `stale` with a fresh token (refresh-token grant, falling back to a new login).
    ///
    /// Concurrent callers are serialized: if another task already refreshed, its token is reused.
    async fn refresh_access_token(&self, stale: &str) -> Result<String> {
        let _guard = self.session.refresh_lock.lock().await;
        let (refresh_token, current) = {
            let state = self
                .session
                .token
                .read()
                .map_err(|_| anyhow::anyhow!("Token lock poisoned"))?;
            match state.as_ref() {
                Some(s) if s.access_token != stale && !s.is_expiring() => {
                    return Ok(s.access_token.clone());
                }
                Some(s) => (s.refresh_token.clone(), Some(s.access_token.clone())),
                None => (None, None),
            }
        };
        let credentials = self
            .session
            .credentials
            .read()
            .map_err(|_| anyhow::anyhow!("Credentials lock poisoned"))?
            .clone();

        let mut new_state = None;
        if let (Some(refresh_token), Some(creds)) = (&refresh_token, &credentials) {
            let mut params = vec![
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", creds.client_id.as_str()),
            ];
            if let Some(secret) = &creds.client_secret {
                params.push(("client_secret", secret));
            }
            match self.request_token(&params).await {
                Ok(state) => new_state = Some(state),
                Err(e) => debug!("Refresh token grant failed, logging in again: {:#}", e),
            }
        }
        let new_state = match new_state {
            Some(state) => state,
            None => {
                let creds = credentials.with_context(|| {
                    if current.is_some() {
                        "Access token expired and no credentials are available to log in again"
                    } else {
                        "Not authenticated"
                    }
                })?;
                self.authenticate(&creds).await?
            }
        };
        let token = new_state.access_token.clone();
        *self
            .session
            .token
            .write()
            .map_err(|_| anyhow::anyhow!("Token lock poisoned"))? = Some(new_state);
        info!("Refreshed Keycloak access token");
        Ok(token)
    }

    pub async fn login(
        &mut self,
        client_id: &str,
        client_secret: Option<&str>,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<()> {
        // Admin tasks authenticate against the master realm.
        let credentials = Credentials {
            client_id: client_id.to_string(),
            client_secret: client_secret.map(String::from),
            username: username.map(String::from),
            password: password.map(String::from),
        };
        let state = self.authenticate(&credentials).await?;
        *self
            .session
            .credentials
            .write()
            .map_err(|_| anyhow::anyhow!("Credentials lock poisoned"))? = Some(credentials);
        *self
            .session
            .token
            .write()
            .map_err(|_| anyhow::anyhow!("Token lock poisoned"))? = Some(state);

        info!("Successfully logged in to Keycloak");
        Ok(())
    }

    /// Returns the current access token.
    pub fn get_token(&self) -> Result<String> {
        self.session
            .token
            .read()
            .map_err(|_| anyhow::anyhow!("Token lock poisoned"))?
            .as_ref()
            .map(|s| s.access_token.clone())
            .context("Not authenticated")
    }

    /// Sets a static access token (never refreshed proactively).
    pub fn set_token(&mut self, token: String) {
        if let Ok(mut state) = self.session.token.write() {
            *state = Some(TokenState {
                access_token: token,
                refresh_token: None,
                expires_at: None,
            });
        }
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

/// Default maximum number of concurrent HTTP requests per client (and its clones).
pub const DEFAULT_CONCURRENCY: usize = 16;
/// Default realm used to obtain admin tokens.
const AUTH_REALM: &str = "master";

/// Returns true if the URL points to the local machine.
fn is_local_url(base_url: &str) -> bool {
    reqwest::Url::parse(base_url).is_ok_and(|url| {
        matches!(
            url.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("[::1]")
        )
    })
}

/// Builds the HTTP client: HTTPS only, unless the server is local or insecure HTTP is allowed.
fn build_http_client(base_url: &str, timeout: Duration, allow_insecure_http: bool) -> Client {
    let https_only = !(allow_insecure_http || is_local_url(base_url));
    Client::builder()
        .timeout(timeout)
        .https_only(https_only)
        .build()
        .unwrap_or_else(|e| {
            log::warn!("Failed to build HTTP client ({e}), falling back to defaults");
            Client::new()
        })
}
/// Tokens are refreshed when they expire within this margin.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(15);
/// Maximum retries for transient failures.
const MAX_RETRIES: u32 = 3;

#[derive(Clone)]
struct Credentials {
    client_id: String,
    client_secret: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

struct TokenState {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<Instant>,
}

impl TokenState {
    fn is_expiring(&self) -> bool {
        self.expires_at
            .is_some_and(|at| Instant::now() + TOKEN_REFRESH_MARGIN >= at)
    }
}

/// Authentication state shared by a client and all its clones.
#[derive(Default)]
struct Session {
    token: RwLock<Option<TokenState>>,
    credentials: RwLock<Option<Credentials>>,
    refresh_lock: tokio::sync::Mutex<()>,
}

/// Extracts the trailing ID from a `Location` response header.
fn location_id(response: &Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|val| val.to_str().ok())
        .and_then(|loc| {
            loc.trim_end_matches('/')
                .split('/')
                .next_back()
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
}

fn backoff_delay(attempt: u32) -> Duration {
    Duration::from_millis(200u64.saturating_mul(1 << attempt.min(5))).min(Duration::from_secs(5))
}

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| Duration::from_secs(secs.min(10)))
}

/// Returns true if the error chain contains an HTTP 404 response from Keycloak.
pub fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().contains("404 Not Found"))
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

/// All authentication flows of a realm (top-level and sub-flows, export format) together with
/// the `GET /flows/{alias}/executions` rows of every top-level flow (all nesting levels).
#[derive(Clone, Debug, Default)]
struct RawAuthenticationFlows {
    flows: Vec<AuthenticationFlowRepresentation>,
    executions: Vec<AuthenticationExecutionInfoRepresentation>,
}

impl KeycloakClient {
    pub fn invalidate_resource_cache<T: 'static>(&self) {
        if let Ok(mut cache) = self.resource_cache.write() {
            cache.remove(&TypeId::of::<T>());
            if TypeId::of::<T>() == TypeId::of::<AuthenticationFlowRepresentation>() {
                cache.remove(&TypeId::of::<RawAuthenticationFlows>());
            }
        }
    }

    pub async fn get_keys(&self) -> Result<crate::models::KeysMetadataRepresentation> {
        let url = self.realm_admin_url() + "/keys";
        self.get(&url).await
    }

    /// Fetches every authentication flow (top-level and nested sub-flows) in export format,
    /// plus the execution rows of every top-level flow. Cached until flows are modified.
    async fn get_flow_tree(&self) -> Result<RawAuthenticationFlows> {
        use futures::stream::StreamExt;

        let type_id = TypeId::of::<RawAuthenticationFlows>();
        if let Ok(cache) = self.resource_cache.read()
            && let Some(cached) = cache.get(&type_id)
            && let Some(tree) = cached.downcast_ref::<RawAuthenticationFlows>()
        {
            return Ok(tree.clone());
        }

        let top_level = self.get_authentication_flows_raw().await?;
        let aliases: Vec<String> = top_level.iter().filter_map(|f| f.alias.clone()).collect();
        let rows: Vec<Result<Vec<AuthenticationExecutionInfoRepresentation>>> =
            futures::stream::iter(
                aliases
                    .into_iter()
                    .map(|alias| async move { self.get_flow_executions(&alias).await }),
            )
            .buffered(10)
            .collect()
            .await;

        let mut executions = Vec::new();
        for result in rows {
            executions.extend(result?);
        }

        // Sub-flows are not listed by `GET /authentication/flows`: discover them through the
        // execution rows (which span all nesting levels) and fetch each one by ID.
        let mut known: HashSet<String> = top_level.iter().filter_map(|f| f.id.clone()).collect();
        let subflow_ids: Vec<String> = executions
            .iter()
            .filter(|row| row.is_subflow())
            .filter_map(|row| row.flow_id.clone())
            .filter(|id| known.insert(id.clone()))
            .collect();
        let subflows: Vec<Result<AuthenticationFlowRepresentation>> = futures::stream::iter(
            subflow_ids
                .into_iter()
                .map(|id| async move { self.get_flow(&id).await }),
        )
        .buffered(10)
        .collect()
        .await;

        let mut flows = top_level;
        for subflow in subflows {
            flows.push(subflow?);
        }

        let tree = RawAuthenticationFlows { flows, executions };
        if let Ok(mut cache) = self.resource_cache.write() {
            cache.insert(type_id, Box::new(tree.clone()));
        }
        Ok(tree)
    }

    /// Returns every authentication flow (top-level and sub-flows) in export format.
    pub async fn get_raw_flows_with_executions(
        &self,
    ) -> Result<Vec<AuthenticationFlowRepresentation>> {
        Ok(self.get_flow_tree().await?.flows)
    }

    pub async fn get_authenticator_configs_internal(
        &self,
    ) -> Result<Vec<AuthenticatorConfigRepresentation>> {
        let tree = self.get_flow_tree().await?;
        let mut seen = HashSet::new();
        let futures: Vec<_> = tree
            .executions
            .iter()
            .filter_map(|row| row.authentication_config.clone())
            .filter(|id| seen.insert(id.clone()))
            .map(|id| async move { self.get_authenticator_config_raw(&id).await })
            .collect();
        futures::future::join_all(futures)
            .await
            .into_iter()
            .collect()
    }

    /// Lists top-level authentication flows (`GET /authentication/flows`).
    pub async fn get_authentication_flows_raw(
        &self,
    ) -> Result<Vec<AuthenticationFlowRepresentation>> {
        self.get(&self.resource_url::<AuthenticationFlowRepresentation>())
            .await
    }

    /// Fetches a single flow (top-level or sub-flow) by ID.
    pub async fn get_flow(&self, id: &str) -> Result<AuthenticationFlowRepresentation> {
        self.get_resource::<AuthenticationFlowRepresentation>(id)
            .await
    }

    /// Builds `{realm}/authentication/flows/{alias}/{suffix...}` with the alias percent-encoded.
    fn flow_url(&self, flow_alias: &str, suffix: &[&str]) -> Result<String> {
        let mut url =
            reqwest::Url::parse(&self.realm_admin_url()).context("Invalid Keycloak base URL")?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid Keycloak base URL"))?
            .extend(["authentication", "flows", flow_alias])
            .extend(suffix);
        Ok(url.to_string())
    }

    /// Lists the execution rows of a flow (all nesting levels, `level` 0 = direct children).
    pub async fn get_flow_executions(
        &self,
        flow_alias: &str,
    ) -> Result<Vec<AuthenticationExecutionInfoRepresentation>> {
        self.get(&self.flow_url(flow_alias, &["executions"])?).await
    }

    pub async fn get_authenticator_config_raw(
        &self,
        id: &str,
    ) -> Result<AuthenticatorConfigRepresentation> {
        let url = format!("{}/authentication/config/{}", self.realm_admin_url(), id);
        self.get(&url).await
    }

    /// Updates an execution row of a flow (requirement and priority).
    pub async fn update_flow_execution(
        &self,
        flow_alias: &str,
        exec: &AuthenticationExecutionInfoRepresentation,
    ) -> Result<()> {
        self.put(&self.flow_url(flow_alias, &["executions"])?, exec)
            .await?;
        self.invalidate_resource_cache::<AuthenticationFlowRepresentation>();
        Ok(())
    }

    /// Adds an execution (`POST /authentication/executions`), returning its ID.
    ///
    /// The body is an `AuthenticationExecutionRepresentation`: `parentFlow` (flow ID) plus either
    /// `authenticator` or `authenticatorFlow: true` with `flowId` (links an existing flow).
    pub async fn add_execution(&self, body: &serde_json::Value) -> Result<Option<String>> {
        let url = format!("{}/authentication/executions", self.realm_admin_url());
        let id = self.post_with_location(&url, body).await?;
        self.invalidate_resource_cache::<AuthenticationFlowRepresentation>();
        Ok(id)
    }

    /// Creates a new sub-flow inside `parent_alias` (`POST /flows/{parent}/executions/flow`).
    pub async fn add_subflow_execution(
        &self,
        parent_alias: &str,
        subflow_alias: &str,
        flow_type: &str,
        description: Option<&str>,
    ) -> Result<()> {
        let mut body = serde_json::json!({
            "alias": subflow_alias,
            "type": flow_type,
            "description": description.unwrap_or_default(),
        });
        if flow_type == "form-flow" {
            body["provider"] = serde_json::json!("registration-page-form");
        }
        self.post(
            &self.flow_url(parent_alias, &["executions", "flow"])?,
            &body,
        )
        .await?;
        self.invalidate_resource_cache::<AuthenticationFlowRepresentation>();
        Ok(())
    }

    /// Removes an execution (`DELETE /authentication/executions/{id}`).
    pub async fn delete_execution(&self, execution_id: &str) -> Result<()> {
        let url = format!(
            "{}/authentication/executions/{}",
            self.realm_admin_url(),
            execution_id
        );
        self.delete(&url).await?;
        self.invalidate_resource_cache::<AuthenticationFlowRepresentation>();
        Ok(())
    }

    /// Creates an authenticator config for an execution and returns the new config ID.
    ///
    /// Keycloak answers `201 Created` with the ID in the `Location` header and an empty body.
    pub async fn create_authenticator_config_for_execution(
        &self,
        execution_id: &str,
        config: &AuthenticatorConfigRepresentation,
    ) -> Result<String> {
        let url = format!(
            "{}/authentication/executions/{}/config",
            self.realm_admin_url(),
            execution_id
        );
        let response = self
            .execute(reqwest::Method::POST, &url, |rb| rb.json(config))
            .await
            .context("POST authenticator config failed")?;
        self.invalidate_resource_cache::<AuthenticatorConfigRepresentation>();
        self.invalidate_resource_cache::<AuthenticationFlowRepresentation>();
        if let Some(id) = location_id(&response) {
            return Ok(id);
        }
        // Older servers return the created representation instead.
        let created: AuthenticatorConfigRepresentation = response
            .json()
            .await
            .context("Failed to parse created authenticator config response")?;
        created
            .id
            .context("Created authenticator config response has no ID")
    }
}

/// Trait for defining specialized resource-mapping behaviors for generic client operations.
#[async_trait]
#[allow(clippy::double_must_use)]
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

    /// Hook run after the resource was created or updated with the given server ID
    /// (e.g. to reconcile sub-resources that the main endpoint ignores).
    async fn post_save(&self, _client: &KeycloakClient, _id: &str) -> Result<()>
    where
        Self: Sync,
    {
        Ok(())
    }

    /// Loads relationships that list/get endpoints do not return (e.g. a user's groups and
    /// role mappings) into a remote representation. With `declared`, only relationships the
    /// local representation declares are loaded; without it (inspect), all of them.
    async fn load_relations(
        &mut self,
        _client: &KeycloakClient,
        _declared: Option<&Self>,
    ) -> Result<()>
    where
        Self: Send + Sync,
    {
        Ok(())
    }

    /// Adjusts the server representation fetched right after saving, before it is compared
    /// with the local file (e.g. to keep values that are reconciled in a later stage).
    fn prepare_enriched(&self, _enriched: &mut Self) {}

    /// True if relationships are reconciled in a later stage than the resource itself, so the
    /// enrichment check right after saving must not load them (they would still be stale).
    const DEFERRED_RELATIONS: bool = false;

    /// Creates the resource on the server, returning its generated ID if known.
    async fn create(client: &KeycloakClient, res: &Self) -> Result<Option<String>>
    where
        Self: Serialize + KeycloakResource + Sync,
    {
        client
            .post_with_location(&client.resource_url::<Self>(), res)
            .await
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for RealmRepresentation {}

#[async_trait]
impl KeycloakResourceMapping for RoleRepresentation {
    /// Realm roles including attributes.
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        let url = format!(
            "{}?briefRepresentation=false",
            client.resource_url::<Self>()
        );
        client.get(&url).await
    }

    /// Composites may reference client roles that do not exist yet: they are reconciled in a
    /// later stage (`relations::reconcile_role_composites`), never sent with the role itself.
    async fn pre_save(mut self, _client: &KeycloakClient) -> Result<Self> {
        self.extra.remove("composites");
        Ok(self)
    }

    async fn load_relations(
        &mut self,
        client: &KeycloakClient,
        declared: Option<&Self>,
    ) -> Result<()> {
        let wanted = match declared {
            Some(d) => d.extra.contains_key("composites"),
            None => self.composite,
        };
        if wanted {
            crate::apply::relations::load_role_composites(client, self).await?;
        }
        Ok(())
    }

    const DEFERRED_RELATIONS: bool = true;

    /// The `Location` of a created role ends with its name, not its ID: look the ID up.
    async fn create(client: &KeycloakClient, res: &Self) -> Result<Option<String>> {
        client.post(&client.resource_url::<Self>(), res).await?;
        Ok(client.get_realm_role(&res.name).await?.id)
    }

    /// Composites are reconciled after the enrichment check: keep the declared flag.
    fn prepare_enriched(&self, enriched: &mut Self) {
        if self.extra.contains_key("composites") {
            enriched.composite = self.composite;
        }
    }
}

#[async_trait]
impl KeycloakResourceMapping for ClientRepresentation {
    /// Client scope links are only honored on creation: reconcile them on every save.
    async fn post_save(&self, client: &KeycloakClient, id: &str) -> Result<()> {
        crate::apply::relations::reconcile_client_scopes(client, self, id).await
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for ClientScopeRepresentation {}

#[async_trait]
impl KeycloakResourceMapping for UserRepresentation {
    /// The list endpoint is paginated (Keycloak returns at most 100 users by default).
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        client.get_paginated(&client.resource_url::<Self>()).await
    }

    /// Group membership and role mappings are not part of the user representation.
    async fn load_relations(
        &mut self,
        client: &KeycloakClient,
        declared: Option<&Self>,
    ) -> Result<()> {
        crate::apply::relations::load_user_relations(client, self, declared.map(|d| &d.extra)).await
    }

    /// Groups are only honored on creation and roles never: reconcile both on every save.
    async fn post_save(&self, client: &KeycloakClient, id: &str) -> Result<()> {
        crate::apply::relations::reconcile_user(client, self, id).await
    }
}

#[async_trait]
impl KeycloakResourceMapping for GroupRepresentation {
    /// Top-level groups with their attributes and role mappings.
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        let url = format!(
            "{}?briefRepresentation=false",
            client.resource_url::<Self>()
        );
        client.get_paginated(&url).await
    }

    /// Sub-groups (Keycloak only returns a `subGroupCount`; children are listed separately).
    async fn load_relations(
        &mut self,
        client: &KeycloakClient,
        declared: Option<&Self>,
    ) -> Result<()> {
        if declared.is_none_or(|d| d.sub_groups.is_some()) {
            client.load_group_children(self).await?;
        }
        Ok(())
    }

    /// Role mappings and declared sub-groups are reconciled separately.
    async fn post_save(&self, client: &KeycloakClient, id: &str) -> Result<()> {
        crate::apply::relations::reconcile_group(client, self, id).await
    }
}

#[cfg(not(tarpaulin_include))]
#[async_trait]
impl KeycloakResourceMapping for IdentityProviderRepresentation {}

#[async_trait]
impl KeycloakResourceMapping for RequiredActionProviderRepresentation {
    /// Required actions cannot be POSTed: they are registered from their provider and then
    /// configured with a PUT.
    async fn create(client: &KeycloakClient, res: &Self) -> Result<Option<String>> {
        client.register_required_action(res).await?;
        let alias = res
            .provider_id
            .clone()
            .or_else(|| res.alias.clone())
            .context("Required action needs a 'providerId' or 'alias'")?;
        let mut registered = res.clone();
        registered.alias = Some(alias.clone());
        client
            .put(&client.object_url::<Self>(&alias), &registered)
            .await?;
        Ok(Some(alias))
    }
}

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
    /// Top-level flows and their (possibly shared) sub-flows, executions in export format.
    async fn fetch_all(client: &KeycloakClient) -> Result<Vec<Self>> {
        client.get_raw_flows_with_executions().await
    }

    /// `POST`/`PUT /authentication/flows` ignore executions: reconcile them separately.
    async fn post_save(&self, client: &KeycloakClient, id: &str) -> Result<()> {
        crate::apply::flow_executions::reconcile(client, self, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_base_url() {
        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        assert_eq!(client.get_base_url(), "http://127.0.0.1:1");
    }

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
    fn test_set_token() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());

        assert!(client.get_token().is_err());

        client.set_token("new_token_value".to_string());

        assert_eq!(client.get_token().unwrap(), "new_token_value");
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
        client.set_token("mock_token".to_string());
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
        client.set_token("mock_token".to_string());
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
        client.set_token("mock_token".to_string());
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
        client.set_token("mock_token".to_string());
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
        client.set_token("mock_token".to_string());
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
        client.set_token("mock_token".to_string());
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

    #[test]
    fn test_invalidate_resource_cache() {
        use std::any::TypeId;

        #[derive(Clone)]
        struct MockResource;

        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());

        // Insert a mock resource type into the cache manually
        {
            let mut cache = client.resource_cache.write().unwrap();
            cache.insert(TypeId::of::<MockResource>(), Box::new(vec![MockResource]));
        }

        // Verify it was inserted
        {
            let cache = client.resource_cache.read().unwrap();
            assert!(cache.contains_key(&TypeId::of::<MockResource>()));
        }

        // Call the method to invalidate
        client.invalidate_resource_cache::<MockResource>();

        // Verify it was removed
        {
            let cache = client.resource_cache.read().unwrap();
            assert!(!cache.contains_key(&TypeId::of::<MockResource>()));
        }
    }

    #[tokio::test]
    async fn test_get_users_paginates() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let page1: Vec<_> = (0..500)
            .map(|i| serde_json::json!({"id": format!("id{i}"), "username": format!("u{i}")}))
            .collect();
        let page2 = serde_json::json!([{"id": "last", "username": "last"}]);
        let m1 = server
            .mock("GET", "/admin/realms/r/users")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("first".into(), "0".into()),
                Matcher::UrlEncoded("max".into(), "500".into()),
            ]))
            .with_body(serde_json::Value::Array(page1).to_string())
            .create_async()
            .await;
        let m2 = server
            .mock("GET", "/admin/realms/r/users")
            .match_query(Matcher::UrlEncoded("first".into(), "500".into()))
            .with_body(page2.to_string())
            .create_async()
            .await;
        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client.set_token("t".to_string());
        let users = client.get_users().await.unwrap();
        assert_eq!(users.len(), 501);
        m1.assert_async().await;
        m2.assert_async().await;
    }

    fn token_body(access: &str, expires_in: u64, refresh: Option<&str>) -> String {
        let mut body = serde_json::json!({"access_token": access, "expires_in": expires_in});
        if let Some(r) = refresh {
            body["refresh_token"] = serde_json::json!(r);
        }
        body.to_string()
    }

    #[tokio::test]
    async fn test_token_refreshed_before_expiry() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let login = server
            .mock("POST", "/realms/master/protocol/openid-connect/token")
            .match_body(Matcher::UrlEncoded("grant_type".into(), "password".into()))
            .with_body(token_body("old", 5, Some("r1")))
            .create_async()
            .await;
        let refresh = server
            .mock("POST", "/realms/master/protocol/openid-connect/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("refresh_token".into(), "r1".into()),
            ]))
            .with_body(token_body("new", 300, Some("r2")))
            .expect(1)
            .create_async()
            .await;
        let api = server
            .mock("GET", "/admin/realms/r/roles")
            .match_query(Matcher::Any)
            .match_header("authorization", "Bearer new")
            .with_body("[]")
            .expect(2)
            .create_async()
            .await;

        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client
            .login("admin-cli", None, Some("u"), Some("p"))
            .await
            .unwrap();
        // Token expires within the refresh margin: it is refreshed once, then reused.
        let clone = client.clone();
        assert!(client.get_roles().await.unwrap().is_empty());
        assert!(clone.get_roles().await.unwrap().is_empty());
        assert_eq!(clone.get_token().unwrap(), "new");
        login.assert_async().await;
        refresh.assert_async().await;
        api.assert_async().await;
    }

    #[tokio::test]
    async fn test_unauthorized_triggers_relogin_and_retry() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let _login = server
            .mock("POST", "/realms/master/protocol/openid-connect/token")
            .match_body(Matcher::UrlEncoded(
                "grant_type".into(),
                "client_credentials".into(),
            ))
            .with_body(token_body("t2", 300, None))
            .create_async()
            .await;
        let rejected = server
            .mock("GET", "/admin/realms/r/roles")
            .match_query(Matcher::Any)
            .match_header("authorization", "Bearer t1")
            .with_status(401)
            .expect(1)
            .create_async()
            .await;
        let accepted = server
            .mock("GET", "/admin/realms/r/roles")
            .match_query(Matcher::Any)
            .match_header("authorization", "Bearer t2")
            .with_body("[]")
            .expect(1)
            .create_async()
            .await;

        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client
            .login("svc", Some("secret"), None, None)
            .await
            .unwrap();
        client.set_token("t1".to_string()); // simulate a token revoked by the server
        assert!(client.get_roles().await.unwrap().is_empty());
        rejected.assert_async().await;
        accepted.assert_async().await;
    }

    #[tokio::test]
    async fn test_unauthorized_without_credentials_reports_error() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/admin/realms/r/roles")
            .match_query(Matcher::Any)
            .with_status(401)
            .create_async()
            .await;
        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client.set_token("static".to_string());
        let err = client.get_roles().await.unwrap_err();
        assert!(err.to_string().contains("401"), "{err}");
    }

    #[tokio::test]
    async fn test_transient_errors_are_retried() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let flaky_calls = Arc::clone(&calls);
        let app = axum::Router::new()
            .route(
                "/admin/realms/r/roles-by-id/1",
                axum::routing::put(move || {
                    let calls = Arc::clone(&flaky_calls);
                    async move {
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            (
                                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                [(axum::http::header::RETRY_AFTER, "0")],
                            )
                                .into_response()
                        } else {
                            axum::http::StatusCode::NO_CONTENT.into_response()
                        }
                    }
                }),
            )
            .route(
                "/admin/realms/r/roles-by-id/2",
                axum::routing::delete(|| async {
                    (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        [(axum::http::header::RETRY_AFTER, "0")],
                    )
                        .into_response()
                }),
            );
        use axum::response::IntoResponse;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut client = KeycloakClient::new(url).with_concurrency(1);
        client.set_target_realm("r".to_string());
        client.set_token("t".to_string());
        let role: RoleRepresentation =
            serde_json::from_value(serde_json::json!({"name": "x"})).unwrap();
        client.update_role("1", &role).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2, "503 must be retried once");

        // Persistent 503 gives up after MAX_RETRIES and reports the failure.
        let err = client.delete_role("2").await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
    }

    #[tokio::test]
    async fn test_refresh_falls_back_to_login() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let _login = server
            .mock("POST", "/realms/master/protocol/openid-connect/token")
            .match_body(Matcher::UrlEncoded("grant_type".into(), "password".into()))
            .with_body(token_body("fresh", 1, Some("r1")))
            .create_async()
            .await;
        let _refresh = server
            .mock("POST", "/realms/master/protocol/openid-connect/token")
            .match_body(Matcher::UrlEncoded(
                "grant_type".into(),
                "refresh_token".into(),
            ))
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant"}"#)
            .create_async()
            .await;
        let api = server
            .mock("GET", "/admin/realms/r/roles")
            .match_query(Matcher::Any)
            .match_header("authorization", "Bearer fresh")
            .with_body("[]")
            .create_async()
            .await;
        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client
            .login("admin-cli", None, Some("u"), Some("p"))
            .await
            .unwrap();
        assert!(client.get_roles().await.unwrap().is_empty());
        api.assert_async().await;
    }

    #[test]
    fn test_flow_url_encodes_aliases() {
        let mut client = KeycloakClient::new("http://127.0.0.1:1".to_string());
        client.set_target_realm("my realm".to_string());
        let url = client
            .flow_url("first broker login #2", &["executions", "flow"])
            .unwrap();
        assert_eq!(
            url,
            "http://127.0.0.1:1/admin/realms/my%20realm/authentication/flows/first%20broker%20login%20%232/executions/flow"
        );
    }

    #[tokio::test]
    async fn test_add_form_subflow_sends_provider() {
        use mockito::{Matcher, Server};
        let mut server = Server::new_async().await;
        let m = server
            .mock(
                "POST",
                "/admin/realms/r/authentication/flows/parent/executions/flow",
            )
            .match_body(Matcher::PartialJson(serde_json::json!({
                "alias": "form", "type": "form-flow", "provider": "registration-page-form"
            })))
            .with_status(201)
            .create_async()
            .await;
        let mut client = KeycloakClient::new(server.url());
        client.set_target_realm("r".to_string());
        client.set_token("t".to_string());
        client
            .add_subflow_execution("parent", "form", "form-flow", Some("d"))
            .await
            .unwrap();
        m.assert_async().await;
    }

    #[test]
    fn test_is_local_url() {
        assert!(is_local_url("http://localhost:8080"));
        assert!(is_local_url("http://127.0.0.1"));
        assert!(is_local_url("http://[::1]:8080"));
        assert!(!is_local_url("http://keycloak:8080"));
        assert!(!is_local_url("not a url"));
    }

    #[tokio::test]
    async fn test_insecure_http_and_auth_realm() {
        use mockito::Server;
        let mut server = Server::new_async().await;
        let token = server
            .mock("POST", "/realms/ops/protocol/openid-connect/token")
            .with_body(r#"{"access_token":"t"}"#)
            .create_async()
            .await;
        // mockito listens on 127.0.0.1: rewrite to a non-local host name resolving to it.
        let url = server.url().replace("127.0.0.1", "localhost.localdomain");
        let mut strict = KeycloakClient::new(url.clone()).with_auth_realm("ops");
        let err = strict
            .login("svc", Some("s"), None, None)
            .await
            .unwrap_err();
        assert!(format!("{:#}", err).contains("scheme"), "{err:#}");

        let mut client = KeycloakClient::new(server.url())
            .with_auth_realm("ops")
            .with_allow_insecure_http(true);
        client.login("svc", Some("s"), None, None).await.unwrap();
        token.assert_async().await;
    }

    #[test]
    fn test_with_timeout() {
        let client = KeycloakClient::new("http://127.0.0.1:1".to_string());

        let initial_debug = format!("{:?}", client.client);

        let client = client.with_timeout(std::time::Duration::from_secs(42));

        let final_debug = format!("{:?}", client.client);

        assert!(
            !initial_debug.contains("42s"),
            "Initial state should not have the new timeout"
        );
        assert!(
            final_debug.contains("42s"),
            "Final state should reflect the updated timeout of 42s"
        );
    }
}
