use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::CorpusSource;
use crate::ingest::IngestReport;

use super::store::{
    complete_discovery, discovery_schema_available, enqueue_discovery, fail_discovery,
    find_discovery_target,
};
use super::types::{
    DiscoveredYouTubeTarget, DiscoveryMethod, DiscoveryPolicy, DiscoveryState, DiscoveryTarget,
    DiscoveryWorkflowInput, EnqueueDiscoveryRequest,
};
use super::youtube::{canonicalize_youtube_target, extract_youtube_targets};

pub fn workflow_input(target: &DiscoveryTarget) -> DiscoveryWorkflowInput {
    DiscoveryWorkflowInput {
        discovery_id: target.id,
        source_kind: target.kind,
        source_url: target.target_url.clone(),
        depth: target.depth,
    }
}

pub async fn record_ingest_discoveries(
    pool: &PgPool,
    source: &CorpusSource,
    report: &IngestReport,
    policy: &DiscoveryPolicy,
) -> anyhow::Result<()> {
    if !discovery_schema_available(pool).await? {
        return Ok(());
    }
    let Some(seed_candidate) = seed_target(source) else {
        return Ok(());
    };
    let seed = match find_discovery_target(
        pool,
        seed_candidate.kind,
        &seed_candidate.canonical_key,
    )
    .await?
    {
        Some(existing) if existing.state == DiscoveryState::Claimed => existing,
        _ => {
            enqueue_discovery(
                pool,
                EnqueueDiscoveryRequest {
                    kind: seed_candidate.kind,
                    canonical_key: seed_candidate.canonical_key,
                    target_url: seed_candidate.target_url,
                    source_video_id: None,
                    parent_target_id: None,
                    method: DiscoveryMethod::Seed,
                    evidence: json!({"ingestRunId": report.run_id}),
                    depth: 0,
                    confidence: 1.0,
                    relevance: 1.0,
                    novelty: 1.0,
                },
                policy,
            )
            .await?
        }
    };

    let ingest_run_id = report.run_id.to_string();
    if report.items.iter().all(|item| item.status == "failed") {
        if seed.state == DiscoveryState::Completed {
            return Ok(());
        }
        fail_discovery(
            pool,
            seed.id,
            Some(&ingest_run_id),
            "ingest failed for every resolved item",
        )
        .await?;
        return Ok(());
    }
    complete_discovery(pool, seed.id, Some(&ingest_run_id)).await?;

    for item in &report.items {
        let Some(video_id) = item.video_id else {
            continue;
        };
        let current = match canonicalize_youtube_target(&item.source_url) {
            Some(target) if target.canonical_key != seed.canonical_key => {
                let target = enqueue_discovery(
                    pool,
                    EnqueueDiscoveryRequest {
                        kind: target.kind,
                        canonical_key: target.canonical_key,
                        target_url: target.target_url,
                        source_video_id: Some(video_id),
                        parent_target_id: Some(seed.id),
                        method: DiscoveryMethod::SourceExpansion,
                        evidence: json!({"ingestRunId": report.run_id}),
                        depth: seed.depth.saturating_add(1),
                        confidence: 1.0,
                        relevance: 1.0,
                        novelty: 1.0,
                    },
                    policy,
                )
                .await?;
                complete_discovery(pool, target.id, Some(&ingest_run_id)).await?;
                target
            }
            _ => seed.clone(),
        };
        discover_from_video(pool, video_id, &current, policy).await?;
    }
    Ok(())
}

