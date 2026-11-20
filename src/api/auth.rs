use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::domain::{Permission, Token, User};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use tracing::error;

const BEARER: &str = "Bearer";

/// Reads `Authorization: Bearer <token>`, authenticates it and puts the `User` in the request extensions.
pub async fn authenticate(State(state): State<ApiState>, mut request: Request, next: Next) -> Response {
    let Some(token) = bearer_token(request.headers()) else {
        return unauthorized();
    };

    match state.user_repository.authenticate(&token).await {
        Ok(Some(user)) => {
            request.extensions_mut().insert(User::from(user));
            next.run(request).await
        }
        Ok(None) => unauthorized(),
        Err(err) => {
            error!("Failed to authenticate: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<Token> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case(BEARER).then(|| Token::parse(token.trim())).flatten()
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, BEARER)], Json(ErrorResponse::new_code_only("unauthorized")))
        .into_response()
}

#[derive(Clone)]
pub struct Access {
    pub read: Permission,
    pub write: Permission,
}

pub async fn require_access(
    State(access): State<Access>,
    Extension(user): Extension<User>,
    request: Request,
    next: Next,
) -> Response {
    let required = match *request.method() {
        Method::GET | Method::HEAD | Method::OPTIONS => access.read,
        _ => access.write,
    };

    check(&user, required, request, next).await
}

pub async fn require_permission(
    State(required): State<Permission>,
    Extension(user): Extension<User>,
    request: Request,
    next: Next,
) -> Response {
    check(&user, required, request, next).await
}

