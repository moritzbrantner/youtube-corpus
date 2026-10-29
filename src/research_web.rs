use std::net::IpAddr;
use std::path::PathBuf;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api_types::SearchTranscriptsInput;
use crate::backfill::{
    BackfillAction, BackfillControlError, BackfillEntry, BackfillProgress, HistoryScope,
    HistoryScopeInput,
};
use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use crate::corpora::{
    Corpus, CorpusLabel, CorpusSourceSummary, CorpusVideoSummary, CreateCorpusRequest, LabelError,
};
use crate::detectors::{
    CorpusDetectorProfiles, DetectorError, DetectorSetting, PlanRunReport, PlanRunRequest,
    ScopeKind, VideoDetectorPlan,
};
use crate::entities::{
    EntityExtractionReport, EntityLookupReport, EntityOccurrencesReport, PersonOccurrencesReport,
};
use crate::ingest::{ingest_corpus, IngestReport, IngestRequest};
use crate::reprocessing::{ReprocessReport, ReprocessRequest, ReprocessStage};
use crate::search::{SearchReport, SearchRequest};
use crate::subscriptions::{
    add_subscription, check_subscriptions, AddSubscriptionRequest, CheckDispatch,
    CheckSubscriptionsReport, CheckSubscriptionsRequest, PendingItem, ProcessingPolicy,
    SubscriptionSourceKind,
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
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/policy",
            post(set_source_policy),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/interval",
            post(set_source_interval),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/pending",
            get(list_source_pending),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/backfills",
            get(list_source_backfills),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/backfills/{backfill_id}/entries",
            get(list_source_backfill_entries),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/backfills/{backfill_id}/{action}",
            post(control_source_backfill),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/pending/approve",
            post(approve_source_pending),
        )
        .route(
            "/api/corpora/{corpus_id}/labels",
            get(list_labels).post(create_label),
        )
        .route(
            "/api/corpora/{corpus_id}/labels/{label_id}",
            patch(rename_label).delete(delete_label),
        )
        .route(
            "/api/corpora/{corpus_id}/sources/{source_id}/labels/{label_id}",
            put(assign_source_label).delete(remove_source_label),
        )
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/labels/{label_id}",
            put(assign_video_label).delete(remove_video_label),
        )
        .route("/api/corpora/{corpus_id}/videos", get(list_videos))
        .route("/api/corpora/{corpus_id}/channels", get(list_channels))
        .route("/api/corpora/{corpus_id}/search", post(search_corpus))
        .route(
            "/api/corpora/{corpus_id}/detector-profiles",
            get(get_detector_profiles),
        )
        .route(
            "/api/corpora/{corpus_id}/detector-profiles/corpus",
            put(set_corpus_detectors),
        )
        .route(
            "/api/corpora/{corpus_id}/detector-profiles/sources/{source_id}",
            put(set_source_detectors),
        )
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/detector-overrides",
            put(set_video_detectors),
        )
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/detector-plan",
            get(get_video_plan),
        )
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/processing-status",
            get(get_processing_status),
        )
        .route(
            "/api/corpora/{corpus_id}/videos/{video_id}/detector-plan/run",
            post(run_video_plan),
        )
        .route(
            "/api/corpora/{corpus_id}/face-matches",
            get(get_face_matches),
        )
        .route(
            "/api/corpora/{corpus_id}/face-matches/run",
            post(run_face_matches),
        )
        .route(
            "/api/corpora/{corpus_id}/face-matches/corrections",
            post(correct_face_matches),
        )
        .route(
            "/api/corpora/{corpus_id}/person-names",
            get(get_person_names),
        )
        .route(
            "/api/corpora/{corpus_id}/person-names/run",
            post(run_person_names),
        )
        .route(
            "/api/corpora/{corpus_id}/person-names/corrections",
            post(correct_person_name),
        )
        .route("/api/corpora/{corpus_id}/entities", get(lookup_entities))
        .route(
            "/api/corpora/{corpus_id}/entities/extract",
            post(extract_entities),
        )
        .route(
            "/api/corpora/{corpus_id}/entity-occurrences",
            get(entity_occurrences),
        )
        .route(
            "/api/corpora/{corpus_id}/people/{person_id}/occurrences",
            get(person_occurrences),
        )
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
    processing_policy: Option<ProcessingPolicy>,
    /// Initial history to backfill for a monitored source; full history when omitted (1.12).
    history: Option<HistoryScopeInput>,
    /// Automatic check interval of a monitored source; the default when omitted (1.18).
    check_interval_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AddCorpusSourceReport {
    source: Option<CorpusSourceSummary>,
    ingest: Option<IngestReport>,
    check: Option<CheckSubscriptionsReport>,
    backfill: Option<BackfillProgress>,
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
    label_id: Option<Uuid>,
    source_id: Option<Uuid>,
    channel_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LabelNameInput {
    name: String,
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

async fn list_corpora(
    State(state): State<ResearchState>,
) -> Result<Json<Vec<Corpus>>, ResearchError> {
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

async fn list_channels(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<crate::corpora::CorpusChannelDirectory>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::list_corpus_channels(&pool, corpus_id)
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
    let history = match input.history {
        Some(history) => history.into_scope().map_err(ResearchError::bad_request)?,
        None => HistoryScope::Full,
    };
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
            processing_policy: input.processing_policy.unwrap_or_default(),
            check_interval_seconds: input.check_interval_seconds,
            migrate: false,
        })
        .await
        .map_err(ResearchError::internal)?;
        crate::source_monitor::wake();
        crate::corpora::link_source(&pool, corpus_id, subscription.id)
            .await
            .map_err(map_corpus_error)?;

        // The chosen history scope is applied in the background (1.10, 1.14); a `none` scope
        // still records the existing videos so monitoring only picks up later uploads.
        let backfill =
            crate::backfill::start_backfill(&pool, subscription.id, Some(corpus_id), history)
                .await
                .map_err(ResearchError::internal)?;
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
                check: None,
                backfill: Some(backfill),
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
            backfill: None,
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
        return Err(ResearchError::not_found(
            "Source is not part of this corpus.",
        ));
    }
    let report = check_subscriptions(CheckSubscriptionsRequest {
        database_url: database_url(&state)?,
        id: Some(source_id),
        include_disabled: true,
        migrate: false,
        dispatch: CheckDispatch::Enqueue,
    })
    .await
    .map_err(ResearchError::internal)?;
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
    crate::source_monitor::wake();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSourceIntervalInput {
    interval_seconds: u64,
}

async fn set_source_interval(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<SetSourceIntervalInput>,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::subscriptions::validate_check_interval(input.interval_seconds)
        .map_err(|error| ResearchError::bad_request(error.to_string()))?;
    crate::subscriptions::set_check_interval(&pool, source_id, input.interval_seconds)
        .await
        .map_err(ResearchError::internal)?;
    crate::source_monitor::wake();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct SetSourcePolicyInput {
    policy: ProcessingPolicy,
}

async fn require_source(
    pool: &sqlx::PgPool,
    corpus_id: Uuid,
    source_id: Uuid,
) -> Result<(), ResearchError> {
    if crate::corpora::source_belongs_to_corpus(pool, corpus_id, source_id)
        .await
        .map_err(ResearchError::internal)?
    {
        Ok(())
    } else {
        Err(ResearchError::not_found(
            "Source is not part of this corpus.",
        ))
    }
}

async fn set_source_policy(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<SetSourcePolicyInput>,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::subscriptions::set_processing_policy(&pool, source_id, input.policy)
        .await
        .map_err(ResearchError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_source_backfills(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Vec<BackfillProgress>>, ResearchError> {
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::backfill::list_backfills(&pool, source_id)
        .await
        .map(Json)
        .map_err(ResearchError::internal)
}

async fn list_source_backfill_entries(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id, backfill_id)): AxumPath<(Uuid, Uuid, Uuid)>,
) -> Result<Json<Vec<BackfillEntry>>, ResearchError> {
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::backfill::list_backfill_entries(&pool, source_id, backfill_id)
        .await
        .map(Json)
        .map_err(ResearchError::internal)
}

async fn control_source_backfill(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id, backfill_id, action)): AxumPath<(Uuid, Uuid, Uuid, String)>,
) -> Result<Json<BackfillProgress>, ResearchError> {
    let action = match action.as_str() {
        "pause" => BackfillAction::Pause,
        "resume" => BackfillAction::Resume,
        "cancel" => BackfillAction::Cancel,
        _ => return Err(ResearchError::not_found("Unknown backfill action.")),
    };
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::backfill::control_backfill(&pool, source_id, backfill_id, action)
        .await
        .map(Json)
        .map_err(|error| match error {
            BackfillControlError::NotFound => ResearchError::not_found("Backfill not found."),
            BackfillControlError::InvalidState { .. } => ResearchError {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
            },
            BackfillControlError::Database(error) => ResearchError::internal(error),
        })
}

async fn list_source_pending(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Vec<PendingItem>>, ResearchError> {
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::subscriptions::list_pending_items(&pool, source_id)
        .await
        .map(Json)
        .map_err(ResearchError::internal)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovePendingInput {
    source_urls: Vec<String>,
}

async fn approve_source_pending(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<ApprovePendingInput>,
) -> Result<Json<IngestReport>, ResearchError> {
    if input.source_urls.is_empty() {
        return Err(ResearchError::bad_request("Select at least one video."));
    }
    let pool = pool(&state).await?;
    require_source(&pool, corpus_id, source_id).await?;
    crate::subscriptions::approve_pending_items(
        &database_url(&state)?,
        source_id,
        &input.source_urls,
    )
    .await
    .map(Json)
    .map_err(|error| {
        if error.to_string().contains("not awaiting review") {
            ResearchError::bad_request(error.to_string())
        } else {
            ResearchError::internal(format!("{error:#}"))
        }
    })
}

async fn list_videos(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Query(input): Query<ListCorpusVideosInput>,
) -> Result<Json<Vec<CorpusVideoSummary>>, ResearchError> {
    let pool = pool(&state).await?;
    let filter = crate::corpora::CorpusVideoFilter {
        label_id: input.label_id,
        source_id: input.source_id,
        channel_id: input.channel_id.filter(|value| !value.trim().is_empty()),
    };
    crate::corpora::list_corpus_videos(&pool, corpus_id, input.limit.unwrap_or(100), &filter)
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
        modality: input.modality,
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
    // Without coverage an empty visible-text result would read as a confirmed negative.
    report.ocr_coverage = Some(
        crate::search::corpus_ocr_coverage(&pool, &allowed_video_ids)
            .await
            .map_err(ResearchError::internal)?,
    );
    Ok(Json(report))
}

fn map_detector_error(error: DetectorError) -> ResearchError {
    match error {
        DetectorError::NotFound(message) => ResearchError::not_found(message),
        DetectorError::Invalid(message) => ResearchError::bad_request(message),
        DetectorError::Other(error) => ResearchError::internal(format!("{error:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct DetectorSettingsInput {
    settings: Vec<DetectorSetting>,
}

async fn get_detector_profiles(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<CorpusDetectorProfiles>, ResearchError> {
    let pool = pool(&state).await?;
    crate::detectors::corpus_profiles(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_detector_error)
}

async fn set_scope(
    state: &ResearchState,
    corpus_id: Uuid,
    scope: ScopeKind,
    scope_id: Uuid,
    settings: &[DetectorSetting],
) -> Result<StatusCode, ResearchError> {
    let pool = pool(state).await?;
    crate::detectors::set_scope_settings(&pool, corpus_id, scope, scope_id, settings)
        .await
        .map_err(map_detector_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_corpus_detectors(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<DetectorSettingsInput>,
) -> Result<StatusCode, ResearchError> {
    set_scope(
        &state,
        corpus_id,
        ScopeKind::Corpus,
        corpus_id,
        &input.settings,
    )
    .await
}

async fn set_source_detectors(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<DetectorSettingsInput>,
) -> Result<StatusCode, ResearchError> {
    set_scope(
        &state,
        corpus_id,
        ScopeKind::Source,
        source_id,
        &input.settings,
    )
    .await
}

async fn set_video_detectors(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<DetectorSettingsInput>,
) -> Result<StatusCode, ResearchError> {
    set_scope(
        &state,
        corpus_id,
        ScopeKind::Video,
        video_id,
        &input.settings,
    )
    .await
}

async fn get_video_plan(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<VideoDetectorPlan>, ResearchError> {
    let pool = pool(&state).await?;
    crate::detectors::video_plan(&pool, corpus_id, video_id)
        .await
        .map(Json)
        .map_err(map_detector_error)
}

async fn get_processing_status(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<crate::detector_status::VideoProcessingStatus>, ResearchError> {
    let pool = pool(&state).await?;
    crate::detector_status::video_status(&pool, corpus_id, video_id)
        .await
        .map(Json)
        .map_err(map_detector_error)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunPlanInput {
    detectors: Option<Vec<String>>,
    refresh: Option<bool>,
    work_dir: Option<String>,
}

async fn run_video_plan(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id)): AxumPath<(Uuid, Uuid)>,
    input: Option<Json<RunPlanInput>>,
) -> Result<Json<PlanRunReport>, ResearchError> {
    let input = input.map(|Json(input)| input).unwrap_or_default();
    crate::detectors::run_video_plan(PlanRunRequest {
        database_url: database_url(&state)?,
        corpus_id,
        video_id,
        work_dir: PathBuf::from(
            input
                .work_dir
                .as_deref()
                .unwrap_or("use-case-output/youtube-corpus"),
        ),
        yt_dlp: state.yt_dlp.clone(),
        model_dir: PathBuf::from(crate::entities::DEFAULT_MODEL_DIR),
        refresh: input.refresh.unwrap_or(false),
        detectors: input.detectors,
        translation_provider: None,
    })
    .await
    .map(Json)
    .map_err(map_detector_error)
}

#[derive(Debug, Deserialize)]
struct EntityLookupInput {
    q: Option<String>,
    limit: Option<i64>,
}

async fn lookup_entities(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Query(input): Query<EntityLookupInput>,
) -> Result<Json<EntityLookupReport>, ResearchError> {
    let pool = pool(&state).await?;
    let limit = input.limit.unwrap_or(50).clamp(1, 200);
    crate::entities::lookup_entities(&pool, corpus_id, input.q.as_deref().unwrap_or(""), limit)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EntityOccurrencesInput {
    entity_type: String,
    entity_key: String,
}

async fn entity_occurrences(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Query(input): Query<EntityOccurrencesInput>,
) -> Result<Json<EntityOccurrencesReport>, ResearchError> {
    let pool = pool(&state).await?;
    crate::entities::entity_occurrences(&pool, corpus_id, &input.entity_type, &input.entity_key)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}

async fn get_face_matches(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
) -> Result<Json<crate::face_matching::FaceMatchReview>, ResearchError> {
    let pool = pool(&state).await?;
    crate::face_matching::review_face_matches(&pool, corpus)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}
async fn run_face_matches(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
) -> Result<Json<crate::face_matching::FaceMatchReview>, ResearchError> {
    let pool = pool(&state).await?;
    let report = crate::face_matching::match_corpus_faces(&pool, corpus)
        .await
        .map_err(map_corpus_error)?;
    crate::person_names::infer_person_names(&pool, corpus)
        .await
        .map_err(map_corpus_error)?;
    Ok(Json(report))
}

async fn get_person_names(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
) -> Result<Json<crate::person_names::PersonNamesReport>, ResearchError> {
    let pool = pool(&state).await?;
    crate::person_names::review_person_names(&pool, corpus)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}
async fn run_person_names(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
) -> Result<Json<crate::person_names::PersonNamesReport>, ResearchError> {
    let pool = pool(&state).await?;
    crate::person_names::infer_person_names(&pool, corpus)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}
async fn correct_person_name(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
    Json(input): Json<crate::person_names::NameCorrectionRequest>,
) -> Result<Json<crate::person_names::PersonNamesReport>, ResearchError> {
    let pool = pool(&state).await?;
    crate::person_names::correct_person_name(&pool, corpus, input)
        .await
        .map(Json)
        .map_err(|error| {
            if error.is::<crate::person_names::NameCorrectionInputError>() {
                ResearchError::bad_request(error.to_string())
            } else {
                map_corpus_error(error)
            }
        })
}
async fn correct_face_matches(
    State(state): State<ResearchState>,
    AxumPath(corpus): AxumPath<Uuid>,
    Json(input): Json<crate::face_matching::FaceCorrectionRequest>,
) -> Result<Json<crate::face_matching::FaceMatchReview>, ResearchError> {
    let pool = pool(&state).await?;
    crate::face_matching::correct_face_matches(&pool, corpus, input)
        .await
        .map(Json)
        .map_err(|error| {
            if error.is::<crate::face_matching::CorrectionInputError>() {
                ResearchError::bad_request(error.to_string())
            } else {
                map_corpus_error(error)
            }
        })
}

async fn person_occurrences(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, person_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<PersonOccurrencesReport>, ResearchError> {
    let pool = pool(&state).await?;
    crate::entities::person_occurrences(&pool, corpus_id, person_id)
        .await
        .map(Json)
        .map_err(map_corpus_error)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtractEntitiesInput {
    model_dir: Option<String>,
    allow_download: Option<bool>,
}

async fn extract_entities(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    input: Option<Json<ExtractEntitiesInput>>,
) -> Result<Json<EntityExtractionReport>, ResearchError> {
    let input = input.map(|Json(input)| input).unwrap_or_default();
    let pool = pool(&state).await?;
    let model_dir = PathBuf::from(
        input
            .model_dir
            .unwrap_or_else(|| crate::entities::DEFAULT_MODEL_DIR.to_string()),
    );
    let report = crate::entities::extract_corpus_entities(
        &pool,
        corpus_id,
        model_dir,
        input.allow_download.unwrap_or(true),
    )
    .await
    .map_err(map_corpus_error)?;
    crate::person_names::infer_person_names(&pool, corpus_id)
        .await
        .map_err(map_corpus_error)?;
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
        return Err(ResearchError::not_found(
            "Video is not part of this corpus.",
        ));
    }
    let status = crate::detector_status::video_status(&pool, corpus_id, video_id)
        .await
        .map_err(map_detector_error)?;
    if status
        .stages
        .iter()
        .any(|s| matches!(s.state, crate::detector_status::StageState::AwaitingReview))
    {
        return Err(ResearchError::bad_request(
            "Approve this video in its source before reprocessing.",
        ));
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

async fn list_labels(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Vec<CorpusLabel>>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::list_labels(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_label_error)
}

async fn create_label(
    State(state): State<ResearchState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<LabelNameInput>,
) -> Result<(StatusCode, Json<CorpusLabel>), ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::create_label(&pool, corpus_id, &input.name)
        .await
        .map(|label| (StatusCode::CREATED, Json(label)))
        .map_err(map_label_error)
}

async fn rename_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, label_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<LabelNameInput>,
) -> Result<Json<CorpusLabel>, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::rename_label(&pool, corpus_id, label_id, &input.name)
        .await
        .map(Json)
        .map_err(map_label_error)
}

async fn delete_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, label_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::delete_label(&pool, corpus_id, label_id)
        .await
        .map_err(map_label_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn assign_source_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id, label_id)): AxumPath<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ResearchError> {
    set_source_label(state, corpus_id, source_id, label_id, true).await
}

async fn remove_source_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, source_id, label_id)): AxumPath<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ResearchError> {
    set_source_label(state, corpus_id, source_id, label_id, false).await
}

async fn set_source_label(
    state: ResearchState,
    corpus_id: Uuid,
    source_id: Uuid,
    label_id: Uuid,
    assigned: bool,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::set_source_label(&pool, corpus_id, source_id, label_id, assigned)
        .await
        .map_err(map_label_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn assign_video_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id, label_id)): AxumPath<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ResearchError> {
    set_video_label(state, corpus_id, video_id, label_id, true).await
}

async fn remove_video_label(
    State(state): State<ResearchState>,
    AxumPath((corpus_id, video_id, label_id)): AxumPath<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, ResearchError> {
    set_video_label(state, corpus_id, video_id, label_id, false).await
}

async fn set_video_label(
    state: ResearchState,
    corpus_id: Uuid,
    video_id: Uuid,
    label_id: Uuid,
    assigned: bool,
) -> Result<StatusCode, ResearchError> {
    let pool = pool(&state).await?;
    crate::corpora::set_video_label(&pool, corpus_id, video_id, label_id, assigned)
        .await
        .map_err(map_label_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn map_label_error(error: LabelError) -> ResearchError {
    match error {
        LabelError::NotFound(message) => ResearchError::not_found(capitalize(&message)),
        LabelError::BlankName => ResearchError::bad_request("Label name must not be blank."),
        LabelError::DuplicateName(name) => ResearchError {
            status: StatusCode::CONFLICT,
            message: format!("A label named \"{name}\" already exists in this corpus."),
        },
        LabelError::Database(error) => ResearchError::internal(error),
    }
}

fn capitalize(message: &str) -> String {
    let mut characters = message.chars();
    match characters.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), characters.as_str()),
        None => String::new(),
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
        assert_eq!(
            normalized_languages(Some(Vec::new())),
            vec!["en".to_string()]
        );
    }
}
