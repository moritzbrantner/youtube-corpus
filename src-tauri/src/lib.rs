use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::PathBuf;
use uuid::Uuid;
use youtube_corpus::search::SearchResult;
use youtube_corpus::status::{ListVideosRequest, VideoStatus};
use youtube_corpus::{
    add_subscription, ingest_corpus, search_corpus, CaptionConfig, CorpusSource, IngestReport,
    SearchMode, SearchRequest, SourceKind, Subscription,
};
use youtube_corpus::config::YtDlpConfig;
use youtube_corpus::subscriptions::{AddSubscriptionRequest, SubscriptionSourceKind};

const REQUIRED_TABLES: &[&str] = &[
    "videos",
    "transcript_streams",
    "transcript_segments",
    "ingest_runs",
    "corpus_subscriptions",
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseStatus {
    configured: bool,
    database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchTranscriptsInput {
    database_url: Option<String>,
    query: String,
    mode: SearchMode,
    top_k: i64,
    source_kind: Option<SourceKind>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusStatusInput {
    database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptContextInput {
    database_url: Option<String>,
    segment_id: String,
    before: Option<i64>,
    after: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListDownloadedFilesInput {
    database_url: Option<String>,
    downloaded_only: Option<bool>,
    parsed_only: Option<bool>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddSourceInput {
    database_url: Option<String>,
    source_kind: AddSourceKind,
    source_url: String,
    name: Option<String>,
    work_dir: Option<String>,
    caption_languages: Option<Vec<String>>,
    captions_enabled: Option<bool>,
    auto_captions_enabled: Option<bool>,
    yt_dlp_args: Option<Vec<String>>,
    asr_enabled: Option<bool>,
    transcriber_command: Option<String>,
    transcriber_args: Option<Vec<String>>,
    max_items: Option<u64>,
    title_contains: Option<String>,
    title_excludes: Option<Vec<String>>,
    duration_min: Option<f64>,
    duration_max: Option<f64>,
    migrate: Option<bool>,
    subscribe: Option<bool>,
    ingest_now: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AddSourceKind {
    Video,
    Channel,
    Playlist,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AddSourceReport {
    source_kind: String,
    source_url: String,
    subscription: Option<Subscription>,
    ingest: Option<IngestReport>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptContextReport {
    #[serde(rename = "match")]
    match_segment: SearchResult,
    segments: Vec<TranscriptContextSegment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptContextSegment {
    segment_id: Uuid,
    video_id: Uuid,
    stream_id: Uuid,
    segment_index: i64,
    source_kind: SourceKind,
    language: Option<String>,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    text: String,
    is_match: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CorpusStatus {
    configured: bool,
    database_url: Option<String>,
    reachable: bool,
    schema_ready: bool,
    message: Option<String>,
    stats: Option<CorpusStats>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CorpusStats {
    videos: i64,
    streams: i64,
    segments: i64,
    subscriptions: i64,
    enabled_subscriptions: i64,
    source_kinds: Vec<SourceKindStat>,
    languages: Vec<LanguageStat>,
    last_ingest_run: Option<LastIngestRun>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceKindStat {
    source_kind: SourceKind,
    streams: i64,
    segments: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageStat {
    language: String,
    segments: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LastIngestRun {
    id: String,
    source_url: Option<String>,
    status: String,
    videos_indexed: i64,
    segments_indexed: i64,
    created_at: String,
}

#[tauri::command]
fn database_status() -> DatabaseStatus {
    let database_url = std::env::var("DATABASE_URL").ok();
    DatabaseStatus {
        configured: database_url.is_some(),
        database_url,
    }
}

#[tauri::command]
async fn corpus_status(input: CorpusStatusInput) -> Result<CorpusStatus, String> {
    let database_url = match resolve_database_url(input.database_url) {
        Some(database_url) => database_url,
        None => {
            return Ok(CorpusStatus {
                configured: false,
                database_url: None,
                reachable: false,
                schema_ready: false,
                message: Some("DATABASE_URL is required.".to_string()),
                stats: None,
            });
        }
    };

    let pool = match youtube_corpus::db::connect(&database_url).await {
        Ok(pool) => pool,
        Err(error) => {
            return Ok(CorpusStatus {
                configured: true,
                database_url: Some(database_url),
                reachable: false,
                schema_ready: false,
                message: Some(error.to_string()),
                stats: None,
            });
        }
    };

    let schema_ready = required_tables_exist(&pool).await?;
    if !schema_ready {
        return Ok(CorpusStatus {
            configured: true,
            database_url: Some(database_url),
            reachable: true,
            schema_ready: false,
            message: Some(
                "Database is reachable but migrations have not been applied.".to_string(),
            ),
            stats: None,
        });
    }

    let stats = load_corpus_stats(&pool).await?;
    Ok(CorpusStatus {
        configured: true,
        database_url: Some(database_url),
        reachable: true,
        schema_ready: true,
        message: None,
        stats: Some(stats),
    })
}

#[tauri::command]
async fn search_transcripts(
    input: SearchTranscriptsInput,
) -> Result<youtube_corpus::SearchReport, String> {
    let database_url = match resolve_database_url(input.database_url) {
        Some(database_url) => database_url,
        None => return Err("DATABASE_URL is required.".to_string()),
    };

    if input.query.trim().is_empty() {
        return Err("query is required.".to_string());
    }

    if input.top_k <= 0 {
        return Err("topK must be positive.".to_string());
    }

    search_corpus(SearchRequest {
        database_url,
        query: input.query,
        top_k: input.top_k,
        mode: input.mode,
        source_kind: input.source_kind,
        video_id: None,
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn transcript_context(
    input: TranscriptContextInput,
) -> Result<TranscriptContextReport, String> {
    let database_url = match resolve_database_url(input.database_url) {
        Some(database_url) => database_url,
        None => return Err("DATABASE_URL is required.".to_string()),
    };

    let segment_id = input.segment_id.trim();
    if segment_id.is_empty() {
        return Err("segmentId is required.".to_string());
    }
    let segment_id = Uuid::parse_str(segment_id).map_err(|error| error.to_string())?;

    let before = input.before.unwrap_or(4).clamp(0, 20);
    let after = input.after.unwrap_or(6).clamp(0, 20);
    let pool = youtube_corpus::db::connect(&database_url)
        .await
        .map_err(|error| error.to_string())?;

    let match_row = sqlx::query(
        "SELECT s.id, s.video_id, s.stream_id, st.source_kind, s.language,
                s.segment_index, s.start_seconds, s.end_seconds, s.text,
                v.source_url, v.title,
                0::float8 AS score,
                0::float8 AS fts_score,
                0::float8 AS semantic_score
         FROM transcript_segments s
         JOIN transcript_streams st ON st.id = s.stream_id
         JOIN videos v ON v.id = s.video_id
         WHERE s.id = $1",
    )
    .bind(segment_id)
    .fetch_optional(&pool)
    .await
    .map_err(|error| error.to_string())?;

    let match_row = match match_row {
        Some(row) => row,
        None => return Err("Segment not found.".to_string()),
    };

    let stream_id: Uuid = match_row
        .try_get("stream_id")
        .map_err(|error| error.to_string())?;
    let segment_index: i64 = match_row
        .try_get("segment_index")
        .map_err(|error| error.to_string())?;
    let start_index = segment_index.saturating_sub(before).max(0);
    let end_index = segment_index.saturating_add(after);

    let match_segment = SearchResult {
        segment_id: match_row
            .try_get::<Uuid, _>("id")
            .map_err(|error| error.to_string())?,
        video_id: match_row
            .try_get::<Uuid, _>("video_id")
            .map_err(|error| error.to_string())?,
        stream_id,
        source_kind: match_row
            .try_get::<String, _>("source_kind")
            .map_err(|error| error.to_string())?,
        language: match_row
            .try_get::<Option<String>, _>("language")
            .map_err(|error| error.to_string())?,
        start_seconds: match_row
            .try_get::<Option<f64>, _>("start_seconds")
            .map_err(|error| error.to_string())?,
        end_seconds: match_row
            .try_get::<Option<f64>, _>("end_seconds")
            .map_err(|error| error.to_string())?,
        text: match_row
            .try_get::<String, _>("text")
            .map_err(|error| error.to_string())?,
        source_url: match_row
            .try_get::<String, _>("source_url")
            .map_err(|error| error.to_string())?,
        title: match_row
            .try_get::<Option<String>, _>("title")
            .map_err(|error| error.to_string())?,
        score: match_row
            .try_get::<f64, _>("score")
            .map_err(|error| error.to_string())?,
        fts_score: match_row
            .try_get::<f64, _>("fts_score")
            .map_err(|error| error.to_string())?,
        semantic_score: match_row
            .try_get::<f64, _>("semantic_score")
            .map_err(|error| error.to_string())?,
    };

    let context_rows = sqlx::query(
        "SELECT s.id, s.video_id, s.stream_id, st.source_kind, s.language,
                s.segment_index, s.start_seconds, s.end_seconds, s.text,
                (s.id = $1) AS is_match
         FROM transcript_segments s
         JOIN transcript_streams st ON st.id = s.stream_id
         WHERE s.stream_id = $2
           AND s.segment_index BETWEEN $3 AND $4
         ORDER BY s.segment_index ASC",
    )
    .bind(segment_id)
    .bind(stream_id)
    .bind(start_index)
    .bind(end_index)
    .fetch_all(&pool)
    .await
    .map_err(|error| error.to_string())?;

    let segments = context_rows
        .into_iter()
        .map(|row| {
            let source_kind: String = row.try_get("source_kind")?;
            Ok(TranscriptContextSegment {
                segment_id: row.try_get("id")?,
                video_id: row.try_get("video_id")?,
                stream_id: row.try_get("stream_id")?,
                segment_index: row.try_get("segment_index")?,
                source_kind: parse_source_kind(&source_kind)?,
                language: row.try_get("language")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                text: row.try_get("text")?,
                is_match: row.try_get("is_match")?,
            })
        })
        .collect::<Result<Vec<_>, StatusQueryError>>()
        .map_err(|error| error.to_string())?;

    Ok(TranscriptContextReport {
        match_segment,
        segments,
    })
}

#[tauri::command]
async fn downloaded_files(input: ListDownloadedFilesInput) -> Result<Vec<VideoStatus>, String> {
    let database_url = match resolve_database_url(input.database_url) {
        Some(database_url) => database_url,
        None => return Err("DATABASE_URL is required.".to_string()),
    };

    let downloaded_only = input.downloaded_only.unwrap_or(true);
    let limit = input.limit;
    let mut videos = youtube_corpus::status::list_videos(ListVideosRequest {
        database_url,
        downloaded_only: false,
        parsed_only: input.parsed_only.unwrap_or(false),
        limit: if downloaded_only { None } else { limit },
        migrate: false,
    })
    .await
    .map_err(|error| error.to_string())?;

    if downloaded_only {
        videos.retain(|video| video.media_downloaded || video.caption_files_downloaded);
        if let Some(limit) = limit.and_then(|value| usize::try_from(value.max(0)).ok()) {
            videos.truncate(limit);
        }
    }

    Ok(videos)
}

#[tauri::command]
async fn add_source(input: AddSourceInput) -> Result<AddSourceReport, String> {
    let database_url = match resolve_database_url(input.database_url.clone()) {
        Some(database_url) => database_url,
        None => return Err("DATABASE_URL is required.".to_string()),
    };

    let source_url = input.source_url.trim().to_string();
    if source_url.is_empty() {
        return Err("Source URL is required.".to_string());
    }

    let work_dir = input
        .work_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("use-case-output/youtube-corpus"));
    let caption = CaptionConfig {
        enabled: input.captions_enabled.unwrap_or(true),
        include_auto_captions: input.auto_captions_enabled.unwrap_or(true),
        languages: normalized_languages(input.caption_languages),
    };
    let transcriber_command = input
        .transcriber_command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let transcriber_args = input.transcriber_args.unwrap_or_default();
    let yt_dlp = YtDlpConfig {
        args: input
            .yt_dlp_args
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
    };
    let title_contains = input
        .title_contains
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let title_excludes = input
        .title_excludes
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let migrate = input.migrate.unwrap_or(false);
    let asr_enabled = input.asr_enabled.unwrap_or(false);
    let ingest_now = input.ingest_now.unwrap_or(true);
    let subscribe = input.subscribe.unwrap_or(false);

    let mut subscription = None;
    if subscribe {
        let source_kind = match input.source_kind {
            AddSourceKind::Channel => SubscriptionSourceKind::Channel,
            AddSourceKind::Playlist => SubscriptionSourceKind::Playlist,
            AddSourceKind::Video => return Err("Only channels and playlists can be subscribed.".to_string()),
        };
        subscription = Some(
            add_subscription(AddSubscriptionRequest {
                database_url: database_url.clone(),
                source_kind,
                source_url: source_url.clone(),
                name: input
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                enabled: true,
                work_dir: work_dir.clone(),
                caption: caption.clone(),
                yt_dlp: yt_dlp.clone(),
                asr_enabled,
                transcriber_command: transcriber_command.clone(),
                transcriber_args: transcriber_args.clone(),
                max_items: input.max_items,
                title_contains: title_contains.clone(),
                title_excludes: title_excludes.clone(),
                duration_min: input.duration_min,
                duration_max: input.duration_max,
                migrate,
            })
            .await
            .map_err(|error| error.to_string())?,
        );
    }

    let ingest = if ingest_now {
        let source = match input.source_kind {
            AddSourceKind::Video => CorpusSource::YoutubeUrl {
                url: source_url.clone(),
            },
            AddSourceKind::Channel => CorpusSource::ChannelUrl {
                url: source_url.clone(),
            },
            AddSourceKind::Playlist => CorpusSource::PlaylistUrl {
                url: source_url.clone(),
            },
        };
        Some(
            ingest_corpus(youtube_corpus::ingest::IngestRequest {
                database_url,
                source,
                work_dir,
                caption,
                yt_dlp,
                asr_enabled,
                transcriber_command,
                transcriber_args,
                max_items: input.max_items,
                title_contains,
                title_excludes,
                duration_min: input.duration_min,
                duration_max: input.duration_max,
                migrate: migrate && !subscribe,
            })
            .await
            .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };

    let source_kind = match input.source_kind {
        AddSourceKind::Video => "video",
        AddSourceKind::Channel => "channel",
        AddSourceKind::Playlist => "playlist",
    };
    Ok(AddSourceReport {
        source_kind: source_kind.to_string(),
        source_url,
        subscription,
        ingest,
    })
}

fn resolve_database_url(database_url: Option<String>) -> Option<String> {
    database_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("DATABASE_URL").ok())
}

async fn required_tables_exist(pool: &sqlx::PgPool) -> Result<bool, String> {
    for table in REQUIRED_TABLES {
        let row = sqlx::query("SELECT to_regclass($1)::text AS table_name")
            .bind(table)
            .fetch_one(pool)
            .await
            .map_err(|error| error.to_string())?;
        let table_name: Option<String> = row
            .try_get("table_name")
            .map_err(|error| error.to_string())?;
        if table_name.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn load_corpus_stats(pool: &sqlx::PgPool) -> Result<CorpusStats, String> {
    let count_row = sqlx::query(
        "SELECT
           (SELECT count(*) FROM videos) AS videos,
           (SELECT count(*) FROM transcript_streams) AS streams,
           (SELECT count(*) FROM transcript_segments) AS segments,
           (SELECT count(*) FROM corpus_subscriptions) AS subscriptions,
           (SELECT count(*) FROM corpus_subscriptions WHERE enabled) AS enabled_subscriptions",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())?;

    let source_rows = sqlx::query(
        "SELECT st.source_kind, count(DISTINCT st.id) AS streams, count(s.id) AS segments
         FROM transcript_streams st
         LEFT JOIN transcript_segments s ON s.stream_id = st.id
         GROUP BY st.source_kind
         ORDER BY segments DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let language_rows = sqlx::query(
        "SELECT coalesce(language, 'unknown') AS language, count(*) AS segments
         FROM transcript_segments
         GROUP BY coalesce(language, 'unknown')
         ORDER BY segments DESC
         LIMIT 8",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let last_ingest_row = sqlx::query(
        "SELECT id::text AS id, source_url, status, videos_indexed, segments_indexed,
                created_at::text AS created_at
         FROM ingest_runs
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;

    Ok(CorpusStats {
        videos: count_row
            .try_get("videos")
            .map_err(|error| error.to_string())?,
        streams: count_row
            .try_get("streams")
            .map_err(|error| error.to_string())?,
        segments: count_row
            .try_get("segments")
            .map_err(|error| error.to_string())?,
        subscriptions: count_row
            .try_get("subscriptions")
            .map_err(|error| error.to_string())?,
        enabled_subscriptions: count_row
            .try_get("enabled_subscriptions")
            .map_err(|error| error.to_string())?,
        source_kinds: source_rows
            .into_iter()
            .map(|row| {
                let source_kind: String = row.try_get("source_kind")?;
                Ok(SourceKindStat {
                    source_kind: parse_source_kind(&source_kind)?,
                    streams: row.try_get("streams")?,
                    segments: row.try_get("segments")?,
                })
            })
            .collect::<Result<Vec<_>, StatusQueryError>>()
            .map_err(|error| error.to_string())?,
        languages: language_rows
            .into_iter()
            .map(|row| {
                Ok(LanguageStat {
                    language: row.try_get("language")?,
                    segments: row.try_get("segments")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(|error| error.to_string())?,
        last_ingest_run: last_ingest_row
            .map(|row| {
                Ok(LastIngestRun {
                    id: row.try_get("id")?,
                    source_url: row.try_get("source_url")?,
                    status: row.try_get("status")?,
                    videos_indexed: row.try_get("videos_indexed")?,
                    segments_indexed: row.try_get("segments_indexed")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .transpose()
            .map_err(|error: sqlx::Error| error.to_string())?,
    })
}

#[derive(Debug)]
enum StatusQueryError {
    Sql(sqlx::Error),
    InvalidSourceKind(String),
}

impl std::fmt::Display for StatusQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "{error}"),
            Self::InvalidSourceKind(value) => write!(formatter, "invalid source_kind: {value}"),
        }
    }
}

impl From<sqlx::Error> for StatusQueryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}

fn parse_source_kind(value: &str) -> Result<SourceKind, StatusQueryError> {
    match value {
        "caption_manual" => Ok(SourceKind::CaptionManual),
        "caption_auto" => Ok(SourceKind::CaptionAuto),
        "asr" => Ok(SourceKind::Asr),
        _ => Err(StatusQueryError::InvalidSourceKind(value.to_string())),
    }
}

fn normalized_languages(languages: Option<Vec<String>>) -> Vec<String> {
    let values = languages
        .unwrap_or_default()
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        vec!["en".to_string()]
    } else {
        values
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            add_source,
            corpus_status,
            database_status,
            downloaded_files,
            search_transcripts,
            transcript_context
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
