use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
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
             ORDER BY score DESC
             LIMIT $2",
        )
        .bind(&request.query)
        .bind(request.top_k)
        .bind(request.source_kind.map(|kind| kind.as_str().to_string()))
        .bind(request.video_id)
        .fetch_all(pool)
        .await?,
    )
}

async fn semantic_search(
    pool: &PgPool,
    request: &SearchRequest,
) -> anyhow::Result<Vec<SearchResult>> {
    let query_vector = query_vector(&request.query)?;
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
             ORDER BY s.embedding <=> $1::vector
             LIMIT $2",
        )
        .bind(query_vector)
        .bind(request.top_k)
        .bind(request.source_kind.map(|kind| kind.as_str().to_string()))
        .bind(request.video_id)
        .fetch_all(pool)
        .await?,
    )
}

async fn hybrid_search(
    pool: &PgPool,
    request: &SearchRequest,
) -> anyhow::Result<Vec<SearchResult>> {
    let query_vector = query_vector(&request.query)?;
    rows_to_results(
        sqlx::query(
            "WITH fts AS (
                SELECT s.id, ts_rank(s.search_vector, plainto_tsquery('simple', $1))::float8 AS score
                FROM transcript_segments s
                JOIN transcript_streams st ON st.id = s.stream_id
                WHERE s.search_vector @@ plainto_tsquery('simple', $1)
                  AND ($4::text IS NULL OR st.source_kind = $4)
                  AND ($5::uuid IS NULL OR s.video_id = $5)
                ORDER BY score DESC
                LIMIT $3
              ),
              sem AS (
                SELECT s.id, (1 - (s.embedding <=> $2::vector))::float8 AS score
                FROM transcript_segments s
                JOIN transcript_streams st ON st.id = s.stream_id
                WHERE s.embedding IS NOT NULL
                  AND ($4::text IS NULL OR st.source_kind = $4)
                  AND ($5::uuid IS NULL OR s.video_id = $5)
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
        .bind(request.source_kind.map(|kind| kind.as_str().to_string()))
        .bind(request.video_id)
        .fetch_all(pool)
        .await?,
    )
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
