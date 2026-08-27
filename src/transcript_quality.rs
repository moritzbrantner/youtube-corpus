use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptQualitySummary {
    pub stream_id: Uuid,
    pub video_id: Uuid,
    pub source_kind: String,
    pub language: Option<String>,
    pub score: f64,
    pub source_priority: i32,
    pub coverage_ratio: f64,
    pub timed_segment_ratio: f64,
    pub text_characters: u64,
    pub segment_count: u64,
    pub reasons: Value,
    pub is_preferred: bool,
    pub evaluated_at: DateTime<Utc>,
}

pub async fn refresh_video_quality(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<TranscriptQualitySummary>> {
    let rows = sqlx::query(
        "WITH stream_stats AS (
           SELECT
             st.id AS stream_id,
             st.video_id,
             st.source_kind,
             st.language,
             count(ts.id)::bigint AS segment_count,
             coalesce(sum(length(ts.text)), 0)::bigint AS text_characters,
             coalesce(
               avg(CASE WHEN ts.start_seconds IS NOT NULL THEN 1.0 ELSE 0.0 END),
               0.0
             )::float8 AS timed_segment_ratio,
             CASE
               WHEN v.duration_seconds IS NOT NULL AND v.duration_seconds > 0 THEN
                 least(coalesce(max(ts.end_seconds), 0.0) / v.duration_seconds, 1.0)::float8
               ELSE 0.0::float8
             END AS coverage_ratio
           FROM transcript_streams st
           JOIN videos v ON v.id = st.video_id
           LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
           WHERE st.video_id = $1
           GROUP BY st.id, st.video_id, st.source_kind, st.language, v.duration_seconds
         ), scored AS (
           SELECT
             stream_id,
             video_id,
             source_kind,
             language,
             CASE source_kind
               WHEN 'caption_manual' THEN 3
               WHEN 'caption_auto' THEN 2
               ELSE 1
             END AS source_priority,
             coverage_ratio,
             timed_segment_ratio,
             text_characters,
             segment_count,
             least(
               1.0,
               CASE source_kind
                 WHEN 'caption_manual' THEN 0.45
                 WHEN 'caption_auto' THEN 0.30
                 ELSE 0.25
               END
               + CASE WHEN text_characters > 0 THEN 0.15 ELSE 0.0 END
               + 0.15 * least(text_characters::float8 / 20000.0, 1.0)
               + 0.15 * timed_segment_ratio
               + 0.10 * coverage_ratio
             )::float8 AS score
           FROM stream_stats
         )
         INSERT INTO transcript_stream_quality (
           stream_id,
           video_id,
           score,
           source_priority,
           coverage_ratio,
           timed_segment_ratio,
           text_characters,
           segment_count,
           reasons,
           evaluator,
           evaluator_version,
           evaluation_config,
           evaluated_at
         )
         SELECT
           stream_id,
           video_id,
           score,
           source_priority,
           coverage_ratio,
           timed_segment_ratio,
           text_characters,
           segment_count,
           jsonb_build_object(
             'sourceKind', source_kind,
             'language', language,
             'sourcePriority', source_priority,
             'coverageRatio', coverage_ratio,
             'timedSegmentRatio', timed_segment_ratio,
             'textCharacters', text_characters,
             'segmentCount', segment_count
           ),
           'youtube-corpus',
           $2,
           jsonb_build_object('algorithm', 'source-coverage-v1'),
           now()
         FROM scored
         ON CONFLICT (stream_id) DO UPDATE SET
           video_id = EXCLUDED.video_id,
           score = EXCLUDED.score,
           source_priority = EXCLUDED.source_priority,
           coverage_ratio = EXCLUDED.coverage_ratio,
           timed_segment_ratio = EXCLUDED.timed_segment_ratio,
           text_characters = EXCLUDED.text_characters,
           segment_count = EXCLUDED.segment_count,
           reasons = EXCLUDED.reasons,
           evaluator = EXCLUDED.evaluator,
           evaluator_version = EXCLUDED.evaluator_version,
           evaluation_config = EXCLUDED.evaluation_config,
           evaluated_at = EXCLUDED.evaluated_at
         RETURNING stream_id",
    )
    .bind(video_id)
    .bind(env!("CARGO_PKG_VERSION"))
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        sqlx::query("DELETE FROM preferred_transcript_streams WHERE video_id = $1")
            .bind(video_id)
            .execute(pool)
            .await?;
        return Ok(Vec::new());
    }

    sqlx::query(
        "INSERT INTO preferred_transcript_streams (
           video_id, stream_id, score, selector, selector_version, selected_at
         )
         SELECT q.video_id, q.stream_id, q.score, 'youtube-corpus', $2, now()
         FROM transcript_stream_quality q
         WHERE q.video_id = $1
         ORDER BY q.score DESC, q.source_priority DESC, q.text_characters DESC, q.stream_id
         LIMIT 1
         ON CONFLICT (video_id) DO UPDATE SET
           stream_id = EXCLUDED.stream_id,
           score = EXCLUDED.score,
           selector = EXCLUDED.selector,
           selector_version = EXCLUDED.selector_version,
           selected_at = EXCLUDED.selected_at",
    )
    .bind(video_id)
    .bind(env!("CARGO_PKG_VERSION"))
    .execute(pool)
    .await?;

    list_video_quality(pool, video_id).await
}

