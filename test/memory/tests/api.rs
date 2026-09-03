#[cfg(test)]
mod tests {
    use crate::memory_test_api::api::{ApiState, router};
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use std::sync::Arc;
    use tokio::sync::oneshot;
    use tower::ServiceExt;

    fn state() -> ApiState {
        let (sender, _receiver) = oneshot::channel();
        ApiState {
            token: Arc::from("test-token"),
            shutdown: Arc::new(std::sync::Mutex::new(Some(sender))),
            busy: Arc::new(std::sync::Mutex::new(false)),
        }
    }

    #[tokio::test]
    async fn health_requires_bearer_token() {
        let response = router(state())
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn body_limit_rejects_oversized_payload() {
        let payload = "x".repeat(256 * 1024 + 1);
        let request = Request::post("/v1/memory/validate")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .unwrap();
        let response = router(state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn validation_rejects_unknown_scenario_and_accepts_scripted_success() {
        let app = router(state());
        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/v1/memory/validate")
                    .body(Body::from(r#"{"scenario":"basic-compression"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let request = Request::post("/v1/memory/validate")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"scenario":"unknown"}"#))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let request = Request::post("/v1/memory/validate")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"scenario":"basic-compression"}"#))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn concurrent_validation_returns_too_many_requests() {
        let app = router(state());
        let request = || {
            Request::post("/v1/memory/validate")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"scenario":"basic-compression","delay_ms":50}"#,
                ))
                .unwrap()
        };
        let first = app.clone().oneshot(request());
        tokio::task::yield_now().await;
        let second = app.clone().oneshot(request());
        let (first, second) = tokio::join!(first, second);
        let statuses = [first.unwrap().status(), second.unwrap().status()];
        assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));
        assert!(statuses.contains(&StatusCode::OK));
    }

    #[tokio::test]
    async fn panic_validation_releases_single_flight_slot() {
        let app = router(state());
        let request = || {
            Request::post("/v1/memory/validate")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"scenario":"basic-compression","panic_section":"promises"}"#,
                ))
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = app
            .oneshot(
                Request::post("/v1/memory/validate")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"scenario":"basic-compression"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
