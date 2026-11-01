//! Opt-in translated text for corpus videos (#69; Atlas 3.9, 3.11, 3.13).
//!
//! The corpus orchestrates translation of a video's original spoken transcript segments and
//! on-screen OCR tracks, stores each translation as a derived record beside its source span and
//! indexes it for search. Planning and model execution belong to native-whisperx: its curated
//! direct/pivot planner and the `translate_timed_text` executor run on neutral timed text, with
//! the native Marian/OPUS-MT provider in production. This module never translates by itself.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use native_whisperx::{
    CuratedLanguage, SegmentTranslationProvider, TranscriptSegmentContract, TranscriptionContract,
    TranslationPlan,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;

/// Exact native-whisperx revision pinned in `Cargo.toml`.
pub const NATIVE_WHISPERX_REVISION: &str = "8c698ffd23d7f86ca63c6b1ef4b6b13b6b9c2e50";
/// Version of the corpus' translated-record shape; part of every configuration identity.
pub const TRANSLATION_RECORD_VERSION: &str = "1";

/// Which original evidence a translation is derived from (Atlas 3.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationModality {
    Spoken,
    VisibleText,
}

impl TranslationModality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spoken => "spoken",
            Self::VisibleText => "visible_text",
        }
    }

    pub fn detector(self) -> &'static str {
        match self {
            Self::Spoken => "translate_spoken",
            Self::VisibleText => "translate_ocr",
        }
    }

    pub fn from_detector(detector: &str) -> Option<Self> {
        match detector {
            "translate_spoken" => Some(Self::Spoken),
            "translate_ocr" => Some(Self::VisibleText),
            _ => None,
        }
    }
}

/// Languages the native provider can translate between; other pairs pivot through English.
pub fn supported_languages() -> Vec<&'static str> {
    CuratedLanguage::ALL.iter().map(|l| l.code()).collect()
}

/// Maps a stored language tag such as `en`, `en-US` or `en-orig` to a curated language.
pub fn curated_language(tag: &str) -> Option<CuratedLanguage> {
    let primary = tag
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    primary.parse().ok()
}

pub fn text_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One original-language span that may be translated.
#[derive(Debug, Clone)]
struct SourceUnit {
    id: Uuid,
    text: String,
    language: Option<String>,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
}

/// The modality's actually available original evidence: the video's preferred transcript stream
/// for speech (the same stream entity extraction reads), every OCR track for visible text.
async fn source_units(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
) -> anyhow::Result<Vec<SourceUnit>> {
    let rows = match modality {
        TranslationModality::Spoken => {
            let Some(stream) = crate::entities::transcript_stream_for(pool, video).await? else {
                return Ok(Vec::new());
            };
            sqlx::query(
                "SELECT t.id, t.text, coalesce(t.language, s.language) AS language,
                        t.start_seconds, t.end_seconds
                 FROM transcript_segments t JOIN transcript_streams s ON s.id = t.stream_id
                 WHERE t.stream_id = $1 ORDER BY t.segment_index",
            )
            .bind(stream)
            .fetch_all(pool)
            .await?
        }
        TranslationModality::VisibleText => {
            sqlx::query(
                "SELECT id, text, language, start_seconds, end_seconds FROM visual_text_tracks
                 WHERE video_id = $1 ORDER BY start_seconds NULLS LAST, track_key, id",
            )
            .bind(video)
            .fetch_all(pool)
            .await?
        }
    };
    rows.into_iter()
        .map(|row| {
            Ok(SourceUnit {
                id: row.try_get("id")?,
                text: row.try_get("text")?,
                language: row.try_get("language")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
            })
        })
        .filter(|unit: &anyhow::Result<SourceUnit>| {
            !matches!(unit, Ok(unit) if unit.text.trim().is_empty())
        })
        .collect()
}

/// Identity of a modality's source evidence for detector attempts.
pub(crate) async fn source_identity(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
) -> anyhow::Result<String> {
    let units = source_units(pool, video, modality).await?;
    let mut digest = Sha256::new();
    for unit in &units {
        digest.update(unit.id.as_bytes());
        digest.update(text_hash(&unit.text).as_bytes());
        digest.update(unit.language.as_deref().unwrap_or("").as_bytes());
        digest.update(format!("{:?}{:?}", unit.start_seconds, unit.end_seconds).as_bytes());
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// A translation provider plus the configuration recorded with its output.
pub struct ProviderHandle {
    pub provider: Box<dyn SegmentTranslationProvider + Send>,
    /// Provider name stored with every translation.
    pub name: String,
    /// Settings that change the produced text; part of each output's configuration identity.
    pub config: Value,
}

/// Creates the translation provider for one stage run. A plan run uses [`native_provider`]
/// unless a caller supplies another implementation of the native provider boundary.
#[derive(Clone)]
pub struct TranslationProviderFactory(pub std::sync::Arc<dyn Fn() -> ProviderHandle + Send + Sync>);

impl std::fmt::Debug for TranslationProviderFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TranslationProviderFactory")
    }
}

