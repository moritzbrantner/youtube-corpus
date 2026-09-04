use serde_json::json;
use uuid::Uuid;
use youtube_corpus::config::CorpusSource;
use youtube_corpus::discovery::{
    claim_next_discovery, complete_discovery, enqueue_discovery, fail_discovery, list_frontier,
    record_ingest_discoveries, DiscoveryKind, DiscoveryMethod, DiscoveryPolicy, DiscoveryState,
    EnqueueDiscoveryRequest,
};
use youtube_corpus::ingest::{IngestItemReport, IngestReport};

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn discovery_frontier_is_idempotent_prioritized_and_expands_ingest_evidence() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();

    sqlx::query("DELETE FROM discovery_evidence")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM discovery_targets")
        .execute(&pool)
        .await
        .unwrap();

    let policy = DiscoveryPolicy::default();
    let low = enqueue_discovery(
        &pool,
        EnqueueDiscoveryRequest {
            kind: DiscoveryKind::Video,
            canonical_key: "youtube:video:low".to_string(),
            target_url: "https://www.youtube.com/watch?v=low".to_string(),
            source_video_id: None,
            parent_target_id: None,
            method: DiscoveryMethod::Manual,
            evidence: json!({"fixture": "low"}),
            depth: 1,
            confidence: 0.6,
            relevance: 0.4,
            novelty: 0.8,
        },
        &policy,
    )
    .await
    .unwrap();
    let high_request = EnqueueDiscoveryRequest {
        kind: DiscoveryKind::Video,
        canonical_key: "youtube:video:high".to_string(),
        target_url: "https://www.youtube.com/watch?v=high".to_string(),
        source_video_id: None,
        parent_target_id: None,
        method: DiscoveryMethod::Manual,
        evidence: json!({"fixture": "high"}),
        depth: 1,
        confidence: 1.0,
        relevance: 1.0,
        novelty: 1.0,
    };
    let high = enqueue_discovery(&pool, high_request.clone(), &policy)
        .await
        .unwrap();
    let duplicate = enqueue_discovery(&pool, high_request, &policy)
        .await
        .unwrap();
    assert_eq!(high.id, duplicate.id);
    assert!(high.priority > low.priority);

    let claimed = claim_next_discovery(&pool, "test-runner", Some("workflow-1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, high.id);
    assert_eq!(claimed.state, DiscoveryState::Claimed);
    assert_eq!(claimed.attempt_count, 1);
    assert_eq!(claimed.workflow_run_id.as_deref(), Some("workflow-1"));
    assert!(claimed.claim_expires_at.is_some());

    sqlx::query(
        "UPDATE discovery_targets SET claim_expires_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(claimed.id)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed = claim_next_discovery(&pool, "recovery-runner", Some("workflow-1-retry"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, claimed.id);
    assert_eq!(reclaimed.attempt_count, 2);
    assert_eq!(reclaimed.claimed_by.as_deref(), Some("recovery-runner"));
    assert_eq!(
        reclaimed.workflow_run_id.as_deref(),
        Some("workflow-1-retry")
    );

    assert!(
        complete_discovery(
            &pool,
            claimed.id,
            claimed.attempt_count,
            Some("workflow-1")
        )
        .await
        .is_err()
    );
    assert!(
        fail_discovery(
            &pool,
            claimed.id,
            claimed.attempt_count,
            Some("workflow-1"),
            "stale worker"
        )
        .await
        .is_err()
    );
    let completed = complete_discovery(
        &pool,
        reclaimed.id,
        reclaimed.attempt_count,
        Some("workflow-1-retry"),
    )
    .await
    .unwrap();
    assert_eq!(completed.state, DiscoveryState::Completed);
    let repeated = complete_discovery(
        &pool,
        reclaimed.id,
        reclaimed.attempt_count,
        Some("workflow-1-retry"),
    )
    .await
    .unwrap();
    assert_eq!(repeated.state, DiscoveryState::Completed);

    let recursive_youtube_id = format!("recursive-{}", Uuid::new_v4());
    let recursive_source_url = format!("https://www.youtube.com/watch?v={recursive_youtube_id}");
    let recursive = enqueue_discovery(
        &pool,
        EnqueueDiscoveryRequest {
            kind: DiscoveryKind::Video,
            canonical_key: format!("youtube:video:{recursive_youtube_id}"),
            target_url: recursive_source_url.clone(),
            source_video_id: None,
            parent_target_id: None,
            method: DiscoveryMethod::Manual,
            evidence: json!({"fixture": "recursive"}),
            depth: 2,
            confidence: 1.0,
            relevance: 1.0,
            novelty: 1.0,
        },
        &policy,
    )
    .await
    .unwrap();
    let recursive_claim = claim_next_discovery(&pool, "test-runner", Some("workflow-2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recursive_claim.id, recursive.id);
    assert_eq!(recursive_claim.depth, 2);

    let failed_report = IngestReport {
        workflow: "youtube_corpus_ingest".to_string(),
        run_id: Uuid::new_v4(),
        videos_seen: 1,
        videos_indexed: 0,
        segments_indexed: 0,
        items: vec![IngestItemReport {
            video_id: None,
            source_url: recursive_source_url.clone(),
            title: None,
            status: "failed".to_string(),
            streams_indexed: 0,
            segments_indexed: 0,
            message: Some("retryable fixture failure".to_string()),
        }],
    };
    record_ingest_discoveries(
        &pool,
        &CorpusSource::YoutubeUrl {
            url: recursive_source_url.clone(),
        },
        &failed_report,
        &policy,
    )
    .await
    .unwrap();

    let state_after_failed_attempt: String =
        sqlx::query_scalar("SELECT state FROM discovery_targets WHERE id = $1")
            .bind(recursive.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let depth_after_failed_attempt: i32 =
        sqlx::query_scalar("SELECT depth FROM discovery_targets WHERE id = $1")
            .bind(recursive.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state_after_failed_attempt, "claimed");
    assert_eq!(depth_after_failed_attempt, 2);

    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, description, channel_url)
         VALUES ($1, $2, $3, 'Discovery fixture', $4, $5)",
    )
    .bind(video_id)
    .bind(&recursive_youtube_id)
    .bind(&recursive_source_url)
    .bind("Related: https://youtu.be/child_123")
    .bind("https://www.youtube.com/@FixtureChannel")
    .execute(&pool)
    .await
    .unwrap();

    let report = IngestReport {
        workflow: "youtube_corpus_ingest".to_string(),
        run_id: Uuid::new_v4(),
        videos_seen: 1,
        videos_indexed: 1,
        segments_indexed: 0,
        items: vec![IngestItemReport {
            video_id: Some(video_id),
            source_url: recursive_source_url.clone(),
            title: Some("Discovery fixture".to_string()),
            status: "no_transcript".to_string(),
            streams_indexed: 0,
            segments_indexed: 0,
            message: None,
        }],
    };
    record_ingest_discoveries(
        &pool,
        &CorpusSource::YoutubeUrl {
            url: recursive_source_url,
        },
        &report,
        &policy,
    )
    .await
    .unwrap();

    let preserved_depth: i32 =
        sqlx::query_scalar("SELECT depth FROM discovery_targets WHERE id = $1")
            .bind(recursive.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let state_before_complete: String =
        sqlx::query_scalar("SELECT state FROM discovery_targets WHERE id = $1")
            .bind(recursive.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(preserved_depth, 2);
    assert_eq!(state_before_complete, "claimed");

    let frontier = list_frontier(&pool, 20).await.unwrap();
    let child = frontier
        .iter()
        .find(|target| target.canonical_key == "youtube:video:child_123")
        .expect("description child should enter the frontier");
    assert_eq!(child.depth, 3);
    assert!(frontier
        .iter()
        .any(|target| target.canonical_key == "youtube:channel:@fixturechannel"));

    let recursive_completed = complete_discovery(
        &pool,
        recursive_claim.id,
        recursive_claim.attempt_count,
        Some("workflow-2"),
    )
    .await
    .unwrap();
    assert_eq!(recursive_completed.state, DiscoveryState::Completed);

    let target_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM discovery_targets WHERE canonical_key = 'youtube:video:high'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let evidence_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM discovery_evidence evidence
         JOIN discovery_targets target ON target.id = evidence.target_id
         WHERE target.canonical_key = 'youtube:video:high'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(target_count, 1);
    assert_eq!(evidence_count, 1);

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM discovery_evidence")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM discovery_targets")
        .execute(&pool)
        .await
        .unwrap();
}
