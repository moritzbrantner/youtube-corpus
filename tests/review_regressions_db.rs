use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, upsert_video_scene, upsert_visual_text_observation,
    upsert_visual_text_track, BeginVisualProcessingRunRequest, ProcessingProvenance,
    UpsertVideoSceneRequest, UpsertVisualTextObservationRequest, UpsertVisualTextTrackRequest,
    VisualProcessingKind, VisualTextRole,
};

/// Regressions for the Codex review on PR #70 (implementation-authored; seeding copied
/// from the independent unified-search acceptance test): OCR evaluation targets keep
/// their track identity, the corpus search route reports OCR coverage, and transcript
/// context for an OCR track carries its provenance.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn ocr_hits_keep_identity_coverage_and_provenance_across_routes() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let corpus_id = Uuid::new_v4();
    let spoken_video = Uuid::new_v4();
    let slide_video = Uuid::new_v4();
    let unknown_video = Uuid::new_v4();
    let spoken_stream = Uuid::new_v4();
    let spoken_segment = Uuid::new_v4();
    let ocr_text = "helios helios helios helios nova";
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Review regression test')")
        .bind(corpus_id)
        .bind(format!("evidence-{corpus_id}"))
        .execute(&pool)
        .await
        .unwrap();
    for (video, channel, date, title) in [
        (
            spoken_video,
            "Speech Source",
            "20250201",
            "A spoken lecture",
        ),
        (slide_video, "Slide Source", "20260401", "A projected slide"),
        (
            unknown_video,
            "Unprocessed Source",
            "20260401",
            "OCR unavailable",
        ),
    ] {
        sqlx::query(
            "INSERT INTO videos (id, source_url, title, channel, upload_date, duration_seconds,
                                 categories, tags)
             VALUES ($1, $2, $3, $4, $5, 90, ARRAY['Education'], ARRAY['history'])",
        )
        .bind(video)
        .bind(format!("https://www.youtube.com/watch?v={video}"))
        .bind(title)
        .bind(channel)
        .bind(date)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
            .bind(corpus_id)
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed', 'helios nova nova nova nova')",
    )
    .bind(spoken_stream)
    .bind(spoken_video)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_segments
         (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
         VALUES ($1, $2, $3, 0, 7, 9, 'helios nova nova nova nova', 'en')",
    )
    .bind(spoken_segment)
    .bind(spoken_video)
    .bind(spoken_stream)
    .execute(&pool)
    .await
    .unwrap();
    youtube_corpus::transcript_quality::refresh_video_quality(&pool, spoken_video)
        .await
        .unwrap();

    let scene_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: slide_video,
            kind: VisualProcessingKind::Scene,
            provenance: provenance("scene-producer"),
        },
    )
    .await
    .unwrap();
    let scene = upsert_video_scene(
        &pool,
        UpsertVideoSceneRequest {
            run_id: scene_run,
            video_id: slide_video,
            scene_index: 2,
            start_frame: 300,
            end_frame: 600,
            start_seconds: 10.0,
            end_seconds: 20.0,
            metadata: json!({"detector":"content"}),
        },
    )
    .await
    .unwrap();
    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: slide_video,
            kind: VisualProcessingKind::Ocr,
            provenance: provenance("ocr-producer"),
        },
    )
    .await
    .unwrap();

    let mut observations = Vec::new();
    for (key, frame, time) in [("slide-frame-1", 340, 12.0), ("slide-frame-2", 360, 12.7)] {
        let observation = upsert_visual_text_observation(
            &pool,
            UpsertVisualTextObservationRequest {
                run_id: ocr_run,
                video_id: slide_video,
                observation_key: key.to_string(),
                text: ocr_text.to_string(),
                language: Some("en".to_string()),
                frame_index: Some(frame),
                timestamp_seconds: Some(time),
                scene_index: Some(2),
                region: None,
                confidence: Some(0.93),
                attributes: json!({"ocr.granularity": "line"}),
            },
        )
        .await
        .unwrap();
        observations.push(observation.id);
    }
    let primary_track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id: slide_video,
            track_key: "slide-title".to_string(),
            text: ocr_text.to_string(),
            role: VisualTextRole::PresentationSlide,
            language: Some("en".to_string()),
            sample_count: 2,
            start_frame: Some(340),
            end_frame: Some(360),
            start_seconds: Some(12.0),
            end_seconds: Some(13.0),
            region: None,
            metadata: json!({"source": "visual-analysis fixture"}),
            observation_ids: observations,
            scene_ids: vec![scene.id],
        },
    )
    .await
    .unwrap();

    // A later independent visible-text moment must remain an independent hit.
    let next_observation = upsert_visual_text_observation(
        &pool,
        UpsertVisualTextObservationRequest {
            run_id: ocr_run,
            video_id: slide_video,
            observation_key: "later-frame".to_string(),
            text: "helios".to_string(),
            language: Some("en".to_string()),
            frame_index: Some(1100),
            timestamp_seconds: Some(45.0),
            scene_index: None,
            region: None,
            confidence: Some(0.9),
            attributes: json!({}),
        },
    )
    .await
    .unwrap();
    let next_track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id: slide_video,
            track_key: "later-title".to_string(),
            text: "helios".to_string(),
            role: VisualTextRole::SceneText,
            language: Some("en".to_string()),
            sample_count: 1,
            start_frame: Some(1100),
            end_frame: Some(1100),
            start_seconds: Some(45.0),
            end_seconds: Some(45.0),
            region: None,
            metadata: json!({}),
            observation_ids: vec![next_observation.id],
            scene_ids: vec![],
        },
    )
    .await
    .unwrap();

    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    // Corpus search route: OCR coverage is reported, so an unprocessed video is unknown.
    let report = post(
        &app,
        &format!("/api/corpora/{corpus_id}/search"),
        json!({"query":"helios", "mode":"fts", "topK":20, "modality":"visible"}),
    )
    .await;
    assert_eq!(report["ocrCoverage"]["indexedVideos"], 1);
    assert_eq!(report["ocrCoverage"]["unprocessedVideos"], 2);

    // Context for an OCR track carries the same provenance as search results.
    let context = post(
        &app,
        "/api/transcript-context",
        json!({"segmentId": primary_track.id.to_string(), "before": 1, "after": 1}),
    )
    .await;
    assert_eq!(context["match"]["sourceKind"], "visual_ocr");
    assert_eq!(context["match"]["ocrEvidence"]["trackKey"], "slide-title");
    assert_eq!(context["match"]["ocrEvidence"]["sampleCount"], 2);

    // An OCR judgment keeps its track identity and is never satisfied by speech.
    let case = post(
        &app,
        &format!("/api/corpora/{corpus_id}/evaluation/judgments"),
        json!({"query":"nova", "mode":"fts", "topK":20, "videoId": slide_video,
               "visualTextTrackId": primary_track.id, "startSeconds": 12.0,
               "endSeconds": 13.0, "relevance": 3}),
    )
    .await;
    assert_eq!(
        case["targets"][0]["visualTextTrackId"],
        primary_track.id.to_string()
    );
    assert!(case["targets"][0]["segmentId"].is_null());
    let run = post(
        &app,
        &format!("/api/corpora/{corpus_id}/evaluation/run"),
        json!({}),
    )
    .await;
    // The judged OCR track is retrieved for "nova" and counts as exactly one hit.
    let cases = run["cases"].as_array().unwrap();
    let nova = cases.iter().find(|c| c["query"] == "nova").unwrap();
    assert_eq!(nova["hits"], 1, "{nova}");
    let _ = (spoken_segment, next_track.id);

    sqlx::query("DELETE FROM corpora WHERE id = $1")
        .bind(corpus_id)
        .execute(&pool)
        .await
        .unwrap();
    for video in [spoken_video, slide_video, unknown_video] {
        sqlx::query("DELETE FROM videos WHERE id = $1")
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }
}

fn provenance(model: &str) -> ProcessingProvenance {
    ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: "test-revision".to_string(),
        model: model.to_string(),
        model_version: "1".to_string(),
        input_hash: "sha256:fixture-media".to_string(),
        config_hash: format!("sha256:{model}"),
        processing_config: json!({"model":model,"sampling":"scene"}),
    }
}

fn json_request(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn post(app: &axum::Router, uri: &str, body: Value) -> Value {
    let response = app.clone().oneshot(json_request(uri, body)).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        status.is_success(),
        "{uri}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

/// Review regression (PR #74): renaming an unknown label is a 404 regardless of the payload,
/// so a blank replacement name must not turn it into a 400.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn renaming_an_unknown_label_is_not_found_even_with_a_blank_name() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));
    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Rename review')")
        .bind(corpus)
        .bind(format!("rename-review-{}", corpus.simple()))
        .execute(&pool)
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/api/corpora/{corpus}/labels/{}", Uuid::new_v4()))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "name": "   " }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    sqlx::query("DELETE FROM corpora WHERE id = $1")
        .bind(corpus)
        .execute(&pool)
        .await
        .unwrap();
}
