//! Source-grounded entity occurrences (Decision Atlas 4.1-4.3, issue #62).
//!
//! Named-entity recognition is consumed from the canonical nlp-stack producer
//! (`text-linguistics` with the local `bert-base-NER` token classifier); this module only
//! adapts its output into time-linked mentions of transcript segments and visible-text (OCR)
//! tracks, and explores them. A mention ("Spoken" or "On-screen text") is never merged with a
//! person shown on screen or heard: those come from the corpus `people` identities owned by the
//! matching (#64) and naming (#65) workflows and stay separate results with their own status.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use text_core::{split_sentence_spans, tokenize, TextProcessingOptions};
use text_linguistics::{
    extract_named_entities_with_labeler, CandleTokenClassifier, EntityType, NamedEntity,
};
use uuid::Uuid;

/// Producer identity recorded on every extraction run.
pub const ENTITY_PRODUCER: &str = "nlp-stack/text-linguistics";
/// Exact nlp-stack revision this repository pins (see `Cargo.toml`).
pub const ENTITY_PRODUCER_VERSION: &str = "50dd68521e425d2ce734d1f2df1d8c818a659f12";
/// Mentions below this producer confidence are dropped as unreliable.
pub const MIN_ENTITY_CONFIDENCE: f32 = 0.6;
/// Default directory for downloaded model bundles (generated material lives in
/// `use-case-output/`).
pub const DEFAULT_MODEL_DIR: &str = "use-case-output/models";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityModality {
    Spoken,
    VisibleText,
}

impl EntityModality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spoken => "spoken",
            Self::VisibleText => "visible_text",
        }
    }
}

pub fn entity_type_name(entity_type: EntityType) -> &'static str {
    match entity_type {
        EntityType::Person => "person",
        EntityType::Organization => "organization",
        EntityType::Location => "location",
        EntityType::Product => "product",
        EntityType::Date => "date",
        EntityType::Amount => "amount",
        EntityType::Law => "law",
        EntityType::Work => "work",
        EntityType::Event => "event",
        EntityType::Misc => "misc",
    }
}

/// One entity recognized in a piece of text, with word-aligned byte offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct RecognizedEntity {
    pub entity_type: &'static str,
    pub surface_text: String,
    pub entity_key: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub confidence: f32,
}

/// The canonical local NER model, loaded once and reused for every text.
pub struct EntityRecognizer {
    labeler: CandleTokenClassifier,
    model: String,
}

