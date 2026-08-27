use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::SearchMode;
use crate::search::{SearchRequest, SearchResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationTarget {
    pub id: Uuid,
    pub video_id: Option<Uuid>,
    pub segment_id: Option<Uuid>,
    pub source_url: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub relevance: u8,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationCase {
    pub id: Uuid,
    pub corpus_id: Uuid,
    pub query: String,
    pub mode: SearchMode,
    pub top_k: i64,
    pub notes: Option<String>,
    pub targets: Vec<EvaluationTarget>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RecordJudgmentRequest {
    pub corpus_id: Uuid,
    pub query: String,
    pub mode: SearchMode,
    pub top_k: i64,
    pub video_id: Option<Uuid>,
    pub segment_id: Option<Uuid>,
    pub source_url: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub relevance: u8,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationCaseReport {
    pub case_id: Uuid,
    pub query: String,
    pub top_k: i64,
    pub relevant_targets: u64,
    pub hits: u64,
    pub recall_at_k: f64,
    pub reciprocal_rank: f64,
    pub ndcg_at_k: f64,
    pub latency_ms: u128,
    pub returned_segment_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationRunReport {
    pub id: Uuid,
    pub corpus_id: Uuid,
    pub cases_count: u64,
    pub recall_at_k: f64,
    pub mean_reciprocal_rank: f64,
    pub ndcg_at_k: f64,
    pub mean_latency_ms: f64,
    pub cases: Vec<EvaluationCaseReport>,
    pub created_at: DateTime<Utc>,
}

pub async fn record_judgment(
    pool: &PgPool,
    request: RecordJudgmentRequest,
) -> anyhow::Result<EvaluationCase> {
    let query = request.query.trim();
    if query.is_empty() {
        anyhow::bail!("evaluation query is required");
    }
    if !(1..=100).contains(&request.top_k) {
        anyhow::bail!("evaluation top_k must be between 1 and 100");
    }
    if !(1..=3).contains(&request.relevance) {
        anyhow::bail!("evaluation relevance must be between 1 and 3");
    }
    if request.video_id.is_none() && request.segment_id.is_none() && request.source_url.is_none() {
        anyhow::bail!("an evaluation judgment needs a video, segment, or source URL target");
    }
    let allowed = crate::corpora::corpus_video_ids(pool, request.corpus_id).await?;
    if let Some(video_id) = request.video_id {
        if !allowed.contains(&video_id) {
            anyhow::bail!("evaluation target video is not part of this corpus");
        }
    }

    let mode = mode_str(request.mode);
    let case_id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "youtube-corpus:evaluation:{}:{}:{}:{}",
            request.corpus_id,
            mode,
            request.top_k,
            query.to_lowercase()
        )
        .as_bytes(),
    );
    sqlx::query(
        "INSERT INTO retrieval_evaluation_cases (id, corpus_id, query, mode, top_k, notes)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (corpus_id, query, mode, top_k) DO UPDATE SET
           notes = coalesce(EXCLUDED.notes, retrieval_evaluation_cases.notes),
           updated_at = now()",
    )
    .bind(case_id)
    .bind(request.corpus_id)
    .bind(query)
    .bind(mode)
    .bind(request.top_k)
    .bind(normalize_optional(request.notes.clone()))
    .execute(pool)
    .await?;

    let target_key = request
        .segment_id
        .map(|id| format!("segment:{id}"))
        .or_else(|| request.video_id.map(|id| format!("video:{id}")))
        .or_else(|| request.source_url.as_ref().map(|url| format!("url:{url}")))
        .expect("target was validated");
    let target_id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "youtube-corpus:evaluation-target:{case_id}:{target_key}:{:?}:{:?}",
            request.start_seconds, request.end_seconds
        )
        .as_bytes(),
    );
    sqlx::query(
        "INSERT INTO retrieval_evaluation_targets (
           id, case_id, video_id, segment_id, source_url,
           start_seconds, end_seconds, relevance, notes
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (id) DO UPDATE SET
           relevance = EXCLUDED.relevance,
           notes = coalesce(EXCLUDED.notes, retrieval_evaluation_targets.notes)",
    )
    .bind(target_id)
    .bind(case_id)
    .bind(request.video_id)
    .bind(request.segment_id)
    .bind(normalize_optional(request.source_url))
    .bind(request.start_seconds)
    .bind(request.end_seconds)
    .bind(i32::from(request.relevance))
    .bind(normalize_optional(request.notes))
    .execute(pool)
    .await?;

    load_case(pool, case_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("saved evaluation case could not be loaded"))
}

pub async fn list_cases(pool: &PgPool, corpus_id: Uuid) -> anyhow::Result<Vec<EvaluationCase>> {
    if crate::corpora::get_corpus(pool, corpus_id).await?.is_none() {
        anyhow::bail!("corpus not found");
    }
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id
         FROM retrieval_evaluation_cases
         WHERE corpus_id = $1
         ORDER BY created_at, id",
    )
    .bind(corpus_id)
    .fetch_all(pool)
    .await?;
    let mut cases = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(case) = load_case(pool, id).await? {
            cases.push(case);
        }
    }
    Ok(cases)
}

