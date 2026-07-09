use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub trait ToOptionString {
    fn to_option_string(&self) -> Option<&str>;
}

impl ToOptionString for String {
    fn to_option_string(&self) -> Option<&str> {
        Some(self.as_str())
    }
}

impl ToOptionString for Option<String> {
    fn to_option_string(&self) -> Option<&str> {
        self.as_deref()
    }
}
pub trait FromOptionString {
    fn set_from_option_string(&mut self, val: Option<String>);
}

impl FromOptionString for String {
    fn set_from_option_string(&mut self, val: Option<String>) {
        if let Some(v) = val {
            *self = v;
        }
    }
}

impl FromOptionString for Option<String> {
    fn set_from_option_string(&mut self, val: Option<String>) {
        if val.is_some() {
            *self = val;
        }
    }
}
pub trait KeycloakResource {
    const API_PATH: &'static str;
    const DIR_NAME: &'static str = Self::API_PATH;
    fn get_id(&self) -> Option<&str>;
    fn set_id(&mut self, id: Option<String>);
    fn get_identity(&self) -> Option<String>;
    fn get_name(&self) -> String;
    fn object_path(id: &str) -> String {
        format!("{}/{}", Self::API_PATH, id)
    }
    fn get_filename(&self) -> String {
        self.get_name()
    }
    fn has_id(&self) -> bool {
        false
    }
    fn clear_metadata(&mut self) {}
}

pub trait ResourceMeta {
    const LABEL: &'static str;
    const SECRET_PREFIX: &'static str;
}

macro_rules! impl_keycloak_resource {
    (
        $type:ty,
        api_path = $api_path:expr,
        $(dir_name = $dir_name:expr,)?
        $(id_field = $id_field:ident,)?
        identity = |$id_self:ident| $id_expr:expr,
        name = |$name_self:ident| $name_expr:expr
        $(, has_id = |$has_id_self:ident| $has_id_expr:expr)?
        $(, clear_metadata = |$clear_self:ident| $clear_expr:block)?
        $(, get_filename = |$filename_self:ident| $filename_expr:expr)?
        $(, object_path = |$obj_id:ident| $obj_path_expr:expr)?
    ) => {
        impl KeycloakResource for $type {
            const API_PATH: &'static str = $api_path;
            $(const DIR_NAME: &'static str = $dir_name;)?

            fn get_id(&self) -> Option<&str> {
                $( return self.$id_field.to_option_string(); )?
                #[allow(unreachable_code)]
                None
            }

            fn set_id(&mut self, _id: Option<String>) {
                $( self.$id_field.set_from_option_string(_id); )?
            }

            fn get_identity(&$id_self) -> Option<String> { ($id_expr).map(|s| s.to_string()) }
            fn get_name(&$name_self) -> String { ($name_expr).to_string() }

            $(fn has_id(&$has_id_self) -> bool { $has_id_expr })?
            $(fn clear_metadata(&mut $clear_self) $clear_expr)?
            $(fn get_filename(&$filename_self) -> String { $filename_expr })?
            $(fn object_path($obj_id: &str) -> String { $obj_path_expr })?
        }
    };
}

macro_rules! impl_resource_meta {
    ($type:ty, label = $label:expr, secret_prefix = $secret_prefix:expr) => {
        impl ResourceMeta for $type {
            const LABEL: &'static str = $label;
            const SECRET_PREFIX: &'static str = $secret_prefix;
        }
    };
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RealmRepresentation {
    pub realm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    RealmRepresentation,
    api_path = "realms",
    id_field = realm,
    identity = |self| Some(self.realm.as_str()),
    name = |self| self.realm.as_str()
);

#[derive(Serialize, Deserialize, Clone)]
pub struct IdentityProviderRepresentation {
    #[serde(rename = "internalId", skip_serializing_if = "Option::is_none")]
    pub internal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(rename = "providerId", skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        rename = "updateProfileFirstLoginMode",
        skip_serializing_if = "Option::is_none"
    )]
    pub update_profile_first_login_mode: Option<String>,
    #[serde(rename = "trustEmail", skip_serializing_if = "Option::is_none")]
    pub trust_email: Option<bool>,
    #[serde(rename = "storeToken", skip_serializing_if = "Option::is_none")]
    pub store_token: Option<bool>,
    #[serde(
        rename = "addReadTokenRoleOnCreate",
        skip_serializing_if = "Option::is_none"
    )]
    pub add_read_token_role_on_create: Option<bool>,
    #[serde(
        rename = "authenticateByDefault",
        skip_serializing_if = "Option::is_none"
    )]
    pub authenticate_by_default: Option<bool>,
    #[serde(rename = "linkOnly", skip_serializing_if = "Option::is_none")]
    pub link_only: Option<bool>,
    #[serde(
        rename = "firstBrokerLoginFlowAlias",
        skip_serializing_if = "Option::is_none"
    )]
    pub first_broker_login_flow_alias: Option<String>,
    #[serde(
        rename = "postBrokerLoginFlowAlias",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_broker_login_flow_alias: Option<String>,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, String>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl std::fmt::Debug for IdentityProviderRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityProviderRepresentation")
            .field("internal_id", &self.internal_id)
            .field("alias", &self.alias)
            .field("provider_id", &self.provider_id)
            .field("enabled", &self.enabled)
            .field(
                "update_profile_first_login_mode",
                &self.update_profile_first_login_mode,
            )
            .field("trust_email", &self.trust_email)
            .field("store_token", &self.store_token)
            .field(
                "add_read_token_role_on_create",
                &self.add_read_token_role_on_create,
            )
            .field("authenticate_by_default", &self.authenticate_by_default)
            .field("link_only", &self.link_only)
            .field(
                "first_broker_login_flow_alias",
                &self.first_broker_login_flow_alias,
            )
            .field(
                "post_broker_login_flow_alias",
                &self.post_broker_login_flow_alias,
            )
            .field("display_name", &self.display_name)
            .field("config", &self.config.as_ref().map(|_| "********"))
            .field("extra", &self.extra)
            .finish()
    }
}