impl EntityRecognizer {
    /// Loads the `bert-base-NER` bundle from `model_dir`, downloading it from Hugging Face
    /// first when it is missing and `allow_download` is set.
    pub fn load(model_dir: &Path, allow_download: bool) -> anyhow::Result<Self> {
        use model_runtime::{HuggingFaceDownloader, ModelBundleStore, ModelPreset};
        let spec = ModelPreset::BertBaseNer.spec();
        let revision = spec.revision_value().unwrap_or("main").to_string();
        let store = ModelBundleStore::new(model_dir)
            .downloader(HuggingFaceDownloader::new().progress(true).max_retries(2));
        let bundle = match store.load(&spec.name, &revision) {
            Ok(bundle) => bundle,
            Err(_) if allow_download => store
                .download(&spec)
                .with_context(|| format!("failed to download NER model {}", spec.name))?,
            Err(error) => anyhow::bail!(
                "NER model {} is not present in {} ({error}); allow the download or place the bundle there",
                spec.name,
                model_dir.display()
            ),
        };
        let labeler = CandleTokenClassifier::from_bundle(bundle)
            .with_context(|| format!("failed to load NER model {}", spec.name))?;
        Ok(Self {
            labeler,
            model: format!("{}@{revision}", spec.name),
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn recognize(&mut self, text: &str) -> anyhow::Result<Vec<RecognizedEntity>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let processing = TextProcessingOptions {
            include_punctuation: true,
            ..TextProcessingOptions::default()
        };
        let sentences = split_sentence_spans(text, &processing);
        let tokens = tokenize(text, &processing);
        let entities =
            extract_named_entities_with_labeler(text, &sentences, &tokens, &mut self.labeler)?;
        Ok(align_entities(text, &entities))
    }
}

/// Snaps producer spans to whole words and merges pieces that cover the same words.
///
/// The token classifier labels sub-word pieces, so one name can arrive as several partial
/// spans ("Alan Turin" + "g"). Expanding to word boundaries and merging overlaps keeps one
/// mention per name; the highest-confidence piece decides the type.
pub fn align_entities(text: &str, entities: &[NamedEntity]) -> Vec<RecognizedEntity> {
    let mut spans: Vec<(usize, usize, &'static str, f32)> = entities
        .iter()
        .filter_map(|entity| {
            let (start, end) = word_bounds(
                text,
                entity.mention.span.byte_start,
                entity.mention.span.byte_end,
            )?;
            Some((
                start,
                end,
                entity_type_name(entity.entity_type),
                entity.confidence,
            ))
        })
        .collect();
    spans.sort_by_key(|span| (span.0, span.1));

    let mut merged: Vec<(usize, usize, &'static str, f32)> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 < last.1 => {
                last.1 = last.1.max(span.1);
                if span.3 > last.3 {
                    last.2 = span.2;
                    last.3 = span.3;
                }
            }
            _ => merged.push(span),
        }
    }

    merged
        .into_iter()
        .filter(|span| span.3 >= MIN_ENTITY_CONFIDENCE)
        .filter_map(|(start, end, entity_type, confidence)| {
            let surface_text = text[start..end].to_string();
            let entity_key = entity_key(&surface_text);
            (!entity_key.is_empty()).then_some(RecognizedEntity {
                entity_type,
                surface_text,
                entity_key,
                byte_start: start,
                byte_end: end,
                confidence,
            })
        })
        .collect()
}

fn word_bounds(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '-';
    let start = text[..start]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(start, |(index, _)| index);
    let end = text[end..]
        .char_indices()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(end, |(index, c)| end + index + c.len_utf8());
    let trimmed_start = start + (text[start..end].len() - text[start..end].trim_start().len());
    let trimmed_end = trimmed_start + text[trimmed_start..end].trim_end().len();
    (trimmed_start < trimmed_end).then_some((trimmed_start, trimmed_end))
}

/// Normalized grouping key: lowercase, inner whitespace collapsed, edge punctuation removed.
pub fn entity_key(surface: &str) -> String {
    surface
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Producer context of one extraction run.
#[derive(Debug, Clone)]
pub struct EntityRunProvenance {
    pub producer: String,
    pub producer_version: String,
    pub model: String,
    pub config_hash: String,
}

impl EntityRunProvenance {
    pub fn canonical(model: &str) -> Self {
        Self {
            producer: ENTITY_PRODUCER.to_string(),
            producer_version: ENTITY_PRODUCER_VERSION.to_string(),
            model: model.to_string(),
            config_hash: format!("word-aligned;min-confidence={MIN_ENTITY_CONFIDENCE}"),
        }
    }
}

/// One mention to persist, tied to exactly one transcript segment or visible-text track.
#[derive(Debug, Clone)]
pub struct EntityMentionInput {
    pub modality: EntityModality,
    pub entity_type: String,
    pub surface_text: String,
    pub entity_key: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub transcript_segment_id: Option<Uuid>,
    pub visual_text_track_id: Option<Uuid>,
    pub byte_start: i32,
    pub byte_end: i32,
    pub confidence: Option<f64>,
}

/// Replaces a video's entity mentions with one new extraction run.
pub async fn replace_entity_run(
    pool: &PgPool,
    video_id: Uuid,
    provenance: &EntityRunProvenance,
    transcript_stream_id: Option<Uuid>,
    segments_analyzed: i32,
    visual_text_tracks_analyzed: i32,
    mentions: &[EntityMentionInput],
) -> anyhow::Result<Uuid> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM entity_extraction_runs WHERE video_id = $1")
        .bind(video_id)
        .execute(&mut *tx)
        .await?;
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO entity_extraction_runs
           (id, video_id, producer, producer_version, model, config_hash, transcript_stream_id,
            segments_analyzed, visual_text_tracks_analyzed)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(run_id)
    .bind(video_id)
    .bind(&provenance.producer)
    .bind(&provenance.producer_version)
    .bind(&provenance.model)
    .bind(&provenance.config_hash)
    .bind(transcript_stream_id)
    .bind(segments_analyzed)
    .bind(visual_text_tracks_analyzed)
    .execute(&mut *tx)
    .await?;
    for mention in mentions {
        sqlx::query(
            "INSERT INTO entity_mentions
               (id, run_id, video_id, modality, entity_type, surface_text, entity_key,
                start_seconds, end_seconds, transcript_segment_id, visual_text_track_id,
                byte_start, byte_end, confidence)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(Uuid::new_v4())
        .bind(run_id)
        .bind(video_id)
        .bind(mention.modality.as_str())
        .bind(&mention.entity_type)
        .bind(&mention.surface_text)
        .bind(&mention.entity_key)
        .bind(mention.start_seconds)
        .bind(mention.end_seconds)
        .bind(mention.transcript_segment_id)
        .bind(mention.visual_text_track_id)
        .bind(mention.byte_start)
        .bind(mention.byte_end)
        .bind(mention.confidence)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(run_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityExtractionReport {
    pub producer: String,
    pub producer_version: String,
    pub model: String,
    pub videos_processed: usize,
    pub videos_without_text: usize,
    pub segments_analyzed: usize,
    pub visual_text_tracks_analyzed: usize,
    pub mentions: usize,
}

struct TextSource {
    modality: EntityModality,
    record_id: Uuid,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    text: String,
}

/// Runs the canonical NER producer over every corpus video's preferred transcript (or best
/// available stream) and its visible-text tracks, replacing each video's previous mentions.
pub async fn extract_corpus_entities(
    pool: &PgPool,
    corpus_id: Uuid,
    model_dir: PathBuf,
    allow_download: bool,
) -> anyhow::Result<EntityExtractionReport> {
    let mut video_ids: Vec<Uuid> = crate::corpora::corpus_video_ids(pool, corpus_id)
        .await?
        .into_iter()
        .collect();
    video_ids.sort();
    let mut recognizer =
        tokio::task::spawn_blocking(move || EntityRecognizer::load(&model_dir, allow_download))
            .await??;
    let provenance = EntityRunProvenance::canonical(recognizer.model());
    let mut report = EntityExtractionReport {
        producer: provenance.producer.clone(),
        producer_version: provenance.producer_version.clone(),
        model: provenance.model.clone(),
        videos_processed: 0,
        videos_without_text: 0,
        segments_analyzed: 0,
        visual_text_tracks_analyzed: 0,
        mentions: 0,
    };

    for video_id in video_ids {
        let stream_id = transcript_stream_for(pool, video_id).await?;
        let mut sources = Vec::new();
        if let Some(stream_id) = stream_id {
            let rows = sqlx::query(
                "SELECT id, start_seconds, end_seconds, text FROM transcript_segments
                 WHERE stream_id = $1 ORDER BY segment_index",
            )
            .bind(stream_id)
            .fetch_all(pool)
            .await?;
            for row in rows {
                sources.push(TextSource {
                    modality: EntityModality::Spoken,
                    record_id: row.try_get("id")?,
                    start_seconds: row.try_get("start_seconds")?,
                    end_seconds: row.try_get("end_seconds")?,
                    text: row.try_get("text")?,
                });
            }
        }
        let segments = sources.len();
        let rows = sqlx::query(
            "SELECT id, start_seconds, end_seconds, text FROM visual_text_tracks
             WHERE video_id = $1 ORDER BY start_seconds NULLS LAST, id",
        )
        .bind(video_id)
        .fetch_all(pool)
        .await?;
        for row in rows {
            sources.push(TextSource {
                modality: EntityModality::VisibleText,
                record_id: row.try_get("id")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                text: row.try_get("text")?,
            });
        }
        let tracks = sources.len() - segments;
        if sources.is_empty() {
            report.videos_without_text += 1;
            continue;
        }

        let (returned, mentions) = tokio::task::spawn_blocking(move || {
            let mut mentions = Vec::new();
            for source in &sources {
                let entities = match recognizer.recognize(&source.text) {
                    Ok(entities) => entities,
                    Err(error) => return (recognizer, Err(error)),
                };
                mentions.extend(
                    entities.into_iter().map(|entity| EntityMentionInput {
                        modality: source.modality,
                        entity_type: entity.entity_type.to_string(),
                        surface_text: entity.surface_text,
                        entity_key: entity.entity_key,
                        start_seconds: source.start_seconds,
                        end_seconds: source.end_seconds,
                        transcript_segment_id: (source.modality == EntityModality::Spoken)
                            .then_some(source.record_id),
                        visual_text_track_id: (source.modality == EntityModality::VisibleText)
                            .then_some(source.record_id),
                        byte_start: entity.byte_start as i32,
                        byte_end: entity.byte_end as i32,
                        confidence: Some(f64::from(entity.confidence)),
                    }),
                );
            }
            (recognizer, Ok(mentions))
        })
        .await?;
        recognizer = returned;
        let mentions = mentions?;
        replace_entity_run(
            pool,
            video_id,
            &provenance,
            stream_id,
            segments as i32,
            tracks as i32,
            &mentions,
        )
        .await?;
        report.videos_processed += 1;
        report.segments_analyzed += segments;
        report.visual_text_tracks_analyzed += tracks;
        report.mentions += mentions.len();
    }
    Ok(report)
}

/// The preferred transcript stream, else the best available one (manual, ASR, automatic).
async fn transcript_stream_for(pool: &PgPool, video_id: Uuid) -> anyhow::Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(
        "SELECT s.id FROM transcript_streams s
         LEFT JOIN preferred_transcript_streams p ON p.stream_id = s.id
         WHERE s.video_id = $1
           AND EXISTS (SELECT 1 FROM transcript_segments t WHERE t.stream_id = s.id)
         ORDER BY (p.stream_id IS NULL),
                  CASE s.source_kind WHEN 'caption_manual' THEN 0 WHEN 'asr' THEN 1 ELSE 2 END,
                  s.created_at
         LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitySummary {
    pub entity_type: String,
    pub entity_key: String,
    pub display_name: String,
    pub spoken_mentions: i64,
    pub visible_text_mentions: i64,
    pub video_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonSummary {
    pub person_id: Uuid,
    pub label: String,
    pub identity_source: Option<String>,
    pub identity_confidence: Option<f64>,
    pub shown_tracks: i64,
    pub heard_tracks: i64,
    pub video_count: i64,
}

/// What has (not) been analyzed, so an empty result is never read as a confirmed negative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityCoverage {
    pub corpus_videos: usize,
    pub entity_extracted_videos: usize,
    pub face_processed_videos: usize,
    pub voice_processed_videos: usize,
    pub people: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityLookupReport {
    pub query: String,
    pub entities: Vec<EntitySummary>,
    pub people: Vec<PersonSummary>,
    pub coverage: EntityCoverage,
}

pub async fn lookup_entities(
    pool: &PgPool,
    corpus_id: Uuid,
    query: &str,
    limit: i64,
) -> anyhow::Result<EntityLookupReport> {
    let video_ids = corpus_video_list(pool, corpus_id).await?;
    let needle = entity_key(query);
    let entities = sqlx::query(
        "SELECT entity_type, entity_key,
                mode() WITHIN GROUP (ORDER BY surface_text) AS display_name,
                count(*) FILTER (WHERE modality = 'spoken') AS spoken,
                count(*) FILTER (WHERE modality = 'visible_text') AS visible,
                count(DISTINCT video_id) AS videos
         FROM entity_mentions
         WHERE video_id = ANY($1) AND ($2 = '' OR strpos(entity_key, $2) > 0)
         GROUP BY entity_type, entity_key
         ORDER BY (entity_key = $2) DESC, count(*) DESC, entity_key, entity_type
         LIMIT $3",
    )
    .bind(&video_ids)
    .bind(&needle)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(EntitySummary {
            entity_type: row.try_get("entity_type")?,
            entity_key: row.try_get("entity_key")?,
            display_name: row.try_get("display_name")?,
            spoken_mentions: row.try_get("spoken")?,
            visible_text_mentions: row.try_get("visible")?,
            video_count: row.try_get("videos")?,
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;

    let people = sqlx::query(
        "WITH tracks AS (
           SELECT pfc.person_id, 'shown' AS kind, ft.id AS track_id, ft.video_id
           FROM person_face_clusters pfc
           JOIN face_cluster_members m ON m.cluster_id = pfc.cluster_id
           JOIN face_tracks ft ON ft.id = m.track_id
           WHERE ft.video_id = ANY($1)
           UNION
           SELECT pvc.person_id, 'heard', vt.id, vt.video_id
           FROM person_voice_clusters pvc
           JOIN voice_cluster_members m ON m.cluster_id = pvc.cluster_id
           JOIN voice_tracks vt ON vt.id = m.track_id
           WHERE vt.video_id = ANY($1)
         )
         SELECT p.id, p.label, p.identity_source, p.identity_confidence,
                count(DISTINCT t.track_id) FILTER (WHERE t.kind = 'shown') AS shown,
                count(DISTINCT t.track_id) FILTER (WHERE t.kind = 'heard') AS heard,
                count(DISTINCT t.video_id) AS videos
         FROM people p
         LEFT JOIN tracks t ON t.person_id = p.id
         WHERE p.corpus_id = $2 AND p.label IS NOT NULL
           AND ($3 = '' OR strpos(lower(p.label), $3) > 0)
         GROUP BY p.id
         ORDER BY count(DISTINCT t.track_id) DESC, p.label, p.id
         LIMIT $4",
    )
    .bind(&video_ids)
    .bind(corpus_id)
    .bind(&needle)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(PersonSummary {
            person_id: row.try_get("id")?,
            label: row.try_get("label")?,
            identity_source: row.try_get("identity_source")?,
            identity_confidence: row.try_get("identity_confidence")?,
            shown_tracks: row.try_get("shown")?,
            heard_tracks: row.try_get("heard")?,
            video_count: row.try_get("videos")?,
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;

    let coverage = entity_coverage(pool, corpus_id, &video_ids).await?;
    Ok(EntityLookupReport {
        query: query.to_string(),
        entities,
        people,
        coverage,
    })
}

async fn entity_coverage(
    pool: &PgPool,
    corpus_id: Uuid,
    video_ids: &[Uuid],
) -> anyhow::Result<EntityCoverage> {
    let row = sqlx::query(
        "SELECT
           (SELECT count(DISTINCT video_id) FROM entity_extraction_runs
             WHERE video_id = ANY($1)) AS extracted,
           (SELECT count(DISTINCT video_id) FROM media_processing_runs
             WHERE video_id = ANY($1) AND modality = 'face') AS faces,
           (SELECT count(DISTINCT video_id) FROM media_processing_runs
             WHERE video_id = ANY($1) AND modality = 'voice') AS voices,
           (SELECT count(*) FROM people WHERE corpus_id = $2) AS people",
    )
    .bind(video_ids)
    .bind(corpus_id)
    .fetch_one(pool)
    .await?;
    let count = |name: &str| -> anyhow::Result<usize> {
        Ok(usize::try_from(row.try_get::<i64, _>(name)?).unwrap_or(0))
    };
    Ok(EntityCoverage {
        corpus_videos: video_ids.len(),
        entity_extracted_videos: count("extracted")?,
        face_processed_videos: count("faces")?,
        voice_processed_videos: count("voices")?,
        people: row.try_get("people")?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityOccurrence {
    pub id: Uuid,
    pub modality: String,
    pub surface_text: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    /// The full transcript segment or visible-text track the mention was found in.
    pub context: String,
    pub byte_start: i32,
    pub byte_end: i32,
    pub confidence: Option<f64>,
    pub transcript_segment_id: Option<Uuid>,
    pub visual_text_track_id: Option<Uuid>,
    pub visual_text_role: Option<String>,
    pub run_id: Uuid,
    pub producer: String,
    pub producer_version: String,
    pub model: String,
    pub config_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityVideoOccurrences {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub spoken_mentions: usize,
    pub visible_text_mentions: usize,
    pub occurrences: Vec<EntityOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityOccurrencesReport {
    pub entity_type: String,
    pub entity_key: String,
    pub videos: Vec<EntityVideoOccurrences>,
}

pub async fn entity_occurrences(
    pool: &PgPool,
    corpus_id: Uuid,
    entity_type: &str,
    entity_key: &str,
) -> anyhow::Result<EntityOccurrencesReport> {
    let video_ids = corpus_video_list(pool, corpus_id).await?;
    let rows = sqlx::query(
        "SELECT m.id, m.video_id, v.title, v.source_url, m.modality, m.surface_text,
                m.start_seconds, m.end_seconds, coalesce(s.text, t.text) AS context,
                m.byte_start, m.byte_end, m.confidence, m.transcript_segment_id,
                m.visual_text_track_id, t.role, r.id AS run_id, r.producer,
                r.producer_version, r.model, r.config_hash
         FROM entity_mentions m
         JOIN entity_extraction_runs r ON r.id = m.run_id
         JOIN videos v ON v.id = m.video_id
         LEFT JOIN transcript_segments s ON s.id = m.transcript_segment_id
         LEFT JOIN visual_text_tracks t ON t.id = m.visual_text_track_id
         WHERE m.video_id = ANY($1) AND m.entity_type = $2 AND m.entity_key = $3
         ORDER BY v.title NULLS LAST, m.video_id, m.start_seconds NULLS LAST, m.byte_start",
    )
    .bind(&video_ids)
    .bind(entity_type)
    .bind(entity_key)
    .fetch_all(pool)
    .await?;

    let mut videos: BTreeMap<Uuid, EntityVideoOccurrences> = BTreeMap::new();
    let mut order = Vec::new();
    for row in rows {
        let video_id: Uuid = row.try_get("video_id")?;
        let occurrence = EntityOccurrence {
            id: row.try_get("id")?,
            modality: row.try_get("modality")?,
            surface_text: row.try_get("surface_text")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
            context: row.try_get("context")?,
            byte_start: row.try_get("byte_start")?,
            byte_end: row.try_get("byte_end")?,
            confidence: row.try_get("confidence")?,
            transcript_segment_id: row.try_get("transcript_segment_id")?,
            visual_text_track_id: row.try_get("visual_text_track_id")?,
            visual_text_role: row.try_get("role")?,
            run_id: row.try_get("run_id")?,
            producer: row.try_get("producer")?,
            producer_version: row.try_get("producer_version")?,
            model: row.try_get("model")?,
            config_hash: row.try_get("config_hash")?,
        };
        let entry = match videos.entry(video_id) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                order.push(video_id);
                entry.insert(EntityVideoOccurrences {
                    video_id,
                    title: row.try_get("title")?,
                    source_url: row.try_get("source_url")?,
                    spoken_mentions: 0,
                    visible_text_mentions: 0,
                    occurrences: Vec::new(),
                })
            }
        };
        if occurrence.modality == "spoken" {
            entry.spoken_mentions += 1;
        } else {
            entry.visible_text_mentions += 1;
        }
        entry.occurrences.push(occurrence);
    }
    Ok(EntityOccurrencesReport {
        entity_type: entity_type.to_string(),
        entity_key: entity_key.to_string(),
        videos: order
            .into_iter()
            .filter_map(|id| videos.remove(&id))
            .collect(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonTrackOccurrence {
    /// `shown` (face track) or `heard` (voice track).
    pub kind: String,
    pub track_id: Uuid,
    pub cluster_id: Uuid,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub link_confidence: Option<f64>,
    pub link_source: Option<String>,
    pub cluster_similarity: Option<f64>,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonVideoOccurrences {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub shown_tracks: usize,
    pub heard_tracks: usize,
    pub occurrences: Vec<PersonTrackOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonOccurrencesReport {
    pub person: PersonSummary,
    pub videos: Vec<PersonVideoOccurrences>,
}

pub async fn person_occurrences(
    pool: &PgPool,
    corpus_id: Uuid,
    person_id: Uuid,
) -> anyhow::Result<PersonOccurrencesReport> {
    let video_ids = corpus_video_list(pool, corpus_id).await?;
    let person = sqlx::query(
        "SELECT id, label, identity_source, identity_confidence FROM people
         WHERE id = $1 AND corpus_id = $2",
    )
    .bind(person_id)
    .bind(corpus_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("person not found"))?;
    let rows = sqlx::query(
        "SELECT 'shown' AS kind, ft.id AS track_id, pfc.cluster_id, ft.video_id,
                ft.start_seconds, ft.end_seconds, pfc.confidence, pfc.source, m.similarity,
                r.processor, r.processor_version, r.model, r.model_version
         FROM person_face_clusters pfc
         JOIN face_cluster_members m ON m.cluster_id = pfc.cluster_id
         JOIN face_tracks ft ON ft.id = m.track_id
         JOIN media_processing_runs r ON r.id = ft.run_id
         WHERE pfc.person_id = $1 AND ft.video_id = ANY($2)
         UNION ALL
         SELECT 'heard', vt.id, pvc.cluster_id, vt.video_id, vt.start_seconds, vt.end_seconds,
                pvc.confidence, pvc.source, m.similarity,
                r.processor, r.processor_version, r.model, r.model_version
         FROM person_voice_clusters pvc
         JOIN voice_cluster_members m ON m.cluster_id = pvc.cluster_id
         JOIN voice_tracks vt ON vt.id = m.track_id
         JOIN media_processing_runs r ON r.id = vt.run_id
         WHERE pvc.person_id = $1 AND vt.video_id = ANY($2)
         ORDER BY video_id, start_seconds",
    )
    .bind(person_id)
    .bind(&video_ids)
    .fetch_all(pool)
    .await?;

    let mut by_video: BTreeMap<Uuid, Vec<PersonTrackOccurrence>> = BTreeMap::new();
    for row in rows {
        by_video
            .entry(row.try_get("video_id")?)
            .or_default()
            .push(PersonTrackOccurrence {
                kind: row.try_get("kind")?,
                track_id: row.try_get("track_id")?,
                cluster_id: row.try_get("cluster_id")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                link_confidence: row.try_get("confidence")?,
                link_source: row.try_get("source")?,
                cluster_similarity: row.try_get("similarity")?,
                processor: row.try_get("processor")?,
                processor_version: row.try_get("processor_version")?,
                model: row.try_get("model")?,
                model_version: row.try_get("model_version")?,
            });
    }
    let ids: Vec<Uuid> = by_video.keys().copied().collect();
    let titles: BTreeMap<Uuid, (Option<String>, String)> =
        sqlx::query("SELECT id, title, source_url FROM videos WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get("id")?,
                    (row.try_get("title")?, row.try_get("source_url")?),
                ))
            })
            .collect::<anyhow::Result<_>>()?;
    let videos: Vec<PersonVideoOccurrences> = by_video
        .into_iter()
        .map(|(video_id, occurrences)| {
            let (title, source_url) = titles.get(&video_id).cloned().unwrap_or_default();
            let mut seen = HashSet::new();
            let mut shown_tracks = 0;
            let mut heard_tracks = 0;
            for occurrence in &occurrences {
                if seen.insert(occurrence.track_id) {
                    if occurrence.kind == "shown" {
                        shown_tracks += 1;
                    } else {
                        heard_tracks += 1;
                    }
                }
            }
            PersonVideoOccurrences {
                video_id,
                title,
                source_url,
                shown_tracks,
                heard_tracks,
                occurrences,
            }
        })
        .collect();
    let summary = PersonSummary {
        person_id,
        label: person
            .try_get::<Option<String>, _>("label")?
            .unwrap_or_default(),
        identity_source: person.try_get("identity_source")?,
        identity_confidence: person.try_get("identity_confidence")?,
        shown_tracks: videos.iter().map(|v| v.shown_tracks as i64).sum(),
        heard_tracks: videos.iter().map(|v| v.heard_tracks as i64).sum(),
        video_count: videos.len() as i64,
    };
    Ok(PersonOccurrencesReport {
        person: summary,
        videos,
    })
}

async fn corpus_video_list(pool: &PgPool, corpus_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    Ok(crate::corpora::corpus_video_ids(pool, corpus_id)
        .await?
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use text_core::TextSpan;

    fn entity(entity_type: EntityType, start: usize, end: usize, confidence: f32) -> NamedEntity {
        NamedEntity {
            id: String::new(),
            entity_type,
            mention: text_linguistics::EntityMentionSpan {
                text: String::new(),
                span: TextSpan {
                    byte_start: start,
                    byte_end: end,
                },
            },
            normalized: String::new(),
            sentence_index: 0,
            token_start: 0,
            token_end: 0,
            confidence,
        }
    }

    #[test]
    fn subword_pieces_snap_to_whole_words_and_merge() {
        // Pieces the bert-base-NER classifier actually returns for this sentence.
        let text = "Welcome to the Helios conference in Berlin with Alan Turing";
        let aligned = align_entities(
            text,
            &[
                entity(EntityType::Misc, 15, 17, 0.85),
                entity(EntityType::Organization, 17, 20, 0.46),
                entity(EntityType::Misc, 20, 21, 0.54),
                entity(EntityType::Location, 36, 42, 0.99),
                entity(EntityType::Person, 48, 58, 0.99),
            ],
        );
        let names: Vec<_> = aligned
            .iter()
            .map(|e| {
                (
                    e.entity_type,
                    e.surface_text.as_str(),
                    e.entity_key.as_str(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("misc", "Helios", "helios"),
                ("location", "Berlin", "berlin"),
                ("person", "Alan Turing", "alan turing"),
            ]
        );
    }

    #[test]
    fn low_confidence_mentions_are_dropped() {
        let aligned = align_entities("maybe Foo here", &[entity(EntityType::Person, 6, 9, 0.4)]);
        assert!(aligned.is_empty());
    }

    #[test]
    fn keys_normalize_case_whitespace_and_edge_punctuation() {
        assert_eq!(entity_key("  Ada   LOVELACE, "), "ada lovelace");
        assert_eq!(entity_key("\"O'Neill\""), "o'neill");
    }
}
