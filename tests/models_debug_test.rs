use kaji::models::*;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_credential_debug() {
    let cred = CredentialRepresentation {
        id: Some("id1".to_string()),
        type_: Some("password".to_string()),
        value: Some("secret_password".to_string()),
        temporary: Some(false),
        extra: HashMap::new(),
    };
    let debug_str = format!("{:?}", cred);
    assert!(debug_str.contains("id: Some(\"id1\")"));
    assert!(debug_str.contains("value: Some(\"********\")"));
    assert!(!debug_str.contains("secret_password"));
}

#[test]
fn test_idp_debug() {
    let mut config = HashMap::new();
    config.insert("clientSecret".to_string(), "very_secret".to_string());
    config.insert("normalParam".to_string(), "normal_val".to_string());

    let idp = IdentityProviderRepresentation {
        internal_id: None,
        alias: Some("google".to_string()),
        provider_id: Some("google".to_string()),
        enabled: Some(true),
        config: Some(config),
        update_profile_first_login_mode: None,
        trust_email: None,
        store_token: None,
        add_read_token_role_on_create: None,
        authenticate_by_default: None,
        link_only: None,
        first_broker_login_flow_alias: None,
        post_broker_login_flow_alias: None,
        display_name: None,
        extra: HashMap::new(),
    };

    let debug_str = format!("{:?}", idp);
    assert!(debug_str.contains("alias: Some(\"google\")"));
    assert!(debug_str.contains("config: Some(\"********\")"));
    assert!(!debug_str.contains("normalParam")); // Config is completely redacted now
    assert!(!debug_str.contains("very_secret"));
}

#[test]
fn test_component_debug() {
    let mut config = HashMap::new();
    config.insert("bindCredential".to_string(), json!(["secret_val"]));
    config.insert("other".to_string(), json!(["val"]));

    let comp = ComponentRepresentation {
        id: Some("c1".to_string()),
        name: Some("ldap".to_string()),
        provider_id: Some("ldap".to_string()),
        provider_type: Some("org.keycloak.storage.UserStorageProvider".to_string()),
        parent_id: None,
        sub_type: None,
        config: Some(config),
        extra: HashMap::new(),
    };

    let debug_str = format!("{:?}", comp);
    assert!(debug_str.contains("name: Some(\"ldap\")"));
    assert!(debug_str.contains("config: Some(\"********\")"));
    assert!(!debug_str.contains("other")); // Config is completely redacted now
    assert!(!debug_str.contains("secret_val"));
}

#[test]
fn test_client_debug() {
    let mut extra = HashMap::new();
    extra.insert("some_extra".to_string(), json!("extra_val"));

    let client = ClientRepresentation {
        id: Some("id1".to_string()),
        client_id: Some("my_client".to_string()),
        secret: Some("client_super_secret".to_string()),
        name: None,
        description: None,
        enabled: None,
        protocol: None,
        redirect_uris: None,
        web_origins: None,
        public_client: None,
        bearer_only: None,
        service_accounts_enabled: None,
        extra,
    };

    let debug_str = format!("{:?}", client);
    assert!(debug_str.contains("client_id: Some(\"my_client\")"));
    assert!(debug_str.contains("secret: Some(\"********\")"));
    assert!(debug_str.contains("extra: \"********\""));
    assert!(!debug_str.contains("client_super_secret"));
    assert!(!debug_str.contains("extra_val"));
}

#[test]
fn test_authenticator_config_debug() {
    let mut config = HashMap::new();
    config.insert("mySecretField".to_string(), json!("secret_value"));

    let auth_config = AuthenticatorConfigRepresentation {
        id: Some("id1".to_string()),
        alias: Some("alias1".to_string()),
        config: Some(config),
        extra: HashMap::new(),
    };

    let debug_str = format!("{:?}", auth_config);
    assert!(debug_str.contains("alias: Some(\"alias1\")"));
    assert!(debug_str.contains("config: Some(\"********\")"));
    assert!(!debug_str.contains("mySecretField"));
    assert!(!debug_str.contains("secret_value"));
}
