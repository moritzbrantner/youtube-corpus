use uuid::Uuid;
use youtube_corpus::{analyze_video, list_face_observations, MediaAnalysisConfig};

#[tokio::test]
#[ignore = "requires DATABASE_URL, FACE_VIDEO_FIXTURE, ONNX Runtime, ffmpeg, and model access"]
async fn face_analysis_executes_and_persists_evidence() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let Ok(media_path) = std::env::var("FACE_VIDEO_FIXTURE") else {
        eprintln!("FACE_VIDEO_FIXTURE unset; skipping");
        return;
    };

    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let video_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title, local_video_path)
         VALUES ($1, $2, $3, 'Face analysis dogfood fixture', $4)",
    )
    .bind(video_id)
    .bind(format!("face-analysis-{video_id}"))
    .bind(format!("https://youtube.test/face-analysis/{video_id}"))
    .bind(&media_path)
    .execute(&pool)
    .await
    .unwrap();

    let config = MediaAnalysisConfig {
        faces: true,
        face_interval_seconds: 1.0,
        face_track_similarity: 0.65,
        model_auto_download: true,
    };
    let report = analyze_video(&pool, video_id, media_path.as_ref(), &config)
        .await
        .unwrap();
    assert!(report.face_observations > 0, "expected at least one detected face");
    assert!(report.face_tracks > 0, "expected at least one face track");

    let observations = list_face_observations(&pool, video_id).await.unwrap();
    assert_eq!(observations.len(), report.face_observations);
    assert!(observations
        .iter()
        .all(|observation| observation.embedding_dimensions.is_some()));

    let track_links: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM face_track_observations fto
         JOIN face_tracks ft ON ft.id = fto.track_id
         WHERE ft.video_id = $1",
    )
    .bind(video_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(track_links as usize, report.face_observations);

    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
}
