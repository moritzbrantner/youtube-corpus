//! Persistent per-detector observations. Attempts never schedule work themselves.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::detectors::{DetectorError, VideoDetectorPlan};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    Disabled,
    AwaitingReview,
    Missing,
    MissingSource,
    Running,
    Completed,
    Skipped,
    Failed,
    Unavailable,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorStatus {
    pub detector: String,
    pub label: String,
    pub target_language: Option<String>,
    pub source_languages: Vec<String>,
    pub state: StageState,
    /// Retained evidence is reported even when disabled or the latest attempt failed.
    pub output_count: i64,
    pub message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProcessingStatus {
    pub video_id: Uuid,
    pub stages: Vec<DetectorStatus>,
}

/// The transaction owns an advisory lock for exactly the lifetime of real work. Dropping it
/// (including cancellation) rolls back the lock. The attempt row commits independently.
pub(crate) struct Attempt {
    id: Uuid,
    lock: Transaction<'static, Postgres>,
}

impl Attempt {
    pub(crate) async fn begin(
        pool: &PgPool,
        video: Uuid,
        detector: &str,
        identity: &Value,
    ) -> anyhow::Result<Self> {
        let mut lock = pool.begin().await?;
        let acquired = sqlx::query_scalar::<_, bool>(
            "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))",
        )
        .bind(format!("detector:{video}:{detector}"))
        .fetch_one(&mut *lock)
        .await?;
        anyhow::ensure!(acquired, "{detector} is already running for this video");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *lock)
            .await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO detector_attempts (id, video_id, detector, identity, state, backend_pid)
             VALUES ($1, $2, $3, $4, 'running', $5)",
        )
        .bind(id)
        .bind(video)
        .bind(detector)
        .bind(identity)
        .bind(pid)
        .execute(pool)
        .await?;
        Ok(Self { id, lock })
    }

    pub(crate) async fn finish(
        self,
        pool: &PgPool,
        state: &str,
        message: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE detector_attempts SET state = $2, message = $3, finished_at = now() WHERE id = $1",
        )
        .bind(self.id)
        .bind(state)
        .bind(message)
        .execute(pool)
        .await?;
        self.lock.rollback().await?;
        Ok(())
    }
}

pub(crate) async fn output_count(
    pool: &PgPool,
    video: Uuid,
    detector: &str,
) -> anyhow::Result<i64> {
    let sql = match detector {
        "captions" => "SELECT count(*) FROM transcript_segments t JOIN transcript_streams s ON s.id = t.stream_id WHERE s.video_id = $1 AND s.source_kind IN ('caption_manual', 'caption_auto')",
        "asr" => "SELECT count(*) FROM transcript_segments t JOIN transcript_streams s ON s.id = t.stream_id WHERE s.video_id = $1 AND s.source_kind = 'asr'",
        "scenes" => "SELECT count(*) FROM video_scenes WHERE video_id = $1",
        "ocr" => "SELECT count(*) FROM visual_text_observations WHERE video_id = $1",
        "entities" => "SELECT count(*) FROM entity_mentions WHERE video_id = $1",
        "faces" => "SELECT count(*) FROM face_observations WHERE video_id = $1",
        "voices" => "SELECT count(*) FROM voice_observations WHERE video_id = $1",
        _ => {
            return match crate::translation::TranslationModality::from_detector(detector) {
                Some(modality) => {
                    crate::translation::current_count(pool, video, modality, None).await
                }
                None => Ok(0),
            }
        }
    };
    Ok(sqlx::query_scalar(sql).bind(video).fetch_one(pool).await?)
}

pub async fn video_status(
    pool: &PgPool,
    corpus: Uuid,
    video: Uuid,
) -> Result<VideoProcessingStatus, DetectorError> {
    let plan = crate::detectors::video_plan(pool, corpus, video).await?;
    status_for_plan(pool, &plan).await.map_err(Into::into)
}

