use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgArguments;
use sqlx::query::Query;
use sqlx::{PgPool, Postgres, Row};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;

use crate::config::{SearchMode, SourceKind};
use crate::ingest::vector_literal;

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub database_url: String,
    pub query: String,
    pub top_k: i64,
    pub mode: SearchMode,
    pub source_kind: Option<SourceKind>,
    pub modality: Option<SearchModality>,
    pub video_id: Option<Uuid>,
    pub language: Option<String>,
    pub transcript_start_min: Option<f64>,
    pub transcript_start_max: Option<f64>,
    pub upload_date_from: Option<String>,
    pub upload_date_to: Option<String>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub channel_query: Option<String>,
    pub title_query: Option<String>,
    pub category_query: Option<String>,
    pub tag_query: Option<String>,
    pub metadata_query: Option<String>,
    pub view_count_min: Option<i64>,
    pub view_count_max: Option<i64>,
    /// Exact YouTube video id (the 11-character id, not the corpus UUID).
    pub youtube_id: Option<String>,
    /// Restricts hits to videos of this corpus.
    pub corpus_id: Option<Uuid>,
    /// Raw diarization label of a stored `speaker_turn` annotation overlapping the hit; not an
    /// identity. Scoped to `corpus_id`'s annotations when a corpus is given.
    pub speaker_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchModality {
    Spoken,
    Visible,
}

