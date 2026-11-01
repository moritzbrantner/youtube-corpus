//! Shared processing workflow definitions (#108, Atlas 6.1-6.2).
//!
//! A definition is authored in the shared Workflow Editor and compiled there to
//! `@moritzbrantner/workflow/compiled` v1. Its executable nodes are this repository's detector
//! stages (`youtube_corpus.detector.<id>`); corpus processing translates it into the corpus's
//! detector plan and runs that through the existing plan runner (`run_video_plan`), so stage
//! reuse, attempts, retries and media lifecycle stay exactly what they were. There is no second
//! scheduler: this module only stores definitions, a corpus's overrides and its run history.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::YtDlpConfig;
use crate::detectors::{DetectorError, DetectorSetting, PlanRunRequest, ScopeKind, StageReport};

/// The Workflow Editor's compiled handoff format this host accepts.
pub const COMPILED_FORMAT: &str = "@moritzbrantner/workflow/compiled";
pub const COMPILED_VERSION: u64 = 1;
/// Node kind of a detector stage: `youtube_corpus.detector.asr`.
pub const DETECTOR_KIND_PREFIX: &str = "youtube_corpus.detector.";
/// Structural nodes a definition may contain; they run nothing.
const STRUCTURAL_KINDS: [&str; 2] = ["control.start", "control.end"];

/// One detector stage of a compiled definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStage {
    pub node_id: String,
    pub detector: String,
    pub label: String,
    pub target_languages: Vec<String>,
}

/// Reads the detector stages of a compiled definition in its topological order. Rejects any
/// node this host has no stage for instead of giving it invented semantics.
pub fn stages_from_compiled(compiled: &Value) -> Result<Vec<WorkflowStage>, DetectorError> {
    let invalid = |message: String| DetectorError::Invalid(message);
    if compiled.get("format").and_then(Value::as_str) != Some(COMPILED_FORMAT) {
        return Err(invalid(format!(
            "the definition is not a {COMPILED_FORMAT} document"
        )));
    }
    if compiled.get("version").and_then(Value::as_u64) != Some(COMPILED_VERSION) {
        return Err(invalid(format!(
            "corpus processing runs compiled version {COMPILED_VERSION} definitions (no iterations or workflow references)"
        )));
    }
    let nodes = compiled
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("the definition has no nodes".to_string()))?;
    let by_id: BTreeMap<&str, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node.get("id")?.as_str()?, node)))
        .collect();
    let order: Vec<&str> = match compiled.get("order").and_then(Value::as_array) {
        Some(order) => order.iter().filter_map(Value::as_str).collect(),
        None => by_id.keys().copied().collect(),
    };
    let mut seen = BTreeSet::new();
    let mut stages = Vec::new();
    for id in order {
        let node = by_id
            .get(id)
            .ok_or_else(|| invalid(format!("order names unknown node {id}")))?;
        let kind = node.get("kind").and_then(Value::as_str).unwrap_or_default();
        if STRUCTURAL_KINDS.contains(&kind) {
            continue;
        }
        let Some(detector_id) = kind.strip_prefix(DETECTOR_KIND_PREFIX) else {
            return Err(invalid(format!(
                "node {id} has kind {kind:?}, which corpus processing cannot run; use {DETECTOR_KIND_PREFIX}<detector> stages and control.start/control.end"
            )));
        };
        let info = crate::detectors::detector(detector_id)
            .ok_or_else(|| invalid(format!("node {id} names unknown detector {detector_id}")))?;
        if !seen.insert(info.id) {
            return Err(invalid(format!(
                "detector {} appears in more than one stage",
                info.id
            )));
        }
        let data = node.get("data");
        let target_languages = languages(data.and_then(|data| data.get("targetLanguages")))
            .map_err(|message| invalid(format!("node {id}: {message}")))?;
        if !info.takes_languages && !target_languages.is_empty() {
            return Err(invalid(format!(
                "node {id}: detector {} does not take target languages",
                info.id
            )));
        }
        stages.push(WorkflowStage {
            node_id: id.to_string(),
            detector: info.id.to_string(),
            label: node
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
                .unwrap_or(info.label)
                .to_string(),
            target_languages,
        });
    }
    if stages.is_empty() {
        return Err(invalid("the definition has no detector stages".to_string()));
    }
    Ok(stages)
}

