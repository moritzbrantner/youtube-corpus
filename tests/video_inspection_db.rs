//! #102: a corpus video's own page. `GET /api/corpora/{corpus}/videos/{video}` returns the
//! catalog metadata, every retained transcript stream (preferred first) with its segments and
//! the OCR tracks with producer provenance, plus whether OCR ran at all. Deep-linked moments
//! resolve by stable evidence id through `/api/transcript-context` for speech and OCR alike.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, upsert_visual_text_observation, upsert_visual_text_track,
    BeginVisualProcessingRunRequest, ProcessingProvenance, UpsertVisualTextObservationRequest,
    UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn context(segment_id: Uuid) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/transcript-context")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "segmentId": segment_id, "before": 1, "after": 1 }).to_string(),
        ))
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn video_page_lists_separate_speech_and_ocr_evidence_and_resolves_moments() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpus = Uuid::new_v4();
    let video = Uuid::new_v4();
    let bare_video = Uuid::new_v4();
    let outside_video = Uuid::new_v4();
    let manual = Uuid::new_v4();
    let auto = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Video page test')")
        .bind(corpus)
        .bind(format!("video-page-{corpus}"))
        .execute(&pool)
        .await
        .unwrap();
    for id in [video, bare_video, outside_video] {
        sqlx::query(
            "INSERT INTO videos (id, source_url, title, channel, description)
             VALUES ($1, $2, 'Inspected lecture', 'Lecture Channel', 'About helios')",
        )
        .bind(id)
        .bind(format!("https://www.youtube.com/watch?v={}", id.simple()))
        .execute(&pool)
        .await
        .unwrap();
    }
    for id in [video, bare_video] {
        sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
            .bind(corpus)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (stream, kind) in [(manual, "caption_manual"), (auto, "caption_auto")] {
        sqlx::query(
            "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
             VALUES ($1, $2, $3, 'en', 'completed')",
        )
        .bind(stream)
        .bind(video)
        .bind(kind)
        .execute(&pool)
        .await
        .unwrap();
    }
    let mut manual_segments = Vec::new();
    for (index, start, text) in [
        (0_i64, 1.0, "welcome"),
        (1, 4.5, "helios rises"),
        (2, 9.0, "goodbye"),
    ] {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transcript_segments
             (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, $4, $5, $5 + 2, $6, 'en')",
        )
        .bind(id)
        .bind(video)
        .bind(manual)
        .bind(index)
        .bind(start)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
        manual_segments.push(id);
    }
    youtube_corpus::transcript_quality::refresh_video_quality(&pool, video)
        .await
        .unwrap();

    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: video,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "test-revision".to_string(),
                model: "ocr-model".to_string(),
                model_version: "1".to_string(),
                input_hash: "sha256:fixture-media".to_string(),
                config_hash: "sha256:ocr".to_string(),
                processing_config: json!({"sampling": "scene"}),
            },
        },
    )
    .await
    .unwrap();
    let observation = upsert_visual_text_observation(
        &pool,
        UpsertVisualTextObservationRequest {
            run_id: ocr_run,
            video_id: video,
            observation_key: "slide-frame".to_string(),
            text: "HELIOS".to_string(),
            language: Some("en".to_string()),
            frame_index: Some(300),
            timestamp_seconds: Some(12.0),
            scene_index: None,
            region: None,
            confidence: Some(0.9),
            attributes: json!({}),
        },
    )
    .await
    .unwrap();
    let track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id: video,
            track_key: "slide-title".to_string(),
            text: "HELIOS".to_string(),
            role: VisualTextRole::PresentationSlide,
            language: Some("en".to_string()),
            sample_count: 1,
            start_frame: Some(300),
            end_frame: Some(330),
            start_seconds: Some(12.0),
            end_seconds: Some(13.0),
            region: None,
            metadata: json!({}),
            observation_ids: vec![observation.id],
            scene_ids: Vec::new(),
        },
    )
    .await
    .unwrap();

    let (status, page) = send(&app, get(&format!("/api/corpora/{corpus}/videos/{video}"))).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["video"]["id"], video.to_string());
    assert_eq!(page["video"]["title"], "Inspected lecture");
    assert_eq!(page["description"], "About helios");
    let streams = page["transcriptStreams"].as_array().unwrap();
    assert_eq!(streams.len(), 2);
    assert_eq!(
        streams[0]["streamId"],
        manual.to_string(),
        "preferred first"
    );
    assert_eq!(streams[0]["isPreferred"], true);
    assert_eq!(streams[0]["sourceKind"], "caption_manual");
    let segments = streams[0]["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[1]["segmentId"], manual_segments[1].to_string());
    assert_eq!(segments[1]["startSeconds"], 4.5);
    assert_eq!(streams[1]["segments"].as_array().unwrap().len(), 0);
    assert_eq!(page["ocrStatus"], "indexed");
    let tracks = page["visualText"].as_array().unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["segmentId"], track.id.to_string());
    assert_eq!(tracks[0]["sourceKind"], "visual_ocr");
    assert_eq!(tracks[0]["startSeconds"], 12.0);
    assert_eq!(tracks[0]["ocrEvidence"]["processor"], "visual-analysis");
    assert_eq!(
        tracks[0]["ocrEvidence"]["observations"][0]["id"],
        observation.id.to_string()
    );

    // Unprocessed evidence is reported as such, not as an empty result.
    let (status, bare) = send(
        &app,
        get(&format!("/api/corpora/{corpus}/videos/{bare_video}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bare}");
    assert_eq!(bare["transcriptStreams"], json!([]));
    assert_eq!(bare["ocrStatus"], "unprocessed");
    assert_eq!(bare["visualText"], json!([]));

    let (status, _) = send(
        &app,
        get(&format!("/api/corpora/{corpus}/videos/{outside_video}")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Moment URLs resolve speech and OCR evidence to their video and exact interval.
    let (status, spoken) = send(&app, context(manual_segments[1])).await;
    assert_eq!(status, StatusCode::OK, "{spoken}");
    assert_eq!(spoken["match"]["videoId"], video.to_string());
    assert_eq!(spoken["match"]["sourceKind"], "caption_manual");
    assert_eq!(spoken["match"]["startSeconds"], 4.5);
    assert_eq!(spoken["match"]["endSeconds"], 6.5);
    assert_eq!(spoken["segments"].as_array().unwrap().len(), 3);
    let (status, visible) = send(&app, context(track.id)).await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    assert_eq!(visible["match"]["videoId"], video.to_string());
    assert_eq!(visible["match"]["sourceKind"], "visual_ocr");
    assert_eq!(visible["match"]["startSeconds"], 12.0);
    assert_eq!(visible["match"]["ocrEvidence"]["trackKey"], "slide-title");
}