impl SearchModality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spoken => "spoken",
            Self::Visible => "visible",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrObservationRef {
    pub id: Uuid,
    pub timestamp_seconds: Option<f64>,
    pub frame_index: Option<i64>,
    pub scene_index: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrSceneRef {
    pub id: Uuid,
    pub scene_index: i64,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrEvidence {
    pub run_id: Uuid,
    pub track_key: String,
    pub role: String,
    pub sample_count: i32,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub config_hash: String,
    pub processing_config: serde_json::Value,
    pub observations: Vec<OcrObservationRef>,
    pub scenes: Vec<OcrSceneRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrCoverage {
    pub indexed_videos: usize,
    pub attempted_without_tracks: usize,
    pub unprocessed_videos: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReport {
    pub query: String,
    pub mode: SearchMode,
    /// Which provider answered. Only `postgres` ranks matches; the opt-in Tantivy pilot (#76)
    /// returns literal substring matches in video/timestamp order with zero scores.
    #[serde(default)]
    pub provider: SearchProviderKind,
    pub results: Vec<SearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_coverage: Option<OcrCoverage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProviderKind {
    /// Default: Postgres FTS/semantic/hybrid ranking over `corpus_search_segments`.
    #[default]
    Postgres,
    /// Opt-in pilot: literal case-folded substring terms over a local Tantivy projection.
    TantivyLiteral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub segment_id: Uuid,
    pub video_id: Uuid,
    pub stream_id: Uuid,
    pub source_kind: String,
    pub language: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
    pub source_url: String,
    pub title: Option<String>,
    pub score: f64,
    pub fts_score: f64,
    pub semantic_score: f64,
    pub ocr_evidence: Option<OcrEvidence>,
    /// Present when this hit is generated translated text rather than original evidence.
    #[serde(default)]
    pub translation: Option<TranslationEvidence>,
    /// Citation identifiers, speaker labels and context, attached by
    /// [`crate::citations::attach_citations`] (the CLI search).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<crate::citations::SearchCitation>,
}

/// Provenance of a translated hit and the original-language evidence it was derived from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationEvidence {
    /// `spoken` (transcript) or `visible_text` (OCR).
    pub modality: String,
    /// Original transcript segment or OCR track the translation was derived from.
    pub source_segment_id: Uuid,
    pub source_language: String,
    pub target_language: String,
    pub original_text: String,
    pub provider: String,
    pub producer_revision: String,
    pub plan: serde_json::Value,
    pub config: serde_json::Value,
}

pub async fn search_corpus(request: SearchRequest) -> anyhow::Result<SearchReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    let mut results = match request.mode {
        SearchMode::Fts => fts_search(&pool, &request).await?,
        SearchMode::Semantic => semantic_search(&pool, &request).await?,
        SearchMode::Hybrid => hybrid_search(&pool, &request).await?,
    };
    attach_translation_evidence(&pool, &mut results).await?;
    attach_ocr_evidence(&pool, &mut results).await?;
    Ok(SearchReport {
        query: request.query,
        mode: request.mode,
        provider: SearchProviderKind::Postgres,
        results,
        ocr_coverage: None,
    })
}

async fn fts_search(pool: &PgPool, request: &SearchRequest) -> anyhow::Result<Vec<SearchResult>> {
    let filters = SearchFilterParams::try_from(request)?;
    rows_to_results(
        sqlx::query(
            "SELECT s.id, s.video_id, s.stream_id, s.source_kind, s.language, s.start_seconds,
                    s.end_seconds, s.text, v.source_url, v.title,
                    ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS fts_score,
                    0::float8 AS semantic_score,
                    ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS score
             FROM corpus_search_segments s
             JOIN videos v ON v.id = s.video_id
             WHERE s.search_vector @@ plainto_tsquery('simple', $1)
               AND ($3::text IS NULL OR s.source_kind = $3)
               AND ($4::uuid IS NULL OR s.video_id = $4)
               AND ($5::text IS NULL OR s.language = $5)
               AND ($6::float8 IS NULL OR s.start_seconds >= $6)
               AND ($7::float8 IS NULL OR s.start_seconds <= $7)
               AND ($8::text IS NULL OR v.upload_date >= $8)
               AND ($9::text IS NULL OR v.upload_date <= $9)
               AND ($10::float8 IS NULL OR v.duration_seconds >= $10)
               AND ($11::float8 IS NULL OR v.duration_seconds <= $11)
               AND (
                 $12::text IS NULL
                 OR v.channel ILIKE ('%' || $12 || '%')
                 OR v.channel_id ILIKE ('%' || $12 || '%')
                 OR v.uploader ILIKE ('%' || $12 || '%')
                 OR v.uploader_id ILIKE ('%' || $12 || '%')
               )
               AND ($13::text IS NULL OR v.title ILIKE ('%' || $13 || '%'))
               AND (
                 $14::text IS NULL
                 OR EXISTS (
                   SELECT 1 FROM unnest(v.categories) category
                   WHERE category ILIKE ('%' || $14 || '%')
                 )
               )
               AND (
                 $15::text IS NULL
                 OR EXISTS (
                   SELECT 1 FROM unnest(v.tags) tag
                   WHERE tag ILIKE ('%' || $15 || '%')
                 )
               )
               AND ($16::text IS NULL OR v.metadata::text ILIKE ('%' || $16 || '%'))
               AND ($17::bigint IS NULL OR v.view_count >= $17)
               AND ($18::bigint IS NULL OR v.view_count <= $18)
               AND ($19::text IS NULL
                    OR ($19 = 'visible' AND s.source_kind = 'visual_ocr')
                    OR ($19 = 'spoken' AND s.source_kind <> 'visual_ocr'))
               AND ($20::text IS NULL OR v.youtube_id = $20)
               AND ($21::uuid IS NULL OR EXISTS (
                 SELECT 1 FROM corpus_videos cv
                 WHERE cv.corpus_id = $21 AND cv.video_id = s.video_id
               ))
               AND ($22::text IS NULL OR EXISTS (
                 SELECT 1 FROM research_annotations ra
                 WHERE ra.kind = 'speaker_turn' AND ra.label = $22
                   AND ra.video_id = s.video_id
                   AND ($21::uuid IS NULL OR ra.corpus_id = $21)
                   AND (ra.segment_id = s.id
                        OR (ra.start_seconds < s.end_seconds
                            AND ra.end_seconds > s.start_seconds))
               ))
             ORDER BY score DESC
             LIMIT $2",
        )
        .bind(&request.query)
        .bind(request.top_k)
        .bind_search_filters(&filters)
        .fetch_all(pool)
        .await?,
    )
}

async fn semantic_search(
    pool: &PgPool,
    request: &SearchRequest,
) -> anyhow::Result<Vec<SearchResult>> {
    let query_vector = query_vector(&request.query)?;
    let filters = SearchFilterParams::try_from(request)?;
    rows_to_results(
        sqlx::query(
            "SELECT s.id, s.video_id, s.stream_id, s.source_kind, s.language, s.start_seconds,
                    s.end_seconds, s.text, v.source_url, v.title,
                    0::float8 AS fts_score,
                    (1 - (s.embedding <=> $1::vector))::float8 AS semantic_score,
                    (1 - (s.embedding <=> $1::vector))::float8 AS score
             FROM corpus_search_segments s
             JOIN videos v ON v.id = s.video_id
             WHERE s.embedding IS NOT NULL
               AND ($3::text IS NULL OR s.source_kind = $3)
               AND ($4::uuid IS NULL OR s.video_id = $4)
               AND ($5::text IS NULL OR s.language = $5)
               AND ($6::float8 IS NULL OR s.start_seconds >= $6)
               AND ($7::float8 IS NULL OR s.start_seconds <= $7)
               AND ($8::text IS NULL OR v.upload_date >= $8)
               AND ($9::text IS NULL OR v.upload_date <= $9)
               AND ($10::float8 IS NULL OR v.duration_seconds >= $10)
               AND ($11::float8 IS NULL OR v.duration_seconds <= $11)
               AND (
                 $12::text IS NULL
                 OR v.channel ILIKE ('%' || $12 || '%')
                 OR v.channel_id ILIKE ('%' || $12 || '%')
                 OR v.uploader ILIKE ('%' || $12 || '%')
                 OR v.uploader_id ILIKE ('%' || $12 || '%')
               )
               AND ($13::text IS NULL OR v.title ILIKE ('%' || $13 || '%'))
               AND (
                 $14::text IS NULL
                 OR EXISTS (
                   SELECT 1 FROM unnest(v.categories) category
                   WHERE category ILIKE ('%' || $14 || '%')
                 )
               )
               AND (
                 $15::text IS NULL
                 OR EXISTS (
                   SELECT 1 FROM unnest(v.tags) tag
                   WHERE tag ILIKE ('%' || $15 || '%')
                 )
               )
               AND ($16::text IS NULL OR v.metadata::text ILIKE ('%' || $16 || '%'))
               AND ($17::bigint IS NULL OR v.view_count >= $17)
               AND ($18::bigint IS NULL OR v.view_count <= $18)
               AND ($19::text IS NULL
                    OR ($19 = 'visible' AND s.source_kind = 'visual_ocr')
                    OR ($19 = 'spoken' AND s.source_kind <> 'visual_ocr'))
               AND ($20::text IS NULL OR v.youtube_id = $20)
               AND ($21::uuid IS NULL OR EXISTS (
                 SELECT 1 FROM corpus_videos cv
                 WHERE cv.corpus_id = $21 AND cv.video_id = s.video_id
               ))
               AND ($22::text IS NULL OR EXISTS (
                 SELECT 1 FROM research_annotations ra
                 WHERE ra.kind = 'speaker_turn' AND ra.label = $22
                   AND ra.video_id = s.video_id
                   AND ($21::uuid IS NULL OR ra.corpus_id = $21)
                   AND (ra.segment_id = s.id
                        OR (ra.start_seconds < s.end_seconds
                            AND ra.end_seconds > s.start_seconds))
               ))
             ORDER BY s.embedding <=> $1::vector
             LIMIT $2",
        )
        .bind(query_vector)
        .bind(request.top_k)
        .bind_search_filters(&filters)
        .fetch_all(pool)
        .await?,
    )
}

async fn hybrid_search(
    pool: &PgPool,
    request: &SearchRequest,
) -> anyhow::Result<Vec<SearchResult>> {
    let query_vector = query_vector(&request.query)?;
    let filters = SearchFilterParams::try_from(request)?;
    rows_to_results(
        sqlx::query(
            "WITH fts AS (
                SELECT s.id, ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS score
                FROM corpus_search_segments s
                JOIN videos v ON v.id = s.video_id
                WHERE s.search_vector @@ plainto_tsquery('simple', $1)
                  AND ($4::text IS NULL OR s.source_kind = $4)
                  AND ($5::uuid IS NULL OR s.video_id = $5)
                  AND ($6::text IS NULL OR s.language = $6)
                  AND ($7::float8 IS NULL OR s.start_seconds >= $7)
                  AND ($8::float8 IS NULL OR s.start_seconds <= $8)
                  AND ($9::text IS NULL OR v.upload_date >= $9)
                  AND ($10::text IS NULL OR v.upload_date <= $10)
                  AND ($11::float8 IS NULL OR v.duration_seconds >= $11)
                  AND ($12::float8 IS NULL OR v.duration_seconds <= $12)
                  AND (
                    $13::text IS NULL
                    OR v.channel ILIKE ('%' || $13 || '%')
                    OR v.channel_id ILIKE ('%' || $13 || '%')
                    OR v.uploader ILIKE ('%' || $13 || '%')
                    OR v.uploader_id ILIKE ('%' || $13 || '%')
                  )
                  AND ($14::text IS NULL OR v.title ILIKE ('%' || $14 || '%'))
                  AND (
                    $15::text IS NULL
                    OR EXISTS (
                      SELECT 1 FROM unnest(v.categories) category
                      WHERE category ILIKE ('%' || $15 || '%')
                    )
                  )
                  AND (
                    $16::text IS NULL
                    OR EXISTS (
                      SELECT 1 FROM unnest(v.tags) tag
                      WHERE tag ILIKE ('%' || $16 || '%')
                    )
                  )
                  AND ($17::text IS NULL OR v.metadata::text ILIKE ('%' || $17 || '%'))
                  AND ($18::bigint IS NULL OR v.view_count >= $18)
                  AND ($19::bigint IS NULL OR v.view_count <= $19)
                  AND ($20::text IS NULL
                    OR ($20 = 'visible' AND s.source_kind = 'visual_ocr')
                    OR ($20 = 'spoken' AND s.source_kind <> 'visual_ocr'))
                  AND ($21::text IS NULL OR v.youtube_id = $21)
                  AND ($22::uuid IS NULL OR EXISTS (
                    SELECT 1 FROM corpus_videos cv
                    WHERE cv.corpus_id = $22 AND cv.video_id = s.video_id
                  ))
                  AND ($23::text IS NULL OR EXISTS (
                    SELECT 1 FROM research_annotations ra
                    WHERE ra.kind = 'speaker_turn' AND ra.label = $23
                      AND ra.video_id = s.video_id
                      AND ($22::uuid IS NULL OR ra.corpus_id = $22)
                      AND (ra.segment_id = s.id
                           OR (ra.start_seconds < s.end_seconds
                               AND ra.end_seconds > s.start_seconds))
                  ))
                ORDER BY score DESC
                LIMIT $3
              ),
              sem AS (
                SELECT s.id, (1 - (s.embedding <=> $2::vector))::float8 AS score
                FROM corpus_search_segments s
                JOIN videos v ON v.id = s.video_id
                WHERE s.embedding IS NOT NULL
                  AND ($4::text IS NULL OR s.source_kind = $4)
                  AND ($5::uuid IS NULL OR s.video_id = $5)
                  AND ($6::text IS NULL OR s.language = $6)
                  AND ($7::float8 IS NULL OR s.start_seconds >= $7)
                  AND ($8::float8 IS NULL OR s.start_seconds <= $8)
                  AND ($9::text IS NULL OR v.upload_date >= $9)
                  AND ($10::text IS NULL OR v.upload_date <= $10)
                  AND ($11::float8 IS NULL OR v.duration_seconds >= $11)
                  AND ($12::float8 IS NULL OR v.duration_seconds <= $12)
                  AND (
                    $13::text IS NULL
                    OR v.channel ILIKE ('%' || $13 || '%')
                    OR v.channel_id ILIKE ('%' || $13 || '%')
                    OR v.uploader ILIKE ('%' || $13 || '%')
                    OR v.uploader_id ILIKE ('%' || $13 || '%')
                  )
                  AND ($14::text IS NULL OR v.title ILIKE ('%' || $14 || '%'))
                  AND (
                    $15::text IS NULL
                    OR EXISTS (
                      SELECT 1 FROM unnest(v.categories) category
                      WHERE category ILIKE ('%' || $15 || '%')
                    )
                  )
                  AND (
                    $16::text IS NULL
                    OR EXISTS (
                      SELECT 1 FROM unnest(v.tags) tag
                      WHERE tag ILIKE ('%' || $16 || '%')
                    )
                  )
                  AND ($17::text IS NULL OR v.metadata::text ILIKE ('%' || $17 || '%'))
                  AND ($18::bigint IS NULL OR v.view_count >= $18)
                  AND ($19::bigint IS NULL OR v.view_count <= $19)
                  AND ($20::text IS NULL
                    OR ($20 = 'visible' AND s.source_kind = 'visual_ocr')
                    OR ($20 = 'spoken' AND s.source_kind <> 'visual_ocr'))
                  AND ($21::text IS NULL OR v.youtube_id = $21)
                  AND ($22::uuid IS NULL OR EXISTS (
                    SELECT 1 FROM corpus_videos cv
                    WHERE cv.corpus_id = $22 AND cv.video_id = s.video_id
                  ))
                  AND ($23::text IS NULL OR EXISTS (
                    SELECT 1 FROM research_annotations ra
                    WHERE ra.kind = 'speaker_turn' AND ra.label = $23
                      AND ra.video_id = s.video_id
                      AND ($22::uuid IS NULL OR ra.corpus_id = $22)
                      AND (ra.segment_id = s.id
                           OR (ra.start_seconds < s.end_seconds
                               AND ra.end_seconds > s.start_seconds))
                  ))
                ORDER BY s.embedding <=> $2::vector
                LIMIT $3
              ),
              merged AS (
                SELECT COALESCE(fts.id, sem.id) AS id,
                       COALESCE(fts.score, 0)::float8 AS fts_score,
                       COALESCE(sem.score, 0)::float8 AS semantic_score
                FROM fts FULL OUTER JOIN sem ON fts.id = sem.id
              )
              SELECT s.id, s.video_id, s.stream_id, s.source_kind, s.language, s.start_seconds,
                     s.end_seconds, s.text, v.source_url, v.title, m.fts_score, m.semantic_score,
                     (0.35 * m.fts_score + 0.65 * m.semantic_score)::float8 AS score
              FROM merged m
              JOIN corpus_search_segments s ON s.id = m.id
              JOIN videos v ON v.id = s.video_id
              ORDER BY score DESC
              LIMIT $3",
        )
        .bind(&request.query)
        .bind(query_vector)
        .bind(request.top_k)
        .bind_search_filters(&filters)
        .fetch_all(pool)
        .await?,
    )
}

trait BindSearchFilters<'q> {
    fn bind_search_filters(self, filters: &'q SearchFilterParams) -> Self;
}

impl<'q> BindSearchFilters<'q> for Query<'q, Postgres, PgArguments> {
    fn bind_search_filters(self, filters: &'q SearchFilterParams) -> Self {
        self.bind(filters.source_kind.as_deref())
            .bind(filters.video_id)
            .bind(filters.language.as_deref())
            .bind(filters.transcript_start_min)
            .bind(filters.transcript_start_max)
            .bind(filters.upload_date_from.as_deref())
            .bind(filters.upload_date_to.as_deref())
            .bind(filters.duration_min)
            .bind(filters.duration_max)
            .bind(filters.channel_query.as_deref())
            .bind(filters.title_query.as_deref())
            .bind(filters.category_query.as_deref())
            .bind(filters.tag_query.as_deref())
            .bind(filters.metadata_query.as_deref())
            .bind(filters.view_count_min)
            .bind(filters.view_count_max)
            .bind(filters.modality.as_deref())
            .bind(filters.youtube_id.as_deref())
            .bind(filters.corpus_id)
            .bind(filters.speaker_label.as_deref())
    }
}

fn rows_to_results(rows: Vec<sqlx::postgres::PgRow>) -> anyhow::Result<Vec<SearchResult>> {
    Ok(rows
        .into_iter()
        .map(|row| SearchResult {
            segment_id: row.get("id"),
            video_id: row.get("video_id"),
            stream_id: row.get("stream_id"),
            source_kind: row.get("source_kind"),
            language: row.get("language"),
            start_seconds: row.get("start_seconds"),
            end_seconds: row.get("end_seconds"),
            text: row.get("text"),
            source_url: row.get("source_url"),
            title: row.get("title"),
            score: row.get("score"),
            fts_score: row.get("fts_score"),
            semantic_score: row.get("semantic_score"),
            ocr_evidence: None,
            translation: None,
            citation: None,
        })
        .collect())
}

/// Marks generated translations and links them to their original evidence.
pub(crate) async fn attach_translation_evidence(
    pool: &PgPool,
    results: &mut [SearchResult],
) -> anyhow::Result<()> {
    let ids: Vec<Uuid> = results.iter().map(|result| result.segment_id).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT tr.id, tr.modality, coalesce(tr.transcript_segment_id, tr.visual_text_track_id) AS source_id,
                tr.source_language, tr.target_language, coalesce(s.text, t.text) AS original_text,
                tr.provider, tr.producer_revision, tr.plan, tr.config
         FROM translated_segments tr
         LEFT JOIN transcript_segments s ON s.id = tr.transcript_segment_id
         LEFT JOIN visual_text_tracks t ON t.id = tr.visual_text_track_id
         WHERE tr.id = ANY($1::uuid[])",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let mut by_id = HashMap::new();
    for row in rows {
        by_id.insert(
            row.try_get::<Uuid, _>("id")?,
            TranslationEvidence {
                modality: row.try_get("modality")?,
                source_segment_id: row.try_get("source_id")?,
                source_language: row.try_get("source_language")?,
                target_language: row.try_get("target_language")?,
                original_text: row
                    .try_get::<Option<String>, _>("original_text")?
                    .unwrap_or_default(),
                provider: row.try_get("provider")?,
                producer_revision: row.try_get("producer_revision")?,
                plan: row.try_get("plan")?,
                config: row.try_get("config")?,
            },
        );
    }
    for result in results {
        result.translation = by_id.remove(&result.segment_id);
    }
    Ok(())
}

/// Enrich already-ranked track hits from canonical OCR evidence. Observations are not
/// independent search hits: one track is one relevant on-screen moment.
pub(crate) async fn attach_ocr_evidence(
    pool: &PgPool,
    results: &mut [SearchResult],
) -> anyhow::Result<()> {
    // A translated OCR hit carries the provenance of the OCR track it was derived from.
    let evidence_id = |result: &SearchResult| {
        result
            .translation
            .as_ref()
            .map_or(result.segment_id, |translation| {
                translation.source_segment_id
            })
    };
    let ids: Vec<Uuid> = results
        .iter()
        .filter(|result| result.source_kind == "visual_ocr")
        .map(evidence_id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }

    let rows = sqlx::query(
        r#"SELECT t.id, t.run_id, t.track_key, t.role, t.sample_count,
                  r.processor, r.processor_version, r.model, r.model_version,
                  r.input_hash, r.config_hash, r.processing_config,
                  coalesce((
                    SELECT jsonb_agg(jsonb_build_object(
                      'id', o.id,
                      'timestampSeconds', o.timestamp_seconds,
                      'frameIndex', o.frame_index,
                      'sceneIndex', o.scene_index
                    ) ORDER BY o.timestamp_seconds NULLS LAST, o.id)
                    FROM visual_text_track_observations link
                    JOIN visual_text_observations o ON o.id = link.observation_id
                    WHERE link.track_id = t.id
                  ), '[]'::jsonb) AS observations,
                  coalesce((
                    SELECT jsonb_agg(jsonb_build_object(
                      'id', scene.id,
                      'sceneIndex', scene.scene_index,
                      'startSeconds', scene.start_seconds,
                      'endSeconds', scene.end_seconds
                    ) ORDER BY scene.start_seconds, scene.id)
                    FROM visual_text_track_scenes link
                    JOIN video_scenes scene ON scene.id = link.scene_id
                    WHERE link.track_id = t.id
                  ), '[]'::jsonb) AS scenes
           FROM visual_text_tracks t
           JOIN media_processing_runs r ON r.id = t.run_id AND r.modality = 'ocr'
           WHERE t.id = ANY($1::uuid[])"#,
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    let mut by_id = HashMap::new();
    for row in rows {
        let observations: serde_json::Value = row.try_get("observations")?;
        let scenes: serde_json::Value = row.try_get("scenes")?;
        by_id.insert(
            row.try_get::<Uuid, _>("id")?,
            OcrEvidence {
                run_id: row.try_get("run_id")?,
                track_key: row.try_get("track_key")?,
                role: row.try_get("role")?,
                sample_count: row.try_get("sample_count")?,
                processor: row.try_get("processor")?,
                processor_version: row.try_get("processor_version")?,
                model: row.try_get("model")?,
                model_version: row.try_get("model_version")?,
                input_hash: row.try_get("input_hash")?,
                config_hash: row.try_get("config_hash")?,
                processing_config: row.try_get("processing_config")?,
                observations: serde_json::from_value(observations)?,
                scenes: serde_json::from_value(scenes)?,
            },
        );
    }
    for result in results {
        result.ocr_evidence = by_id.get(&evidence_id(result)).cloned();
    }
    Ok(())
}

fn query_vector(query: &str) -> anyhow::Result<String> {
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let vector = embedder.embed_text(query)?;
    Ok(vector_literal(vector.as_slice()))
}

struct SearchFilterParams {
    source_kind: Option<String>,
    modality: Option<String>,
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
    youtube_id: Option<String>,
    corpus_id: Option<Uuid>,
    speaker_label: Option<String>,
}

impl TryFrom<&SearchRequest> for SearchFilterParams {
    type Error = anyhow::Error;

    fn try_from(request: &SearchRequest) -> anyhow::Result<Self> {
        Ok(Self {
            source_kind: request.source_kind.map(|kind| kind.as_str().to_string()),
            modality: request
                .modality
                .map(|modality| modality.as_str().to_string()),
            video_id: request.video_id,
            language: trim_filter(&request.language),
            transcript_start_min: request.transcript_start_min,
            transcript_start_max: request.transcript_start_max,
            upload_date_from: normalize_upload_date_filter(&request.upload_date_from)?,
            upload_date_to: normalize_upload_date_filter(&request.upload_date_to)?,
            duration_min: request.duration_min,
            duration_max: request.duration_max,
            channel_query: trim_filter(&request.channel_query),
            title_query: trim_filter(&request.title_query),
            category_query: trim_filter(&request.category_query),
            tag_query: trim_filter(&request.tag_query),
            metadata_query: trim_filter(&request.metadata_query),
            view_count_min: request.view_count_min,
            view_count_max: request.view_count_max,
            youtube_id: trim_filter(&request.youtube_id),
            corpus_id: request.corpus_id,
            speaker_label: trim_filter(&request.speaker_label),
        })
    }
}

fn trim_filter(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn normalize_upload_date_filter(
    value: &Option<String>,
) -> anyhow::Result<Option<String>> {
    let Some(value) = trim_filter(value) else {
        return Ok(None);
    };

    if value.len() == 8 && value.chars().all(|character| character.is_ascii_digit()) {
        return Ok(Some(value));
    }

    if value.len() == 10 {
        let bytes = value.as_bytes();
        let valid_date_input = bytes[4] == b'-'
            && bytes[7] == b'-'
            && value
                .chars()
                .enumerate()
                .all(|(index, character)| index == 4 || index == 7 || character.is_ascii_digit());
        if valid_date_input {
            return Ok(Some(value.replace('-', "")));
        }
    }

    anyhow::bail!("upload date filters must use YYYYMMDD or YYYY-MM-DD")
}

/// OCR coverage of a set of corpus videos. A missing OCR processing run is unknown
/// coverage, never a negative finding.
pub async fn corpus_ocr_coverage(
    pool: &sqlx::PgPool,
    video_ids: &std::collections::HashSet<uuid::Uuid>,
) -> anyhow::Result<OcrCoverage> {
    let ids: Vec<uuid::Uuid> = video_ids.iter().copied().collect();
    let rows: Vec<(uuid::Uuid, bool, bool)> = sqlx::query_as(
        "SELECT v.id,
                EXISTS (SELECT 1 FROM media_processing_runs r
                        WHERE r.video_id = v.id AND r.modality = 'ocr') AS attempted,
                EXISTS (SELECT 1 FROM visual_text_tracks t
                        WHERE t.video_id = v.id) AS indexed
         FROM videos v WHERE v.id = ANY($1::uuid[])",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let mut coverage = OcrCoverage {
        indexed_videos: 0,
        attempted_without_tracks: 0,
        unprocessed_videos: 0,
    };
    for (_, attempted, indexed) in rows {
        if indexed {
            coverage.indexed_videos += 1;
        } else if attempted {
            coverage.attempted_without_tracks += 1;
        } else {
            coverage.unprocessed_videos += 1;
        }
    }
    Ok(coverage)
}