/// `targetLanguages` as a list of language codes; the editor may also store `"de, fr"`.
fn languages(value: Option<&Value>) -> Result<Vec<String>, String> {
    let codes: Vec<String> = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(text)) => text.split([',', ' ']).map(str::to_string).collect(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "targetLanguages must be strings".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("targetLanguages must be a list of language codes".to_string()),
    };
    let mut unique = Vec::new();
    for code in codes.into_iter().map(|code| code.trim().to_lowercase()) {
        if !code.is_empty() && !unique.contains(&code) {
            unique.push(code);
        }
    }
    Ok(unique)
}

/// `sha256:<hex>` of the definition's canonical JSON (keys sorted, no whitespace), matching the
/// Workflow Editor's compiled-workflow fingerprint convention.
pub fn fingerprint(compiled: &Value) -> String {
    fn canonical(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                out.push('{');
                let sorted: BTreeMap<&String, &Value> = map.iter().collect();
                for (index, (key, value)) in sorted.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    canonical(value, out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    canonical(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut json = String::new();
    canonical(compiled, &mut json);
    let digest = Sha256::digest(json.as_bytes());
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: Uuid,
    pub name: String,
    pub version: i32,
    pub fingerprint: String,
    pub stages: Vec<WorkflowStage>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The latest version of a definition, with the editor document to reopen it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinition {
    #[serde(flatten)]
    pub summary: WorkflowSummary,
    pub document: Value,
    pub compiled: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkflow {
    pub name: String,
    pub document: Value,
    pub compiled: Value,
}

type VersionRow = (
    Uuid,
    String,
    chrono::DateTime<chrono::Utc>,
    i32,
    String,
    Value,
    Value,
);

macro_rules! latest_versions {
    () => {
        "SELECT w.id, w.name, w.updated_at, v.version, v.fingerprint, v.document, v.compiled
         FROM processing_workflows w
         JOIN LATERAL (
             SELECT * FROM processing_workflow_versions
             WHERE workflow_id = w.id ORDER BY version DESC LIMIT 1
         ) v ON true"
    };
}

fn definition(row: VersionRow) -> Result<WorkflowDefinition, DetectorError> {
    let (id, name, updated_at, version, fingerprint, document, compiled) = row;
    Ok(WorkflowDefinition {
        summary: WorkflowSummary {
            id,
            name,
            version,
            fingerprint,
            stages: stages_from_compiled(&compiled)?,
            updated_at,
        },
        document,
        compiled,
    })
}

pub async fn list_workflows(pool: &PgPool) -> Result<Vec<WorkflowSummary>, DetectorError> {
    let rows: Vec<VersionRow> =
        sqlx::query_as(concat!(latest_versions!(), " ORDER BY w.name, w.id"))
            .fetch_all(pool)
            .await?;
    rows.into_iter()
        .map(|row| definition(row).map(|definition| definition.summary))
        .collect()
}

pub async fn get_workflow(pool: &PgPool, id: Uuid) -> Result<WorkflowDefinition, DetectorError> {
    let row: Option<VersionRow> = sqlx::query_as(concat!(latest_versions!(), " WHERE w.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    definition(row.ok_or_else(|| DetectorError::NotFound(format!("workflow {id} not found")))?)
}

/// Creates a definition (`id: None`) or saves a new version of one. Saving an unchanged
/// compiled definition keeps its version and only updates the name and editor document.
pub async fn save_workflow(
    pool: &PgPool,
    id: Option<Uuid>,
    input: SaveWorkflow,
) -> Result<WorkflowDefinition, DetectorError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(DetectorError::Invalid(
            "the workflow needs a name".to_string(),
        ));
    }
    stages_from_compiled(&input.compiled)?;
    let fingerprint = fingerprint(&input.compiled);
    let mut tx = pool.begin().await?;
    let id = match id {
        None => {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO processing_workflows (id, name) VALUES ($1, $2)")
                .bind(id)
                .bind(name)
                .execute(&mut *tx)
                .await?;
            id
        }
        Some(id) => {
            let updated = sqlx::query(
                "UPDATE processing_workflows SET name = $2, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() == 0 {
                return Err(DetectorError::NotFound(format!("workflow {id} not found")));
            }
            id
        }
    };
    let latest: Option<(i32, String)> = sqlx::query_as(
        "SELECT version, fingerprint FROM processing_workflow_versions
         WHERE workflow_id = $1 ORDER BY version DESC LIMIT 1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    match latest {
        Some((version, current)) if current == fingerprint => {
            sqlx::query(
                "UPDATE processing_workflow_versions SET document = $3
                 WHERE workflow_id = $1 AND version = $2",
            )
            .bind(id)
            .bind(version)
            .bind(&input.document)
            .execute(&mut *tx)
            .await?;
        }
        latest => {
            sqlx::query(
                "INSERT INTO processing_workflow_versions
                   (workflow_id, version, document, compiled, fingerprint)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(latest.map_or(1, |(version, _)| version + 1))
            .bind(&input.document)
            .bind(&input.compiled)
            .bind(&fingerprint)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    get_workflow(pool, id).await
}

/// One corpus's override of one definition stage. Absent fields keep the definition's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageOverride {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_languages: Option<Vec<String>>,
}

/// A definition stage as this corpus runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveStage {
    #[serde(flatten)]
    pub stage: WorkflowStage,
    pub enabled: bool,
    /// The definition's own target languages, when this corpus overrides them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_target_languages: Option<Vec<String>>,
    /// Whether this corpus overrides the definition for this stage.
    pub overridden: bool,
}

pub fn effective_stages(
    stages: &[WorkflowStage],
    overrides: &[StageOverride],
) -> Vec<EffectiveStage> {
    stages
        .iter()
        .map(|stage| {
            let found = overrides
                .iter()
                .find(|entry| entry.node_id == stage.node_id);
            let mut effective = EffectiveStage {
                stage: stage.clone(),
                enabled: found.and_then(|entry| entry.enabled).unwrap_or(true),
                definition_target_languages: None,
                overridden: found.is_some(),
            };
            if let Some(languages) = found.and_then(|entry| entry.target_languages.clone()) {
                effective.definition_target_languages = Some(stage.target_languages.clone());
                effective.stage.target_languages = languages;
            }
            effective
        })
        .collect()
}

/// The corpus detector plan a definition translates to: every enabled stage, nothing else.
pub fn detector_settings(stages: &[EffectiveStage]) -> Vec<DetectorSetting> {
    stages
        .iter()
        .filter(|stage| stage.enabled)
        .map(|stage| DetectorSetting {
            detector: stage.stage.detector.clone(),
            enabled: true,
            target_languages: stage.stage.target_languages.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunVideo {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub stages: Vec<StageReport>,
    /// Why the video's plan did not run at all.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub videos: Vec<RunVideo>,
    pub failures: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: Uuid,
    pub corpus_id: Uuid,
    pub workflow_id: Uuid,
    pub workflow_version: i32,
    pub fingerprint: String,
    pub stages: Vec<EffectiveStage>,
    pub refresh: bool,
    pub state: String,
    pub report: Option<RunReport>,
    pub message: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The definition as one corpus sees it: its effective stages and this corpus's runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusWorkflow {
    pub workflow: WorkflowSummary,
    pub overrides: Vec<StageOverride>,
    pub stages: Vec<EffectiveStage>,
    pub runs: Vec<WorkflowRun>,
}

/// Runs this process is executing. A `running` row nobody executes was cut off by a restart.
static ACTIVE_RUNS: LazyLock<Mutex<HashSet<Uuid>>> = LazyLock::new(Default::default);

async fn require_corpus(pool: &PgPool, corpus_id: Uuid) -> Result<(), DetectorError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM corpora WHERE id = $1)")
        .bind(corpus_id)
        .fetch_one(pool)
        .await?;
    if exists {
        Ok(())
    } else {
        Err(DetectorError::NotFound(format!(
            "corpus {corpus_id} not found"
        )))
    }
}

async fn overrides(
    pool: &PgPool,
    corpus_id: Uuid,
    workflow_id: Uuid,
) -> Result<Vec<StageOverride>, DetectorError> {
    let stored: Option<Value> = sqlx::query_scalar(
        "SELECT stages FROM corpus_workflow_settings WHERE corpus_id = $1 AND workflow_id = $2",
    )
    .bind(corpus_id)
    .bind(workflow_id)
    .fetch_optional(pool)
    .await?;
    Ok(stored
        .map(serde_json::from_value)
        .transpose()
        .map_err(anyhow::Error::from)?
        .unwrap_or_default())
}

type RunRow = (
    Uuid,
    Uuid,
    Uuid,
    i32,
    String,
    Value,
    bool,
    String,
    Option<Value>,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
);

macro_rules! select_runs {
    () => {
        "SELECT id, corpus_id, workflow_id, workflow_version, fingerprint, stages, refresh, state,
                report, message, started_at, finished_at
         FROM corpus_workflow_runs "
    };
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, DetectorError> {
    serde_json::from_value(value).map_err(|error| DetectorError::Other(error.into()))
}

fn run(row: RunRow) -> Result<WorkflowRun, DetectorError> {
    Ok(WorkflowRun {
        id: row.0,
        corpus_id: row.1,
        workflow_id: row.2,
        workflow_version: row.3,
        fingerprint: row.4,
        stages: decode(row.5)?,
        refresh: row.6,
        state: row.7,
        report: row.8.map(decode).transpose()?,
        message: row.9,
        started_at: row.10,
        finished_at: row.11,
    })
}

async fn mark_interrupted(pool: &PgPool, corpus_id: Uuid) -> Result<(), DetectorError> {
    let active: Vec<Uuid> = ACTIVE_RUNS
        .lock()
        .expect("active runs lock")
        .iter()
        .copied()
        .collect();
    sqlx::query(
        "UPDATE corpus_workflow_runs
         SET state = 'interrupted', finished_at = now(),
             message = 'The server stopped before this run finished; stage attempts it recorded remain.'
         WHERE corpus_id = $1 AND state = 'running' AND NOT (id = ANY($2))",
    )
    .bind(corpus_id)
    .bind(&active)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn corpus_workflow(
    pool: &PgPool,
    corpus_id: Uuid,
    workflow_id: Uuid,
) -> Result<CorpusWorkflow, DetectorError> {
    require_corpus(pool, corpus_id).await?;
    let workflow = get_workflow(pool, workflow_id).await?.summary;
    let overrides = overrides(pool, corpus_id, workflow_id).await?;
    mark_interrupted(pool, corpus_id).await?;
    let rows: Vec<RunRow> = sqlx::query_as(concat!(
        select_runs!(),
        "WHERE corpus_id = $1 AND workflow_id = $2 ORDER BY started_at DESC, id LIMIT 50"
    ))
    .bind(corpus_id)
    .bind(workflow_id)
    .fetch_all(pool)
    .await?;
    Ok(CorpusWorkflow {
        stages: effective_stages(&workflow.stages, &overrides),
        workflow,
        overrides,
        runs: rows.into_iter().map(run).collect::<Result<_, _>>()?,
    })
}

pub async fn get_run(
    pool: &PgPool,
    corpus_id: Uuid,
    run_id: Uuid,
) -> Result<WorkflowRun, DetectorError> {
    mark_interrupted(pool, corpus_id).await?;
    let row: Option<RunRow> =
        sqlx::query_as(concat!(select_runs!(), "WHERE corpus_id = $1 AND id = $2"))
            .bind(corpus_id)
            .bind(run_id)
            .fetch_optional(pool)
            .await?;
    run(row.ok_or_else(|| DetectorError::NotFound(format!("workflow run {run_id} not found")))?)
}

/// Replaces this corpus's overrides of a definition. Overrides for stages the latest version
/// no longer has are rejected, as are languages for stages that take none.
pub async fn set_corpus_overrides(
    pool: &PgPool,
    corpus_id: Uuid,
    workflow_id: Uuid,
    stages: &[StageOverride],
) -> Result<CorpusWorkflow, DetectorError> {
    require_corpus(pool, corpus_id).await?;
    let workflow = get_workflow(pool, workflow_id).await?.summary;
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for entry in stages {
        let stage = workflow
            .stages
            .iter()
            .find(|stage| stage.node_id == entry.node_id)
            .ok_or_else(|| {
                DetectorError::Invalid(format!("the workflow has no stage {}", entry.node_id))
            })?;
        if !seen.insert(entry.node_id.as_str()) {
            return Err(DetectorError::Invalid(format!(
                "stage {} is listed twice",
                entry.node_id
            )));
        }
        let target_languages = match &entry.target_languages {
            Some(codes) => {
                let takes = crate::detectors::detector(&stage.detector)
                    .is_some_and(|info| info.takes_languages);
                if !takes {
                    return Err(DetectorError::Invalid(format!(
                        "stage {} does not take target languages",
                        entry.node_id
                    )));
                }
                Some(languages(Some(&Value::from(codes.clone()))).map_err(DetectorError::Invalid)?)
            }
            None => None,
        };
        if entry.enabled.is_some() || target_languages.is_some() {
            normalized.push(StageOverride {
                node_id: entry.node_id.clone(),
                enabled: entry.enabled,
                target_languages,
            });
        }
    }
    sqlx::query(
        "INSERT INTO corpus_workflow_settings (corpus_id, workflow_id, stages)
         VALUES ($1, $2, $3)
         ON CONFLICT (corpus_id, workflow_id)
         DO UPDATE SET stages = EXCLUDED.stages, updated_at = now()",
    )
    .bind(corpus_id)
    .bind(workflow_id)
    .bind(serde_json::to_value(&normalized).map_err(anyhow::Error::from)?)
    .execute(pool)
    .await?;
    corpus_workflow(pool, corpus_id, workflow_id).await
}

#[derive(Debug, Clone)]
pub struct StartRun {
    pub database_url: String,
    pub corpus_id: Uuid,
    pub workflow_id: Uuid,
    pub refresh: bool,
    pub work_dir: PathBuf,
    pub model_dir: PathBuf,
    pub yt_dlp: YtDlpConfig,
}

/// Applies the definition's effective stages as the corpus detector plan and runs every corpus
/// video's plan in the background. Returns the recorded run at once; its report fills in when
/// every video finished.
pub async fn start_run(request: StartRun) -> Result<WorkflowRun, DetectorError> {
    let pool = crate::db::connect(&request.database_url).await?;
    let current = corpus_workflow(&pool, request.corpus_id, request.workflow_id).await?;
    if current.runs.iter().any(|run| run.state == "running") {
        return Err(DetectorError::Invalid(
            "this workflow is already running for this corpus".to_string(),
        ));
    }
    let settings = detector_settings(&current.stages);
    if settings.is_empty() {
        return Err(DetectorError::Invalid(
            "every stage of this workflow is disabled for this corpus".to_string(),
        ));
    }
    crate::detectors::set_scope_settings(
        &pool,
        request.corpus_id,
        ScopeKind::Corpus,
        request.corpus_id,
        &settings,
    )
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO corpus_workflow_runs
           (id, corpus_id, workflow_id, workflow_version, fingerprint, stages, refresh, state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'running')",
    )
    .bind(id)
    .bind(request.corpus_id)
    .bind(request.workflow_id)
    .bind(current.workflow.version)
    .bind(&current.workflow.fingerprint)
    .bind(serde_json::to_value(&current.stages).map_err(anyhow::Error::from)?)
    .bind(request.refresh)
    .execute(&pool)
    .await?;
    ACTIVE_RUNS.lock().expect("active runs lock").insert(id);
    let run = get_run(&pool, request.corpus_id, id).await?;
    tokio::spawn(async move {
        let outcome = execute_run(&pool, &request).await;
        let (state, report, message) = match outcome {
            Ok(report) => ("completed", Some(report), None),
            Err(error) => ("failed", None, Some(format!("{error:#}"))),
        };
        let stored = sqlx::query(
            "UPDATE corpus_workflow_runs
             SET state = $2, report = $3, message = $4, finished_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(state)
        .bind(report.map(|report| serde_json::to_value(report).unwrap_or(Value::Null)))
        .bind(message)
        .execute(&pool)
        .await;
        if let Err(error) = stored {
            tracing::error!(%error, run = %id, "failed to record workflow run outcome");
        }
        ACTIVE_RUNS.lock().expect("active runs lock").remove(&id);
    });
    Ok(run)
}

async fn execute_run(pool: &PgPool, request: &StartRun) -> anyhow::Result<RunReport> {
    let videos: Vec<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT v.id, v.title FROM corpus_videos cv JOIN videos v ON v.id = cv.video_id
         WHERE cv.corpus_id = $1 ORDER BY v.upload_date NULLS LAST, v.title, v.id",
    )
    .bind(request.corpus_id)
    .fetch_all(pool)
    .await?;
    let mut report = RunReport::default();
    for (video_id, title) in videos {
        let outcome = crate::detectors::run_video_plan(PlanRunRequest {
            database_url: request.database_url.clone(),
            corpus_id: request.corpus_id,
            video_id,
            work_dir: request.work_dir.clone(),
            yt_dlp: request.yt_dlp.clone(),
            model_dir: request.model_dir.clone(),
            refresh: request.refresh,
            detectors: None,
            translation_provider: None,
            keep_media: false,
            transcriber: None,
        })
        .await;
        let video = match outcome {
            Ok(plan) => RunVideo {
                video_id,
                title,
                stages: plan.stages,
                error: None,
            },
            Err(error) => RunVideo {
                video_id,
                title,
                stages: Vec::new(),
                error: Some(error.to_string()),
            },
        };
        report.failures += usize::from(video.error.is_some())
            + video
                .stages
                .iter()
                .filter(|stage| stage.outcome == crate::detectors::StageOutcome::Failed)
                .count();
        report.videos.push(video);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compiled(nodes: Value) -> Value {
        let order: Vec<Value> = nodes
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].clone())
            .collect();
        json!({ "format": COMPILED_FORMAT, "version": 1, "nodes": nodes, "edges": [], "order": order })
    }

    #[test]
    fn reads_detector_stages_in_order_and_skips_structural_nodes() {
        let definition = compiled(json!([
            { "id": "start", "kind": "control.start", "outputs": [] },
            { "id": "subs", "label": "Captions", "kind": "youtube_corpus.detector.captions" },
            { "id": "de", "kind": "youtube_corpus.detector.translate_spoken",
              "data": { "targetLanguages": "de, FR" } },
            { "id": "end", "kind": "control.end" }
        ]));
        let stages = stages_from_compiled(&definition).unwrap();
        assert_eq!(
            stages,
            [
                WorkflowStage {
                    node_id: "subs".into(),
                    detector: "captions".into(),
                    label: "Captions".into(),
                    target_languages: vec![],
                },
                WorkflowStage {
                    node_id: "de".into(),
                    detector: "translate_spoken".into(),
                    label: "Translate spoken transcript".into(),
                    target_languages: vec!["de".into(), "fr".into()],
                },
            ]
        );
    }

    #[test]
    fn rejects_nodes_without_a_corpus_stage() {
        for (nodes, needle) in [
            (
                json!([{ "id": "http", "kind": "http.request" }]),
                "cannot run",
            ),
            (
                json!([{ "id": "x", "kind": "youtube_corpus.detector.nope" }]),
                "unknown detector",
            ),
            (
                json!([
                    { "id": "a", "kind": "youtube_corpus.detector.asr" },
                    { "id": "b", "kind": "youtube_corpus.detector.asr" }
                ]),
                "more than one stage",
            ),
            (
                json!([{ "id": "a", "kind": "youtube_corpus.detector.asr", "data": { "targetLanguages": ["de"] } }]),
                "does not take target languages",
            ),
            (
                json!([{ "id": "s", "kind": "control.start" }]),
                "no detector stages",
            ),
        ] {
            let error = stages_from_compiled(&compiled(nodes))
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "{error}");
        }
        let mut v2 = compiled(json!([{ "id": "a", "kind": "youtube_corpus.detector.asr" }]));
        v2["version"] = json!(2);
        assert!(stages_from_compiled(&v2).is_err());
    }

    #[test]
    fn corpus_overrides_change_enablement_and_languages_only_for_that_corpus() {
        let stages = stages_from_compiled(&compiled(json!([
            { "id": "subs", "kind": "youtube_corpus.detector.captions" },
            { "id": "asr", "kind": "youtube_corpus.detector.asr" },
            { "id": "tr", "kind": "youtube_corpus.detector.translate_spoken",
              "data": { "targetLanguages": ["de"] } }
        ])))
        .unwrap();
        let effective = effective_stages(
            &stages,
            &[
                StageOverride {
                    node_id: "asr".into(),
                    enabled: Some(false),
                    target_languages: None,
                },
                StageOverride {
                    node_id: "tr".into(),
                    enabled: None,
                    target_languages: Some(vec!["fr".into()]),
                },
            ],
        );
        assert_eq!(
            detector_settings(&effective),
            [
                DetectorSetting {
                    detector: "captions".into(),
                    enabled: true,
                    target_languages: vec![]
                },
                DetectorSetting {
                    detector: "translate_spoken".into(),
                    enabled: true,
                    target_languages: vec!["fr".into()]
                },
            ]
        );
        assert_eq!(
            effective[2].definition_target_languages,
            Some(vec!["de".to_string()])
        );
        assert_eq!(detector_settings(&effective_stages(&stages, &[])).len(), 3);
    }

    #[test]
    fn fingerprint_ignores_key_order() {
        let a = json!({ "format": COMPILED_FORMAT, "version": 1, "nodes": [{ "id": "a", "kind": "k" }] });
        let b = json!({ "nodes": [{ "kind": "k", "id": "a" }], "version": 1, "format": COMPILED_FORMAT });
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert!(fingerprint(&a).starts_with("sha256:"));
    }
}
