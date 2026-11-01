//! HTTP API for shared processing workflow definitions (#108).

use std::path::PathBuf;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::YtDlpConfig;
use crate::detectors::DetectorError;
use crate::processing_workflows::{
    CorpusWorkflow, SaveWorkflow, StageOverride, StartRun, WorkflowDefinition, WorkflowRun,
    WorkflowSummary,
};

#[derive(Clone)]
struct WorkflowState {
    database_url: Option<String>,
    yt_dlp: YtDlpConfig,
}

pub fn router(database_url: Option<String>, yt_dlp: YtDlpConfig) -> Router {
    Router::new()
        .route(
            "/api/processing-workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/api/processing-workflows/{workflow_id}",
            get(get_workflow).put(update_workflow),
        )
        .route(
            "/api/corpora/{corpus_id}/processing-workflows/{workflow_id}",
            get(corpus_workflow),
        )
        .route(
            "/api/corpora/{corpus_id}/processing-workflows/{workflow_id}/overrides",
            put(set_overrides),
        )
        .route(
            "/api/corpora/{corpus_id}/processing-workflows/{workflow_id}/runs",
            post(start_run),
        )
        .route(
            "/api/corpora/{corpus_id}/processing-workflow-runs/{run_id}",
            get(get_run),
        )
        .with_state(WorkflowState {
            database_url,
            yt_dlp,
        })
}

struct ApiError(StatusCode, String);

impl From<DetectorError> for ApiError {
    fn from(error: DetectorError) -> Self {
        match error {
            DetectorError::NotFound(message) => Self(StatusCode::NOT_FOUND, message),
            DetectorError::Invalid(message) => Self(StatusCode::BAD_REQUEST, message),
            DetectorError::Other(error) => {
                Self(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}"))
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(serde_json::json!({ "error": { "message": self.1 } })),
        )
            .into_response()
    }
}

fn database_url(state: &WorkflowState) -> Result<&str, ApiError> {
    state
        .database_url
        .as_deref()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "DATABASE_URL is required.".into()))
}

async fn pool(state: &WorkflowState) -> Result<sqlx::PgPool, ApiError> {
    crate::db::connect(database_url(state)?)
        .await
        .map_err(|error| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")))
}

async fn list_workflows(
    State(state): State<WorkflowState>,
) -> Result<Json<Vec<WorkflowSummary>>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::list_workflows(&pool).await?,
    ))
}

async fn create_workflow(
    State(state): State<WorkflowState>,
    Json(input): Json<SaveWorkflow>,
) -> Result<(StatusCode, Json<WorkflowDefinition>), ApiError> {
    let pool = pool(&state).await?;
    let saved = crate::processing_workflows::save_workflow(&pool, None, input).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn get_workflow(
    State(state): State<WorkflowState>,
    AxumPath(workflow_id): AxumPath<Uuid>,
) -> Result<Json<WorkflowDefinition>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::get_workflow(&pool, workflow_id).await?,
    ))
}

async fn update_workflow(
    State(state): State<WorkflowState>,
    AxumPath(workflow_id): AxumPath<Uuid>,
    Json(input): Json<SaveWorkflow>,
) -> Result<Json<WorkflowDefinition>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::save_workflow(&pool, Some(workflow_id), input).await?,
    ))
}

async fn corpus_workflow(
    State(state): State<WorkflowState>,
    AxumPath((corpus_id, workflow_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<CorpusWorkflow>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::corpus_workflow(&pool, corpus_id, workflow_id).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverridesInput {
    stages: Vec<StageOverride>,
}

async fn set_overrides(
    State(state): State<WorkflowState>,
    AxumPath((corpus_id, workflow_id)): AxumPath<(Uuid, Uuid)>,
    Json(input): Json<OverridesInput>,
) -> Result<Json<CorpusWorkflow>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::set_corpus_overrides(
            &pool,
            corpus_id,
            workflow_id,
            &input.stages,
        )
        .await?,
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartRunInput {
    #[serde(default)]
    refresh: bool,
    work_dir: Option<String>,
}

async fn start_run(
    State(state): State<WorkflowState>,
    AxumPath((corpus_id, workflow_id)): AxumPath<(Uuid, Uuid)>,
    input: Option<Json<StartRunInput>>,
) -> Result<(StatusCode, Json<WorkflowRun>), ApiError> {
    let input = input.map(|Json(input)| input).unwrap_or_default();
    let run = crate::processing_workflows::start_run(StartRun {
        database_url: database_url(&state)?.to_string(),
        corpus_id,
        workflow_id,
        refresh: input.refresh,
        work_dir: PathBuf::from(
            input
                .work_dir
                .as_deref()
                .unwrap_or("use-case-output/youtube-corpus"),
        ),
        model_dir: PathBuf::from(crate::entities::DEFAULT_MODEL_DIR),
        yt_dlp: state.yt_dlp.clone(),
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

async fn get_run(
    State(state): State<WorkflowState>,
    AxumPath((corpus_id, run_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<WorkflowRun>, ApiError> {
    let pool = pool(&state).await?;
    Ok(Json(
        crate::processing_workflows::get_run(&pool, corpus_id, run_id).await?,
    ))
}
