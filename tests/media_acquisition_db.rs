//! Plan-driven media acquisition against Postgres with a stubbed yt-dlp (#126, #120).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use uuid::Uuid;
use youtube_corpus::media_acquisition::{ensure_media_with, MediaAcquisition, MediaNeed};
use youtube_corpus::yt_dlp::{
    CommandFuture, CommandRunner, CommandRunnerOutput, YtDlpClient, YtDlpCommandSpec,
};
use youtube_corpus::YtDlpConfig;

/// "Downloads" an audio fixture for audio-only selectors and an audio+video one otherwise.
struct FakeYtDlp {
    audio: PathBuf,
    video: PathBuf,
    formats: Mutex<Vec<String>>,
}

impl CommandRunner for FakeYtDlp {
    fn run<'a>(&'a self, spec: YtDlpCommandSpec) -> CommandFuture<'a> {
        Box::pin(async move {
            let arg = |name: &str| {
                spec.args[spec.args.iter().position(|a| a == name).unwrap() + 1].clone()
            };
            let format = arg("--format");
            let base = arg("-o").replace("%(id)s", "fake").replace(".%(ext)s", "");
            let (fixture, ext, info) = if format.starts_with("ba") {
                (
                    &self.audio,
                    "webm",
                    serde_json::json!({"format_id": "251-5", "acodec": "opus", "vcodec": "none", "language": "en"}),
                )
            } else {
                (
                    &self.video,
                    "mp4",
                    serde_json::json!({"format_id": "134+251-5", "requested_formats": [
                        {"format_id": "134", "acodec": "none", "vcodec": "avc1", "height": 240},
                        {"format_id": "251-5", "acodec": "opus", "vcodec": "none", "language": "en"}]}),
                )
            };
            let media = format!("{base}.{ext}");
            std::fs::copy(fixture, &media).unwrap();
            std::fs::write(format!("{base}.info.json"), info.to_string()).unwrap();
            self.formats.lock().unwrap().push(format);
            Ok(CommandRunnerOutput {
                stdout: format!("{media}\n").into_bytes(),
                stderr: Vec::new(),
                exit_status: Some(0),
            })
        })
    }
}

fn ffmpeg(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
    let path = dir.join(name);
    std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .arg(&path)
        .status()
        .ok()?
        .success()
        .then_some(path)
}

async fn acquisition(pool: &sqlx::PgPool, video: Uuid) -> MediaAcquisition {
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT media_acquisition FROM videos WHERE id = $1")
            .bind(video)
            .fetch_one(pool)
            .await
            .unwrap();
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn acquires_only_what_the_plan_needs_and_reacquires_removed_media() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (Some(audio), Some(video)) = (
        ffmpeg(
            dir.path(),
            "a.webm",
            &["-f", "lavfi", "-i", "sine=duration=1", "-c:a", "libopus"],
        ),
        ffmpeg(
            dir.path(),
            "v.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=1",
                "-shortest",
                "-pix_fmt",
                "yuv420p",
            ],
        ),
    ) else {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let runner = Arc::new(FakeYtDlp {
        audio,
        video,
        formats: Mutex::new(Vec::new()),
    });
    let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner.clone());
    let work_dir = dir.path().join("work");
    let video_id = Uuid::new_v4();
    sqlx::query("INSERT INTO videos (id, source_url) VALUES ($1, $2)")
        .bind(video_id)
        .bind(format!("https://www.youtube.com/watch?v={video_id}"))
        .execute(&pool)
        .await
        .unwrap();
    let calls = || runner.formats.lock().unwrap().clone();

    // Metadata/captions-only plans download nothing.
    let none = ensure_media_with(&pool, &client, video_id, MediaNeed::Nothing, &work_dir)
        .await
        .unwrap();
    assert!(none.is_none());
    assert!(calls().is_empty());

    // ASR-only: an audio stream, recorded with format id, language, size and digest.
    let audio_path = ensure_media_with(&pool, &client, video_id, MediaNeed::Audio, &work_dir)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls(), ["ba/wa*"]);
    let recorded = acquisition(&pool, video_id).await;
    assert_eq!(recorded.path, audio_path);
    assert_eq!(recorded.format_id.as_deref(), Some("251-5"));
    assert_eq!(recorded.audio_language.as_deref(), Some("en"));
    assert_eq!(
        recorded.bytes,
        std::fs::metadata(&audio_path).unwrap().len()
    );
    let stored: Option<String> =
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id = $1")
            .bind(video_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), audio_path.to_str());

    // A second audio consumer reuses the unchanged file.
    ensure_media_with(&pool, &client, video_id, MediaNeed::Audio, &work_dir)
        .await
        .unwrap();
    assert_eq!(calls().len(), 1);

    // Enabling a video stage later acquires video once; it then also serves audio.
    let video_path = ensure_media_with(
        &pool,
        &client,
        video_id,
        MediaNeed::Video { max_height: 360 },
        &work_dir,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(calls()[1], "bv*[height<=360]+ba/b[height<=360]/wv*+ba/w");
    let recorded = acquisition(&pool, video_id).await;
    assert_eq!(recorded.format_id.as_deref(), Some("134+251-5"));
    assert_eq!(recorded.audio_format_id.as_deref(), Some("251-5"));
    assert_eq!(recorded.height, Some(240));
    for need in [MediaNeed::Audio, MediaNeed::Video { max_height: 360 }] {
        ensure_media_with(&pool, &client, video_id, need, &work_dir)
            .await
            .unwrap();
    }
    assert_eq!(calls().len(), 2);

    // Removed media is acquired again instead of looking cached.
    std::fs::remove_file(&video_path).unwrap();
    let again = ensure_media_with(&pool, &client, video_id, MediaNeed::Audio, &work_dir)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls().len(), 3);
    assert!(again.is_file());

    // Local input files are a read-only source: never acquired.
    let local = Uuid::new_v4();
    let local_path = dir.path().join("lecture.mp4");
    std::fs::write(&local_path, b"local").unwrap();
    sqlx::query("INSERT INTO videos (id, source_url, local_video_path) VALUES ($1, $2, $2)")
        .bind(local)
        .bind(local_path.to_string_lossy().into_owned())
        .execute(&pool)
        .await
        .unwrap();
    let used = ensure_media_with(
        &pool,
        &client,
        local,
        MediaNeed::Video { max_height: 720 },
        &work_dir,
    )
    .await
    .unwrap();
    assert_eq!(used, Some(local_path));
    assert_eq!(calls().len(), 3);
}
