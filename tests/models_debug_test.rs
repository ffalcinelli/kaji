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
