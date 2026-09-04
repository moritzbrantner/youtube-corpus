use serde_json::json;
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, list_video_scenes, list_visual_text_tracks, search_corpus,
    upsert_video_scene, upsert_visual_text_observation, upsert_visual_text_track,
    BeginVisualProcessingRunRequest, ProcessingProvenance, SearchMode, SearchRequest, SourceKind,
    UpsertVideoSceneRequest, UpsertVisualTextObservationRequest, UpsertVisualTextTrackRequest,
    VisualBoundingBox, VisualProcessingKind, VisualTextRole,
};

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn visual_timeline_is_idempotent_scene_linked_and_searchable() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title)
         VALUES ($1, $2, $3, 'Visual timeline fixture')",
    )
    .bind(video_id)
    .bind(format!("visual-{video_id}"))
    .bind(format!("https://youtube.test/{video_id}"))
    .execute(&pool)
    .await
    .unwrap();

    let scene_provenance = ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: "test".to_string(),
        model: "scenedetect-core-content".to_string(),
        model_version: "0.1.0".to_string(),
        input_hash: "sha256:visual-fixture".to_string(),
        config_hash: "sha256:scene-config".to_string(),
        processing_config: json!({"threshold": 27.0, "minSceneLength": 15}),
    };
    let scene_request = BeginVisualProcessingRunRequest {
        video_id,
        kind: VisualProcessingKind::Scene,
        provenance: scene_provenance,
    };
    let scene_run = begin_visual_processing_run(&pool, scene_request.clone())
        .await
        .unwrap();
    let scene_rerun = begin_visual_processing_run(&pool, scene_request).await.unwrap();
    assert_eq!(scene_run, scene_rerun);

    let scene = upsert_video_scene(
        &pool,
        UpsertVideoSceneRequest {
            run_id: scene_run,
            video_id,
            scene_index: 4,
            start_frame: 300,
            end_frame: 449,
            start_seconds: 10.0,
            end_seconds: 15.0,
            metadata: json!({"detector": "content"}),
        },
    )
    .await
    .unwrap();
    let scene_rerun = upsert_video_scene(
        &pool,
        UpsertVideoSceneRequest {
            run_id: scene_run,
            video_id,
            scene_index: 4,
            start_frame: 300,
            end_frame: 450,
            start_seconds: 10.0,
            end_seconds: 15.03,
            metadata: json!({"detector": "content", "reviewed": true}),
        },
    )
    .await
    .unwrap();
    assert_eq!(scene.id, scene_rerun.id);
    assert_eq!(scene_rerun.end_frame, 450);

    let ocr_provenance = ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: "test".to_string(),
        model: "fixture-ocr".to_string(),
        model_version: "1".to_string(),
        input_hash: "sha256:visual-fixture".to_string(),
        config_hash: "sha256:ocr-config".to_string(),
        processing_config: json!({"sampling": "scene-aware"}),
    };
    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Ocr,
            provenance: ocr_provenance,
        },
    )
    .await
    .unwrap();

    let observation = upsert_visual_text_observation(
        &pool,
        UpsertVisualTextObservationRequest {
            run_id: ocr_run,
            video_id,
            observation_key: "frame-330-line-0".to_string(),
            text: "The Five Ways of Thomas Aquinas".to_string(),
            language: Some("en".to_string()),
            frame_index: Some(330),
            timestamp_seconds: Some(11.0),
            scene_index: Some(4),
            region: Some(VisualBoundingBox {
                x: 40,
                y: 25,
                width: 720,
                height: 100,
            }),
            confidence: Some(0.94),
            attributes: json!({"ocr.granularity": "line"}),
        },
    )
    .await
    .unwrap();
    let observation_rerun = upsert_visual_text_observation(
        &pool,
        UpsertVisualTextObservationRequest {
            run_id: ocr_run,
            video_id,
            observation_key: "frame-330-line-0".to_string(),
            text: "The Five Ways of Thomas Aquinas".to_string(),
            language: Some("en".to_string()),
            frame_index: Some(330),
            timestamp_seconds: Some(11.0),
            scene_index: Some(4),
            region: Some(VisualBoundingBox {
                x: 40,
                y: 25,
                width: 720,
                height: 100,
            }),
            confidence: Some(0.95),
            attributes: json!({"ocr.granularity": "line"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(observation.id, observation_rerun.id);
    assert_eq!(observation_rerun.confidence, Some(0.95));

    let track_request = UpsertVisualTextTrackRequest {
        run_id: ocr_run,
        video_id,
        track_key: "slide-five-ways".to_string(),
        text: "The Five Ways of Thomas Aquinas".to_string(),
        role: VisualTextRole::PresentationSlide,
        language: Some("en".to_string()),
        sample_count: 3,
        start_frame: Some(330),
        end_frame: Some(420),
        start_seconds: Some(11.0),
        end_seconds: Some(14.0),
        region: Some(VisualBoundingBox {
            x: 40,
            y: 25,
            width: 720,
            height: 100,
        }),
        metadata: json!({"semanticClassifier": "video-text-v1"}),
        observation_ids: vec![observation.id, observation.id],
        scene_ids: vec![scene.id, scene.id],
    };
    let track = upsert_visual_text_track(&pool, track_request.clone())
        .await
        .unwrap();
    let track_rerun = upsert_visual_text_track(&pool, track_request).await.unwrap();
    assert_eq!(track.id, track_rerun.id);

    let observation_links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM visual_text_track_observations WHERE track_id = $1",
    )
    .bind(track.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(observation_links, 1);
    let scene_links: i64 =
        sqlx::query_scalar("SELECT count(*) FROM visual_text_track_scenes WHERE track_id = $1")
            .bind(track.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(scene_links, 1);

    let scenes = list_video_scenes(&pool, video_id).await.unwrap();
    let tracks = list_visual_text_tracks(&pool, video_id).await.unwrap();
    assert_eq!(scenes.len(), 1);
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].role, VisualTextRole::PresentationSlide);

    let report = search_corpus(SearchRequest {
        database_url: database_url.clone(),
        query: "Five Ways Aquinas".to_string(),
        top_k: 5,
        mode: SearchMode::Fts,
        source_kind: Some(SourceKind::VisualOcr),
        video_id: Some(video_id),
        language: None,
        transcript_start_min: None,
        transcript_start_max: None,
        upload_date_from: None,
        upload_date_to: None,
        duration_min: None,
        duration_max: None,
        channel_query: None,
        title_query: None,
        category_query: None,
        tag_query: None,
        metadata_query: None,
        view_count_min: None,
        view_count_max: None,
    })
    .await
    .unwrap();
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].segment_id, track.id);
    assert_eq!(report.results[0].stream_id, ocr_run);
    assert_eq!(report.results[0].source_kind, "visual_ocr");
    assert_eq!(report.results[0].start_seconds, Some(11.0));
    assert!(report.results[0].text.contains("Thomas Aquinas"));

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}