/// The production provider: native-whisperx Marian/OPUS-MT on Candle. Planned models resolve
/// from `model_dir` and download from Hugging Face on first use.
pub fn native_provider(model_dir: PathBuf) -> ProviderHandle {
    let config = native_whisperx::NativeOpusMtTranslationProviderConfig {
        model_dir: Some(model_dir),
        ..Default::default()
    };
    let recorded = json!({ "maxNewTokens": config.max_new_tokens });
    ProviderHandle {
        provider: Box::new(native_whisperx::NativeOpusMtTranslationProvider::new(
            config,
        )),
        name: "native-whisperx marian-candle".to_string(),
        config: recorded,
    }
}

fn configuration(provider: &ProviderHandle, plan: &TranslationPlan) -> (Value, Value, String) {
    let plan_json = serde_json::to_value(plan).unwrap_or(Value::Null);
    let config = json!({
        "provider": provider.name,
        "producerRevision": NATIVE_WHISPERX_REVISION,
        "recordVersion": TRANSLATION_RECORD_VERSION,
        "settings": provider.config,
    });
    let hash = text_hash(&json!({ "plan": plan_json, "config": config }).to_string());
    (plan_json, config, hash)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetTranslationReport {
    pub target_language: String,
    /// `completed`, `reused`, `failed`, `unavailable` or `skipped`.
    pub state: String,
    pub message: Option<String>,
    pub translated: i32,
    pub reused: i32,
    pub untranslatable: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModalityTranslationReport {
    pub video_id: Uuid,
    pub modality: TranslationModality,
    pub targets: Vec<TargetTranslationReport>,
}

/// Translates a video's selected modality into each requested target language, generating only
/// missing or outdated translations. Unsupported targets and source languages are reported as
/// unavailable; a failing target does not stop the others and never stores partial output.
pub async fn translate_video_modality(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
    targets: &[String],
    mut provider: ProviderHandle,
    refresh: bool,
) -> anyhow::Result<ModalityTranslationReport> {
    let units = source_units(pool, video, modality).await?;
    // Remove translations whose source span no longer exists.
    sqlx::query(
        "DELETE FROM translated_segments tr WHERE tr.video_id = $1 AND tr.modality = $2
           AND NOT EXISTS (SELECT 1 FROM transcript_segments s WHERE s.id = tr.transcript_segment_id)
           AND NOT EXISTS (SELECT 1 FROM visual_text_tracks t WHERE t.id = tr.visual_text_track_id)",
    )
    .bind(video)
    .bind(modality.as_str())
    .execute(pool)
    .await?;
    let mut existing: HashMap<(Uuid, String), (String, String)> = HashMap::new();
    for row in sqlx::query(
        "SELECT coalesce(transcript_segment_id, visual_text_track_id) AS source_id, target_language,
                source_text_hash, config_hash
         FROM translated_segments WHERE video_id = $1 AND modality = $2",
    )
    .bind(video)
    .bind(modality.as_str())
    .fetch_all(pool)
    .await?
    {
        existing.insert(
            (row.try_get("source_id")?, row.try_get("target_language")?),
            (row.try_get("source_text_hash")?, row.try_get("config_hash")?),
        );
    }
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;

    let mut by_language: BTreeMap<Option<&'static str>, Vec<&SourceUnit>> = BTreeMap::new();
    let mut unknown_languages = BTreeSet::new();
    for unit in &units {
        let language = unit.language.as_deref().and_then(curated_language);
        if language.is_none() {
            unknown_languages.insert(unit.language.clone().unwrap_or_else(|| "unknown".into()));
        }
        by_language
            .entry(language.map(|l| l.code()))
            .or_default()
            .push(unit);
    }

    let mut reports = Vec::new();
    for requested in targets {
        let target_code = requested.trim().to_ascii_lowercase();
        let Ok(target) = target_code.parse::<CuratedLanguage>() else {
            let report = TargetTranslationReport {
                target_language: target_code.clone(),
                state: "unavailable".into(),
                message: Some(unsupported_target_message(&target_code)),
                translated: 0,
                reused: 0,
                untranslatable: units.len() as i32,
            };
            record_run(pool, video, modality, &report).await?;
            reports.push(report);
            continue;
        };
        let (mut translated, mut reused, mut untranslatable, mut same) = (0, 0, 0, 0);
        let mut failure = None;
        for (language, group) in &by_language {
            let Some(source) = language.and_then(|code| code.parse::<CuratedLanguage>().ok())
            else {
                untranslatable += group.len() as i32;
                continue;
            };
            if source == target {
                same += group.len() as i32;
                continue;
            }
            let plan = TranslationPlan::new(source, target)?;
            let (plan_json, config, config_hash) = configuration(&provider, &plan);
            let missing: Vec<&SourceUnit> = group
                .iter()
                .copied()
                .filter(|unit| {
                    refresh
                        || existing.get(&(unit.id, target.code().to_string()))
                            != Some(&(text_hash(&unit.text), config_hash.clone()))
                })
                .collect();
            reused += (group.len() - missing.len()) as i32;
            if missing.is_empty() {
                continue;
            }
            let contract = TranscriptionContract::new(
                missing
                    .iter()
                    .enumerate()
                    .map(|(index, unit)| {
                        let mut segment = TranscriptSegmentContract::new(index as u64, &unit.text);
                        segment.language = Some(source.code().to_string());
                        segment
                    })
                    .collect(),
            );
            let (returned, result) = tokio::task::spawn_blocking(move || {
                let mut provider = provider;
                let result = native_whisperx::translate_timed_text(
                    &contract,
                    &plan,
                    &mut *provider.provider,
                )
                .map_err(|error| error.to_string());
                (provider, result)
            })
            .await?;
            provider = returned;
            let output = match result {
                Ok(output) => output,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            let segments = &output.transcript().segments;
            anyhow::ensure!(
                segments.len() == missing.len(),
                "the translation provider returned {} segments for {} source spans",
                segments.len(),
                missing.len()
            );
            let mut tx = pool.begin().await?;
            for (unit, segment) in missing.iter().zip(segments) {
                let text = segment.text.trim();
                let embedding = embedder
                    .embed_text(text)
                    .ok()
                    .map(|vector| crate::ingest::vector_literal(vector.as_slice()));
                let (spoken, visible) = match modality {
                    TranslationModality::Spoken => (Some(unit.id), None),
                    TranslationModality::VisibleText => (None, Some(unit.id)),
                };
                sqlx::query(
                    "INSERT INTO translated_segments (id, video_id, modality, transcript_segment_id,
                        visual_text_track_id, source_text_hash, source_language, target_language,
                        start_seconds, end_seconds, text, provider, producer_revision, plan, config,
                        config_hash, embedding)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17::vector)
                     ON CONFLICT ((coalesce(transcript_segment_id, visual_text_track_id)), target_language)
                     DO UPDATE SET id = EXCLUDED.id, source_text_hash = EXCLUDED.source_text_hash,
                        source_language = EXCLUDED.source_language, start_seconds = EXCLUDED.start_seconds,
                        end_seconds = EXCLUDED.end_seconds, text = EXCLUDED.text,
                        provider = EXCLUDED.provider, producer_revision = EXCLUDED.producer_revision,
                        plan = EXCLUDED.plan, config = EXCLUDED.config,
                        config_hash = EXCLUDED.config_hash, embedding = EXCLUDED.embedding,
                        created_at = now()",
                )
                .bind(Uuid::new_v4())
                .bind(video)
                .bind(modality.as_str())
                .bind(spoken)
                .bind(visible)
                .bind(text_hash(&unit.text))
                .bind(source.code())
                .bind(target.code())
                .bind(unit.start_seconds)
                .bind(unit.end_seconds)
                .bind(text)
                .bind(&provider.name)
                .bind(NATIVE_WHISPERX_REVISION)
                .bind(&plan_json)
                .bind(&config)
                .bind(&config_hash)
                .bind(embedding)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            translated += missing.len() as i32;
        }
        let mut notes = Vec::new();
        if untranslatable > 0 {
            notes.push(format!(
                "{untranslatable} spans have no supported source language ({}); supported: {}",
                unknown_languages
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                supported_languages().join(", ")
            ));
        }
        if same > 0 {
            notes.push(format!("{same} spans are already in {}", target.code()));
        }
        let (state, mut message) = if let Some(error) = failure {
            ("failed", error)
        } else if translated + reused == 0 && untranslatable > 0 {
            (
                "unavailable",
                "No source span can be translated".to_string(),
            )
        } else if units.is_empty() {
            (
                "skipped",
                "No source evidence is available to translate".to_string(),
            )
        } else if translated + reused == 0 {
            ("skipped", "Nothing to translate".to_string())
        } else if translated > 0 {
            (
                "completed",
                format!("{translated} translated, {reused} reused"),
            )
        } else {
            ("reused", format!("{reused} translations reused"))
        };
        for note in notes {
            message.push_str("; ");
            message.push_str(&note);
        }
        let report = TargetTranslationReport {
            target_language: target.code().to_string(),
            state: state.to_string(),
            message: Some(message),
            translated,
            reused,
            untranslatable,
        };
        record_run(pool, video, modality, &report).await?;
        reports.push(report);
    }
    Ok(ModalityTranslationReport {
        video_id: video,
        modality,
        targets: reports,
    })
}

fn unsupported_target_message(target: &str) -> String {
    format!(
        "Translation into {target} is not supported by the translation provider (supported: {}).",
        supported_languages().join(", ")
    )
}

async fn record_run(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
    report: &TargetTranslationReport,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO translation_runs (video_id, modality, target_language, state, message,
            translated, reused, untranslatable)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (video_id, modality, target_language) DO UPDATE SET state = EXCLUDED.state,
            message = EXCLUDED.message, translated = EXCLUDED.translated, reused = EXCLUDED.reused,
            untranslatable = EXCLUDED.untranslatable, finished_at = now()",
    )
    .bind(video)
    .bind(modality.as_str())
    .bind(&report.target_language)
    .bind(&report.state)
    .bind(&report.message)
    .bind(report.translated)
    .bind(report.reused)
    .bind(report.untranslatable)
    .execute(pool)
    .await?;
    Ok(())
}

/// Searchable (current) translations of one modality, optionally for one target language.
pub(crate) async fn current_count(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
    target: Option<&str>,
) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM translated_segments tr
         LEFT JOIN transcript_segments s ON s.id = tr.transcript_segment_id
         LEFT JOIN visual_text_tracks t ON t.id = tr.visual_text_track_id
         WHERE tr.video_id = $1 AND tr.modality = $2 AND ($3::text IS NULL OR tr.target_language = $3)
           AND encode(sha256(convert_to(coalesce(s.text, t.text), 'UTF8')), 'hex') = tr.source_text_hash",
    )
    .bind(video)
    .bind(modality.as_str())
    .bind(target)
    .fetch_one(pool)
    .await?)
}

