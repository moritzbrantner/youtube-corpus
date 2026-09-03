use serde_json::json;
use uuid::Uuid;
use youtube_corpus::multimodal::{
    begin_processing_run, link_face_observation_to_track, link_voice_observation_to_track,
    list_face_observations, list_voice_observations, upsert_face_observation, upsert_face_track,
    upsert_voice_observation, upsert_voice_track, BeginProcessingRunRequest, BoundingBox,
    MediaModality, ProcessingProvenance, UpsertFaceObservationRequest, UpsertFaceTrackRequest,
    UpsertVoiceObservationRequest, UpsertVoiceTrackRequest,
};

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn multimodal_results_are_idempotent_and_video_scoped() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title)
         VALUES ($1, $2, $3, 'Multimodal persistence fixture')",
    )
    .bind(video_id)
    .bind(format!("multimodal-{video_id}"))
    .bind(format!("https://youtube.test/{video_id}"))
    .execute(&pool)
    .await
    .unwrap();

    let face_provenance = ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: "test".to_string(),
        model: "opencv-sface-onnx".to_string(),
        model_version: "2021dec".to_string(),
        input_hash: "sha256:fixture-video".to_string(),
        config_hash: "sha256:face-config".to_string(),
        processing_config: json!({"frameStep": 6}),
    };
    let face_request = BeginProcessingRunRequest {
        video_id,
        modality: MediaModality::Face,
        provenance: face_provenance,
    };
    let face_run = begin_processing_run(&pool, face_request.clone())
        .await
        .unwrap();
    let face_rerun = begin_processing_run(&pool, face_request).await.unwrap();
    assert_eq!(face_run.id, face_rerun.id);

    let face = upsert_face_observation(
        &pool,
        UpsertFaceObservationRequest {
            run_id: face_run.id,
            video_id,
            observation_key: "frame-42-face-0".to_string(),
            start_seconds: 1.25,
            end_seconds: None,
            frame_index: Some(42),
            region: BoundingBox {
                x: 100.0,
                y: 50.0,
                width: 80.0,
                height: 90.0,
            },
            detection_score: Some(0.95),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            quality: json!({"blur": 0.05}),
        },
    )
    .await
    .unwrap();
    let face_rerun = upsert_face_observation(
        &pool,
        UpsertFaceObservationRequest {
            run_id: face_run.id,
            video_id,
            observation_key: "frame-42-face-0".to_string(),
            start_seconds: 1.25,
            end_seconds: None,
            frame_index: Some(42),
            region: BoundingBox {
                x: 101.0,
                y: 50.0,
                width: 80.0,
                height: 90.0,
            },
            detection_score: Some(0.96),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            quality: json!({"blur": 0.04}),
        },
    )
    .await
    .unwrap();
    assert_eq!(face.id, face_rerun.id);
    assert_eq!(face_rerun.region.x, 101.0);

    let face_track = upsert_face_track(
        &pool,
        UpsertFaceTrackRequest {
            run_id: face_run.id,
            video_id,
            track_key: "face-track-0".to_string(),
            start_seconds: 1.25,
            end_seconds: 2.5,
            representative_embedding: Some(vec![0.1, 0.2, 0.3]),
            quality: json!({"observations": 1}),
        },
    )
    .await
    .unwrap();
    link_face_observation_to_track(&pool, face_track.id, face.id)
        .await
        .unwrap();
    link_face_observation_to_track(&pool, face_track.id, face.id)
        .await
        .unwrap();

    let voice_provenance = ProcessingProvenance {
        processor: "audio-analysis".to_string(),
        processor_version: "test".to_string(),
        model: "spectral-speaker-embedding".to_string(),
        model_version: "test".to_string(),
        input_hash: "sha256:fixture-audio".to_string(),
        config_hash: "sha256:voice-config".to_string(),
        processing_config: json!({"vad": "rms"}),
    };
    let voice_run = begin_processing_run(
        &pool,
        BeginProcessingRunRequest {
            video_id,
            modality: MediaModality::Voice,
            provenance: voice_provenance,
        },
    )
    .await
    .unwrap();
    let voice = upsert_voice_observation(
        &pool,
        UpsertVoiceObservationRequest {
            run_id: voice_run.id,
            video_id,
            observation_key: "speaker-0-turn-0".to_string(),
            start_seconds: 3.0,
            end_seconds: 7.5,
            confidence: Some(0.9),
            transcript_segment_id: None,
            embedding: Some(vec![0.4, 0.5]),
            quality: json!({"snr": 22.0}),
        },
    )
    .await
    .unwrap();
    let voice_track = upsert_voice_track(
        &pool,
        UpsertVoiceTrackRequest {
            run_id: voice_run.id,
            video_id,
            track_key: "speaker-0".to_string(),
            start_seconds: 3.0,
            end_seconds: 7.5,
            representative_embedding: Some(vec![0.4, 0.5]),
            quality: json!({"turns": 1}),
        },
    )
    .await
    .unwrap();
    link_voice_observation_to_track(&pool, voice_track.id, voice.id)
        .await
        .unwrap();

    let faces = list_face_observations(&pool, video_id).await.unwrap();
    let voices = list_voice_observations(&pool, video_id).await.unwrap();
    assert_eq!(faces.len(), 1);
    assert_eq!(voices.len(), 1);
    assert_eq!(faces[0].embedding_dimensions, Some(3));
    assert_eq!(voices[0].embedding_dimensions, Some(2));

    let face_links: i64 =
        sqlx::query_scalar("SELECT count(*) FROM face_track_observations WHERE track_id = $1")
            .bind(face_track.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(face_links, 1);

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}
