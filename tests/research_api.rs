use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn corpus_routes_share_the_standard_database_error_contract() {
    let response = youtube_corpus::web::app_for_tests_with_research(None)
        .oneshot(
            Request::builder()
                .uri("/api/corpora")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
