use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::annotations::{AnnotationSourceKind, CreateAnnotationRequest, ResearchAnnotation};
use crate::api_types::SearchTranscriptsInput;
use crate::evaluation::{EvaluationCase, EvaluationRunReport, RecordJudgmentRequest};
use crate::search::{SearchReport, SearchRequest};
use crate::transcript_quality::TranscriptQualitySummary;

#[derive(Clone)]
struct ResearchQualityState {
    database_url: Option<String>,
}

pub fn router(database_url: Option<String>) -> Router {
    Router::new()
        .route(
            "/api/corpora/{corpus_id}/transcript-quality",
            get(list_quality),
        )
        .route(
            "/api/corpora/{corpus_id}/transcript-quality/refresh",
            post(refresh_quality),
        )
        .route(
            "/api/corpora/{corpus_id}/preferred-search",
            post(preferred_search),
        )
        .route(
            "/api/corpora/{corpus_id}/annotations",
            get(list_annotations).post(create_annotation),
        )
        .route(
            "/api/corpora/{corpus_id}/evaluation/cases",
            get(list_evaluation_cases),
        )
        .route(
            "/api/corpora/{corpus_id}/evaluation/judgments",
            post(record_evaluation_judgment),
        )
        .route(
            "/api/corpora/{corpus_id}/evaluation/run",
            post(run_evaluation),
        )
        .with_state(ResearchQualityState { database_url })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnnotationListInput {
    kind: Option<String>,
    video_id: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAnnotationInput {
    video_id: Uuid,
    stream_id: Option<Uuid>,
    segment_id: Option<Uuid>,
    kind: String,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    label: Option<String>,
    text: Option<String>,
    #[serde(default)]
    payload: Value,
    source_kind: Option<AnnotationSourceKind>,
    processor: Option<String>,
    processor_version: Option<String>,
    #[serde(default)]
    processing_config: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordJudgmentInput {
    query: String,
    mode: crate::config::SearchMode,
    top_k: i64,
    video_id: Option<Uuid>,
    segment_id: Option<Uuid>,
    source_url: Option<String>,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    relevance: Option<u8>,
    notes: Option<String>,
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
struct ResearchQualityError {
    status: StatusCode,
    message: String,
}

impl ResearchQualityError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
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

impl IntoResponse for ResearchQualityError {
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

async fn pool(state: &ResearchQualityState) -> Result<sqlx::PgPool, ResearchQualityError> {
    let database_url = state
        .database_url
        .as_deref()
        .ok_or_else(|| ResearchQualityError::bad_request("DATABASE_URL is required."))?;
    crate::db::connect(database_url)
        .await
        .map_err(ResearchQualityError::internal)
}

fn database_url(state: &ResearchQualityState) -> Result<String, ResearchQualityError> {
    state
        .database_url
        .clone()
        .ok_or_else(|| ResearchQualityError::bad_request("DATABASE_URL is required."))
}

async fn list_quality(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Vec<TranscriptQualitySummary>>, ResearchQualityError> {
    let pool = pool(&state).await?;
    crate::transcript_quality::list_corpus_quality(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_domain_error)
}

async fn refresh_quality(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Vec<TranscriptQualitySummary>>, ResearchQualityError> {
    let pool = pool(&state).await?;
    crate::transcript_quality::refresh_corpus_quality(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_domain_error)
}

async fn preferred_search(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<SearchTranscriptsInput>,
) -> Result<Json<SearchReport>, ResearchQualityError> {
    if input.query.trim().is_empty() {
        return Err(ResearchQualityError::bad_request("query is required."));
    }
    if input.top_k <= 0 {
        return Err(ResearchQualityError::bad_request("topK must be positive."));
    }
    let pool = pool(&state).await?;
    let allowed_video_ids = crate::corpora::corpus_video_ids(&pool, corpus_id)
        .await
        .map_err(map_domain_error)?;
    let preferred_stream_ids = crate::transcript_quality::preferred_stream_ids(&pool)
        .await
        .map_err(ResearchQualityError::internal)?;
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
    .map_err(ResearchQualityError::internal)?;
    report.results.retain(|result| {
        allowed_video_ids.contains(&result.video_id)
            && preferred_stream_ids.contains(&result.stream_id)
    });
    report
        .results
        .truncate(usize::try_from(requested_top_k).unwrap_or(usize::MAX));
    Ok(Json(report))
}

async fn list_annotations(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Query(input): Query<AnnotationListInput>,
) -> Result<Json<Vec<ResearchAnnotation>>, ResearchQualityError> {
    let pool = pool(&state).await?;
    crate::annotations::list_annotations(
        &pool,
        corpus_id,
        input.kind.as_deref(),
        input.video_id,
        input.limit.unwrap_or(100),
    )
    .await
    .map(Json)
    .map_err(map_domain_error)
}

async fn create_annotation(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<CreateAnnotationInput>,
) -> Result<(StatusCode, Json<ResearchAnnotation>), ResearchQualityError> {
    let pool = pool(&state).await?;
    let annotation = crate::annotations::create_annotation(
        &pool,
        CreateAnnotationRequest {
            corpus_id,
            video_id: input.video_id,
            stream_id: input.stream_id,
            segment_id: input.segment_id,
            kind: input.kind,
            start_seconds: input.start_seconds,
            end_seconds: input.end_seconds,
            label: input.label,
            text: input.text,
            payload: input.payload,
            source_kind: input.source_kind.unwrap_or(AnnotationSourceKind::User),
            processor: input.processor,
            processor_version: input.processor_version,
            processing_config: input.processing_config,
        },
    )
    .await
    .map_err(map_domain_error)?;
    Ok((StatusCode::CREATED, Json(annotation)))
}

async fn list_evaluation_cases(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<Vec<EvaluationCase>>, ResearchQualityError> {
    let pool = pool(&state).await?;
    crate::evaluation::list_cases(&pool, corpus_id)
        .await
        .map(Json)
        .map_err(map_domain_error)
}

async fn record_evaluation_judgment(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
    Json(input): Json<RecordJudgmentInput>,
) -> Result<(StatusCode, Json<EvaluationCase>), ResearchQualityError> {
    let pool = pool(&state).await?;
    let case = crate::evaluation::record_judgment(
        &pool,
        RecordJudgmentRequest {
            corpus_id,
            query: input.query,
            mode: input.mode,
            top_k: input.top_k,
            video_id: input.video_id,
            segment_id: input.segment_id,
            source_url: input.source_url,
            start_seconds: input.start_seconds,
            end_seconds: input.end_seconds,
            relevance: input.relevance.unwrap_or(1),
            notes: input.notes,
        },
    )
    .await
    .map_err(map_domain_error)?;
    Ok((StatusCode::CREATED, Json(case)))
}

async fn run_evaluation(
    State(state): State<ResearchQualityState>,
    AxumPath(corpus_id): AxumPath<Uuid>,
) -> Result<Json<EvaluationRunReport>, ResearchQualityError> {
    let pool = pool(&state).await?;
    crate::evaluation::run_evaluation(&pool, &database_url(&state)?, corpus_id)
        .await
        .map(Json)
        .map_err(map_domain_error)
}

fn map_domain_error(error: anyhow::Error) -> ResearchQualityError {
    let message = error.to_string();
    if message.contains("required")
        || message.contains("must")
        || message.contains("not part")
        || message.contains("not found")
        || message.contains("no retrieval evaluation")
        || message.contains("none contain relevance")
    {
        ResearchQualityError::bad_request(message)
    } else {
        ResearchQualityError::internal(error)
    }
}
