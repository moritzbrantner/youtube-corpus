//! Per-video detector stage status against the effective plan (#60, Atlas 2.1, 2.2, 2.6, 1.8).

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn video(pool: &PgPool, title: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO videos (id, youtube_id, source_url, title) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(format!("yt{id}"))
        .bind(format!("https://www.youtube.com/watch?v={id}"))
        .bind(title)
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn stream(pool: &PgPool, video: Uuid, kind: &str, segments: usize, message: Option<&str>) {
    let stream = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, message)
         VALUES ($1, $2, $3, 'en', 'completed', $4)",
    )
    .bind(stream)
    .bind(video)
    .bind(kind)
    .bind(message)
    .execute(pool)
    .await
    .unwrap();
    for index in 0..segments {
        sqlx::query(
            "INSERT INTO transcript_segments (id, video_id, stream_id, segment_index, text)
             VALUES ($1, $2, $3, $4, 'hello')",
        )
        .bind(Uuid::new_v4())
        .bind(video)
        .bind(stream)
        .bind(index as i64)
        .execute(pool)
        .await
        .unwrap();
    }
}

fn stage<'a>(status: &'a Value, detector: &str) -> &'a Value {
    status["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["detector"] == detector)
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn stages_report_complete_failed_unavailable_disabled_and_awaiting_review() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Stage status')")
        .bind(corpus)
        .bind(format!("stages-{corpus}"))
        .execute(&pool)
        .await
        .unwrap();
    let analyzed = video(&pool, "Analyzed").await;
    sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
        .bind(corpus)
        .bind(analyzed)
        .execute(&pool)
        .await
        .unwrap();
    stream(&pool, analyzed, "caption_manual", 3, None).await;
    stream(
        &pool,
        analyzed,
        "asr",
        0,
        Some("whisper exited with status 1"),
    )
    .await;

    // A video known only through a review-first source whose item still waits.
    let held = video(&pool, "Held").await;
    let review = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO corpus_subscriptions
           (id, source_kind, source_url, name, work_dir, caption, processing_policy)
         VALUES ($1, 'playlist', $2, 'Review', 'use-case-output/test', $3, 'review_first')",
    )
    .bind(review)
    .bind(format!("https://www.youtube.com/playlist?list=PL{review}"))
    .bind(json!({"enabled": true, "include_auto_captions": true, "languages": ["en"]}))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO corpus_source_memberships (corpus_id, subscription_id) VALUES ($1, $2)",
    )
    .bind(corpus)
    .bind(review)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO corpus_subscription_items (id, subscription_id, source_url, youtube_id, status)
         VALUES ($1, $2, $3, $4, 'awaiting_review')",
    )
    .bind(Uuid::new_v4())
    .bind(review)
    .bind(format!("https://www.youtube.com/watch?v={held}"))
    .bind(format!("yt{held}"))
    .execute(&pool)
    .await
    .unwrap();

    let base = format!("/api/corpora/{corpus}");
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("{base}/detector-profiles/corpus"),
        Some(json!({"settings": [
            {"detector": "captions", "enabled": true},
            {"detector": "asr", "enabled": true},
            {"detector": "scenes", "enabled": true},
            {"detector": "ocr", "enabled": true}
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, analyzed_status) = call(
        &app,
        Method::GET,
        &format!("{base}/videos/{analyzed}/detector-status"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{analyzed_status}");
    assert_eq!(analyzed_status["awaitingReview"], false);
    let captions = stage(&analyzed_status, "captions");
    assert_eq!(captions["state"], "complete");
    assert_eq!(captions["outputCount"], 3);
    let asr = stage(&analyzed_status, "asr");
    assert_eq!(asr["state"], "failed");
    assert_eq!(asr["message"], "whisper exited with status 1");
    assert_eq!(stage(&analyzed_status, "ocr")["state"], "unavailable");
    assert_eq!(stage(&analyzed_status, "scenes")["state"], "not_run");
    assert_eq!(stage(&analyzed_status, "entities")["state"], "disabled");

    // Targeted retry runs only the requested stage and records its real failure.
    let (status, run) = call(
        &app,
        Method::POST,
        &format!("{base}/videos/{analyzed}/detector-plan/run"),
        Some(json!({"only": ["scenes"], "refresh": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let stages = run["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["detector"], "scenes");
    assert_eq!(stages[0]["outcome"], "failed");
    let (_, after) = call(
        &app,
        Method::GET,
        &format!("{base}/videos/{analyzed}/detector-status"),
        None,
    )
    .await;
    let scenes = stage(&after, "scenes");
    assert_eq!(scenes["state"], "failed");
    assert!(scenes["message"]
        .as_str()
        .unwrap()
        .contains("no retained local media"));
    assert!(scenes["lastAttemptAt"].is_string());
    // Completed captions were not touched by the scenes retry.
    assert_eq!(stage(&after, "captions")["state"], "complete");

    let (status, held_status) = call(
        &app,
        Method::GET,
        &format!("{base}/videos/{held}/detector-status"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{held_status}");
    assert_eq!(held_status["awaitingReview"], true);
    assert_eq!(stage(&held_status, "captions")["state"], "awaiting_review");
    assert_eq!(stage(&held_status, "ocr")["state"], "unavailable");
    assert_eq!(stage(&held_status, "entities")["state"], "disabled");

    // Once the item is processed through any source, it no longer awaits review.
    sqlx::query(
        "UPDATE corpus_subscription_items SET status = 'indexed' WHERE subscription_id = $1",
    )
    .bind(review)
    .execute(&pool)
    .await
    .unwrap();
    let (_, released) = call(
        &app,
        Method::GET,
        &format!("{base}/videos/{held}/detector-status"),
        None,
    )
    .await;
    assert_eq!(released["awaitingReview"], false);
    assert_eq!(stage(&released, "captions")["state"], "not_run");
}
