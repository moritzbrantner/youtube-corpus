use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::types::{
    admission_decision, DiscoveryEvidence, DiscoveryKind, DiscoveryMethod, DiscoveryPolicy,
    DiscoveryState, DiscoveryTarget, EnqueueDiscoveryRequest, DEFAULT_CLAIM_LEASE_SECONDS,
};

pub async fn discovery_schema_available(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar::<_, bool>("SELECT to_regclass('public.discovery_targets') IS NOT NULL")
            .fetch_one(pool)
            .await?,
    )
}

pub(crate) async fn find_discovery_target(
    pool: &PgPool,
    kind: DiscoveryKind,
    canonical_key: &str,
) -> anyhow::Result<Option<DiscoveryTarget>> {
    let row = sqlx::query(
        "SELECT id, kind, canonical_key, target_url, state, depth, priority,
                confidence, relevance, novelty, claimed_by, workflow_run_id,
                attempt_count, last_error, discovered_at, updated_at, claimed_at,
                claim_expires_at, completed_at
         FROM discovery_targets
         WHERE kind = $1 AND canonical_key = $2",
    )
    .bind(kind.as_str())
    .bind(canonical_key)
    .fetch_optional(pool)
    .await?;
    row.map(target_from_row).transpose()
}

pub async fn enqueue_discovery(
    pool: &PgPool,
    request: EnqueueDiscoveryRequest,
    policy: &DiscoveryPolicy,
) -> anyhow::Result<DiscoveryTarget> {
    validate_request(&request)?;
    let decision = admission_decision(&request, policy);
    let target_id = stable_target_id(request.kind, &request.canonical_key);
    let row = sqlx::query(
        "INSERT INTO discovery_targets (
           id, kind, canonical_key, target_url, state, depth, priority,
           confidence, relevance, novelty
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (kind, canonical_key) DO UPDATE SET
           target_url = EXCLUDED.target_url,
           depth = LEAST(discovery_targets.depth, EXCLUDED.depth),
           priority = GREATEST(discovery_targets.priority, EXCLUDED.priority),
           confidence = GREATEST(discovery_targets.confidence, EXCLUDED.confidence),
           relevance = GREATEST(discovery_targets.relevance, EXCLUDED.relevance),
           novelty = GREATEST(discovery_targets.novelty, EXCLUDED.novelty),
           state = CASE
             WHEN discovery_targets.state IN ('completed', 'claimed') THEN discovery_targets.state
             WHEN EXCLUDED.state = 'pending' THEN 'pending'
             ELSE discovery_targets.state
           END,
           last_error = CASE
             WHEN discovery_targets.state = 'failed' AND EXCLUDED.state = 'pending' THEN NULL
             ELSE discovery_targets.last_error
           END,
           updated_at = now()
         RETURNING id, kind, canonical_key, target_url, state, depth, priority,
                   confidence, relevance, novelty, claimed_by, workflow_run_id,
                   attempt_count, last_error, discovered_at, updated_at, claimed_at,
                   claim_expires_at, completed_at",
    )
    .bind(target_id)
    .bind(request.kind.as_str())
    .bind(&request.canonical_key)
    .bind(&request.target_url)
    .bind(decision.state.as_str())
    .bind(i32::try_from(request.depth)?)
    .bind(decision.priority)
    .bind(request.confidence)
    .bind(request.relevance)
    .bind(request.novelty)
    .fetch_one(pool)
    .await?;
    let target = target_from_row(row)?;

    let evidence_key = json!({
        "targetId": target.id,
        "sourceVideoId": request.source_video_id,
        "parentTargetId": request.parent_target_id,
        "method": request.method,
        "evidence": request.evidence.clone(),
        "depth": request.depth,
        "confidence": request.confidence,
        "relevance": request.relevance,
        "novelty": request.novelty,
    });
    let evidence_id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        serde_json::to_string(&evidence_key)?.as_bytes(),
    );
    sqlx::query(
        "INSERT INTO discovery_evidence (
           id, target_id, source_video_id, parent_target_id, method, evidence,
           confidence, relevance, novelty, depth
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(evidence_id)
    .bind(target.id)
    .bind(request.source_video_id)
    .bind(request.parent_target_id)
    .bind(request.method.as_str())
    .bind(request.evidence)
    .bind(request.confidence)
    .bind(request.relevance)
    .bind(request.novelty)
    .bind(i32::try_from(request.depth)?)
    .execute(pool)
    .await?;

    Ok(target)
}

