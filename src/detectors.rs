//! Detector profiles and effective per-video detector plans (Atlas 2.3, 2.6, 2.8, 2.10; #63).
//!
//! A corpus supplies the default selection. A tracked source of the corpus may switch single
//! detectors on or off relative to that default, and a video may be overridden within the corpus
//! only. A video's inherited plan is the union of every applicable contributor (each tracked
//! source that discovered it, plus the corpus default when the video was added directly or has
//! no tracked source). Detector algorithms stay in their owning capability repositories; this
//! module only configures, previews and runs the producers this repository already integrates.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::YtDlpConfig;

/// One analysis capability that can be requested for a video.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub producer: &'static str,
    /// Whether this repository can run the producer today.
    pub available: bool,
    /// Why the detector cannot run, or what it needs.
    pub note: Option<&'static str>,
    /// Whether the detector takes target languages.
    pub takes_languages: bool,
}

pub const DETECTORS: &[DetectorInfo] = &[
    DetectorInfo {
        id: "captions",
        label: "YouTube captions",
        producer: "yt-dlp captions",
        available: true,
        note: None,
        takes_languages: false,
    },
    DetectorInfo {
        id: "asr",
        label: "Speech transcription (ASR)",
        producer: "external transcriber command (Whisper-compatible)",
        available: true,
        note: Some("Needs a Whisper-compatible transcriber on PATH."),
        takes_languages: false,
    },
    DetectorInfo {
        id: "scenes",
        label: "Scene detection",
        producer: "visual-analysis scenedetect-core:content",
        available: true,
        note: Some("Needs the video's media retained locally."),
        takes_languages: false,
    },
    DetectorInfo {
        id: "ocr",
        label: "On-screen text (OCR)",
        producer: "visual-analysis OCR",
        available: false,
        note: Some(
            "No OCR producer is integrated yet (#71); existing OCR evidence stays searchable.",
        ),
        takes_languages: false,
    },
    DetectorInfo {
        id: "entities",
        label: "Named entities",
        producer: "nlp-stack text-linguistics bert-base-NER",
        available: true,
        note: None,
        takes_languages: false,
    },
    DetectorInfo {
        id: "translate_spoken",
        label: "Translate spoken transcript",
        producer: "translation (#69)",
        available: false,
        note: Some("Translation generation is not implemented yet (#69)."),
        takes_languages: true,
    },
    DetectorInfo {
        id: "translate_ocr",
        label: "Translate on-screen text",
        producer: "translation (#69)",
        available: false,
        note: Some("Translation generation is not implemented yet (#69)."),
        takes_languages: true,
    },
];

pub fn detector(id: &str) -> Option<&'static DetectorInfo> {
    DETECTORS.iter().find(|detector| detector.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Corpus,
    Source,
    Video,
}

impl ScopeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Corpus => "corpus",
            Self::Source => "source",
            Self::Video => "video",
        }
    }
}

/// An explicit setting of one detector in one scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorSetting {
    pub detector: String,
    pub enabled: bool,
    #[serde(default)]
    pub target_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    pub source_id: Uuid,
    pub name: Option<String>,
    pub source_url: String,
    pub settings: Vec<DetectorSetting>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusDetectorProfiles {
    pub detectors: &'static [DetectorInfo],
    pub corpus: Vec<DetectorSetting>,
    pub sources: Vec<SourceProfile>,
}

#[derive(Debug, thiserror::Error)]
pub enum DetectorError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for DetectorError {
    fn from(error: sqlx::Error) -> Self {
        Self::Other(error.into())
    }
}

