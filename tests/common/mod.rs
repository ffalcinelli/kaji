#![allow(dead_code)]
use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
use serde::Deserialize;
use tokio::net::TcpListener;

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct TokenRequest {
    pub grant_type: String,
    pub client_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_secret: Option<String>,
}

pub async fn start_mock_server() -> String {
    let app = Router::new()
        .route(
            "/realms/master/protocol/openid-connect/token",
            post(token_handler),
        )
        .route("/admin/realms", axum::routing::get(get_realms_handler))
        .route(
            "/admin/realms/{realm}",
            axum::routing::get(get_realm_handler).put(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/clients",
            axum::routing::get(get_clients_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/roles",
            axum::routing::get(get_roles_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances",
            axum::routing::get(get_idps_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/identity-provider/instances/{alias}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/clients/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/roles-by-id/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/client-scopes",
            axum::routing::get(get_client_scopes_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/groups",
            axum::routing::get(get_groups_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/users",
            axum::routing::get(get_users_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows",
            axum::routing::get(get_flows_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/required-actions",
            axum::routing::get(get_required_actions_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/register-required-action",
            axum::routing::post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/components",
            axum::routing::get(get_components_handler).post(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/client-scopes/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/groups/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/users/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/required-actions/{alias}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/components/{id}",
            axum::routing::put(generic_handler).delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/keys",
            axum::routing::get(get_keys_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/flows/{flow}/executions",
            axum::routing::get(get_flow_executions_handler).put(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/config/{id}",
            axum::routing::get(get_config_handler)
                .put(generic_handler)
                .delete(generic_handler),
        )
        .route(
            "/admin/realms/{realm}/authentication/executions/{execution}/config",
            axum::routing::post(create_config_handler),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://127.0.0.1:{}", port)
}

async fn token_handler(axum::Form(payload): axum::Form<TokenRequest>) -> impl IntoResponse {
    let is_password_grant = payload.grant_type == "password"
        && payload.username.as_deref() == Some("admin")
        && payload.password.as_deref() == Some("admin");

    let is_client_credentials_grant = payload.grant_type == "client_credentials"
        && payload.client_id == "admin-cli"
        && payload.client_secret.as_deref() == Some("secret");

    if payload.username.as_deref() == Some("bad_json") {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "not_an_access_token": "something"
            })),
        );
    }

    if is_password_grant || is_client_credentials_grant {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "access_token": "mock_token",
                "expires_in": 300
            })),
        )
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Invalid user credentials"
            })),
        )
    }
}

async fn get_realm_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    let display_name = if realm == "test-realm" {
        "Test Realm".to_string()
    } else {
        format!("{} Realm", realm)
    };

    if realm == "test-realm" || realm == "master" {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "realm": realm,
                "enabled": true,
                "displayName": display_name
            })),
        )
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({})))
    }
}

async fn get_clients_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                {
                    "id": "1",
                    "clientId": "client-1",
                    "name": "Client 1",
                    "enabled": true,
                    "secret": "client-1-secret"
                },
                {
                    "id": "2",
                    "clientId": "client-2",
                    "enabled": false
                }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_client_scopes_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "id": "scope1", "name": "scope-1", "protocol": "openid-connect" }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_idps_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                {
                    "internalId": "idp-1",
                    "alias": "google",
                    "providerId": "google",
                    "enabled": true,
                    "config": {
                        "clientSecret": "idp-secret"
                    }
                }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_groups_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "id": "g1", "name": "group-1", "path": "/group-1" }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_users_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "id": "u1", "username": "user-1", "enabled": true }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_flows_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "id": "f1", "alias": "flow-1", "providerId": "basic-flow" }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_required_actions_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "alias": "action-1", "name": "Action 1", "providerId": "action-provider", "enabled": true }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_components_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                { "id": "c1", "name": "component-1", "providerId": "ldap", "providerType": "org.keycloak.storage.UserStorageProvider" }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_realms_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!([
            {
                "realm": "master",
                "enabled": true
            },
            {
                "realm": "test-realm",
                "enabled": true
            }
        ])),
    )
}

async fn get_keys_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "keys": [
                    {
                        "kid": "key-1",
                        "type": "RSA",
                        "algorithm": "RS256",
                        "publicKey": "pubkey",
                        "status": "ACTIVE",
                        "providerId": "rsa-1",
                        "validTo": now + 100000 // 100 seconds from now, will trigger warning
                    }
                ]
            })),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!({"keys": []})))
    }
}

async fn generic_handler() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn get_roles_handler(
    axum::extract::Path(realm): axum::extract::Path<String>,
) -> impl IntoResponse {
    if realm == "error-realm" {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Internal Server Error"})),
        );
    }
    if realm == "test-realm" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                {
                    "id": "r1",
                    "name": "role-1",
                    "description": "Role 1"
                },
                {
                    "id": "r2",
                    "name": "role-2"
                }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_flow_executions_handler(
    axum::extract::Path((realm, flow)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    if realm == "test-realm" && flow == "flow-1" {
        (
            StatusCode::OK,
            Json(serde_json::json!([
                {
                    "id": "exec-1",
                    "authenticator": "review-profile",
                    "authenticatorConfig": "config-1",
                    "requirement": "REQUIRED",
                    "priority": 1
                }
            ])),
        )
    } else {
        (StatusCode::OK, Json(serde_json::json!([])))
    }
}

async fn get_config_handler(
    axum::extract::Path((realm, id)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    if realm == "test-realm" && id == "config-1" {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": "config-1",
                "alias": "review profile config",
                "config": {
                    "loa-condition-level": "4"
                }
            })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not found"})),
        )
    }
}

async fn create_config_handler(
    axum::extract::Path((realm, exec_id)): axum::extract::Path<(String, String)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    if realm == "test-realm" && exec_id == "exec-1" {
        let mut response_body = body;
        if let Some(obj) = response_body.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!("config-1"));
        }
        (StatusCode::CREATED, Json(response_body))
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "bad request"})),
        )
    }
}
