#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn migrations_run_against_postgres() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_name = 'transcript_segments'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);

    let transcriber_timeout_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.columns
         WHERE table_name = 'corpus_subscriptions'
           AND column_name = 'transcriber_timeout_seconds'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(transcriber_timeout_columns, 1);

    let corpus_tables: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.tables
         WHERE table_name IN ('corpora', 'corpus_source_memberships', 'corpus_video_memberships')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(corpus_tables, 3);

    let provenance_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.columns
         WHERE (table_name = 'videos' AND column_name IN (
                  'metadata_checksum', 'metadata_retrieved_at', 'metadata_processing_revision'
                ))
            OR (table_name = 'transcript_streams' AND column_name IN (
                  'content_checksum', 'retrieved_at', 'processing_revision', 'processing_config'
                ))",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(provenance_columns, 7);

    let research_quality_tables: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.tables
         WHERE table_name IN (
           'transcript_stream_quality',
           'preferred_transcript_streams',
           'research_annotations',
           'retrieval_evaluation_cases',
           'retrieval_evaluation_targets',
           'retrieval_evaluation_runs'
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(research_quality_tables, 6);

    let multimodal_tables: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.tables
         WHERE table_name IN (
           'media_processing_runs',
           'face_observations',
           'face_tracks',
           'face_track_observations',
           'voice_observations',
           'voice_tracks',
           'voice_track_observations',
           'face_clusters',
           'face_cluster_members',
           'voice_clusters',
           'voice_cluster_members',
           'people',
           'person_face_clusters',
           'person_voice_clusters'
         )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(multimodal_tables, 14);

    let annotation_checksum: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM information_schema.columns
         WHERE table_name = 'research_annotations'
           AND column_name = 'content_checksum'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(annotation_checksum, 1);

    let default_corpus: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM corpora WHERE slug = 'default' AND is_default = true",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(default_corpus, 1);
}
