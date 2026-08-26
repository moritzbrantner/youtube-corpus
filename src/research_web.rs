use std::net::IpAddr;
use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api_types::SearchTranscriptsInput;
use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use crate::corpora::{Corpus, CorpusSourceSummary, CorpusVideoSummary, CreateCorpusRequest};
use crate::ingest::{ingest_corpus, IngestReport, IngestRequest};
use crate::reprocessing::{ReprocessReport, ReprocessRequest, ReprocessStage};
use crate::search::{SearchReport, SearchRequest};
use crate::subscriptions::{
    add_subscription, check_subscriptions, AddSubscriptionRequest, CheckSubscriptionsReport,
    CheckSubscriptionsRequest, SubscriptionSourceKind,
};

#[derive(Clone)]
struct ResearchState {
    database_url: Option<String>,
    yt_dlp: YtDlpConfig,
}

pub fn router(database_url: Option<String>, yt_dlp: YtDlpConfig) -> Router {
    let state = ResearchState {
        database_url,
        yt_dlp,
    };
    Router::new()
        .route("/api/corpora", get(list_corpora).post(create_corpus))
        .route("/api/corpora/{corpus_id}", get(get_corpus))
        .route(
            "/api/corpora/{corpus_id}/sources",
            get(list_sources).post(add_source),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/check",
            post(check_source),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/enabled",
            post(set_source_enabled),
        )
        .route("/api/corpora/{corpus_id}/videos", get(list_videos))
        .route("/api/corpora/{corpus_id}/search", post(search_corpus))
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/reprocess",
            post(reprocess_video),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCorpusInput {
    name: String,
    slug: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddCorpusSourceInput {
    source_kind: SubscriptionSourceKind,
    source_url: String,
    name: Option<String>,
    work_dir: Option<String>,
    monitor: Option<bool>,
    ingest_now: Option<bool>,
    caption_languages: Option<Vec<String>>,
    auto_captions_enabled: Option<bool>,
    asr_enabled: Option<bool>,
    transcriber_command: Option<String>,
    transcriber_args: Option<Vec<String>>,
    transcriber_timeout_seconds: Option<u64>,
    max_items: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AddCorpusSourceReport {
    source: Option<CorpusSourceSummary>,
    ingest: Option<IngestReport>,
    check: Option<CheckSubscriptionsReport>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSourceEnabledInput {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListCorpusVideosInput {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReprocessVideoInput {
    stage: ReprocessStage,
    work_dir: Option<String>,
    caption_languages: Option<Vec<String>>,
    transcriber_command: Option<String>,
    transcriber_args: Option<Vec<String>>,
    transcriber_timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorMessage,
}

#[derive(Debug, Serialize)]
struct ErrorMessage {
    message: String,
}

#[derive(Debug)]
struct ResearchError {
    status: StatusCode,
    message: String,
}

impl ResearchError {
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

impl IntoResponse for ResearchError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorMessage {
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

async fn pool(state: &ResearchState) -> Result<sqlx::PgPool, ResearchError> {
    let database_url = state
        .database_url
        .as_deref()
        .ok_or_else(|| ResearchError::bad_request("DATABASE_URL is required."))?;
    crate::db::connect(database_url)
        .await
        .map_err(ResearchError::internal)
}

fn database_url(state: &ResearchState) -> Result<String, ResearchError> {
    state
        .database_url
        .clone()
        .ok_or_else(|| ResearchError::bad_request("DATABASE_URL is required."))
}

async fn list_corpora(State(state): State<ResearchState>) -> Result<Json<Vec<Corpus>>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::list_corpora(&pool)
        .await
        .map(Json)
        .map_err(ResearchError::internal)
}

async fn create_corpus(
    State(state): State<ResearchState>,
    Json(input): Json<CreateCorpusInput>,
) -> Result<(StatusCode, Json<Corpus>), ResearchError> {
    let pool = pool(&state).await?;
    let corpus = crate::corpora::create_corpus(
        &pool,
        CreateCorpusRequest {
            name: input.name,
            slug: input.slug,
            description: input.description,
        },
    )
    .await
    .map_err(|error| ResearchError::bad_request(error.to_string()))?;
    Ok((StatusCode::CREATED, Json(corpus)))
}

async fn get_corpus(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Corpus>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::get_corpus(&pool, corpus_id)
        .await
        .map_err(ResearchError::internal)?
        .map(Json)
        .ok_or_else(|| ResearchError::not_found("Corpus not found."))
}

async fn list_sources(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Vec<CorpusSourceSummary>>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::list_corpus_sources(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}

async fn add_source(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<AddCorpusSourceInput>,
) -> Result<(StatusCode, Json<AddCorpusSourceReport>), ResearchError> {
    let source_url = input.source_url.trim().to_string();
    if source_url.is_empty() {
        return Err(ResearchError::bad_request("sourceUrl is required."));
    }
    let database_url = database_url(&state)?;
    let pool = pool(&state).await?;
    if crate::corpora::get_corpus(&pool, corpus_id)
        .await
        .map_err(ResearchError::internal)?
        .is_none()
    {
        return Err(ResearchError::not_found("Corpus not found."));
    }

    let monitor = input.monitor.unwrap_or(true);
    let ingest_now = input.ingest_now.unwrap_or(true);
    if !monitor && !ingest_now {
        return Err(ResearchError::bad_request(
            "A non-monitored source must be ingested now so it can belong to the corpus.",
        ));
    }
    let work_dir = PathBuf::from(
        input
            .work_dir
            .as_deref()
            .unwrap_or("use-case-output/youtube-corpus"),
    );
    let caption = CaptionConfig {
        enabled: true,
        include_auto_captions: input.auto_captions_enabled.unwrap_or(true),
        languages: normalized_languages(input.caption_languages),
    };
    let transcriber_command = input.transcriber_command.map(PathBuf::from);
    let transcriber_args = input.transcriber_args.unwrap_or_default();
    let asr_enabled = input.asr_enabled.unwrap_or(false);

    if monitor {
        let subscription = add_subscription(AddSubscriptionRequest {
            database_url: database_url.clone(),
            source_kind: input.source_kind,
            source_url: source_url.clone(),
            name: input.name,
            enabled: true,
            work_dir,
            caption,
            yt_dlp: state.yt_dlp.clone(),
            asr_enabled,
            transcriber_command,
            transcriber_args,
            transcriber_timeout_seconds: input.transcriber_timeout_seconds,
            max_items: input.max_items,
            title_contains: None,
            title_excludes: Vec::new(),
            duration_min: None,
            duration_max: None,
            migrate: false,
        })
        .await
        .map_err(ResearchError::internal)?;
        crate::corpora::link_source(&pool, corpus_id, subscription.id)
            .await
            .map_err(map_corpus_error)?;

        let check = if ingest_now {
            let report = check_subscriptions(CheckSubscriptionsRequest {
                database_url,
                id: Some(subscription.id),
                include_disabled: true,
                migrate: false,
            })
            .await
            .map_err(ResearchError::internal)?;
            persist_check_outcome(&pool, subscription.id, &report).await?;
            Some(report)
        } else {
            None
        };
        let source = crate::corpora::list_corpus_sources(&pool, corpus_id)
            .await
            .map_err(map_corpus_error)?
            .into_iter()
            .find(|source| source.id == subscription.id);
        return Ok((
            StatusCode::CREATED,
            Json(AddCorpusSourceReport {
                source,
                ingest: None,
                check,
            }),
        ));
    }

    let report = ingest_corpus(IngestRequest {
        run_id: Some(Uuid::new_v4()),
        database_url,
        source: subscription_source(input.source_kind, source_url),
        work_dir,
        caption,
        yt_dlp: state.yt_dlp.clone(),
        asr_enabled,
        transcriber_command,
        transcriber_args,
        transcriber_timeout_seconds: input.transcriber_timeout_seconds,
        max_items: input.max_items,
        title_contains: None,
        title_excludes: Vec::new(),
        duration_min: None,
        duration_max: None,
        migrate: false,
    })
    .await
    .map_err(ResearchError::internal)?;
    for item in &report.items {
        if let Some(video_id) = item.video_id {
            crate::corpora::link_video(&pool, corpus_id, video_id)
                .await
                .map_err(map_corpus_error)?;
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(AddCorpusSourceReport {
            source: None,
            ingest: Some(report),
            check: None,
        }),
    ))
}

async fn check_source(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<CheckSubscriptionsReport>, ResearchError> {
    let pool = pool(&state).await?;
    if !crate::corpora::source_belongs_to_corpus(&pool, corpus_id, source_id)
        .await
        .map_err(ResearchError::internal)?
    {
        return Err(ResearchError::not_found("Source is not part of this corpus."));
    }
    let report = check_subscriptions(CheckSubscriptionsRequest {
        database_url: database_url(&state)?,
        id: Some(source_id),
        include_disabled: true,
        migrate: false,
    })
    .await
    .map_err(ResearchError::internal)?;
    persist_check_outcome(&pool, source_id, &report).await?;
    Ok(Json(report))
}

async fn set_source_enabled(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<SetSourceEnabledInput>,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::set_source_enabled(&pool, corpus_id, source_id, input.enabled)
        .await
        .map_err(map_corpus_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_videos(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Query(input): Query<ListCorpusVideosInput>,
) -> Result<Json<Vec<CorpusVideoSummary>>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::list_corpus_videos(&pool, corpus_id, input.limit.unwrap_or(100))
        .await
        .map(Json)
        .map_err(map_corpus_error)
}

async fn search_corpus(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<SearchTranscriptsInput>,
) -> Result<Json<SearchReport>, ResearchError> {
    if input.query.trim().is_empty() {
        return Err(ResearchError::bad_request("query is required."));
    }
    if input.top_k <= 0 {
        return Err(ResearchError::bad_request("topK must be positive."));
    }
    let pool = pool(&state).await?;
    let allowed_video_ids = crate::corpora::corpus_video_ids(&pool, corpus_id)
        .await
        .map_err(map_corpus_error)?;
    let requested_top_k = input.top_k;
    let candidate_top_k = requested_top_k.saturating_mul(20).clamp(100, 1000);
    let mut report = crate::search::search_corpus(SearchRequest {
        database_url: database_url(&state)?,
        query: input.query,
        top_k: candidate_top_k,
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
    .map_err(ResearchError::internal)?;
    report
        .results
        .retain(|result| allowed_video_ids.contains(&result.video_id));
    let truncate_to = usize::try_from(requested_top_k).unwrap_or(usize::MAX);
    report.results.truncate(truncate_to);
    Ok(Json(report))
}

async fn reprocess_video(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<ReprocessVideoInput>,
) -> Result<Json<ReprocessReport>, ResearchError> {
    let pool = pool(&state).await?;
    let allowed_video_ids = crate::corpora::corpus_video_ids(&pool, corpus_id)
        .await
        .map_err(map_corpus_error)?;
    if !allowed_video_ids.contains(&video_id) {
        return Err(ResearchError::not_found("Video is not part of this corpus."));
    }
    crate::reprocessing::reprocess_video(ReprocessRequest {
        database_url: database_url(&state)?,
        video_id,
        stage: input.stage,
        work_dir: PathBuf::from(
            input
                .work_dir
                .as_deref()
                .unwrap_or("use-case-output/youtube-corpus"),
        ),
        caption_languages: normalized_languages(input.caption_languages),
        yt_dlp: state.yt_dlp.clone(),
        transcriber_command: input.transcriber_command.map(PathBuf::from),
        transcriber_args: input.transcriber_args.unwrap_or_default(),
        transcriber_timeout_seconds: input.transcriber_timeout_seconds,
    })
    .await
    .map(Json)
    .map_err(ResearchError::internal)
}

async fn persist_check_outcome(
    pool: &sqlx::PgPool,
    subscription_id: Uuid,
    report: &CheckSubscriptionsReport,
) -> Result<(), ResearchError> {
    let item = report.items.iter().find(|item| item.subscription.id == subscription_id);
    let status = item.map(|item| item.status.as_str()).unwrap_or("completed");
    let message = item.and_then(|item| item.message.as_deref());
    crate::corpora::record_check_outcome(pool, subscription_id, status, message)
        .await
        .map_err(ResearchError::internal)
}

fn normalized_languages(languages: Option<Vec<String>>) -> Vec<String> {
    let languages = languages.unwrap_or_else(|| vec!["en".to_string()]);
    let normalized = languages
        .into_iter()
        .map(|language| language.trim().to_lowercase())
        .filter(|language| !language.is_empty())
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        vec!["en".to_string()]
    } else {
        normalized
    }
}

fn subscription_source(kind: SubscriptionSourceKind, url: String) -> CorpusSource {
    match kind {
        SubscriptionSourceKind::Channel => CorpusSource::ChannelUrl { url },
        SubscriptionSourceKind::Playlist => CorpusSource::PlaylistUrl { url },
    }
}

fn map_corpus_error(error: anyhow::Error) -> ResearchError {
    if error.to_string().contains("not found") || error.to_string().contains("not part") {
        ResearchError::not_found(error.to_string())
    } else {
        ResearchError::internal(error)
    }
}

#[allow(dead_code)]
fn _assert_send_sync(_: IpAddr) {}

#[cfg(test)]
mod tests {
    use super::normalized_languages;

    #[test]
    fn normalizes_caption_languages() {
        assert_eq!(
            normalized_languages(Some(vec![" EN ".to_string(), "de".to_string()])),
            vec!["en".to_string(), "de".to_string()]
        );
        assert_eq!(normalized_languages(Some(Vec::new())), vec!["en".to_string()]);
    }
}