async fn check(user: &User, required: Permission, request: Request, next: Next) -> Response {
    if user.has(required) {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, Json(ErrorResponse::new("forbidden", format!("missing permission '{required}'"))))
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{body_json, create_state_with_database, insert_user_with_token};
    use crate::database::{Database, DatabaseError};
    use crate::flow_registry::FlowRegistry;
    use axum::Router;
    use axum::body::Body;
    use axum::middleware::from_fn_with_state;
    use axum::routing::{any, get};
    use http_body_util::BodyExt;
    use rstest::rstest;
    use tower::ServiceExt;

    async fn ok() -> &'static str {
        "ok"
    }

    async fn whoami(Extension(user): Extension<User>) -> String {
        user.name
    }

    /// `/resource` needs read or write by method, `/single` needs `ReadDevices` for every method
    async fn app(permissions: &[Permission]) -> (Router, Database, String) {
        let (fixture, database) = create_state_with_database(FlowRegistry::new(vec![]));
        let authorization = insert_user_with_token(&database, permissions).await;
        let state = fixture.state;

        let resource = Router::new().route("/resource", any(ok)).route_layer(from_fn_with_state(
            Access { read: Permission::ReadFlows, write: Permission::WriteFlows },
            require_access,
        ));
        let single = Router::new()
            .route("/single", any(ok))
            .route_layer(from_fn_with_state(Permission::ReadDevices, require_permission));
        let router = resource
            .merge(single)
            .route("/whoami", get(whoami))
            .route_layer(from_fn_with_state(state.clone(), authenticate))
            .with_state(state);
        (router, database, authorization)
    }

    async fn send(app: Router, method: Method, uri: &str, authorization: Option<&str>) -> Response {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(value) = authorization {
            request = request.header(header::AUTHORIZATION, value);
        }
        app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
    }

    async fn assert_unauthorized(response: Response) {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert_eq!(body_json(response).await, serde_json::json!({ "code": "unauthorized" }));
    }

    #[tokio::test]
    async fn a_valid_token_reaches_the_handler() {
        let (app, _, authorization) = app(&[Permission::ReadFlows]).await;

        let response = send(app, Method::GET, "/resource", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_scheme_is_case_insensitive() {
        let (app, _, authorization) = app(&[Permission::ReadFlows]).await;
        let lowercase = authorization.replacen("Bearer", "bearer", 1);

        let response = send(app, Method::GET, "/resource", Some(&lowercase)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_authenticated_user_is_available_to_the_handler() {
        let (app, _, authorization) = app(&[]).await;

        let response = send(app, Method::GET, "/whoami", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes, "johan");
    }

    #[tokio::test]
    async fn a_missing_header_is_unauthorized() {
        let (app, _, _) = app(&[Permission::ReadFlows]).await;

        assert_unauthorized(send(app, Method::GET, "/resource", None).await).await;
    }

    #[rstest]
    #[case::empty("")]
    #[case::scheme_only("Bearer")]
    #[case::no_scheme("hearth_abc")]
    #[case::token_without_prefix("Bearer abc")]
    #[case::unknown_token("Bearer hearth_0000000000000000000000000000000000000000000000000000000000000000")]
    #[tokio::test]
    async fn an_invalid_header_is_unauthorized(#[case] authorization: &str) {
        let (app, _, _) = app(&[Permission::ReadFlows]).await;

        assert_unauthorized(send(app, Method::GET, "/resource", Some(authorization)).await).await;
    }

    #[tokio::test]
    async fn another_scheme_is_unauthorized_even_with_a_valid_token() {
        let (app, _, authorization) = app(&[Permission::ReadFlows]).await;
        let basic = authorization.replacen("Bearer", "Basic", 1);

        assert_unauthorized(send(app, Method::GET, "/resource", Some(&basic)).await).await;
    }

    #[tokio::test]
    async fn a_token_of_a_deleted_user_is_unauthorized() {
        let (app, database, authorization) = app(&[Permission::ReadFlows]).await;
        database.call(|conn| conn.execute("DELETE FROM users", []).map_err(DatabaseError::from)).await.unwrap();

        assert_unauthorized(send(app, Method::GET, "/resource", Some(&authorization)).await).await;
    }

    #[tokio::test]
    async fn a_storage_failure_is_an_internal_server_error() {
        let (app, database, authorization) = app(&[Permission::ReadFlows]).await;
        database.call(|conn| conn.execute("DROP TABLE api_tokens", []).map_err(DatabaseError::from)).await.unwrap();

        let response = send(app, Method::GET, "/resource", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await, serde_json::json!({ "code": "storageError" }));
    }

    #[rstest]
    #[case::get(Method::GET)]
    #[case::head(Method::HEAD)]
    #[case::options(Method::OPTIONS)]
    #[tokio::test]
    async fn safe_methods_need_the_read_permission(#[case] method: Method) {
        let (app, _, authorization) = app(&[Permission::ReadFlows]).await;

        let response = send(app, method, "/resource", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[rstest]
    #[case::post(Method::POST)]
    #[case::put(Method::PUT)]
    #[case::patch(Method::PATCH)]
    #[case::delete(Method::DELETE)]
    #[tokio::test]
    async fn other_methods_need_the_write_permission(#[case] method: Method) {
        let (app, _, authorization) = app(&[Permission::WriteFlows]).await;

        let response = send(app, method, "/resource", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[rstest]
    #[case::get_without_read(Method::GET, &[Permission::WriteFlows], "readFlows")]
    #[case::head_without_read(Method::HEAD, &[Permission::WriteFlows], "readFlows")]
    #[case::post_without_write(Method::POST, &[Permission::ReadFlows], "writeFlows")]
    #[case::delete_without_write(Method::DELETE, &[Permission::ReadFlows], "writeFlows")]
    #[case::no_permissions(Method::POST, &[], "writeFlows")]
    #[tokio::test]
    async fn a_missing_permission_is_forbidden(
        #[case] method: Method,
        #[case] permissions: &[Permission],
        #[case] missing: &str,
    ) {
        let (app, _, authorization) = app(permissions).await;

        let response = send(app, method.clone(), "/resource", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        if method != Method::HEAD {
            let body = body_json(response).await;
            assert_eq!(body["code"], "forbidden");
            assert_eq!(body["message"], format!("missing permission '{missing}'"));
        }
    }

    #[rstest]
    #[case::get(Method::GET)]
    #[case::post(Method::POST)]
    #[case::delete(Method::DELETE)]
    #[tokio::test]
    async fn require_permission_applies_to_every_method(#[case] method: Method) {
        let (app, _, authorization) = app(&[Permission::ReadDevices]).await;

        let response = send(app, method, "/single", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[rstest]
    #[case::get(Method::GET)]
    #[case::post(Method::POST)]
    #[tokio::test]
    async fn require_permission_forbids_a_user_without_it(#[case] method: Method) {
        let (app, _, authorization) = app(&[Permission::ReadFlows, Permission::WriteFlows]).await;

        let response = send(app, method, "/single", Some(&authorization)).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_json(response).await["message"], "missing permission 'readDevices'");
    }
}