pub async fn list_frontier(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<DiscoveryTarget>> {
    let rows = sqlx::query(
        "SELECT id, kind, canonical_key, target_url, state, depth, priority,
                confidence, relevance, novelty, claimed_by, workflow_run_id,
                attempt_count, last_error, discovered_at, updated_at, claimed_at,
                claim_expires_at, completed_at
         FROM discovery_targets
         WHERE state = 'pending'
         ORDER BY priority DESC, discovered_at ASC, id ASC
         LIMIT $1",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(target_from_row).collect()
}

pub async fn list_discovery_evidence(
    pool: &PgPool,
    target_id: Uuid,
) -> anyhow::Result<Vec<DiscoveryEvidence>> {
    let rows = sqlx::query(
        "SELECT id, target_id, source_video_id, parent_target_id, method, evidence,
                confidence, relevance, novelty, depth, created_at
         FROM discovery_evidence
         WHERE target_id = $1
         ORDER BY created_at ASC, id ASC",
    )
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(evidence_from_row).collect()
}

pub async fn claim_next_discovery(
    pool: &PgPool,
    claimed_by: &str,
    workflow_run_id: Option<&str>,
) -> anyhow::Result<Option<DiscoveryTarget>> {
    claim_next_discovery_with_lease(
        pool,
        claimed_by,
        workflow_run_id,
        DEFAULT_CLAIM_LEASE_SECONDS,
    )
    .await
}

pub async fn claim_next_discovery_with_lease(
    pool: &PgPool,
    claimed_by: &str,
    workflow_run_id: Option<&str>,
    lease_seconds: i64,
) -> anyhow::Result<Option<DiscoveryTarget>> {
    if claimed_by.trim().is_empty() {
        anyhow::bail!("claimed_by must not be empty");
    }
    if lease_seconds <= 0 {
        anyhow::bail!("lease_seconds must be positive");
    }
    let row = sqlx::query(
        "WITH next AS (
           SELECT id
           FROM discovery_targets
           WHERE state = 'pending'
              OR (state = 'claimed' AND claim_expires_at < now())
           ORDER BY priority DESC, discovered_at ASC, id ASC
           FOR UPDATE SKIP LOCKED
           LIMIT 1
         )
         UPDATE discovery_targets target
         SET state = 'claimed',
             claimed_by = $1,
             workflow_run_id = COALESCE($2, target.workflow_run_id),
             attempt_count = target.attempt_count + 1,
             claimed_at = now(),
             claim_expires_at = now() + make_interval(secs => $3::double precision),
             updated_at = now()
         FROM next
         WHERE target.id = next.id
         RETURNING target.id, target.kind, target.canonical_key, target.target_url, target.state,
                   target.depth, target.priority, target.confidence, target.relevance, target.novelty,
                   target.claimed_by, target.workflow_run_id, target.attempt_count, target.last_error,
                   target.discovered_at, target.updated_at, target.claimed_at,
                   target.claim_expires_at, target.completed_at",
    )
    .bind(claimed_by)
    .bind(workflow_run_id)
    .bind(lease_seconds as f64)
    .fetch_optional(pool)
    .await?;
    row.map(target_from_row).transpose()
}

pub async fn complete_discovery(
    pool: &PgPool,
    id: Uuid,
    claim_attempt: u32,
    workflow_run_id: Option<&str>,
) -> anyhow::Result<DiscoveryTarget> {
    transition_claimed_target(
        pool,
        id,
        claim_attempt,
        DiscoveryState::Completed,
        workflow_run_id,
        None,
    )
    .await
}

pub async fn fail_discovery(
    pool: &PgPool,
    id: Uuid,
    claim_attempt: u32,
    workflow_run_id: Option<&str>,
    error: &str,
) -> anyhow::Result<DiscoveryTarget> {
    transition_claimed_target(
        pool,
        id,
        claim_attempt,
        DiscoveryState::Failed,
        workflow_run_id,
        Some(error),
    )
    .await
}

pub(crate) async fn complete_unclaimed_discovery(
    pool: &PgPool,
    id: Uuid,
    workflow_run_id: Option<&str>,
) -> anyhow::Result<Option<DiscoveryTarget>> {
    let row = sqlx::query(
        "UPDATE discovery_targets
         SET state = 'completed',
             workflow_run_id = CASE
               WHEN state = 'completed' THEN workflow_run_id
               ELSE COALESCE($2, workflow_run_id)
             END,
             last_error = CASE WHEN state = 'completed' THEN last_error ELSE NULL END,
             claim_expires_at = NULL,
             completed_at = COALESCE(completed_at, now()),
             updated_at = CASE WHEN state = 'completed' THEN updated_at ELSE now() END
         WHERE id = $1 AND state <> 'claimed'
         RETURNING id, kind, canonical_key, target_url, state, depth, priority,
                   confidence, relevance, novelty, claimed_by, workflow_run_id,
                   attempt_count, last_error, discovered_at, updated_at, claimed_at,
                   claim_expires_at, completed_at",
    )
    .bind(id)
    .bind(workflow_run_id)
    .fetch_optional(pool)
    .await?;
    row.map(target_from_row).transpose()
}

async fn transition_claimed_target(
    pool: &PgPool,
    id: Uuid,
    claim_attempt: u32,
    state: DiscoveryState,
    workflow_run_id: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<DiscoveryTarget> {
    let claim_attempt = i32::try_from(claim_attempt)?;
    let completed = state == DiscoveryState::Completed;
    let row = sqlx::query(
        "UPDATE discovery_targets
         SET state = $3,
             workflow_run_id = CASE
               WHEN state = 'claimed' THEN COALESCE($4, workflow_run_id)
               ELSE workflow_run_id
             END,
             last_error = CASE WHEN state = 'claimed' THEN $5 ELSE last_error END,
             claim_expires_at = CASE WHEN state = 'claimed' THEN NULL ELSE claim_expires_at END,
             completed_at = CASE
               WHEN state = 'claimed' AND $6 THEN now()
               ELSE completed_at
             END,
             updated_at = CASE WHEN state = 'claimed' THEN now() ELSE updated_at END
         WHERE id = $1
           AND attempt_count = $2
           AND (state = 'claimed' OR state = $3)
         RETURNING id, kind, canonical_key, target_url, state, depth, priority,
                   confidence, relevance, novelty, claimed_by, workflow_run_id,
                   attempt_count, last_error, discovered_at, updated_at, claimed_at,
                   claim_expires_at, completed_at",
    )
    .bind(id)
    .bind(claim_attempt)
    .bind(state.as_str())
    .bind(workflow_run_id)
    .bind(error)
    .bind(completed)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        anyhow::bail!(
            "discovery target {id} is not owned by claim attempt {claim_attempt} for transition to {}",
            state.as_str()
        );
    };
    target_from_row(row)
}

fn validate_request(request: &EnqueueDiscoveryRequest) -> anyhow::Result<()> {
    if request.canonical_key.trim().is_empty() {
        anyhow::bail!("canonical_key must not be empty");
    }
    if request.target_url.trim().is_empty() {
        anyhow::bail!("target_url must not be empty");
    }
    for (name, value) in [
        ("confidence", request.confidence),
        ("relevance", request.relevance),
        ("novelty", request.novelty),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            anyhow::bail!("{name} must be a finite value between 0 and 1");
        }
    }
    Ok(())
}