/// Replaces the explicit settings of one scope. For a source or video, detectors left out
/// inherit again; for the corpus, detectors left out are off.
pub async fn set_scope_settings(
    pool: &PgPool,
    corpus_id: Uuid,
    scope: ScopeKind,
    scope_id: Uuid,
    settings: &[DetectorSetting],
) -> Result<(), DetectorError> {
    require_scope(pool, corpus_id, scope, scope_id).await?;
    let mut seen = BTreeSet::new();
    for setting in settings {
        let info = detector(&setting.detector).ok_or_else(|| {
            DetectorError::Invalid(format!("unknown detector {}", setting.detector))
        })?;
        if !seen.insert(setting.detector.as_str()) {
            return Err(DetectorError::Invalid(format!(
                "detector {} is listed twice",
                setting.detector
            )));
        }
        if !info.takes_languages && !setting.target_languages.is_empty() {
            return Err(DetectorError::Invalid(format!(
                "detector {} does not take target languages",
                setting.detector
            )));
        }
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM detector_settings WHERE corpus_id = $1 AND scope_kind = $2 AND scope_id = $3",
    )
    .bind(corpus_id)
    .bind(scope.as_str())
    .bind(scope_id)
    .execute(&mut *tx)
    .await?;
    for setting in settings {
        sqlx::query(
            "INSERT INTO detector_settings
               (corpus_id, scope_kind, scope_id, detector, enabled, target_languages)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(corpus_id)
        .bind(scope.as_str())
        .bind(scope_id)
        .bind(&setting.detector)
        .bind(setting.enabled)
        .bind(normalize_languages(&setting.target_languages))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn normalize_languages(languages: &[String]) -> Vec<String> {
    languages
        .iter()
        .map(|language| language.trim().to_lowercase())
        .filter(|language| !language.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

async fn require_scope(
    pool: &PgPool,
    corpus_id: Uuid,
    scope: ScopeKind,
    scope_id: Uuid,
) -> Result<(), DetectorError> {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM corpora WHERE id = $1)")
            .bind(corpus_id)
            .fetch_one(pool)
            .await?;
    if !exists {
        return Err(DetectorError::NotFound("corpus not found".to_string()));
    }
    let belongs = match scope {
        ScopeKind::Corpus => scope_id == corpus_id,
        ScopeKind::Source => {
            crate::corpora::source_belongs_to_corpus(pool, corpus_id, scope_id).await?
        }
        ScopeKind::Video => crate::corpora::corpus_video_ids(pool, corpus_id)
            .await?
            .contains(&scope_id),
    };
    if !belongs {
        return Err(DetectorError::NotFound(format!(
            "{} is not part of this corpus",
            scope.as_str()
        )));
    }
    Ok(())
}

async fn scope_settings(
    pool: &PgPool,
    corpus_id: Uuid,
    scope: ScopeKind,
) -> anyhow::Result<BTreeMap<Uuid, Vec<DetectorSetting>>> {
    let rows = sqlx::query(
        "SELECT scope_id, detector, enabled, target_languages FROM detector_settings
         WHERE corpus_id = $1 AND scope_kind = $2 ORDER BY detector",
    )
    .bind(corpus_id)
    .bind(scope.as_str())
    .fetch_all(pool)
    .await?;
    let mut settings: BTreeMap<Uuid, Vec<DetectorSetting>> = BTreeMap::new();
    for row in rows {
        settings
            .entry(row.try_get("scope_id")?)
            .or_default()
            .push(DetectorSetting {
                detector: row.try_get("detector")?,
                enabled: row.try_get("enabled")?,
                target_languages: row.try_get("target_languages")?,
            });
    }
    Ok(settings)
}

pub async fn corpus_profiles(
    pool: &PgPool,
    corpus_id: Uuid,
) -> Result<CorpusDetectorProfiles, DetectorError> {
    require_scope(pool, corpus_id, ScopeKind::Corpus, corpus_id).await?;
    let mut corpus = scope_settings(pool, corpus_id, ScopeKind::Corpus).await?;
    let mut sources = scope_settings(pool, corpus_id, ScopeKind::Source).await?;
    let source_rows = sqlx::query(
        "SELECT s.id, s.name, s.source_url FROM corpus_source_memberships m
         JOIN corpus_subscriptions s ON s.id = m.subscription_id
         WHERE m.corpus_id = $1 ORDER BY s.name NULLS LAST, s.source_url",
    )
    .bind(corpus_id)
    .fetch_all(pool)
    .await?;
    let mut profiles = Vec::new();
    for row in source_rows {
        let source_id: Uuid = row.try_get("id")?;
        profiles.push(SourceProfile {
            source_id,
            name: row.try_get("name")?,
            source_url: row.try_get("source_url")?,
            settings: sources.remove(&source_id).unwrap_or_default(),
        });
    }
    Ok(CorpusDetectorProfiles {
        detectors: DETECTORS,
        corpus: corpus.remove(&corpus_id).unwrap_or_default(),
        sources: profiles,
    })
}

/// A profile that requested a detector for a video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanContributor {
    /// `corpus` or `source`.
    pub kind: String,
    pub id: Uuid,
    pub name: Option<String>,
    /// Whether the request comes from an explicit setting of this scope rather than inherited
    /// from the corpus default.
    pub explicit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedDetector {
    pub detector: String,
    pub label: String,
    pub available: bool,
    pub note: Option<String>,
    /// Result of inheritance (union over contributors), before any video override.
    pub inherited: bool,
    /// Explicit video override within this corpus, if any.
    pub video_override: Option<bool>,
    pub enabled: bool,
    pub contributors: Vec<PlanContributor>,
    pub target_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetectorPlan {
    pub corpus_id: Uuid,
    pub video_id: Uuid,
    pub added_directly: bool,
    pub sources: Vec<PlanContributor>,
    pub detectors: Vec<PlannedDetector>,
    pub video_overrides: Vec<DetectorSetting>,
}

/// Resolves the effective detector plan of one video inside one corpus.
pub async fn video_plan(
    pool: &PgPool,
    corpus_id: Uuid,
    video_id: Uuid,
) -> Result<VideoDetectorPlan, DetectorError> {
    require_scope(pool, corpus_id, ScopeKind::Video, video_id).await?;
    let added_directly = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM corpus_video_memberships WHERE corpus_id = $1 AND video_id = $2)",
    )
    .bind(corpus_id)
    .bind(video_id)
    .fetch_one(pool)
    .await?;
    let source_rows = sqlx::query(
        "SELECT DISTINCT s.id, s.name
         FROM corpus_source_memberships sm
         JOIN corpus_subscriptions s ON s.id = sm.subscription_id
         JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
         JOIN videos v ON v.id = $2
           AND (v.source_url = i.source_url OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id))
         WHERE sm.corpus_id = $1
         ORDER BY s.name NULLS LAST, s.id",
    )
    .bind(corpus_id)
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let sources = source_rows
        .into_iter()
        .map(|row| {
            Ok(PlanContributor {
                kind: "source".to_string(),
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                explicit: false,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let corpus_settings = scope_settings(pool, corpus_id, ScopeKind::Corpus)
        .await?
        .remove(&corpus_id)
        .unwrap_or_default();
    let source_settings = scope_settings(pool, corpus_id, ScopeKind::Source).await?;
    let video_overrides = scope_settings(pool, corpus_id, ScopeKind::Video)
        .await?
        .remove(&video_id)
        .unwrap_or_default();

    let corpus_name = sqlx::query_scalar::<_, String>("SELECT name FROM corpora WHERE id = $1")
        .bind(corpus_id)
        .fetch_one(pool)
        .await?;
    let mut contributors: Vec<(PlanContributor, &[DetectorSetting])> = Vec::new();
    if added_directly || sources.is_empty() {
        contributors.push((
            PlanContributor {
                kind: "corpus".to_string(),
                id: corpus_id,
                name: Some(corpus_name),
                explicit: true,
            },
            &[],
        ));
    }
    for source in &sources {
        let settings = source_settings
            .get(&source.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        contributors.push((source.clone(), settings));
    }

    let find = |settings: &[DetectorSetting], id: &str| -> Option<DetectorSetting> {
        settings.iter().find(|s| s.detector == id).cloned()
    };
    let detectors = DETECTORS
        .iter()
        .map(|info| {
            let corpus_default = find(&corpus_settings, info.id);
            let mut requested_by = Vec::new();
            let mut languages = BTreeSet::new();
            for (contributor, own) in &contributors {
                let (setting, explicit) = match find(own, info.id) {
                    Some(setting) => (Some(setting), true),
                    None => (corpus_default.clone(), contributor.kind == "corpus"),
                };
                if let Some(setting) = setting.filter(|setting| setting.enabled) {
                    languages.extend(setting.target_languages);
                    requested_by.push(PlanContributor {
                        explicit,
                        ..contributor.clone()
                    });
                }
            }
            let inherited = !requested_by.is_empty();
            let override_setting = find(&video_overrides, info.id);
            let enabled = override_setting
                .as_ref()
                .map_or(inherited, |setting| setting.enabled);
            let target_languages = match &override_setting {
                Some(setting) if setting.enabled && !setting.target_languages.is_empty() => {
                    setting.target_languages.clone()
                }
                _ if enabled => languages.into_iter().collect(),
                _ => Vec::new(),
            };
            PlannedDetector {
                detector: info.id.to_string(),
                label: info.label.to_string(),
                available: info.available,
                note: info.note.map(str::to_string),
                inherited,
                video_override: override_setting.map(|setting| setting.enabled),
                enabled,
                contributors: requested_by,
                target_languages,
            }
        })
        .collect();

    Ok(VideoDetectorPlan {
        corpus_id,
        video_id,
        added_directly,
        sources,
        detectors,
        video_overrides,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageOutcome {
    /// Matching output already existed, so nothing ran.
    Reused,
    Completed,
    Failed,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageReport {
    pub detector: String,
    pub outcome: StageOutcome,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRunReport {
    pub video_id: Uuid,
    pub stages: Vec<StageReport>,
}

#[derive(Debug, Clone)]
pub struct PlanRunRequest {
    pub database_url: String,
    pub corpus_id: Uuid,
    pub video_id: Uuid,
    pub work_dir: PathBuf,
    pub yt_dlp: YtDlpConfig,
    pub model_dir: PathBuf,
    /// Re-run stages even when matching output exists.
    pub refresh: bool,
    /// Restrict the run to these detectors (targeted retry); `None` runs the whole plan.
    pub only: Option<Vec<String>>,
}

/// Runs every enabled detector of a video's effective plan, reusing existing output of the
/// same producer unless a refresh is requested. Unavailable detectors are reported, never
/// faked; a failing stage does not stop the others.
pub async fn run_video_plan(request: PlanRunRequest) -> Result<PlanRunReport, DetectorError> {
    let pool = crate::db::connect(&request.database_url).await?;
    let plan = video_plan(&pool, request.corpus_id, request.video_id).await?;
    let mut stages = Vec::new();
    let selected = |detector: &str| {
        request
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|id| id == detector))
    };
    for planned in plan
        .detectors
        .iter()
        .filter(|planned| planned.enabled && selected(&planned.detector))
    {
        let detector = planned.detector.as_str();
        if !planned.available {
            stages.push(StageReport {
                detector: detector.to_string(),
                outcome: StageOutcome::Unavailable,
                message: planned.note.clone(),
            });
            continue;
        }
        let report = match existing_output(&pool, request.video_id, detector).await {
            Ok(true) if !request.refresh => StageReport {
                detector: detector.to_string(),
                outcome: StageOutcome::Reused,
                message: None,
            },
            Ok(_) => {
                record_attempt(&pool, request.video_id, detector, "running", None).await?;
                let report = match run_stage(&pool, &request, detector).await {
                    Ok(message) => StageReport {
                        detector: detector.to_string(),
                        outcome: StageOutcome::Completed,
                        message,
                    },
                    Err(error) => StageReport {
                        detector: detector.to_string(),
                        outcome: StageOutcome::Failed,
                        message: Some(format!("{error:#}")),
                    },
                };
                let state = if report.outcome == StageOutcome::Completed {
                    "completed"
                } else {
                    "failed"
                };
                record_attempt(
                    &pool,
                    request.video_id,
                    detector,
                    state,
                    report.message.as_deref(),
                )
                .await?;
                report
            }
            Err(error) => StageReport {
                detector: detector.to_string(),
                outcome: StageOutcome::Failed,
                message: Some(format!("{error:#}")),
            },
        };
        stages.push(report);
    }
    Ok(PlanRunReport {
        video_id: request.video_id,
        stages,
    })
}

async fn record_attempt(
    pool: &PgPool,
    video_id: Uuid,
    detector: &str,
    state: &str,
    message: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO detector_stage_attempts (video_id, detector, state, message, started_at, finished_at)
         VALUES ($1, $2, $3, $4, now(), CASE WHEN $3 = 'running' THEN NULL ELSE now() END)
         ON CONFLICT (video_id, detector) DO UPDATE SET
           state = EXCLUDED.state,
           message = EXCLUDED.message,
           started_at = CASE WHEN EXCLUDED.state = 'running' THEN now()
                             ELSE detector_stage_attempts.started_at END,
           finished_at = EXCLUDED.finished_at",
    )
    .bind(video_id)
    .bind(detector)
    .bind(state)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Truthful state of one detector stage for a video (Atlas 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    /// Not part of the effective plan in this corpus.
    Disabled,
    /// Every applicable source holds the video for review and nothing ran yet.
    AwaitingReview,
    /// Requested, but no producer is integrated.
    Unavailable,
    /// Requested and never attempted.
    NotRun,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageStatus {
    pub detector: String,
    pub label: String,
    pub enabled: bool,
    pub available: bool,
    pub state: StageState,
    /// Stored output of this kind, whether or not the stage is currently requested.
    pub output_count: i64,
    /// What the count measures (segments, scenes, tracks, mentions).
    pub output_unit: String,
    pub message: Option<String>,
    pub last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStageStatus {
    pub corpus_id: Uuid,
    pub video_id: Uuid,
    pub awaiting_review: bool,
    pub stages: Vec<StageStatus>,
}

/// Per-video stage inventory against the effective detector plan of one corpus.
pub async fn video_stage_status(
    pool: &PgPool,
    corpus_id: Uuid,
    video_id: Uuid,
) -> Result<VideoStageStatus, DetectorError> {
    let plan = video_plan(pool, corpus_id, video_id).await?;
    // Held for review only if not added directly and every applicable source item waits.
    let awaiting_review = !plan.added_directly
        && sqlx::query_scalar::<_, Option<bool>>(
            "SELECT bool_and(i.status = 'awaiting_review')
             FROM corpus_source_memberships sm
             JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
             JOIN videos v ON v.id = $2
               AND (v.source_url = i.source_url OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id))
             WHERE sm.corpus_id = $1",
        )
        .bind(corpus_id)
        .bind(video_id)
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
    let attempts: BTreeMap<String, (String, Option<String>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query(
            "SELECT detector, state, message, started_at FROM detector_stage_attempts
             WHERE video_id = $1",
        )
        .bind(video_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("detector")?,
                (
                    row.try_get("state")?,
                    row.try_get("message")?,
                    row.try_get("started_at")?,
                ),
            ))
        })
        .collect::<anyhow::Result<_>>()?;

    let mut stages = Vec::new();
    for planned in &plan.detectors {
        let (output_count, output_unit) = output_count(pool, video_id, &planned.detector).await?;
        let mut message = None;
        let attempt = attempts.get(&planned.detector);
        let state = if !planned.enabled {
            StageState::Disabled
        } else if attempt.is_some_and(|(state, _, _)| state == "running") {
            StageState::Running
        } else if output_count > 0 {
            StageState::Complete
        } else if !planned.available {
            message = planned.note.clone();
            StageState::Unavailable
        } else if let Some((state, attempt_message, _)) =
            attempt.filter(|(state, _, _)| state == "failed")
        {
            debug_assert_eq!(state, "failed");
            message = attempt_message.clone();
            StageState::Failed
        } else if let Some(stream_message) =
            failed_transcript_message(pool, video_id, &planned.detector).await?
        {
            message = Some(stream_message);
            StageState::Failed
        } else if awaiting_review {
            StageState::AwaitingReview
        } else {
            StageState::NotRun
        };
        stages.push(StageStatus {
            detector: planned.detector.clone(),
            label: planned.label.clone(),
            enabled: planned.enabled,
            available: planned.available,
            state,
            output_count,
            output_unit: output_unit.to_string(),
            message,
            last_attempt_at: attempt.map(|(_, _, at)| *at),
        });
    }
    Ok(VideoStageStatus {
        corpus_id,
        video_id,
        awaiting_review,
        stages,
    })
}

async fn output_count(
    pool: &PgPool,
    video_id: Uuid,
    detector: &str,
) -> anyhow::Result<(i64, &'static str)> {
    let (sql, unit) = match detector {
        "captions" => (
            "SELECT count(*) FROM transcript_segments t JOIN transcript_streams s ON s.id = t.stream_id
             WHERE s.video_id = $1 AND s.source_kind IN ('caption_manual', 'caption_auto')",
            "segments",
        ),
        "asr" => (
            "SELECT count(*) FROM transcript_segments t JOIN transcript_streams s ON s.id = t.stream_id
             WHERE s.video_id = $1 AND s.source_kind = 'asr'",
            "segments",
        ),
        "scenes" => (
            "SELECT count(*) FROM video_scenes WHERE video_id = $1",
            "scenes",
        ),
        "ocr" => (
            "SELECT count(*) FROM visual_text_tracks WHERE video_id = $1",
            "text tracks",
        ),
        "entities" => (
            "SELECT count(*) FROM entity_mentions m JOIN entity_extraction_runs r ON r.id = m.run_id
             WHERE r.video_id = $1",
            "mentions",
        ),
        _ => return Ok((0, "outputs")),
    };
    let count = sqlx::query_scalar::<_, i64>(sql)
        .bind(video_id)
        .fetch_one(pool)
        .await?;
    // An extraction run without mentions is still a completed stage.
    if detector == "entities" && count == 0 {
        let ran = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM entity_extraction_runs WHERE video_id = $1)",
        )
        .bind(video_id)
        .fetch_one(pool)
        .await?;
        if ran {
            return Ok((0, unit));
        }
    }
    Ok((count, unit))
}

/// The message of a transcript stream that was attempted but stored no segments.
async fn failed_transcript_message(
    pool: &PgPool,
    video_id: Uuid,
    detector: &str,
) -> anyhow::Result<Option<String>> {
    let kinds: &[&str] = match detector {
        "captions" => &["caption_manual", "caption_auto"],
        "asr" => &["asr"],
        _ => return Ok(None),
    };
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT s.message FROM transcript_streams s
         WHERE s.video_id = $1 AND s.source_kind = ANY($2) AND s.message IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM transcript_segments t WHERE t.stream_id = s.id)
         ORDER BY s.created_at DESC LIMIT 1",
    )
    .bind(video_id)
    .bind(kinds)
    .fetch_optional(pool)
    .await?
    .flatten())
}

async fn existing_output(pool: &PgPool, video_id: Uuid, detector: &str) -> anyhow::Result<bool> {
    let sql = match detector {
        "captions" => {
            "SELECT EXISTS(SELECT 1 FROM transcript_streams s
               WHERE s.video_id = $1 AND s.source_kind IN ('caption_manual', 'caption_auto')
                 AND EXISTS (SELECT 1 FROM transcript_segments t WHERE t.stream_id = s.id))"
        }
        "asr" => {
            "SELECT EXISTS(SELECT 1 FROM transcript_streams s
               WHERE s.video_id = $1 AND s.source_kind = 'asr'
                 AND EXISTS (SELECT 1 FROM transcript_segments t WHERE t.stream_id = s.id))"
        }
        "scenes" => {
            "SELECT EXISTS(SELECT 1 FROM media_processing_runs r
               WHERE r.video_id = $1 AND r.modality = 'scene'
                 AND EXISTS (SELECT 1 FROM video_scenes s WHERE s.run_id = r.id))"
        }
        "entities" => {
            "SELECT EXISTS(SELECT 1 FROM entity_extraction_runs r
               WHERE r.video_id = $1 AND r.producer_version = $2)"
        }
        _ => anyhow::bail!("detector {detector} has no runner"),
    };
    Ok(sqlx::query_scalar::<_, bool>(sql)
        .bind(video_id)
        .bind(crate::entities::ENTITY_PRODUCER_VERSION)
        .fetch_one(pool)
        .await?)
}