impl_keycloak_resource!(
    IdentityProviderRepresentation,
    api_path = "identity-provider/instances",
    dir_name = "identity-providers",
    id_field = internal_id,
    identity = |self| self.alias.as_deref().or(self.internal_id.as_deref()),
    name = |self| self.alias.as_deref().unwrap_or("unknown"),
    has_id = |self| self.internal_id.is_some(),
    clear_metadata = |self| {
        self.internal_id = None;
    }
);

impl_resource_meta!(
    IdentityProviderRepresentation,
    label = "identity providers",
    secret_prefix = "idp"
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "clientId", skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(rename = "redirectUris", skip_serializing_if = "Option::is_none")]
    pub redirect_uris: Option<Vec<String>>,
    #[serde(rename = "webOrigins", skip_serializing_if = "Option::is_none")]
    pub web_origins: Option<Vec<String>>,
    #[serde(rename = "publicClient", skip_serializing_if = "Option::is_none")]
    pub public_client: Option<bool>,
    #[serde(rename = "bearerOnly", skip_serializing_if = "Option::is_none")]
    pub bearer_only: Option<bool>,
    #[serde(
        rename = "serviceAccountsEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub service_accounts_enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    ClientRepresentation,
    api_path = "clients",
    id_field = id,
    identity = |self| self.client_id.as_deref().or(self.id.as_deref()),
    name = |self| self
        .client_id
        .as_deref()
        .or(self.name.as_deref())
        .unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    }
);

impl_resource_meta!(
    ClientRepresentation,
    label = "clients",
    secret_prefix = "client"
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoleRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "containerId", skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    #[serde(default)]
    pub composite: bool,
    #[serde(rename = "clientRole", default)]
    pub client_role: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    RoleRepresentation,
    api_path = "roles",
    id_field = id,
    identity = |self| Some(self.name.as_str()).or(self.id.as_deref()),
    name = |self| self.name.as_str(),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
        self.container_id = None;
    },
    object_path = |id| format!("roles-by-id/{}", id)
);

impl_resource_meta!(RoleRepresentation, label = "roles", secret_prefix = "role");

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClientScopeRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<HashMap<String, String>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    ClientScopeRepresentation,
    api_path = "client-scopes",
    id_field = id,
    identity = |self| self.name.as_deref().or(self.id.as_deref()),
    name = |self| self.name.as_deref().unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    }
);

impl_resource_meta!(
    ClientScopeRepresentation,
    label = "client scopes",
    secret_prefix = "client_scope"
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroupRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "subGroups", skip_serializing_if = "Option::is_none")]
    pub sub_groups: Option<Vec<GroupRepresentation>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    GroupRepresentation,
    api_path = "groups",
    id_field = id,
    identity = |self| self
        .path
        .as_deref()
        .or(self.id.as_deref())
        .or(self.name.as_deref()),
    name = |self| self
        .name
        .as_deref()
        .or(self.path.as_deref())
        .unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    },
    get_filename = |self| format!(
        "{}-{}",
        self.get_name(),
        self.id.as_deref().unwrap_or("unknown")
    )
);