/// Per-target stage state derived from stored evidence and the latest recorded outcome.
#[derive(Debug, Clone)]
pub(crate) struct TargetStatus {
    /// One of `completed`, `missing`, `missing_source`, `failed`, `unavailable`, `skipped`.
    pub state: &'static str,
    pub message: Option<String>,
}

pub(crate) async fn target_status(
    pool: &PgPool,
    video: Uuid,
    modality: TranslationModality,
    target: &str,
) -> anyhow::Result<TargetStatus> {
    let Ok(target_language) = target.parse::<CuratedLanguage>() else {
        return Ok(TargetStatus {
            state: "unavailable",
            message: Some(unsupported_target_message(target)),
        });
    };
    let units = source_units(pool, video, modality).await?;
    let translated: BTreeSet<(Uuid, String)> = sqlx::query(
        "SELECT coalesce(transcript_segment_id, visual_text_track_id) AS source_id, source_text_hash
         FROM translated_segments WHERE video_id = $1 AND modality = $2 AND target_language = $3",
    )
    .bind(video)
    .bind(modality.as_str())
    .bind(target)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| Ok((row.try_get("source_id")?, row.try_get("source_text_hash")?)))
    .collect::<anyhow::Result<_>>()?;
    let (mut translatable, mut covered, mut unsupported) = (0, 0, 0);
    for unit in &units {
        match unit.language.as_deref().and_then(curated_language) {
            None => unsupported += 1,
            Some(source) if source == target_language => {}
            Some(_) => {
                translatable += 1;
                if translated.contains(&(unit.id, text_hash(&unit.text))) {
                    covered += 1;
                }
            }
        }
    }
    let last_run = sqlx::query(
        "SELECT state, message FROM translation_runs
         WHERE video_id = $1 AND modality = $2 AND target_language = $3",
    )
    .bind(video)
    .bind(modality.as_str())
    .bind(target)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = &last_run {
        if row.try_get::<String, _>("state")? == "failed" {
            return Ok(TargetStatus {
                state: "failed",
                message: row.try_get("message")?,
            });
        }
    }
    let unsupported_note = (unsupported > 0).then(|| {
        format!(
            "{unsupported} spans have no supported source language (supported: {})",
            supported_languages().join(", ")
        )
    });
    let (state, message) = if units.is_empty() {
        (
            "missing_source",
            Some("No source evidence is available to translate.".to_string()),
        )
    } else if translatable == 0 && unsupported > 0 {
        ("unavailable", unsupported_note)
    } else if translatable == 0 {
        (
            "skipped",
            Some(format!("The source text is already in {target}.")),
        )
    } else if covered == translatable {
        ("completed", unsupported_note)
    } else {
        (
            "missing",
            Some(format!("{covered} of {translatable} spans translated")),
        )
    };
    Ok(TargetStatus { state, message })
}
