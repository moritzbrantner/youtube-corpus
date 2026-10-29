//! Opt-in Tantivy literal search pilot (#76) against the real Postgres projection and the
//! existing `/api/search` route: same query on both providers, spoken-only, visible-only and
//! missing-OCR records, exact source/timestamp IDs, edits, deletes and restarts.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::search_index::TantivySearch;
use youtube_corpus::{
    begin_visual_processing_run, upsert_visual_text_track, BeginVisualProcessingRunRequest,
    ProcessingProvenance, UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn tantivy_pilot_matches_literal_terms_and_follows_postgres_edits() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    // Unique term and channel keep this test independent of other rows in the shared database.
    let token = format!("pilot{}", &Uuid::new_v4().simple().to_string()[..10]);
    let channel = format!("Pilot Channel {token}");
    let spoken_video = Uuid::new_v4();
    let slide_video = Uuid::new_v4();
    for (video, title) in [(spoken_video, "Spoken only"), (slide_video, "Slides only")] {
        sqlx::query(
            "INSERT INTO videos (id, source_url, title, channel, upload_date, duration_seconds)
             VALUES ($1, $2, $3, $4, '20260401', 90)",
        )
        .bind(video)
        .bind(format!("https://www.youtube.com/watch?v={video}"))
        .bind(title)
        .bind(&channel)
        .execute(&pool)
        .await
        .unwrap();
    }
    let stream = Uuid::new_v4();
    let segment = Uuid::new_v4();
    let spoken_text = format!("{token} planetary orbit");
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed', $3)",
    )
    .bind(stream)
    .bind(spoken_video)
    .bind(&spoken_text)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_segments
         (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
         VALUES ($1, $2, $3, 0, 7.25, 9, $4, 'en')",
    )
    .bind(segment)
    .bind(spoken_video)
    .bind(stream)
    .bind(&spoken_text)
    .execute(&pool)
    .await
    .unwrap();
    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: slide_video,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".into(),
                processor_version: "test-revision".into(),
                model: "ocr".into(),
                model_version: "1".into(),
                input_hash: "sha256:fixture-media".into(),
                config_hash: "sha256:ocr".into(),
                processing_config: json!({}),
            },
        },
    )
    .await
    .unwrap();
    let track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id: slide_video,
            track_key: "slide".into(),
            text: format!("{token} planet slide"),
            role: VisualTextRole::PresentationSlide,
            language: Some("en".into()),
            sample_count: 1,
            start_frame: Some(360),
            end_frame: Some(390),
            start_seconds: Some(12.0),
            end_seconds: Some(13.0),
            region: None,
            metadata: json!({}),
            observation_ids: vec![],
            scene_ids: vec![],
        },
    )
    .await
    .unwrap();

    let index_dir = tempfile::tempdir().unwrap();
    let index = Arc::new(TantivySearch::open(index_dir.path()).unwrap());
    let first = index.sync(&pool).await.unwrap();
    assert!(first.live_passages >= 2);
    let tantivy = youtube_corpus::web::app_for_tests_with_search_index(
        Some(database_url.clone()),
        index.clone(),
    );
    let postgres = youtube_corpus::web::app_for_tests(Some(database_url.clone()));
    let query = |extra: Value| {
        let mut body = json!({"query": format!("{token} planet"), "mode": "fts", "topK": 20,
                              "channelQuery": channel});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    // Same query: the pilot's substring `planet` also matches `planetary`; Postgres FTS does not.
    let literal = search(&tantivy, query(json!({}))).await;
    assert_eq!(literal["provider"], "tantivy_literal");
    let hits = ids(&literal);
    assert_eq!(hits.len(), 2);
    assert!(hits.contains(&segment.to_string()) && hits.contains(&track.id.to_string()));
    let spoken_hit = hit(&literal, segment);
    assert_eq!(spoken_hit["startSeconds"], 7.25);
    assert_eq!(spoken_hit["videoId"], spoken_video.to_string());
    assert_eq!(spoken_hit["streamId"], stream.to_string());
    let slide_hit = hit(&literal, track.id);
    assert_eq!(slide_hit["sourceKind"], "visual_ocr");
    assert_eq!(slide_hit["ocrEvidence"]["runId"], ocr_run.to_string());
    // The spoken-only video has no OCR run: unprocessed, not a visible-text negative.
    assert_eq!(literal["ocrCoverage"]["indexedVideos"], 1);
    assert_eq!(literal["ocrCoverage"]["unprocessedVideos"], 1);

    let ranked = search(&postgres, query(json!({}))).await;
    assert_eq!(ranked["provider"], "postgres");
    assert_eq!(ids(&ranked), [track.id.to_string()]);

    let visible = search(&tantivy, query(json!({"modality": "visible"}))).await;
    assert_eq!(ids(&visible), [track.id.to_string()]);
    let spoken = search(&tantivy, query(json!({"modality": "spoken"}))).await;
    assert_eq!(ids(&spoken), [segment.to_string()]);
    let unsupported = tantivy
        .clone()
        .oneshot(json_request(query(json!({"mode": "hybrid"}))))
        .await
        .unwrap();
    assert_eq!(unsupported.status(), StatusCode::BAD_REQUEST);

    let compared = youtube_corpus::search_index::compare(
        &index,
        &pool,
        youtube_corpus::search::SearchRequest {
            database_url: database_url.clone(),
            query: format!("{token} planet"),
            top_k: 20,
            mode: youtube_corpus::SearchMode::Fts,
            source_kind: None,
            modality: None,
            video_id: None,
            language: None,
            transcript_start_min: None,
            transcript_start_max: None,
            upload_date_from: None,
            upload_date_to: None,
            duration_min: None,
            duration_max: None,
            channel_query: Some(channel.clone()),
            title_query: None,
            category_query: None,
            tag_query: None,
            metadata_query: None,
            view_count_min: None,
            view_count_max: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(compared.shared, 1);
    assert_eq!(compared.only_tantivy, [segment]);
    assert!(compared.only_postgres.is_empty());

    // Edit and delete in Postgres, then restart: reopen the same directory and sync.
    drop(tantivy);
    drop(index);
    sqlx::query("UPDATE transcript_segments SET text = $2 WHERE id = $1")
        .bind(segment)
        .bind(format!("{token} moon orbit"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM visual_text_tracks WHERE id = $1")
        .bind(track.id)
        .execute(&pool)
        .await
        .unwrap();
    let index = Arc::new(TantivySearch::open(index_dir.path()).unwrap());
    let resync = index.sync(&pool).await.unwrap();
    assert!(resync.upserted >= 1 && resync.deleted >= 1);
    let tantivy =
        youtube_corpus::web::app_for_tests_with_search_index(Some(database_url), index.clone());
    assert!(ids(&search(&tantivy, query(json!({}))).await).is_empty());
    let moon = search(&tantivy, query(json!({"query": format!("{token} MOON")}))).await;
    assert_eq!(ids(&moon), [segment.to_string()]);

    for video in [spoken_video, slide_video] {
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }
}

fn json_request(body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/search")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn search(app: &axum::Router, body: Value) -> Value {
    let response = app.clone().oneshot(json_request(body)).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

fn ids(report: &Value) -> Vec<String> {
    report["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["segmentId"].as_str().unwrap().to_owned())
        .collect()
}

fn hit(report: &Value, id: Uuid) -> &Value {
    report["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["segmentId"] == id.to_string())
        .unwrap()
}