impl_resource_meta!(
    GroupRepresentation,
    label = "groups",
    secret_prefix = "group"
);

#[derive(Serialize, Deserialize, Clone)]
pub struct CredentialRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl std::fmt::Debug for CredentialRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRepresentation")
            .field("id", &self.id)
            .field("type", &self.type_)
            .field("value", &self.value.as_ref().map(|_| "********"))
            .field("temporary", &self.temporary)
            .field("extra", &self.extra)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(rename = "firstName", skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName", skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "emailVerified", skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<CredentialRepresentation>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    UserRepresentation,
    api_path = "users",
    id_field = id,
    identity = |self| self.username.as_deref().or(self.id.as_deref()),
    name = |self| self.username.as_deref().unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    }
);

impl_resource_meta!(UserRepresentation, label = "users", secret_prefix = "user");

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthenticationExecutionExportRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticator: Option<String>,
    #[serde(
        rename = "authenticatorConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub authenticator_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(rename = "authenticatorFlow", skip_serializing_if = "Option::is_none")]
    pub authenticator_flow: Option<bool>,
    #[serde(rename = "flowAlias", skip_serializing_if = "Option::is_none")]
    pub flow_alias: Option<String>,
    #[serde(rename = "userSetupAllowed", skip_serializing_if = "Option::is_none")]
    pub user_setup_allowed: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthenticationFlowRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "providerId", skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(rename = "topLevel", skip_serializing_if = "Option::is_none")]
    pub top_level: Option<bool>,
    #[serde(rename = "builtIn", skip_serializing_if = "Option::is_none")]
    pub built_in: Option<bool>,
    #[serde(
        rename = "authenticationExecutions",
        skip_serializing_if = "Option::is_none"
    )]
    pub authentication_executions: Option<Vec<AuthenticationExecutionExportRepresentation>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    AuthenticationFlowRepresentation,
    api_path = "authentication/flows",
    dir_name = "authentication-flows",
    id_field = id,
    identity = |self| self.alias.as_deref().or(self.id.as_deref()),
    name = |self| self.alias.as_deref().unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    }
);

impl_resource_meta!(
    AuthenticationFlowRepresentation,
    label = "authentication flows",
    secret_prefix = "flow"
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RequiredActionProviderRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "providerId", skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(rename = "defaultAction", skip_serializing_if = "Option::is_none")]
    pub default_action: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, String>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl_keycloak_resource!(
    RequiredActionProviderRepresentation,
    api_path = "authentication/required-actions",
    dir_name = "required-actions",
    id_field = alias,
    identity = |self| self.alias.as_deref(),
    name = |self| self.alias.as_deref().unwrap_or("unknown")
);

impl_resource_meta!(
    RequiredActionProviderRepresentation,
    label = "required actions",
    secret_prefix = "action"
);

#[derive(Serialize, Deserialize, Clone)]
pub struct ComponentRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "providerId", skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(rename = "providerType", skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(rename = "parentId", skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(rename = "subType", skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, Value>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl std::fmt::Debug for ComponentRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRepresentation")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("provider_id", &self.provider_id)
            .field("provider_type", &self.provider_type)
            .field("parent_id", &self.parent_id)
            .field("sub_type", &self.sub_type)
            .field("config", &self.config.as_ref().map(|_| "********"))
            .field("extra", &self.extra)
            .finish()
    }
}

impl_keycloak_resource!(
    ComponentRepresentation,
    api_path = "components",
    id_field = id,
    identity = |self| self.id.as_deref().or(self.name.as_deref()),
    name = |self| self.name.as_deref().unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    },
    get_filename = |self| format!(
        "{}-{}",
        self.get_name(),
        self.id.as_deref().unwrap_or("unknown")
    )
);

impl_resource_meta!(
    ComponentRepresentation,
    label = "components",
    secret_prefix = "component"
);

#[derive(Serialize, Deserialize, Clone)]
pub struct AuthenticatorConfigRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<HashMap<String, Value>>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl std::fmt::Debug for AuthenticatorConfigRepresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatorConfigRepresentation")
            .field("id", &self.id)
            .field("alias", &self.alias)
            .field("config", &self.config.as_ref().map(|_| "********"))
            .field("extra", &self.extra)
            .finish()
    }
}

