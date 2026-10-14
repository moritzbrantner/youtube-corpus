use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, upsert_video_scene, upsert_visual_text_observation,
    upsert_visual_text_track, BeginVisualProcessingRunRequest, ProcessingProvenance,
    UpsertVideoSceneRequest, UpsertVisualTextObservationRequest, UpsertVisualTextTrackRequest,
    VisualProcessingKind, VisualTextRole,
};

/// Exercises the real corpus HTTP endpoint, Postgres search view, and canonical
/// producer-shaped OCR tracks. The checked-out application still requires a
/// separate browser acceptance pass against an actual local Postgres instance.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn one_corpus_query_ranks_speech_and_ocr_tracks_with_provenance() {
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
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Evidence search test')")
        .bind(corpus_id)
        .bind(format!("evidence-{corpus_id}"))
        .execute(&pool)
        .await
        .unwrap();
    for (video, channel, date, title) in [
        (spoken_video, "Speech Source", "20250201", "A spoken lecture"),
        (slide_video, "Slide Source", "20260401", "A projected slide"),
        (unknown_video, "Unprocessed Source", "20260401", "OCR unavailable"),
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
        sqlx::query(
            "INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)",
        )
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

    let app = youtube_corpus::web::app_for_tests(Some(database_url));
    let speech_favored = query(&app, corpus_id, json!({"query":"nova", "mode":"fts", "topK":20})).await;
    let speech_results = speech_favored["results"].as_array().unwrap();
    assert!(speech_results.iter().any(|r| r["segmentId"] == spoken_segment.to_string()));
    assert!(speech_results.iter().any(|r| r["segmentId"] == primary_track.id.to_string()));
    assert_eq!(speech_results[0]["sourceKind"], "caption_manual");

    let visual_favored = query(&app, corpus_id, json!({"query":"helios", "mode":"fts", "topK":20})).await;
    let visual_results = visual_favored["results"].as_array().unwrap();
    assert_eq!(visual_results[0]["sourceKind"], "visual_ocr");
    assert!(visual_results.iter().any(|r| r["segmentId"] == next_track.id.to_string()));
    assert_eq!(
        visual_results.iter().filter(|r| r["segmentId"] == primary_track.id.to_string()).count(),
        1,
        "two observations of one track must yield one matching moment"
    );
    let track_hit = visual_results.iter().find(|r| r["segmentId"] == primary_track.id.to_string()).unwrap();
    assert_eq!(track_hit["startSeconds"], 12.0);
    assert_eq!(track_hit["ocrEvidence"]["runId"], ocr_run.to_string());
    assert_eq!(track_hit["ocrEvidence"]["sampleCount"], 2);
    assert_eq!(track_hit["ocrEvidence"]["observations"].as_array().unwrap().len(), 2);
    assert_eq!(track_hit["ocrEvidence"]["scenes"][0]["sceneIndex"], 2);
    assert_eq!(track_hit["ocrEvidence"]["processorVersion"], "test-revision");
    assert_eq!(visual_favored["ocrCoverage"]["indexedVideos"], 1);
    assert_eq!(visual_favored["ocrCoverage"]["unprocessedVideos"], 2);

    let only_visible = query(&app, corpus_id, json!({"query":"nova", "mode":"fts", "topK":20, "modality":"visible"})).await;
    assert_eq!(only_visible["results"].as_array().unwrap().len(), 1);
    assert_eq!(only_visible["results"][0]["sourceKind"], "visual_ocr");
    let only_spoken = query(&app, corpus_id, json!({"query":"nova", "mode":"fts", "topK":20, "modality":"spoken"})).await;
    assert_eq!(only_spoken["results"].as_array().unwrap().len(), 1);
    assert_eq!(only_spoken["results"][0]["sourceKind"], "caption_manual");
    let metadata = query(&app, corpus_id, json!({
        "query":"nova", "mode":"fts", "topK":20,
        "channelQuery":"Slide Source", "categoryQuery":"Education",
        "tagQuery":"history", "uploadDateFrom":"2026-01-01"
    })).await;
    assert_eq!(metadata["results"].as_array().unwrap().len(), 1);
    assert_eq!(metadata["results"][0]["videoId"], slide_video.to_string());

    let missing = query(&app, corpus_id, json!({
        "query":"helios", "mode":"fts", "topK":20,
        "modality":"visible", "channelQuery":"Unprocessed Source"
    })).await;
    assert!(missing["results"].as_array().unwrap().is_empty());
    assert_eq!(missing["ocrCoverage"]["unprocessedVideos"], 2);

    let context = app
        .oneshot(json_request(
            "/api/transcript-context",
            json!({"segmentId": primary_track.id}),
        ))
        .await
        .unwrap();
    assert_eq!(context.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(context.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["match"]["sourceKind"], "visual_ocr");
    assert_eq!(body["match"]["startSeconds"], 12.0);

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

async fn query(app: &axum::Router, corpus_id: Uuid, request: Value) -> Value {
    let response = app
        .clone()
        .oneshot(json_request(
            &format!("/api/corpora/{corpus_id}/preferred-search"),
            request,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}
