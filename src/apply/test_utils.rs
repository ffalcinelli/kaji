use crate::models::*;
use anyhow::Result;
use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post, put},
};
use std::sync::Arc;
use tokio::net::TcpListener;

pub async fn start_mock_server() -> Result<(String, Arc<std::sync::atomic::AtomicUsize>)> {
    let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count_clone = Arc::clone(&call_count);

    macro_rules! mock_handler {
        ($status:expr) => {{
            let count = Arc::clone(&count_clone);
            move || {
                let c = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move {
                    if c == 0 {
                        return StatusCode::INTERNAL_SERVER_ERROR;
                    }
                    $status
                }
            }
        }};
    }

    macro_rules! mock_error_handler {
        () => {{
            let count = Arc::clone(&count_clone);
            move || {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async { StatusCode::INTERNAL_SERVER_ERROR }
            }
        }};
    }

    macro_rules! mock_get {
        ($val:expr) => {
            get(|| async { Json(vec![$val]) })
        };
    }

    let app = Router::new()
        .route(
            "/admin/realms/test/client-scopes",
            mock_get!(ClientScopeRepresentation {
                id: Some("existing-id".to_string()),
                name: Some("existing-scope".to_string()),
                description: None,
                protocol: None,
                attributes: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/client-scopes/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/client-scopes",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/groups",
            mock_get!(GroupRepresentation {
                id: Some("existing-id".to_string()),
                name: Some("Existing Group".to_string()),
                path: Some("/existing-group".to_string()),
                sub_groups: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/groups/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/groups",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/roles",
            mock_get!(RoleRepresentation {
                id: Some("existing-id".to_string()),
                name: "existing-role".to_string(),
                description: None,
                container_id: None,
                composite: false,
                client_role: false,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/roles-by-id/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/roles",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/authentication/required-actions",
            mock_get!(RequiredActionProviderRepresentation {
                alias: Some("existing-action".to_string()),
                name: Some("Existing Action".to_string()),
                provider_id: Some("existing-provider".to_string()),
                enabled: Some(true),
                default_action: Some(false),
                priority: Some(0),
                config: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/authentication/required-actions/existing-action",
            put(mock_error_handler!()),
        )
        .route(
            "/admin/realms/test/authentication/register-required-action",
            post(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/authentication/required-actions/new-action",
            put(mock_error_handler!()),
        )
        .route(
            "/admin/realms/test/authentication/flows",
            mock_get!(AuthenticationFlowRepresentation {
                id: Some("existing-id".to_string()),
                alias: Some("existing-flow".to_string()),
                description: Some("Existing Flow".to_string()),
                provider_id: None,
                top_level: Some(true),
                built_in: Some(false),
                authentication_executions: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/authentication/flows/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/authentication/flows",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/users",
            mock_get!(UserRepresentation {
                id: Some("existing-id".to_string()),
                username: Some("existing-user".to_string()),
                enabled: Some(true),
                first_name: None,
                last_name: None,
                email: None,
                email_verified: None,
                credentials: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/users/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/users",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/components",
            mock_get!(ComponentRepresentation {
                id: Some("existing-id".to_string()),
                name: Some("Existing Component".to_string()),
                provider_id: Some("existing-provider".to_string()),
                provider_type: Some("existing-type".to_string()),
                parent_id: Some("test".to_string()),
                sub_type: None,
                config: None,
                extra: Default::default(),
            }),
        )
        .route(
            "/admin/realms/test/components/existing-id",
            put(mock_handler!(StatusCode::OK)),
        )
        .route(
            "/admin/realms/test/components",
            post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/clients",
            mock_get!(ClientRepresentation {
                id: Some("existing-id".to_string()),
                client_id: Some("existing-client".to_string()),
                secret: None,
                name: Some("Existing Client".to_string()),
                description: None,
                enabled: Some(true),
                protocol: None,
                redirect_uris: None,
                web_origins: None,
                public_client: None,
                bearer_only: None,
                service_accounts_enabled: None,
                extra: Default::default(),
            })
            .post(mock_handler!(StatusCode::CREATED)),
        )
        .route(
            "/admin/realms/test/clients/existing-id",
            put(mock_handler!(StatusCode::OK)),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://{}", addr), call_count))
}