async fn discover_from_video(
    pool: &PgPool,
    video_id: Uuid,
    parent: &DiscoveryTarget,
    policy: &DiscoveryPolicy,
) -> anyhow::Result<()> {
    let row = sqlx::query("SELECT description, channel_url FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let next_depth = parent.depth.saturating_add(1);

    let channel_url: Option<String> = row.try_get("channel_url")?;
    if let Some(channel) = channel_url
        .as_deref()
        .and_then(canonicalize_youtube_target)
        .filter(|target| target.canonical_key != parent.canonical_key)
    {
        enqueue_discovery(
            pool,
            discovery_request(
                channel,
                video_id,
                parent,
                DiscoveryMethod::ChannelMetadata,
                json!({"field": "channel_url"}),
                next_depth,
                (1.0, 0.8, 0.7),
            ),
            policy,
        )
        .await?;
    }

    let description: Option<String> = row.try_get("description")?;
    if let Some(description) = description {
        for target in extract_youtube_targets(&description)
            .into_iter()
            .filter(|target| target.canonical_key != parent.canonical_key)
        {
            enqueue_discovery(
                pool,
                discovery_request(
                    target,
                    video_id,
                    parent,
                    DiscoveryMethod::DescriptionLink,
                    json!({"field": "description"}),
                    next_depth,
                    (0.9, 0.75, 0.8),
                ),
                policy,
            )
            .await?;
        }
    }

    let streams = sqlx::query(
        "SELECT id, full_text FROM transcript_streams WHERE video_id = $1 AND full_text IS NOT NULL",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    for stream in streams {
        let stream_id: Uuid = stream.try_get("id")?;
        let full_text: String = stream.try_get("full_text")?;
        for target in extract_youtube_targets(&full_text)
            .into_iter()
            .filter(|target| target.canonical_key != parent.canonical_key)
        {
            enqueue_discovery(
                pool,
                discovery_request(
                    target,
                    video_id,
                    parent,
                    DiscoveryMethod::TranscriptLink,
                    json!({"transcriptStreamId": stream_id}),
                    next_depth,
                    (0.8, 0.65, 0.8),
                ),
                policy,
            )
            .await?;
        }
    }
    Ok(())
}

fn discovery_request(
    target: DiscoveredYouTubeTarget,
    video_id: Uuid,
    parent: &DiscoveryTarget,
    method: DiscoveryMethod,
    evidence: serde_json::Value,
    depth: u32,
    scores: (f64, f64, f64),
) -> EnqueueDiscoveryRequest {
    EnqueueDiscoveryRequest {
        kind: target.kind,
        canonical_key: target.canonical_key,
        target_url: target.target_url,
        source_video_id: Some(video_id),
        parent_target_id: Some(parent.id),
        method,
        evidence,
        depth,
        confidence: scores.0,
        relevance: scores.1,
        novelty: scores.2,
    }
}

fn seed_target(source: &CorpusSource) -> Option<DiscoveredYouTubeTarget> {
    match source {
        CorpusSource::YoutubeUrl { url }
        | CorpusSource::ChannelUrl { url }
        | CorpusSource::PlaylistUrl { url } => canonicalize_youtube_target(url),
        CorpusSource::LocalFile { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::to_value;
    use uuid::Uuid;

    use super::workflow_input;
    use crate::discovery::{DiscoveryKind, DiscoveryState, DiscoveryTarget};

    #[test]
    fn workflow_input_matches_claim_executor_contract() {
        let now = Utc::now();
        let target = DiscoveryTarget {
            id: Uuid::nil(),
            kind: DiscoveryKind::Video,
            canonical_key: "youtube:video:abc".to_string(),
            target_url: "https://www.youtube.com/watch?v=abc".to_string(),
            state: DiscoveryState::Claimed,
            depth: 2,
            priority: 0.8,
            confidence: 0.9,
            relevance: 0.8,
            novelty: 0.7,
            claimed_by: Some("workflow-runner".to_string()),
            workflow_run_id: None,
            attempt_count: 1,
            last_error: None,
            discovered_at: now,
            updated_at: now,
            claimed_at: Some(now),
            claim_expires_at: Some(now),
            completed_at: None,
        };
        let input = workflow_input(&target);
        let value = to_value(input).unwrap();
        assert_eq!(value["sourceKind"], "video");
        assert_eq!(value["sourceUrl"], target.target_url);
        assert_eq!(value["discoveryId"], target.id.to_string());
        assert_eq!(value["depth"], 2);
    }
}
