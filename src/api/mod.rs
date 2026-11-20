mod attention;
mod auth;
mod devices;
mod error;
mod flows;
mod groups;
mod health;
mod metrics;
mod owner_view;
mod parse_request;
mod presets;
mod state;
#[cfg(test)]
mod test_support;
mod zones;

use crate::api::auth::Access;
use crate::domain::Permission;
use axum::Router;
use axum::middleware::from_fn_with_state;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    let public = Router::new().merge(health::router()).merge(metrics::router());

    let protected = Router::new()
        .merge(with_permission(devices::router(), Permission::ReadDevices))
        .merge(with_access(flows::router(), Permission::ReadFlows, Permission::WriteFlows))
        .merge(with_access(groups::router(), Permission::ReadLayout, Permission::WriteLayout))
        .merge(with_access(zones::router(), Permission::ReadLayout, Permission::WriteLayout))
        .merge(with_access(presets::router(), Permission::ReadPresets, Permission::WritePresets))
        .merge(with_permission(attention::router(), Permission::ReadAttention))
        .route_layer(from_fn_with_state(state.clone(), auth::authenticate));

    public.merge(protected).with_state(state)
}

fn with_access(router: Router<ApiState>, read: Permission, write: Permission) -> Router<ApiState> {
    router.route_layer(from_fn_with_state(Access { write, read }, auth::require_access))
}

fn with_permission(router: Router<ApiState>, permission: Permission) -> Router<ApiState> {
    router.route_layer(from_fn_with_state(permission, auth::require_permission))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{create_state_with_database, insert_user_with_token};
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use rstest::rstest;
    use tower::ServiceExt;

    async fn get(uri: &str, permissions: Option<&[Permission]>) -> StatusCode {
        let (fixture, database) = create_state_with_database(FlowRegistry::new(vec![]));
        let mut request = Request::builder().uri(uri);
        if let Some(permissions) = permissions {
            request = request.header(header::AUTHORIZATION, insert_user_with_token(&database, permissions).await);
        }
        router(fixture.state).oneshot(request.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    #[rstest]
    #[case::health("/health")]
    #[case::metrics("/metrics")]
    #[tokio::test]
    async fn public_routes_need_no_token(#[case] uri: &str) {
        assert_eq!(get(uri, None).await, StatusCode::OK);
    }

    #[rstest]
    #[case::devices("/api/devices")]
    #[case::flows("/api/flows")]
    #[case::groups("/api/groups")]
    #[case::zones("/api/zones")]
    #[case::presets("/api/presets")]
    #[case::attention("/api/attention")]
    #[tokio::test]
    async fn protected_routes_need_a_token(#[case] uri: &str) {
        assert_eq!(get(uri, None).await, StatusCode::UNAUTHORIZED);
    }

    #[rstest]
    #[case::devices("/api/devices", Permission::ReadDevices)]
    #[case::flows("/api/flows", Permission::ReadFlows)]
    #[case::groups("/api/groups", Permission::ReadLayout)]
    #[case::zones("/api/zones", Permission::ReadLayout)]
    #[case::presets("/api/presets", Permission::ReadPresets)]
    #[case::attention("/api/attention", Permission::ReadAttention)]
    #[tokio::test]
    async fn protected_routes_need_their_read_permission(#[case] uri: &str, #[case] permission: Permission) {
        assert_eq!(get(uri, Some(&[permission])).await, StatusCode::OK);
        assert_eq!(get(uri, Some(&[])).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_unknown_route_is_not_found() {
        assert_eq!(get("/api/nothing", None).await, StatusCode::NOT_FOUND);
    }
}
