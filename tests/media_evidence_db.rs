use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, export_media_evidence_batch, export_source_span_batch,
    parse_hash_response, persist_sponsorblock_snapshot, upsert_video_scene,
    upsert_visual_text_observation, upsert_visual_text_track, BeginVisualProcessingRunRequest,
    ProcessingProvenance, UpsertVideoSceneRequest, UpsertVisualTextObservationRequest,
    UpsertVisualTextTrackRequest, VisualBoundingBox, VisualProcessingKind, VisualTextRole,
    SPONSORBLOCK_DATA_LICENSE,
};

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn multimodal_evidence_preserves_separate_but_aligned_channels() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    let youtube_id = "ifI_fwg55k8";
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, channel, uploader)
         VALUES ($1, $2, $3, 'Aquinas lecture', 'Philosophy Channel', 'Lecturer')",
    )
    .bind(video_id)
    .bind(youtube_id)
    .bind(format!("https://www.youtube.com/watch?v={youtube_id}"))
    .execute(&pool)
    .await
    .unwrap();

    let scene_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Scene,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "scene-fixture".to_string(),
                model: "scenedetect-core:content".to_string(),
                model_version: "fixture".to_string(),
                input_hash: "sha256:video".to_string(),
                config_hash: "sha256:scene-config".to_string(),
                processing_config: json!({"threshold": 27.0}),
            },
        },
    )
    .await
    .unwrap();
    let scene = upsert_video_scene(
        &pool,
        UpsertVideoSceneRequest {
            run_id: scene_run,
            video_id,
            scene_index: 0,
            start_frame: 0,
            end_frame: 149,
            start_seconds: 0.0,
            end_seconds: 5.0,
            metadata: json!({"detector": "content"}),
        },
    )
    .await
    .unwrap();

    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "ocr-fixture".to_string(),
                model: "trocr".to_string(),
                model_version: "fixture".to_string(),
                input_hash: "sha256:video".to_string(),
                config_hash: "sha256:ocr-config".to_string(),
                processing_config: json!({"sampling": "scene-aware"}),
            },
        },
    )
    .await
    .unwrap();
    let observation = upsert_visual_text_observation(
        &pool,
        UpsertVisualTextObservationRequest {
            run_id: ocr_run,
            video_id,
            observation_key: "scene-0-title".to_string(),
            text: "Thomas Aquinas — Five Ways".to_string(),
            language: Some("en".to_string()),
            frame_index: Some(60),
            timestamp_seconds: Some(2.0),
            scene_index: Some(0),
            region: Some(VisualBoundingBox {
                x: 20,
                y: 30,
                width: 640,
                height: 80,
            }),
            confidence: Some(0.96),
            attributes: json!({"granularity": "line"}),
        },
    )
    .await
    .unwrap();
    let track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id,
            track_key: "slide-title".to_string(),
            text: "Thomas Aquinas — Five Ways".to_string(),
            role: VisualTextRole::PresentationSlide,
            language: Some("en".to_string()),
            sample_count: 1,
            start_frame: Some(60),
            end_frame: Some(60),
            start_seconds: Some(2.0),
            end_seconds: Some(2.0),
            region: Some(VisualBoundingBox {
                x: 20,
                y: 30,
                width: 640,
                height: 80,
            }),
            metadata: json!({"derived": true}),
            observation_ids: vec![observation.id],
            scene_ids: vec![scene.id],
        },
    )
    .await
    .unwrap();

    let hash = sha256_hex(youtube_id.as_bytes());
    let response = serde_json::json!([{
        "videoID": youtube_id,
        "hash": hash,
        "segments": [{
            "category": "intro",
            "segment": [0.0, 1.5],
            "UUID": "intro-fixture",
            "videoDuration": 600.0
        }]
    }]);
    let snapshot = parse_hash_response(
        youtube_id,
        &["intro".to_string(), "sponsor".to_string()],
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap();
    persist_sponsorblock_snapshot(&pool, video_id, &snapshot)
        .await
        .unwrap();

    let evidence = export_media_evidence_batch(&pool, video_id, "git:test")
        .await
        .unwrap();
    assert_eq!(evidence.scenes.len(), 1);
    assert_eq!(evidence.ocr_observations.len(), 1);
    assert_eq!(evidence.ocr_tracks.len(), 1);
    assert_eq!(evidence.ocr_tracks[0].id, track.id.to_string());
    assert_eq!(evidence.ocr_tracks[0].scene_ids, vec![scene.id.to_string()]);
    assert_eq!(
        evidence.sponsorblock.as_ref().unwrap().data_license,
        SPONSORBLOCK_DATA_LICENSE
    );
    assert_eq!(
        evidence.sponsorblock.as_ref().unwrap().segments[0].category,
        "intro"
    );

    let spans = export_source_span_batch(&pool, "git:test", Some(video_id))
        .await
        .unwrap();
    let ocr_source = spans
        .sources
        .iter()
        .find(|source| source.kind == "youtube_visual_text")
        .expect("OCR source");
    let ocr_span = spans
        .spans
        .iter()
        .find(|span| span.id == track.id.to_string())
        .expect("OCR span");
    assert_eq!(ocr_span.source_id, ocr_source.id);
    assert_eq!(
        ocr_span.metadata["mediaEvidenceRef"],
        track.id.to_string()
    );

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