impl_keycloak_resource!(
    AuthenticatorConfigRepresentation,
    api_path = "authentication/config",
    dir_name = "authenticator-configs",
    id_field = id,
    identity = |self| self.alias.as_deref(),
    name = |self| self.alias.as_deref().unwrap_or("unknown"),
    has_id = |self| self.id.is_some(),
    clear_metadata = |self| {
        self.id = None;
    },
    get_filename = |self| format!(
        "{}-{}",
        self.get_name(),
        self.id.as_deref().unwrap_or("unknown")
    )
);

impl_resource_meta!(
    AuthenticatorConfigRepresentation,
    label = "authenticator configs",
    secret_prefix = "authenticatorconfig"
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyMetadataRepresentation {
    #[serde(rename = "providerId")]
    pub provider_id: Option<String>,
    #[serde(rename = "providerPriority")]
    pub provider_priority: Option<i64>,
    pub kid: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub key_type: Option<String>,
    pub algorithm: Option<String>,
    #[serde(rename = "publicKey")]
    pub public_key: Option<String>,
    pub certificate: Option<String>,
    pub use_: Option<String>,
    #[serde(rename = "validTo")]
    pub valid_to: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeysMetadataRepresentation {
    pub active: Option<HashMap<String, String>>,
    pub keys: Option<Vec<KeyMetadataRepresentation>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn test_serialize_deserialize<T: Serialize + DeserializeOwned>(obj: &T) -> (Value, T) {
        let json_str = serde_json::to_string(obj).expect("Failed to serialize object");
        let json_val: Value = serde_json::from_str(&json_str).expect("Failed to parse json");
        let deserialized: T =
            serde_json::from_str(&json_str).expect("Failed to deserialize object");
        (json_val, deserialized)
    }

    #[test]
    fn test_realm_serialization() {
        let mut extra = HashMap::new();
        extra.insert("someExtraField".to_string(), json!("someValue"));

        let realm = RealmRepresentation {
            realm: "myrealm".to_string(),
            enabled: Some(true),
            display_name: Some("My Realm".to_string()),
            extra,
        };

        let (json_val, deserialized) = test_serialize_deserialize(&realm);

        assert_eq!(json_val["realm"], "myrealm");
        assert_eq!(json_val["displayName"], "My Realm");
        assert_eq!(json_val["someExtraField"], "someValue");

        assert_eq!(deserialized.realm, "myrealm");
        assert_eq!(deserialized.display_name, Some("My Realm".to_string()));
        assert_eq!(
            deserialized.extra.get("someExtraField"),
            Some(&json!("someValue"))
        );
    }

    #[test]
    fn test_identity_provider_serialization() {
        let idp = IdentityProviderRepresentation {
            internal_id: None,
            alias: Some("google".to_string()),
            provider_id: Some("google".to_string()),
            enabled: Some(true),
            update_profile_first_login_mode: Some("on".to_string()),
            trust_email: None,
            store_token: None,
            add_read_token_role_on_create: None,
            authenticate_by_default: None,
            link_only: None,
            first_broker_login_flow_alias: None,
            post_broker_login_flow_alias: None,
            display_name: None,
            config: None,
            extra: HashMap::new(),
        };

        let (json_val, deserialized) = test_serialize_deserialize(&idp);

        assert_eq!(json_val["providerId"], "google");
        assert_eq!(json_val["updateProfileFirstLoginMode"], "on");

        assert_eq!(
            deserialized.update_profile_first_login_mode,
            Some("on".to_string())
        );
    }

    #[test]
    fn test_client_serialization() {
        let client = ClientRepresentation {
            id: None,
            client_id: Some("my-client".to_string()),
            name: None,
            description: None,
            enabled: None,
            protocol: None,
            redirect_uris: Some(vec!["http://localhost/*".to_string()]),
            web_origins: None,
            public_client: Some(true),
            bearer_only: None,
            service_accounts_enabled: None,
            extra: HashMap::new(),
        };

        let (json_val, deserialized) = test_serialize_deserialize(&client);

        assert_eq!(json_val["clientId"], "my-client");
        assert_eq!(json_val["publicClient"], true);
        assert_eq!(json_val["redirectUris"][0], "http://localhost/*");

        assert_eq!(deserialized.client_id, Some("my-client".to_string()));
        assert_eq!(
            deserialized.redirect_uris,
            Some(vec!["http://localhost/*".to_string()])
        );
    }

    #[test]
    fn test_role_serialization() {
        let role = RoleRepresentation {
            id: None,
            name: "admin".to_string(),
            description: None,
            container_id: Some("realm-id".to_string()),
            composite: false,
            client_role: true,
            extra: HashMap::new(),
        };

        let (json_val, deserialized) = test_serialize_deserialize(&role);

        assert_eq!(json_val["containerId"], "realm-id");
        assert_eq!(json_val["clientRole"], true);

        assert_eq!(deserialized.container_id, Some("realm-id".to_string()));
    }

    #[test]
    fn test_group_serialization() {
        let sub_group = GroupRepresentation {
            id: None,
            name: Some("subgroup".to_string()),
            path: None,
            sub_groups: None,
            extra: HashMap::new(),
        };

        let group = GroupRepresentation {
            id: None,
            name: Some("group".to_string()),
            path: None,
            sub_groups: Some(vec![sub_group]),
            extra: HashMap::new(),
        };

        let (json_val, deserialized) = test_serialize_deserialize(&group);

        assert_eq!(json_val["subGroups"][0]["name"], "subgroup");

        assert_eq!(
            deserialized.sub_groups.expect("Failed to get sub_groups")[0].name,
            Some("subgroup".to_string())
        );
    }

    #[test]
    fn test_user_serialization() {
        let user = UserRepresentation {
            id: None,
            username: Some("jdoe".to_string()),
            enabled: None,
            first_name: Some("John".to_string()),
            last_name: Some("Doe".to_string()),
            email: None,
            email_verified: Some(true),
            credentials: None,
            extra: HashMap::new(),
        };

        let (json_val, deserialized) = test_serialize_deserialize(&user);

        assert_eq!(json_val["firstName"], "John");
        assert_eq!(json_val["lastName"], "Doe");
        assert_eq!(json_val["emailVerified"], true);

        assert_eq!(deserialized.first_name, Some("John".to_string()));
    }

    #[test]
    fn test_debug_implementations() {
        let mut config = HashMap::new();
        config.insert("clientSecret".to_string(), "sensitive".to_string());
        let idp = IdentityProviderRepresentation {
            internal_id: None,
            alias: Some("google".to_string()),
            provider_id: Some("google".to_string()),
            enabled: Some(true),
            update_profile_first_login_mode: None,
            trust_email: None,
            store_token: None,
            add_read_token_role_on_create: None,
            authenticate_by_default: None,
            link_only: None,
            first_broker_login_flow_alias: None,
            post_broker_login_flow_alias: None,
            display_name: None,
            config: Some(config),
            extra: HashMap::new(),
        };
        let debug_str = format!("{:?}", idp);
        assert!(debug_str.contains("********"));

        let cred = CredentialRepresentation {
            id: Some("id".to_string()),
            type_: Some("password".to_string()),
            value: Some("mypassword".to_string()),
            temporary: Some(false),
            extra: HashMap::new(),
        };
        assert!(format!("{:?}", cred).contains("********"));

        let mut comp_config = HashMap::new();
        comp_config.insert("secret".to_string(), serde_json::json!("sensitive"));
        let comp = ComponentRepresentation {
            id: Some("id".to_string()),
            name: Some("comp".to_string()),
            provider_id: Some("p".to_string()),
            provider_type: Some("t".to_string()),
            parent_id: None,
            sub_type: None,
            config: Some(comp_config),
            extra: HashMap::new(),
        };
        assert!(format!("{:?}", comp).contains("********"));
    }

    #[test]
    fn test_to_from_option_string() {
        let s = "test".to_string();
        assert_eq!(s.to_option_string(), Some("test"));

        let os: Option<String> = Some("test".to_string());
        assert_eq!(os.to_option_string(), Some("test"));

        let mut s2 = "old".to_string();
        s2.set_from_option_string(Some("new".to_string()));
        assert_eq!(s2, "new");
        s2.set_from_option_string(None);
        assert_eq!(s2, "new");

        let mut os2: Option<String> = Some("old".to_string());
        os2.set_from_option_string(Some("new".to_string()));
        assert_eq!(os2, Some("new".to_string()));
        os2.set_from_option_string(None);
        assert_eq!(os2, Some("new".to_string()));
    }

    #[test]
    fn test_object_path() {
        assert_eq!(
            ClientRepresentation::object_path("123-abc"),
            "clients/123-abc"
        );
        assert_eq!(
            RoleRepresentation::object_path("456-def"),
            "roles-by-id/456-def"
        );
    }
}