pub async fn refresh_corpus_quality(
    pool: &PgPool,
    corpus_id: Uuid,
) -> anyhow::Result<Vec<TranscriptQualitySummary>> {
    let video_ids = crate::corpora::corpus_video_ids(pool, corpus_id).await?;
    for video_id in video_ids {
        refresh_video_quality(pool, video_id).await?;
    }
    list_corpus_quality(pool, corpus_id).await
}

pub async fn list_video_quality(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<TranscriptQualitySummary>> {
    let rows = sqlx::query(
        "SELECT q.stream_id, q.video_id, st.source_kind, st.language, q.score,
                q.source_priority, q.coverage_ratio, q.timed_segment_ratio,
                q.text_characters, q.segment_count, q.reasons, q.evaluated_at,
                (p.stream_id = q.stream_id) AS is_preferred
         FROM transcript_stream_quality q
         JOIN transcript_streams st ON st.id = q.stream_id
         LEFT JOIN preferred_transcript_streams p ON p.video_id = q.video_id
         WHERE q.video_id = $1
         ORDER BY q.score DESC, q.source_priority DESC, q.text_characters DESC, q.stream_id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(quality_from_row).collect()
}

pub async fn list_corpus_quality(
    pool: &PgPool,
    corpus_id: Uuid,
) -> anyhow::Result<Vec<TranscriptQualitySummary>> {
    let allowed = crate::corpora::corpus_video_ids(pool, corpus_id).await?;
    let rows = sqlx::query(
        "SELECT q.stream_id, q.video_id, st.source_kind, st.language, q.score,
                q.source_priority, q.coverage_ratio, q.timed_segment_ratio,
                q.text_characters, q.segment_count, q.reasons, q.evaluated_at,
                (p.stream_id = q.stream_id) AS is_preferred
         FROM transcript_stream_quality q
         JOIN transcript_streams st ON st.id = q.stream_id
         LEFT JOIN preferred_transcript_streams p ON p.video_id = q.video_id
         ORDER BY q.video_id, q.score DESC, q.source_priority DESC, q.text_characters DESC, q.stream_id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .filter(|row| {
            row.try_get::<Uuid, _>("video_id")
                .map(|video_id| allowed.contains(&video_id))
                .unwrap_or(false)
        })
        .map(quality_from_row)
        .collect()
}

pub async fn preferred_stream_ids(pool: &PgPool) -> anyhow::Result<HashSet<Uuid>> {
    let ids = sqlx::query_scalar::<_, Uuid>("SELECT stream_id FROM preferred_transcript_streams")
        .fetch_all(pool)
        .await?;
    Ok(ids.into_iter().collect())
}

fn quality_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<TranscriptQualitySummary> {
    let text_characters: i64 = row.try_get("text_characters")?;
    let segment_count: i64 = row.try_get("segment_count")?;
    Ok(TranscriptQualitySummary {
        stream_id: row.try_get("stream_id")?,
        video_id: row.try_get("video_id")?,
        source_kind: row.try_get("source_kind")?,
        language: row.try_get("language")?,
        score: row.try_get("score")?,
        source_priority: row.try_get("source_priority")?,
        coverage_ratio: row.try_get("coverage_ratio")?,
        timed_segment_ratio: row.try_get("timed_segment_ratio")?,
        text_characters: text_characters.max(0) as u64,
        segment_count: segment_count.max(0) as u64,
        reasons: row.try_get("reasons")?,
        is_preferred: row.try_get("is_preferred")?,
        evaluated_at: row.try_get("evaluated_at")?,
    })
}

#[cfg(test)]
mod tests {
    fn score(source: &str, characters: u64, timed: f64, coverage: f64) -> f64 {
        let base = match source {
            "caption_manual" => 0.45,
            "caption_auto" => 0.30,
            _ => 0.25,
        };
        let has_text = if characters > 0 { 0.15 } else { 0.0 };
        (base
            + has_text
            + 0.15 * (characters as f64 / 20_000.0).min(1.0)
            + 0.15 * timed.clamp(0.0, 1.0)
            + 0.10 * coverage.clamp(0.0, 1.0))
        .min(1.0)
    }

    #[test]
    fn manual_captions_win_when_quality_is_comparable() {
        let manual = score("caption_manual", 10_000, 0.95, 0.95);
        let automatic = score("caption_auto", 10_000, 0.95, 0.95);
        assert!(manual > automatic);
    }

    #[test]
    fn empty_manual_caption_can_lose_to_complete_automatic_caption() {
        let manual = score("caption_manual", 0, 0.0, 0.0);
        let automatic = score("caption_auto", 20_000, 1.0, 1.0);
        assert!(automatic > manual);
    }
}