pub async fn run_evaluation(
    pool: &PgPool,
    database_url: &str,
    corpus_id: Uuid,
) -> anyhow::Result<EvaluationRunReport> {
    let cases = list_cases(pool, corpus_id).await?;
    if cases.is_empty() {
        anyhow::bail!("no retrieval evaluation cases have been recorded for this corpus");
    }
    let allowed_video_ids = crate::corpora::corpus_video_ids(pool, corpus_id).await?;
    let preferred_stream_ids = crate::transcript_quality::preferred_stream_ids(pool).await?;
    let mut case_reports = Vec::with_capacity(cases.len());

    for case in cases {
        if case.targets.is_empty() {
            continue;
        }
        let started = Instant::now();
        let candidate_top_k = case.top_k.saturating_mul(20).clamp(100, 1000);
        let report = crate::search::search_corpus(SearchRequest {
            database_url: database_url.to_string(),
            query: case.query.clone(),
            top_k: candidate_top_k,
            mode: case.mode,
            source_kind: None,
            video_id: None,
            language: None,
            transcript_start_min: None,
            transcript_start_max: None,
            upload_date_from: None,
            upload_date_to: None,
            duration_min: None,
            duration_max: None,
            channel_query: None,
            title_query: None,
            category_query: None,
            tag_query: None,
            metadata_query: None,
            view_count_min: None,
            view_count_max: None,
        })
        .await?;
        let results = report
            .results
            .into_iter()
            .filter(|result| {
                allowed_video_ids.contains(&result.video_id)
                    && preferred_stream_ids.contains(&result.stream_id)
            })
            .take(case.top_k as usize)
            .collect::<Vec<_>>();
        let latency_ms = started.elapsed().as_millis();
        case_reports.push(score_case(&case, &results, latency_ms));
    }

    if case_reports.is_empty() {
        anyhow::bail!("evaluation cases exist but none contain relevance judgments");
    }
    let divisor = case_reports.len() as f64;
    let recall_at_k = case_reports
        .iter()
        .map(|case| case.recall_at_k)
        .sum::<f64>()
        / divisor;
    let mean_reciprocal_rank = case_reports
        .iter()
        .map(|case| case.reciprocal_rank)
        .sum::<f64>()
        / divisor;
    let ndcg_at_k = case_reports.iter().map(|case| case.ndcg_at_k).sum::<f64>() / divisor;
    let mean_latency_ms = case_reports
        .iter()
        .map(|case| case.latency_ms as f64)
        .sum::<f64>()
        / divisor;
    let id = Uuid::new_v4();
    let created_at = Utc::now();
    let result = EvaluationRunReport {
        id,
        corpus_id,
        cases_count: case_reports.len() as u64,
        recall_at_k,
        mean_reciprocal_rank,
        ndcg_at_k,
        mean_latency_ms,
        cases: case_reports,
        created_at,
    };
    sqlx::query(
        "INSERT INTO retrieval_evaluation_runs (
           id, corpus_id, cases_count, recall_at_k, mean_reciprocal_rank,
           ndcg_at_k, mean_latency_ms, report, created_at
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(result.id)
    .bind(result.corpus_id)
    .bind(result.cases_count as i64)
    .bind(result.recall_at_k)
    .bind(result.mean_reciprocal_rank)
    .bind(result.ndcg_at_k)
    .bind(result.mean_latency_ms)
    .bind(serde_json::to_value(&result)?)
    .bind(result.created_at)
    .execute(pool)
    .await?;
    Ok(result)
}

async fn load_case(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<EvaluationCase>> {
    let Some(row) = sqlx::query(
        "SELECT id, corpus_id, query, mode, top_k, notes, created_at, updated_at
         FROM retrieval_evaluation_cases
         WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let target_rows = sqlx::query(
        "SELECT id, video_id, segment_id, source_url, start_seconds, end_seconds,
                relevance, notes
         FROM retrieval_evaluation_targets
         WHERE case_id = $1
         ORDER BY relevance DESC, created_at, id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let targets = target_rows
        .into_iter()
        .map(|row| {
            let relevance: i32 = row.try_get("relevance")?;
            Ok(EvaluationTarget {
                id: row.try_get("id")?,
                video_id: row.try_get("video_id")?,
                segment_id: row.try_get("segment_id")?,
                source_url: row.try_get("source_url")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                relevance: relevance.clamp(1, 3) as u8,
                notes: row.try_get("notes")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mode: String = row.try_get("mode")?;
    Ok(Some(EvaluationCase {
        id: row.try_get("id")?,
        corpus_id: row.try_get("corpus_id")?,
        query: row.try_get("query")?,
        mode: parse_mode(&mode)?,
        top_k: row.try_get("top_k")?,
        notes: row.try_get("notes")?,
        targets,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    }))
}

fn score_case(
    case: &EvaluationCase,
    results: &[SearchResult],
    latency_ms: u128,
) -> EvaluationCaseReport {
    let mut matched_targets = HashSet::new();
    let mut gains = Vec::with_capacity(results.len());
    let mut reciprocal_rank = 0.0;
    for (index, result) in results.iter().enumerate() {
        let mut gain = 0_u8;
        for target in &case.targets {
            if target_matches(target, result) {
                matched_targets.insert(target.id);
                gain = gain.max(target.relevance);
            }
        }
        gains.push(gain);
        if reciprocal_rank == 0.0 && gain > 0 {
            reciprocal_rank = 1.0 / (index as f64 + 1.0);
        }
    }
    let relevant_targets = case.targets.len() as u64;
    let hits = matched_targets.len() as u64;
    let recall_at_k = if relevant_targets == 0 {
        0.0
    } else {
        hits as f64 / relevant_targets as f64
    };
    let dcg = discounted_gain(&gains);
    let mut ideal = case
        .targets
        .iter()
        .map(|target| target.relevance)
        .collect::<Vec<_>>();
    ideal.sort_unstable_by(|left, right| right.cmp(left));
    ideal.truncate(results.len());
    let ideal_dcg = discounted_gain(&ideal);
    let ndcg_at_k = if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg / ideal_dcg
    };
    EvaluationCaseReport {
        case_id: case.id,
        query: case.query.clone(),
        top_k: case.top_k,
        relevant_targets,
        hits,
        recall_at_k,
        reciprocal_rank,
        ndcg_at_k,
        latency_ms,
        returned_segment_ids: results.iter().map(|result| result.segment_id).collect(),
    }
}

fn target_matches(target: &EvaluationTarget, result: &SearchResult) -> bool {
    if target.segment_id.is_some_and(|id| id != result.segment_id) {
        return false;
    }
    if target.video_id.is_some_and(|id| id != result.video_id) {
        return false;
    }
    if target
        .source_url
        .as_deref()
        .is_some_and(|url| url != result.source_url)
    {
        return false;
    }
    if let Some(start) = target.start_seconds {
        if result
            .end_seconds
            .unwrap_or(result.start_seconds.unwrap_or(0.0))
            < start
        {
            return false;
        }
    }
    if let Some(end) = target.end_seconds {
        if result.start_seconds.unwrap_or(f64::INFINITY) > end {
            return false;
        }
    }
    true
}

fn discounted_gain(gains: &[u8]) -> f64 {
    gains
        .iter()
        .enumerate()
        .map(|(index, relevance)| {
            if *relevance == 0 {
                0.0
            } else {
                (2_f64.powi(i32::from(*relevance)) - 1.0) / ((index as f64 + 2.0).log2())
            }
        })
        .sum()
}

fn mode_str(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Fts => "fts",
        SearchMode::Semantic => "semantic",
        SearchMode::Hybrid => "hybrid",
    }
}

fn parse_mode(mode: &str) -> anyhow::Result<SearchMode> {
    match mode {
        "fts" => Ok(SearchMode::Fts),
        "semantic" => Ok(SearchMode::Semantic),
        "hybrid" => Ok(SearchMode::Hybrid),
        value => anyhow::bail!("unknown evaluation search mode: {value}"),
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{discounted_gain, target_matches, EvaluationTarget};
    use crate::search::SearchResult;
    use uuid::Uuid;

    fn result() -> SearchResult {
        SearchResult {
            segment_id: Uuid::nil(),
            video_id: Uuid::from_u128(1),
            stream_id: Uuid::from_u128(2),
            source_kind: "caption_manual".to_string(),
            language: Some("en".to_string()),
            start_seconds: Some(10.0),
            end_seconds: Some(14.0),
            text: "example".to_string(),
            source_url: "https://example.test/video".to_string(),
            title: Some("Example".to_string()),
            score: 1.0,
            fts_score: 1.0,
            semantic_score: 0.0,
        }
    }

    #[test]
    fn time_range_targets_match_overlapping_segments() {
        let target = EvaluationTarget {
            id: Uuid::from_u128(3),
            video_id: Some(Uuid::from_u128(1)),
            segment_id: None,
            source_url: None,
            start_seconds: Some(12.0),
            end_seconds: Some(16.0),
            relevance: 2,
            notes: None,
        };
        assert!(target_matches(&target, &result()));
    }

    #[test]
    fn discounted_gain_rewards_earlier_relevant_results() {
        assert!(discounted_gain(&[3, 0, 0]) > discounted_gain(&[0, 0, 3]));
    }
}
