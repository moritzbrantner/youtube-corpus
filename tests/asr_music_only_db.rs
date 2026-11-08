//! Opt-in real-audio acceptance for native ASR through corpus ingest (audio-analysis #233).
//!
//! Music-only audio must not store fabricated transcript text, while speech is kept. Needs a
//! pgvector Postgres (`DATABASE_URL`), the cached `openai/whisper-small` model, a cached Silero
//! VAD model that loads with ONNX Runtime >= 1.24 (for example the Torch Hub copy and
//! `ORT_DYLIB_PATH`), and two local files: `YOUTUBE_CORPUS_ASR_MUSIC_ONLY` (for example the
//! CC BY Blender 3.6 LTS reel, first 85 s) and `YOUTUBE_CORPUS_ASR_SPEECH` (English speech
//! containing "Blender").

mod fixture_embedder;

use std::path::PathBuf;

use serde_json::Value;
use youtube_corpus::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use youtube_corpus::ingest::{ingest_corpus, IngestItemReport, IngestRequest};

fn request(database_url: &str, path: PathBuf, work_dir: PathBuf) -> IngestRequest {
    IngestRequest {
        run_id: None,
        database_url: database_url.to_string(),
        source: CorpusSource::LocalFile { path },
        work_dir,
        caption: CaptionConfig {
            enabled: false,
            ..CaptionConfig::default()
        },
        yt_dlp: YtDlpConfig::default(),
        asr_enabled: true,
        transcriber_command: None,
        transcriber_args: Vec::new(),
        transcriber_timeout_seconds: None,
        max_items: None,
        title_contains: None,
        title_excludes: Vec::new(),
        duration_min: None,
        duration_max: None,
        migrate: true,
        keep_media: false,
    }
}

async fn ingest_one(database_url: &str, path: PathBuf) -> IngestItemReport {
    let work_dir = tempfile::tempdir().unwrap();
    let report = ingest_corpus(request(database_url, path, work_dir.path().to_path_buf()))
        .await
        .unwrap();
    assert_eq!(report.items.len(), 1);
    report.items.into_iter().next().unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, cached Whisper small and Silero VAD models, ONNX Runtime and local audio"]
async fn music_only_audio_stores_no_asr_text_and_speech_is_kept() {
    let (Ok(database_url), Some(music), Some(speech)) = (
        std::env::var("DATABASE_URL"),
        std::env::var_os("YOUTUBE_CORPUS_ASR_MUSIC_ONLY"),
        std::env::var_os("YOUTUBE_CORPUS_ASR_SPEECH"),
    ) else {
        eprintln!("DATABASE_URL or the audio files are unset; skipping");
        return;
    };
    fixture_embedder::install_fixture_embedder();
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();

    let music = ingest_one(&database_url, PathBuf::from(music)).await;
    assert_eq!(music.status, "no_transcript", "{music:?}");
    assert_eq!(music.segments_indexed, 0);
    let music_id = music.video_id.expect("the video row is stored");
    let stored: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM transcript_streams WHERE video_id = $1),
                (SELECT count(*) FROM transcript_segments WHERE video_id = $1)",
    )
    .bind(music_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, (0, 0), "music-only audio stored transcript text");

    let speech = ingest_one(&database_url, PathBuf::from(speech)).await;
    assert_eq!(speech.status, "indexed", "{speech:?}");
    let speech_id = speech.video_id.expect("the video row is stored");
    let (text, config): (String, Value) = sqlx::query_as(
        "SELECT full_text, processing_config FROM transcript_streams
         WHERE video_id = $1 AND source_kind = 'asr'",
    )
    .bind(speech_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(text.contains("Blender"), "speech lost: {text:?}");
    let run = &config["asrRun"];
    assert_eq!(run["selectedVad"]["method"], "silero", "{run}");
    assert_eq!(
        run["speechEvidence"]["vad"], "modelSpeechCandidate",
        "{run}"
    );
}