fn stable_target_id(kind: DiscoveryKind, canonical_key: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("youtube-corpus:discovery:{}:{canonical_key}", kind.as_str()).as_bytes(),
    )
}

fn target_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<DiscoveryTarget> {
    let kind: String = row.try_get("kind")?;
    let state: String = row.try_get("state")?;
    let depth: i32 = row.try_get("depth")?;
    let attempt_count: i32 = row.try_get("attempt_count")?;
    Ok(DiscoveryTarget {
        id: row.try_get("id")?,
        kind: DiscoveryKind::from_str(&kind)?,
        canonical_key: row.try_get("canonical_key")?,
        target_url: row.try_get("target_url")?,
        state: DiscoveryState::from_str(&state)?,
        depth: u32::try_from(depth)?,
        priority: row.try_get("priority")?,
        confidence: row.try_get("confidence")?,
        relevance: row.try_get("relevance")?,
        novelty: row.try_get("novelty")?,
        claimed_by: row.try_get("claimed_by")?,
        workflow_run_id: row.try_get("workflow_run_id")?,
        attempt_count: u32::try_from(attempt_count)?,
        last_error: row.try_get("last_error")?,
        discovered_at: row.try_get("discovered_at")?,
        updated_at: row.try_get("updated_at")?,
        claimed_at: row.try_get("claimed_at")?,
        claim_expires_at: row.try_get("claim_expires_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

fn evidence_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<DiscoveryEvidence> {
    let method: String = row.try_get("method")?;
    let depth: i32 = row.try_get("depth")?;
    Ok(DiscoveryEvidence {
        id: row.try_get("id")?,
        target_id: row.try_get("target_id")?,
        source_video_id: row.try_get("source_video_id")?,
        parent_target_id: row.try_get("parent_target_id")?,
        method: DiscoveryMethod::from_str(&method)?,
        evidence: row.try_get("evidence")?,
        confidence: row.try_get("confidence")?,
        relevance: row.try_get("relevance")?,
        novelty: row.try_get("novelty")?,
        depth: u32::try_from(depth)?,
        created_at: row.try_get::<DateTime<Utc>, _>("created_at")?,
    })
}
