//! #43: re-ingesting producer output with the same provenance is idempotent, while a changed
//! producer config creates a separate, distinguishable evidence set linked to the same media span.

use serde_json::json;
use uuid::Uuid;
use youtube_corpus::{
    begin_visual_processing_run, export_media_evidence_batch, list_video_scenes,
    upsert_video_scene, BeginVisualProcessingRunRequest, ProcessingProvenance,
    UpsertVideoSceneRequest, VisualProcessingKind,
};

fn scene_provenance(config_hash: &str, threshold: f64) -> ProcessingProvenance {
    ProcessingProvenance {
        processor: "visual-analysis".to_string(),
        processor_version: "rev-0123abc".to_string(),
        model: "scenedetect-core-content".to_string(),
        model_version: "0.1.0".to_string(),
        input_hash: "sha256:visual-provenance-fixture".to_string(),
        config_hash: config_hash.to_string(),
        processing_config: json!({"threshold": threshold}),
    }
}

async fn ingest_scene(
    pool: &sqlx::PgPool,
    video_id: Uuid,
    provenance: ProcessingProvenance,
) -> Uuid {
    let run_id = begin_visual_processing_run(
        pool,
        BeginVisualProcessingRunRequest {
            video_id,
            kind: VisualProcessingKind::Scene,
            provenance,
        },
    )
    .await
    .unwrap();
    upsert_video_scene(
        pool,
        UpsertVideoSceneRequest {
            run_id,
            video_id,
            scene_index: 0,
            start_frame: 0,
            end_frame: 299,
            start_seconds: 0.0,
            end_seconds: 10.0,
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    run_id
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn changed_producer_config_creates_a_distinguishable_evidence_set() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title)
         VALUES ($1, $2, $3, 'Visual provenance fixture')",
    )
    .bind(video_id)
    .bind(format!("provenance-{video_id}"))
    .bind(format!("https://youtube.test/{video_id}"))
    .execute(&pool)
    .await
    .unwrap();

    // Same producer, same config, ingested twice: one record set.
    let first = ingest_scene(&pool, video_id, scene_provenance("sha256:config-a", 27.0)).await;
    let again = ingest_scene(&pool, video_id, scene_provenance("sha256:config-a", 27.0)).await;
    assert_eq!(first, again);
    let scenes = list_video_scenes(&pool, video_id).await.unwrap();
    assert_eq!(scenes.len(), 1);

    // Changed config: a second set, not an overwrite of the first.
    let changed = ingest_scene(&pool, video_id, scene_provenance("sha256:config-b", 30.0)).await;
    assert_ne!(first, changed);
    let scenes = list_video_scenes(&pool, video_id).await.unwrap();
    assert_eq!(scenes.len(), 2);
    let mut run_ids: Vec<Uuid> = scenes.iter().map(|scene| scene.run_id).collect();
    run_ids.sort();
    let mut expected = vec![first, changed];
    expected.sort();
    assert_eq!(run_ids, expected);
    for scene in &scenes {
        assert_eq!(scene.video_id, video_id);
        assert_eq!((scene.start_seconds, scene.end_seconds), (0.0, 10.0));
    }

    // Retrieval exposes each set with its exact producer provenance.
    let batch = export_media_evidence_batch(&pool, video_id, "test-revision")
        .await
        .unwrap();
    assert_eq!(batch.scenes.len(), 2);
    let mut configs: Vec<(String, String, String, serde_json::Value)> = batch
        .scenes
        .iter()
        .map(|scene| {
            assert_eq!(scene.provenance.processor, "visual-analysis");
            assert_eq!(scene.provenance.processor_version, "rev-0123abc");
            assert_eq!(scene.provenance.model, "scenedetect-core-content");
            assert_eq!(scene.provenance.model_version, "0.1.0");
            (
                scene.provenance.run_id.clone(),
                scene.provenance.config_hash.clone(),
                scene.provenance.input_hash.clone(),
                scene.provenance.processing_config.clone(),
            )
        })
        .collect();
    configs.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(configs[0].1, "sha256:config-a");
    assert_eq!(configs[0].0, first.to_string());
    assert_eq!(configs[0].3, json!({"threshold": 27.0}));
    assert_eq!(configs[1].1, "sha256:config-b");
    assert_eq!(configs[1].0, changed.to_string());
    assert_eq!(configs[1].3, json!({"threshold": 30.0}));
    assert!(configs
        .iter()
        .all(|c| c.2 == "sha256:visual-provenance-fixture"));

    // Deleting the video cascades to its runs and scenes, so reruns on a shared database stay clean.
    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}
