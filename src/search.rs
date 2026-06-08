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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReport {
    pub query: String,
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
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
}

pub async fn search_corpus(request: SearchRequest) -> anyhow::Result<SearchReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    let results = match request.mode {
        SearchMode::Fts => fts_search(&pool, &request).await?,
        SearchMode::Semantic => semantic_search(&pool, &request).await?,
        SearchMode::Hybrid => hybrid_search(&pool, &request).await?,
    };
    Ok(SearchReport {
        query: request.query,
        mode: request.mode,
        results,
    })
}

async fn fts_search(pool: &PgPool, request: &SearchRequest) -> anyhow::Result<Vec<SearchResult>> {
    let filters = SearchFilterParams::try_from(request)?;
    rows_to_results(
        sqlx::query(
            "SELECT s.id, s.video_id, s.stream_id, st.source_kind, s.language, s.start_seconds,
                    s.end_seconds, s.text, v.source_url, v.title,
                    ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS fts_score,
                    0::float8 AS semantic_score,
                    ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS score
             FROM transcript_segments s
             JOIN transcript_streams st ON st.id = s.stream_id
             JOIN videos v ON v.id = s.video_id
             WHERE s.search_vector @@ plainto_tsquery('simple', $1)
               AND ($3::text IS NULL OR st.source_kind = $3)
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
            "SELECT s.id, s.video_id, s.stream_id, st.source_kind, s.language, s.start_seconds,
                    s.end_seconds, s.text, v.source_url, v.title,
                    0::float8 AS fts_score,
                    (1 - (s.embedding <=> $1::vector))::float8 AS semantic_score,
                    (1 - (s.embedding <=> $1::vector))::float8 AS score
             FROM transcript_segments s
             JOIN transcript_streams st ON st.id = s.stream_id
             JOIN videos v ON v.id = s.video_id
             WHERE s.embedding IS NOT NULL
               AND ($3::text IS NULL OR st.source_kind = $3)
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
                FROM transcript_segments s
                JOIN transcript_streams st ON st.id = s.stream_id
                JOIN videos v ON v.id = s.video_id
                WHERE s.search_vector @@ plainto_tsquery('simple', $1)
                  AND ($4::text IS NULL OR st.source_kind = $4)
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
                ORDER BY score DESC
                LIMIT $3
              ),
              sem AS (
                SELECT s.id, (1 - (s.embedding <=> $2::vector))::float8 AS score
                FROM transcript_segments s
                JOIN transcript_streams st ON st.id = s.stream_id
                JOIN videos v ON v.id = s.video_id
                WHERE s.embedding IS NOT NULL
                  AND ($4::text IS NULL OR st.source_kind = $4)
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
                ORDER BY s.embedding <=> $2::vector
                LIMIT $3
              ),
              merged AS (
                SELECT COALESCE(fts.id, sem.id) AS id,
                       COALESCE(fts.score, 0)::float8 AS fts_score,
                       COALESCE(sem.score, 0)::float8 AS semantic_score
                FROM fts FULL OUTER JOIN sem ON fts.id = sem.id
              )
              SELECT s.id, s.video_id, s.stream_id, st.source_kind, s.language, s.start_seconds,
                     s.end_seconds, s.text, v.source_url, v.title, m.fts_score, m.semantic_score,
                     (0.35 * m.fts_score + 0.65 * m.semantic_score)::float8 AS score
              FROM merged m
              JOIN transcript_segments s ON s.id = m.id
              JOIN transcript_streams st ON st.id = s.stream_id
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
        })
        .collect())
}

fn query_vector(query: &str) -> anyhow::Result<String> {
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let vector = embedder.embed_text(query)?;
    Ok(vector_literal(vector.as_slice()))
}

struct SearchFilterParams {
    source_kind: Option<String>,
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
}

impl TryFrom<&SearchRequest> for SearchFilterParams {
    type Error = anyhow::Error;

    fn try_from(request: &SearchRequest) -> anyhow::Result<Self> {
        Ok(Self {
            source_kind: request.source_kind.map(|kind| kind.as_str().to_string()),
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

fn normalize_upload_date_filter(value: &Option<String>) -> anyhow::Result<Option<String>> {
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
