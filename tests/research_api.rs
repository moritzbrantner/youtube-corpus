use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn corpus_routes_share_the_standard_database_error_contract() {
    let app = youtube_corpus::web::app_for_tests_with_research(None);
    for uri in [
        "/api/corpora",
        "/api/corpora/00000000-0000-5000-8000-000000000001/transcript-quality",
        "/api/corpora/00000000-0000-5000-8000-000000000001/annotations",
        "/api/corpora/00000000-0000-5000-8000-000000000001/evaluation/cases",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}