async fn status_for_plan(
    pool: &PgPool,
    plan: &VideoDetectorPlan,
) -> anyhow::Result<VideoProcessingStatus> {
    // Only hold for review when no source has admitted or already processed this shared video.
    let awaiting_review: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM corpus_subscription_items i JOIN videos v
           ON i.source_url = v.source_url OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
           WHERE v.id = $1 AND i.status = 'awaiting_review')
         AND NOT EXISTS(SELECT 1 FROM corpus_subscription_items i JOIN videos v
           ON i.source_url = v.source_url OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id)
           JOIN corpus_subscriptions s ON s.id = i.subscription_id
           WHERE v.id = $1 AND ((s.enabled AND s.processing_policy = 'automatic') OR i.status IN ('indexed', 'no_transcript', 'skipped')))",
    ).bind(plan.video_id).fetch_one(pool).await?;
    let attempts = sqlx::query(
        "SELECT DISTINCT ON (detector) detector, state, message, started_at, finished_at,
          EXISTS(SELECT 1 FROM pg_locks l WHERE l.pid = a.backend_pid AND l.locktype = 'advisory'
            AND l.granted AND l.classid::bigint = ((hashtextextended('detector:' || a.video_id::text || ':' || a.detector, 0) >> 32) & 4294967295)
            AND l.objid::bigint = (hashtextextended('detector:' || a.video_id::text || ':' || a.detector, 0) & 4294967295)) AS alive
         FROM detector_attempts a WHERE video_id = $1 ORDER BY detector, started_at DESC, id DESC",
    ).bind(plan.video_id).fetch_all(pool).await?;
    let mut stages = Vec::new();
    for planned in &plan.detectors {
        let count = output_count(pool, plan.video_id, &planned.detector).await?;
        let attempt = attempts
            .iter()
            .find(|r| r.get::<String, _>("detector") == planned.detector);
        let mut state = if !planned.enabled {
            StageState::Disabled
        } else if awaiting_review && count == 0 {
            StageState::AwaitingReview
        } else if !planned.available {
            StageState::Unavailable
        } else if count > 0
            || (matches!(planned.detector.as_str(), "faces" | "voices")
                && sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM media_processing_runs WHERE video_id = $1 AND modality = $2 AND processing_config->>'complete' = 'true')")
                    .bind(plan.video_id).bind(if planned.detector == "faces" { "face" } else { "voice" }).fetch_one(pool).await?)
        {
            StageState::Completed
        } else {
            StageState::Missing
        };
        let mut message = if planned.enabled {
            planned.note.clone()
        } else {
            None
        };
        if planned.enabled {
            if let Some(row) = attempt {
                let recorded: String = row.try_get("state")?;
                state = match recorded.as_str() {
                    "running" if row.try_get("alive")? => StageState::Running,
                    "running" => StageState::Interrupted,
                    "failed" => StageState::Failed,
                    "skipped" => StageState::Skipped,
                    "unavailable" => StageState::Unavailable,
                    "completed" => StageState::Completed,
                    other => anyhow::bail!("unknown recorded detector state {other}"),
                };
                message = row.try_get("message")?;
            }
        }
        let source_languages: Vec<String> = if matches!(
            planned.detector.as_str(),
            "ocr" | "translate_ocr"
        ) {
            sqlx::query_scalar("SELECT DISTINCT language FROM visual_text_tracks WHERE video_id = $1 AND language IS NOT NULL ORDER BY language")
                .bind(plan.video_id).fetch_all(pool).await?
        } else {
            sqlx::query_scalar("SELECT DISTINCT language FROM transcript_streams WHERE video_id = $1 AND language IS NOT NULL
                AND ($2 = 'translate_spoken' OR ($2 = 'asr' AND source_kind = 'asr')
                     OR ($2 = 'captions' AND source_kind IN ('caption_manual','caption_auto'))) ORDER BY language")
                .bind(plan.video_id).bind(&planned.detector).fetch_all(pool).await?
        };
        let languages = if planned.target_languages.is_empty() {
            vec![None]
        } else {
            planned.target_languages.iter().cloned().map(Some).collect()
        };
        for language in languages {
            let mut entry = DetectorStatus {
                detector: planned.detector.clone(),
                label: planned.label.clone(),
                target_language: language,
                source_languages: source_languages.clone(),
                state: state.clone(),
                output_count: count,
                message: message.clone(),
                started_at: attempt.map(|r| r.try_get("started_at")).transpose()?,
                finished_at: attempt
                    .map(|r| r.try_get("finished_at"))
                    .transpose()?
                    .flatten(),
            };
            if let Some(modality) =
                crate::translation::TranslationModality::from_detector(&planned.detector)
            {
                // Each target language reports its own retained output and state, so one
                // translated target or modality never makes another look translated.
                if let Some(target) = &entry.target_language {
                    entry.output_count = crate::translation::current_count(
                        pool,
                        plan.video_id,
                        modality,
                        Some(target),
                    )
                    .await?;
                }
                let settled = !planned.enabled
                    || matches!(
                        entry.state,
                        StageState::AwaitingReview | StageState::Running
                    );
                if settled {
                    // Disabled, held for review or running: the general state stands.
                } else if let Some(target) = &entry.target_language {
                    let status =
                        crate::translation::target_status(pool, plan.video_id, modality, target)
                            .await?;
                    entry.state = match status.state {
                        "completed" => StageState::Completed,
                        "failed" => StageState::Failed,
                        "unavailable" => StageState::Unavailable,
                        "skipped" => StageState::Skipped,
                        "missing_source" => StageState::MissingSource,
                        _ => StageState::Missing,
                    };
                    entry.message = status.message;
                } else {
                    entry.state = StageState::Unavailable;
                    entry.message =
                        Some("Select at least one target language for this translation.".into());
                }
            }
            stages.push(entry);
        }
    }
    Ok(VideoProcessingStatus {
        video_id: plan.video_id,
        stages,
    })
}

pub(crate) fn ingest_identity(request: &crate::ingest::IngestRequest, detector: &str) -> Value {
    // Changes in the other modality do not invalidate this stage.
    let config = match detector {
        "captions" => json!({ "caption": request.caption, "ytDlp": request.yt_dlp }),
        "asr" => json!({ "command": request.transcriber_command, "args": request.transcriber_args,
            "timeoutSeconds": request.transcriber_timeout_seconds }),
        _ => Value::Null,
    };
    json!({ "processorRevision": crate::provenance::PROCESSOR_SOURCE_REVISION,
        "source": request.source.source_url(), "config": config })
}

/// Hash actual bytes at the producer boundary; timestamps alone do not prove unchanged media.
pub(crate) fn file_identity(path: &std::path::Path) -> anyhow::Result<Value> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(
        json!({ "path": path, "sha256": hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>() }),
    )
}
