use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::config::{CaptionConfig, CorpusSource, SearchMode, SourceKind, YtDlpConfig};
use crate::ingest::{ingest_corpus, IngestReport, IngestRequest};
use crate::search::{search_corpus, SearchRequest, SearchResult};
use crate::status::{ListVideosRequest, VideoStatus};
use crate::subscriptions::{
    add_subscription, AddSubscriptionRequest, Subscription, SubscriptionSourceKind,
};

const REQUIRED_TABLES: &[&str] = &[
    "videos",
    "transcript_streams",
    "transcript_segments",
    "ingest_runs",
    "corpus_subscriptions",
];

#[derive(RustEmbed)]
#[folder = "dist/"]
struct WebAssets;

#[derive(Debug, Clone)]
pub struct WebServerConfig {
    pub database_url: Option<String>,
    pub host: IpAddr,
    pub port: u16,
    pub open_browser: bool,
    pub migrate: bool,
}

#[derive(Clone)]
struct AppState {
    database_url: Option<String>,
}

pub async fn serve(config: WebServerConfig) -> anyhow::Result<()> {
    let database_url = resolve_database_url(config.database_url);
    if config.migrate {
        if let Some(database_url) = &database_url {
            let pool = crate::db::connect(database_url).await?;
            crate::db::migrate(&pool).await?;
        }
    }

    let state = AppState { database_url };
    let address = SocketAddr::from((config.host, config.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    let local_address = listener.local_addr()?;
    let url = browser_url(local_address);

    println!("serving YouTube Corpus at {url}");
    if config.open_browser {
        if let Err(error) = open::that_detached(&url) {
            tracing::warn!(%error, "failed to open browser");
        }
    }

    axum::serve(listener, app(state)).await?;
    Ok(())
}

fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/database-status", get(database_status))
        .route("/corpus-status", get(corpus_status))
        .route("/search", post(search_transcripts))
        .route("/transcript-context", post(transcript_context))
        .route("/downloaded-files", get(downloaded_files))
        .route("/sources", post(add_source))
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .fallback(static_asset)
        .with_state(state)
}

fn browser_url(address: SocketAddr) -> String {
    let host = if address.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        address.ip().to_string()
    };
    format!("http://{host}:{}", address.port())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseStatus {
    configured: bool,
    database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchTranscriptsInput {
    query: String,
    mode: SearchMode,
    top_k: i64,
    source_kind: Option<SourceKind>,
    video_id: Option<Uuid>,
    language: Option<String>,
    transcript_start_min: Option<f64>,
    transcript_start_max: Option<f64>,
    upload_date_from: Option<String>,
    upload_date_to: Option<String>,
    duration_min: Option<f64>,
    duration_max: Option<f64>,
    channel_query: Option<String>,
    title_query: Option<String>,
    category_query: Option<String>,
    tag_query: Option<String>,
    metadata_query: Option<String>,
    view_count_min: Option<i64>,
    view_count_max: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptContextInput {
    segment_id: String,
    before: Option<i64>,
    after: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListDownloadedFilesInput {
    downloaded_only: Option<bool>,
    parsed_only: Option<bool>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddSourceInput {
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

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: ApiErrorMessage,
}

#[derive(Debug, Serialize)]
struct ApiErrorMessage {
    message: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                error: ApiErrorMessage {
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

async fn database_status(State(state): State<AppState>) -> Json<DatabaseStatus> {
    Json(DatabaseStatus {
        configured: state.database_url.is_some(),
        database_url: state.database_url.as_deref().map(mask_database_url),
    })
}

async fn corpus_status(State(state): State<AppState>) -> Result<Json<CorpusStatus>, ApiError> {
    let Some(database_url) = state.database_url.clone() else {
        return Ok(Json(CorpusStatus {
            configured: false,
            database_url: None,
            reachable: false,
            schema_ready: false,
            message: Some("DATABASE_URL is required.".to_string()),
            stats: None,
        }));
    };

    let pool = match crate::db::connect(&database_url).await {
        Ok(pool) => pool,
        Err(error) => {
            return Ok(Json(CorpusStatus {
                configured: true,
                database_url: Some(mask_database_url(&database_url)),
                reachable: false,
                schema_ready: false,
                message: Some(error.to_string()),
                stats: None,
            }));
        }
    };

    let schema_ready = required_tables_exist(&pool).await?;
    if !schema_ready {
        return Ok(Json(CorpusStatus {
            configured: true,
            database_url: Some(mask_database_url(&database_url)),
            reachable: true,
            schema_ready: false,
            message: Some(
                "Database is reachable but migrations have not been applied.".to_string(),
            ),
            stats: None,
        }));
    }

    let stats = load_corpus_stats(&pool).await?;
    Ok(Json(CorpusStatus {
        configured: true,
        database_url: Some(mask_database_url(&database_url)),
        reachable: true,
        schema_ready: true,
        message: None,
        stats: Some(stats),
    }))
}

async fn search_transcripts(
    State(state): State<AppState>,
    Json(input): Json<SearchTranscriptsInput>,
) -> Result<Json<crate::search::SearchReport>, ApiError> {
    let database_url = required_database_url(&state)?;
    if input.query.trim().is_empty() {
        return Err(ApiError::bad_request("query is required."));
    }
    if input.top_k <= 0 {
        return Err(ApiError::bad_request("topK must be positive."));
    }

    search_corpus(SearchRequest {
        database_url,
        query: input.query,
        top_k: input.top_k,
        mode: input.mode,
        source_kind: input.source_kind,
        video_id: input.video_id,
        language: input.language,
        transcript_start_min: input.transcript_start_min,
        transcript_start_max: input.transcript_start_max,
        upload_date_from: input.upload_date_from,
        upload_date_to: input.upload_date_to,
        duration_min: input.duration_min,
        duration_max: input.duration_max,
        channel_query: input.channel_query,
        title_query: input.title_query,
        category_query: input.category_query,
        tag_query: input.tag_query,
        metadata_query: input.metadata_query,
        view_count_min: input.view_count_min,
        view_count_max: input.view_count_max,
    })
    .await
    .map(Json)
    .map_err(ApiError::internal)
}

async fn transcript_context(
    State(state): State<AppState>,
    Json(input): Json<TranscriptContextInput>,
) -> Result<Json<TranscriptContextReport>, ApiError> {
    let database_url = required_database_url(&state)?;
    let segment_id = input.segment_id.trim();
    if segment_id.is_empty() {
        return Err(ApiError::bad_request("segmentId is required."));
    }
    let segment_id =
        Uuid::parse_str(segment_id).map_err(|error| ApiError::bad_request(error.to_string()))?;

    let before = input.before.unwrap_or(4).clamp(0, 20);
    let after = input.after.unwrap_or(6).clamp(0, 20);
    let pool = crate::db::connect(&database_url)
        .await
        .map_err(ApiError::internal)?;

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
    .map_err(ApiError::internal)?;

    let match_row = match match_row {
        Some(row) => row,
        None => return Err(ApiError::not_found("Segment not found.")),
    };

    let stream_id: Uuid = match_row.try_get("stream_id").map_err(ApiError::internal)?;
    let segment_index: i64 = match_row
        .try_get("segment_index")
        .map_err(ApiError::internal)?;
    let start_index = segment_index.saturating_sub(before).max(0);
    let end_index = segment_index.saturating_add(after);

    let match_segment = SearchResult {
        segment_id: match_row
            .try_get::<Uuid, _>("id")
            .map_err(ApiError::internal)?,
        video_id: match_row
            .try_get::<Uuid, _>("video_id")
            .map_err(ApiError::internal)?,
        stream_id,
        source_kind: match_row
            .try_get::<String, _>("source_kind")
            .map_err(ApiError::internal)?,
        language: match_row
            .try_get::<Option<String>, _>("language")
            .map_err(ApiError::internal)?,
        start_seconds: match_row
            .try_get::<Option<f64>, _>("start_seconds")
            .map_err(ApiError::internal)?,
        end_seconds: match_row
            .try_get::<Option<f64>, _>("end_seconds")
            .map_err(ApiError::internal)?,
        text: match_row
            .try_get::<String, _>("text")
            .map_err(ApiError::internal)?,
        source_url: match_row
            .try_get::<String, _>("source_url")
            .map_err(ApiError::internal)?,
        title: match_row
            .try_get::<Option<String>, _>("title")
            .map_err(ApiError::internal)?,
        score: match_row
            .try_get::<f64, _>("score")
            .map_err(ApiError::internal)?,
        fts_score: match_row
            .try_get::<f64, _>("fts_score")
            .map_err(ApiError::internal)?,
        semantic_score: match_row
            .try_get::<f64, _>("semantic_score")
            .map_err(ApiError::internal)?,
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
    .map_err(ApiError::internal)?;

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
        .map_err(ApiError::internal)?;

    Ok(Json(TranscriptContextReport {
        match_segment,
        segments,
    }))
}

async fn downloaded_files(
    State(state): State<AppState>,
    Query(input): Query<ListDownloadedFilesInput>,
) -> Result<Json<Vec<VideoStatus>>, ApiError> {
    let database_url = required_database_url(&state)?;
    let downloaded_only = input.downloaded_only.unwrap_or(true);
    let limit = input.limit;
    let mut videos = crate::status::list_videos(ListVideosRequest {
        database_url,
        downloaded_only: false,
        parsed_only: input.parsed_only.unwrap_or(false),
        limit: if downloaded_only { None } else { limit },
        migrate: false,
    })
    .await
    .map_err(ApiError::internal)?;

    if downloaded_only {
        videos.retain(|video| video.media_downloaded || video.caption_files_downloaded);
        if let Some(limit) = limit.and_then(|value| usize::try_from(value.max(0)).ok()) {
            videos.truncate(limit);
        }
    }

    Ok(Json(videos))
}

async fn add_source(
    State(state): State<AppState>,
    Json(input): Json<AddSourceInput>,
) -> Result<Json<AddSourceReport>, ApiError> {
    let database_url = required_database_url(&state)?;
    let source_url = input.source_url.trim().to_string();
    if source_url.is_empty() {
        return Err(ApiError::bad_request("Source URL is required."));
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
            AddSourceKind::Video => {
                return Err(ApiError::bad_request(
                    "Only channels and playlists can be subscribed.",
                ))
            }
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
            .map_err(ApiError::internal)?,
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
            ingest_corpus(IngestRequest {
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
            .map_err(ApiError::internal)?,
        )
    } else {
        None
    };

    let source_kind = match input.source_kind {
        AddSourceKind::Video => "video",
        AddSourceKind::Channel => "channel",
        AddSourceKind::Playlist => "playlist",
    };
    Ok(Json(AddSourceReport {
        source_kind: source_kind.to_string(),
        source_url,
        subscription,
        ingest,
    }))
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("API endpoint not found.")
}

async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let asset_path = if path.is_empty() { "index.html" } else { path };
    if let Some(asset) = WebAssets::get(asset_path) {
        return asset_response(asset_path, asset.data.into_owned());
    }

    if let Some(asset) = WebAssets::get("index.html") {
        return asset_response("index.html", asset.data.into_owned());
    }

    (
        StatusCode::NOT_FOUND,
        "frontend assets were not found; run `bun run build` before starting the server",
    )
        .into_response()
}

fn asset_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("text/plain")),
    );
    response
}

fn resolve_database_url(database_url: Option<String>) -> Option<String> {
    database_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("DATABASE_URL").ok())
}

fn required_database_url(state: &AppState) -> Result<String, ApiError> {
    state
        .database_url
        .clone()
        .ok_or_else(|| ApiError::bad_request("DATABASE_URL is required."))
}

fn mask_database_url(database_url: &str) -> String {
    let Ok(mut url) = url::Url::parse(database_url) else {
        return database_url.to_string();
    };
    if url.password().is_some() {
        let _ = url.set_password(Some("*****"));
    }
    url.to_string()
}

async fn required_tables_exist(pool: &sqlx::PgPool) -> Result<bool, ApiError> {
    for table in REQUIRED_TABLES {
        let row = sqlx::query("SELECT to_regclass($1)::text AS table_name")
            .bind(table)
            .fetch_one(pool)
            .await
            .map_err(ApiError::internal)?;
        let table_name: Option<String> = row.try_get("table_name").map_err(ApiError::internal)?;
        if table_name.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn load_corpus_stats(pool: &sqlx::PgPool) -> Result<CorpusStats, ApiError> {
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
    .map_err(ApiError::internal)?;

    let source_rows = sqlx::query(
        "SELECT st.source_kind, count(DISTINCT st.id) AS streams, count(s.id) AS segments
         FROM transcript_streams st
         LEFT JOIN transcript_segments s ON s.stream_id = st.id
         GROUP BY st.source_kind
         ORDER BY segments DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::internal)?;

    let language_rows = sqlx::query(
        "SELECT coalesce(language, 'unknown') AS language, count(*) AS segments
         FROM transcript_segments
         GROUP BY coalesce(language, 'unknown')
         ORDER BY segments DESC
         LIMIT 8",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::internal)?;

    let last_ingest_row = sqlx::query(
        "SELECT id::text AS id, source_url, status, videos_indexed, segments_indexed,
                created_at::text AS created_at
         FROM ingest_runs
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::internal)?;

    Ok(CorpusStats {
        videos: count_row.try_get("videos").map_err(ApiError::internal)?,
        streams: count_row.try_get("streams").map_err(ApiError::internal)?,
        segments: count_row.try_get("segments").map_err(ApiError::internal)?,
        subscriptions: count_row
            .try_get("subscriptions")
            .map_err(ApiError::internal)?,
        enabled_subscriptions: count_row
            .try_get("enabled_subscriptions")
            .map_err(ApiError::internal)?,
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
            .map_err(ApiError::internal)?,
        languages: language_rows
            .into_iter()
            .map(|row| {
                Ok(LanguageStat {
                    language: row.try_get("language")?,
                    segments: row.try_get("segments")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(ApiError::internal)?,
        last_ingest_run: last_ingest_row
            .map(|row| {
                Ok::<LastIngestRun, sqlx::Error>(LastIngestRun {
                    id: row.try_get("id")?,
                    source_url: row.try_get("source_url")?,
                    status: row.try_get("status")?,
                    videos_indexed: row.try_get("videos_indexed")?,
                    segments_indexed: row.try_get("segments_indexed")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .transpose()
            .map_err(ApiError::internal)?,
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

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    fn test_app(database_url: Option<String>) -> Router {
        app(AppState { database_url })
    }

    async fn request(method: Method, uri: &str, body: Option<&str>) -> Response {
        test_app(None)
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.unwrap_or_default().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn database_status_without_database_url_is_unconfigured() {
        let response = request(Method::GET, "/api/database-status", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["configured"], false);
        assert_eq!(value["databaseUrl"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn corpus_status_without_database_url_is_unconfigured() {
        let response = request(Method::GET, "/api/corpus-status", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["configured"], false);
        assert_eq!(value["reachable"], false);
        assert_eq!(value["schemaReady"], false);
    }

    #[tokio::test]
    async fn search_with_empty_query_returns_bad_request() {
        let response = request(
            Method::POST,
            "/api/search",
            Some(r#"{"query":"","mode":"hybrid","topK":5}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn transcript_context_with_invalid_uuid_returns_bad_request() {
        let response = request(
            Method::POST,
            "/api/transcript-context",
            Some(r#"{"segmentId":"not-a-uuid"}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_api_route_returns_not_found() {
        let response = request(Method::GET, "/api/missing", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_non_api_route_returns_index_html() {
        let response = request(Method::GET, "/search/deep-link", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.starts_with("text/html"));
    }
}