async fn run_stage(
    pool: &PgPool,
    request: &PlanRunRequest,
    detector: &str,
) -> anyhow::Result<Option<String>> {
    use crate::reprocessing::{reprocess_video, ReprocessRequest, ReprocessStage};
    let reprocess = |stage| ReprocessRequest {
        database_url: request.database_url.clone(),
        video_id: request.video_id,
        stage,
        work_dir: request.work_dir.clone(),
        caption_languages: vec!["en".to_string()],
        yt_dlp: request.yt_dlp.clone(),
        transcriber_command: None,
        transcriber_args: Vec::new(),
        transcriber_timeout_seconds: None,
    };
    match detector {
        "captions" | "asr" => {
            let stage = if detector == "captions" {
                ReprocessStage::Captions
            } else {
                ReprocessStage::Asr
            };
            reprocess_video(reprocess(stage)).await?;
            if !existing_output(pool, request.video_id, detector).await? {
                anyhow::bail!("the producer finished without storing a {detector} transcript");
            }
            Ok(None)
        }
        "scenes" => {
            let report =
                crate::visual_processing::analyze_stored_video_scenes(pool, request.video_id)
                    .await?;
            Ok(Some(format!("{} scenes", report.scenes.len())))
        }
        "entities" => {
            let report = crate::entities::extract_video_entities(
                pool,
                request.video_id,
                request.model_dir.clone(),
                true,
            )
            .await?;
            Ok(Some(format!("{} mentions", report.mentions)))
        }
        _ => anyhow::bail!("detector {detector} has no runner"),
    }
}
