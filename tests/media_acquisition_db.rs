//! Plan-driven media acquisition and release against Postgres with a stubbed yt-dlp
//! (#126, #120, #127).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use uuid::Uuid;
use youtube_corpus::detectors::{
    release_processed_media, run_video_plan, set_scope_settings, DetectorSetting, PlanRunRequest,
    ScopeKind, StageOutcome,
};
use youtube_corpus::media_acquisition::{ensure_media_with, lease, MediaAcquisition, MediaNeed};
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

struct Release {
    pool: sqlx::PgPool,
    database_url: String,
    dir: tempfile::TempDir,
    runner: Arc<FakeYtDlp>,
    client: YtDlpClient,
}

impl Release {
    async fn new() -> Option<Self> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let dir = tempfile::tempdir().unwrap();
        let audio = ffmpeg(
            dir.path(),
            "a.webm",
            &["-f", "lavfi", "-i", "sine=duration=1", "-c:a", "libopus"],
        )?;
        let video = ffmpeg(
            dir.path(),
            "v.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=2",
                "-shortest",
                "-pix_fmt",
                "yuv420p",
            ],
        )?;
        let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let runner = Arc::new(FakeYtDlp {
            audio,
            video,
            formats: Mutex::new(Vec::new()),
        });
        let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner.clone());
        Some(Self {
            pool,
            database_url,
            dir,
            runner,
            client,
        })
    }

    fn work_dir(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn downloads(&self) -> usize {
        self.runner.formats.lock().unwrap().len()
    }

    async fn remote_video(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO videos (id, source_url) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("https://www.youtube.com/watch?v={id}"))
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn acquire(&self, video: Uuid, need: MediaNeed) -> PathBuf {
        ensure_media_with(&self.pool, &self.client, video, need, &self.work_dir())
            .await
            .unwrap()
            .unwrap()
    }

    fn request(&self, corpus_id: Uuid, video_id: Uuid, keep_media: bool) -> PlanRunRequest {
        PlanRunRequest {
            database_url: self.database_url.clone(),
            corpus_id,
            video_id,
            work_dir: self.work_dir(),
            yt_dlp: YtDlpConfig::default(),
            model_dir: self.dir.path().join("models"),
            refresh: false,
            detectors: None,
            translation_provider: None,
            keep_media,
        }
    }

    async fn release(
        &self,
        video: Uuid,
        keep_media: bool,
    ) -> youtube_corpus::detectors::MediaCleanup {
        release_processed_media(&self.pool, &self.request(Uuid::nil(), video, keep_media))
            .await
            .unwrap()
    }

    async fn local_path(&self, video: Uuid) -> Option<String> {
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id = $1")
            .bind(video)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn deletes_media_after_every_enabled_media_stage_stored_output_and_reuses_it_after() {
    let Some(t) = Release::new().await else {
        eprintln!("DATABASE_URL or ffmpeg unavailable; skipping");
        return;
    };
    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Media release')")
        .bind(corpus)
        .bind(format!("media-release-{corpus}"))
        .execute(&t.pool)
        .await
        .unwrap();
    let video = t.remote_video().await;
    sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
        .bind(corpus)
        .bind(video)
        .execute(&t.pool)
        .await
        .unwrap();
    set_scope_settings(
        &t.pool,
        corpus,
        ScopeKind::Corpus,
        corpus,
        &[DetectorSetting {
            detector: "scenes".to_string(),
            enabled: true,
            target_languages: Vec::new(),
        }],
    )
    .await
    .unwrap();
    let path = t.acquire(video, MediaNeed::Video { max_height: 360 }).await;
    let info_json = path.with_extension("info.json");
    assert!(info_json.is_file());

    // Scenes has not stored output yet: the media stays.
    let early = t.release(video, false).await;
    assert!(early.deleted.is_empty());
    assert!(early.kept.unwrap().contains("scenes"), "kept for scenes");
    assert!(path.is_file());

    // A run holding the media blocks release.
    let held = lease(&t.pool, video).await.unwrap();
    let blocked = t.release(video, false).await;
    assert!(blocked.deleted.is_empty());
    assert!(blocked.kept.unwrap().contains("another run"));
    held.end().await.unwrap();

    // The plan run stores scenes, then deletes the media it no longer needs.
    let report = run_video_plan(t.request(corpus, video, false))
        .await
        .unwrap();
    let scenes = report
        .stages
        .iter()
        .find(|stage| stage.detector == "scenes")
        .unwrap();
    assert_eq!(scenes.outcome, StageOutcome::Completed, "{scenes:?}");
    assert_eq!(report.media.deleted, std::slice::from_ref(&path));
    assert!(report.media.kept.is_none());
    assert!(!path.exists());
    assert!(!info_json.exists());
    assert_eq!(t.local_path(video).await, None);
    let record = acquisition(&t.pool, video).await;
    assert!(record.deleted_at.is_some());
    assert!(record.deletion_reason.unwrap().contains("current output"));
    assert!(record.content_fnv1a64.is_some());
    assert_eq!(record.path, path);
    assert_eq!(record.format_id.as_deref(), Some("134+251-5"));
    assert_eq!(record.sha256.len(), 64);
    let scene_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM video_scenes WHERE video_id = $1")
            .bind(video)
            .fetch_one(&t.pool)
            .await
            .unwrap();
    assert!(scene_rows > 0);

    // The stored scenes stay current without the file: nothing reruns or downloads.
    let downloads = t.downloads();
    let again = run_video_plan(t.request(corpus, video, false))
        .await
        .unwrap();
    assert_eq!(again.stages[0].outcome, StageOutcome::Reused, "{again:?}");
    assert!(again.media.deleted.is_empty());
    assert_eq!(t.downloads(), downloads);

    // A later plan that needs the media acquires it again and keeps the history.
    let reacquired = t.acquire(video, MediaNeed::Video { max_height: 360 }).await;
    assert!(reacquired.is_file());
    assert_eq!(t.downloads(), downloads + 1);
    let record = acquisition(&t.pool, video).await;
    assert!(record.deleted_at.is_none());
    assert_eq!(record.superseded.len(), 1);
    assert!(record.superseded[0].deleted_at.is_some());
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn deletes_superseded_files_keeps_on_request_and_is_idempotent() {
    let Some(t) = Release::new().await else {
        eprintln!("DATABASE_URL or ffmpeg unavailable; skipping");
        return;
    };
    let video = t.remote_video().await;
    let audio = t.acquire(video, MediaNeed::Audio).await;
    let video_path = t.acquire(video, MediaNeed::Video { max_height: 360 }).await;
    assert_ne!(audio, video_path);
    assert!(audio.is_file());

    // Keeping media still deletes the superseded audio nothing points at.
    let kept = t.release(video, true).await;
    assert_eq!(kept.deleted, std::slice::from_ref(&audio));
    assert_eq!(kept.kept.as_deref(), Some("kept on request"));
    assert!(!audio.exists());
    assert!(video_path.is_file());
    let record = acquisition(&t.pool, video).await;
    assert!(record.deleted_at.is_none());
    assert!(record.superseded[0]
        .deletion_reason
        .as_deref()
        .unwrap()
        .contains("superseded"));

    // Outside any corpus nothing else needs it; a repeated release changes nothing.
    assert_eq!(
        t.release(video, false).await.deleted,
        std::slice::from_ref(&video_path)
    );
    assert!(!video_path.exists());
    let first = acquisition(&t.pool, video).await;
    assert!(t.release(video, false).await.deleted.is_empty());
    assert_eq!(acquisition(&t.pool, video).await, first);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn never_deletes_changed_symlinked_unrecorded_or_local_files() {
    let Some(t) = Release::new().await else {
        eprintln!("DATABASE_URL or ffmpeg unavailable; skipping");
        return;
    };
    // Changed bytes are not the recorded download.
    let changed = t.remote_video().await;
    let path = t.acquire(changed, MediaNeed::Audio).await;
    std::fs::write(&path, b"someone else's file").unwrap();
    let outcome = t.release(changed, false).await;
    assert!(outcome.deleted.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), b"someone else's file");
    assert!(acquisition(&t.pool, changed).await.deleted_at.is_none());

    // A symlink in place of the download, even to identical bytes, is left alone.
    let linked = t.remote_video().await;
    let path = t.acquire(linked, MediaNeed::Audio).await;
    let outside = t.dir.path().join("outside.webm");
    std::fs::rename(&path, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(t.release(linked, false).await.deleted.is_empty());
    assert!(outside.is_file());
    assert!(std::fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());

    // A record pointing outside the video's media directory is refused.
    let traversal = t.remote_video().await;
    let path = t.acquire(traversal, MediaNeed::Audio).await;
    let escaped = t.dir.path().join("work").join("elsewhere.webm");
    std::fs::copy(&path, &escaped).unwrap();
    let mut record = acquisition(&t.pool, traversal).await;
    record.path = path
        .parent()
        .unwrap()
        .join("..")
        .join("..")
        .join("elsewhere.webm");
    sqlx::query("UPDATE videos SET media_acquisition = $2, local_video_path = $3 WHERE id = $1")
        .bind(traversal)
        .bind(serde_json::to_value(&record).unwrap())
        .bind(record.path.to_string_lossy().into_owned())
        .execute(&t.pool)
        .await
        .unwrap();
    assert!(t.release(traversal, false).await.deleted.is_empty());
    assert!(escaped.is_file());

    // Media from before acquisitions were recorded has no proof it was downloaded.
    let unrecorded = t.remote_video().await;
    let legacy = t.dir.path().join("legacy.mp4");
    std::fs::write(&legacy, b"legacy").unwrap();
    sqlx::query("UPDATE videos SET local_video_path = $2 WHERE id = $1")
        .bind(unrecorded)
        .bind(legacy.to_string_lossy().into_owned())
        .execute(&t.pool)
        .await
        .unwrap();
    assert!(t.release(unrecorded, false).await.deleted.is_empty());
    assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy");

    // A local --input file is a read-only source.
    let local = Uuid::new_v4();
    let input = t.dir.path().join("lecture.mp4");
    std::fs::write(&input, b"local lecture").unwrap();
    sqlx::query("INSERT INTO videos (id, source_url, local_video_path) VALUES ($1, $2, $2)")
        .bind(local)
        .bind(input.to_string_lossy().into_owned())
        .execute(&t.pool)
        .await
        .unwrap();
    assert!(t.release(local, false).await.deleted.is_empty());
    assert_eq!(std::fs::read(&input).unwrap(), b"local lecture");
    assert_eq!(t.local_path(local).await.as_deref(), input.to_str());
}
