//! Resumable re-embedding of the stored corpus with the current embedding model (#192).
//!
//! Walks transcript segments, OCR text tracks and translated spans in id order and fills
//! `embedding` / `embedding_model` in batches. Each batch is committed on its own, so an
//! interrupted run resumes where it stopped: rows already embedded by the current model
//! (same model id, revision and pooling) are skipped. Rows with blank text get no vector.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::embeddings::{vector_literal, CorpusEmbedder, EmbeddingIdentity};

/// A table holding embedded corpus text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReembedTarget {
    TranscriptSegments,
    VisualTextTracks,
    TranslatedSegments,
}

impl ReembedTarget {
    pub const ALL: [Self; 3] = [
        Self::TranscriptSegments,
        Self::VisualTextTracks,
        Self::TranslatedSegments,
    ];

    pub fn table(self) -> &'static str {
        match self {
            Self::TranscriptSegments => "transcript_segments",
            Self::VisualTextTracks => "visual_text_tracks",
            Self::TranslatedSegments => "translated_segments",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReembedRequest {
    pub targets: Vec<ReembedTarget>,
    /// Rows per embedding call and transaction.
    pub batch_size: usize,
    /// Stop after this many rows per table (for trial runs).
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReembedTableReport {
    pub table: String,
    /// Rows that still needed the current model when the run started.
    pub pending: u64,
    pub embedded: u64,
    pub seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReembedReport {
    pub model: EmbeddingIdentity,
    pub tables: Vec<ReembedTableReport>,
    /// Embeddings per second over the whole run.
    pub embeddings_per_second: f64,
}

/// Fills missing or stale vectors with `embedder`. `progress` receives one line per batch.
pub async fn reembed_corpus(
    pool: &PgPool,
    embedder: std::sync::Arc<CorpusEmbedder>,
    request: &ReembedRequest,
    mut progress: impl FnMut(&str),
) -> anyhow::Result<ReembedReport> {
    anyhow::ensure!(request.batch_size > 0, "batch size must be positive");
    let model = crate::embeddings::model_row_id(pool, embedder.identity()).await?;
    let started = Instant::now();
    let mut tables = Vec::new();
    let mut total = 0_u64;
    for target in &request.targets {
        let report = reembed_table(pool, &embedder, model, *target, request, &mut progress).await?;
        total += report.embedded;
        tables.push(report);
    }
    let seconds = started.elapsed().as_secs_f64();
    Ok(ReembedReport {
        model: embedder.identity().clone(),
        tables,
        embeddings_per_second: if seconds > 0.0 {
            total as f64 / seconds
        } else {
            0.0
        },
    })
}

async fn reembed_table(
    pool: &PgPool,
    embedder: &std::sync::Arc<CorpusEmbedder>,
    model: i32,
    target: ReembedTarget,
    request: &ReembedRequest,
    progress: &mut impl FnMut(&str),
) -> anyhow::Result<ReembedTableReport> {
    let table = target.table();
    // Table names come from the closed `ReembedTarget` set, never from input.
    let pending_filter = "embedding_model IS DISTINCT FROM $1 AND btrim(coalesce(text, '')) <> ''";
    let pending: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE {pending_filter}"
    )))
    .bind(model)
    .fetch_one(pool)
    .await?;
    let pending = pending.max(0) as u64;
    let goal = request.limit.map_or(pending, |limit| limit.min(pending));
    let select = format!(
        "SELECT id, text FROM {table}
         WHERE {pending_filter} AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id
         LIMIT $3"
    );
    let update = format!(
        "UPDATE {table} SET embedding = $2::vector, embedding_model = $3
         WHERE id = $1 AND text = $4"
    );
    let started = Instant::now();
    let mut embedded = 0_u64;
    let mut after: Option<Uuid> = None;
    while embedded < goal {
        let take = (goal - embedded).min(request.batch_size as u64) as i64;
        let rows = sqlx::query(sqlx::AssertSqlSafe(select.clone()))
            .bind(model)
            .bind(after)
            .bind(take)
            .fetch_all(pool)
            .await?;
        if rows.is_empty() {
            break;
        }
        let mut ids = Vec::with_capacity(rows.len());
        let mut texts = Vec::with_capacity(rows.len());
        for row in &rows {
            ids.push(row.try_get::<Uuid, _>("id")?);
            texts.push(row.try_get::<String, _>("text")?);
        }
        after = ids.last().copied();
        let batch_embedder = embedder.clone();
        let batch_texts = texts.clone();
        let vectors = tokio::task::spawn_blocking(move || {
            let inputs = batch_texts
                .iter()
                .map(|text| text.trim())
                .collect::<Vec<_>>();
            batch_embedder.embed_passages(&inputs)
        })
        .await??;
        let mut tx = pool.begin().await?;
        for ((id, text), vector) in ids.iter().zip(&texts).zip(&vectors) {
            // Matching on the text skips a row whose text changed since it was read; the
            // writer that changed it embedded it itself.
            sqlx::query(sqlx::AssertSqlSafe(update.clone()))
                .bind(id)
                .bind(vector_literal(vector))
                .bind(model)
                .bind(text)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        embedded += ids.len() as u64;
        let seconds = started.elapsed().as_secs_f64();
        let rate = embedded as f64 / seconds.max(f64::EPSILON);
        let remaining = goal.saturating_sub(embedded) as f64 / rate.max(f64::EPSILON);
        progress(&format!(
            "{table}: {embedded}/{goal} embedded ({rate:.1}/s, about {} left)",
            format_duration(remaining)
        ));
    }
    Ok(ReembedTableReport {
        table: table.to_string(),
        pending,
        embedded,
        seconds: started.elapsed().as_secs_f64(),
    })
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.round() as u64;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_readable() {
        assert_eq!(format_duration(42.0), "42s");
        assert_eq!(format_duration(125.0), "2m05s");
        assert_eq!(format_duration(7260.0), "2h01m");
    }

    #[test]
    fn every_embedded_table_is_a_target() {
        let tables = ReembedTarget::ALL.map(ReembedTarget::table);
        assert_eq!(
            tables,
            [
                "transcript_segments",
                "visual_text_tracks",
                "translated_segments"
            ]
        );
    }
}
