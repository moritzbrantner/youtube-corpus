use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const DISCOVERY_WORKFLOW_ID: &str = "youtube-corpus.discovery.process";
pub const DEFAULT_CLAIM_LEASE_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryKind {
    Video,
    Channel,
    Playlist,
}

impl DiscoveryKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Channel => "channel",
            Self::Playlist => "playlist",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "video" => Ok(Self::Video),
            "channel" => Ok(Self::Channel),
            "playlist" => Ok(Self::Playlist),
            _ => anyhow::bail!("unknown discovery kind: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryState {
    Pending,
    Claimed,
    Completed,
    Deferred,
    Failed,
}

impl DiscoveryState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Completed => "completed",
            Self::Deferred => "deferred",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "completed" => Ok(Self::Completed),
            "deferred" => Ok(Self::Deferred),
            "failed" => Ok(Self::Failed),
            _ => anyhow::bail!("unknown discovery state: {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMethod {
    Seed,
    SourceExpansion,
    ChannelMetadata,
    DescriptionLink,
    TranscriptLink,
    Manual,
}

impl DiscoveryMethod {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::SourceExpansion => "source_expansion",
            Self::ChannelMetadata => "channel_metadata",
            Self::DescriptionLink => "description_link",
            Self::TranscriptLink => "transcript_link",
            Self::Manual => "manual",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "seed" => Ok(Self::Seed),
            "source_expansion" => Ok(Self::SourceExpansion),
            "channel_metadata" => Ok(Self::ChannelMetadata),
            "description_link" => Ok(Self::DescriptionLink),
            "transcript_link" => Ok(Self::TranscriptLink),
            "manual" => Ok(Self::Manual),
            _ => anyhow::bail!("unknown discovery method: {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPolicy {
    pub max_depth: u32,
    pub min_confidence: f64,
    pub min_relevance: f64,
    pub min_priority: f64,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self {
            max_depth: 3,
            min_confidence: 0.5,
            min_relevance: 0.0,
            min_priority: 0.35,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredYouTubeTarget {
    pub kind: DiscoveryKind,
    pub canonical_key: String,
    pub target_url: String,
}

#[derive(Debug, Clone)]
pub struct EnqueueDiscoveryRequest {
    pub kind: DiscoveryKind,
    pub canonical_key: String,
    pub target_url: String,
    pub source_video_id: Option<Uuid>,
    pub parent_target_id: Option<Uuid>,
    pub method: DiscoveryMethod,
    pub evidence: Value,
    pub depth: u32,
    pub confidence: f64,
    pub relevance: f64,
    pub novelty: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryTarget {
    pub id: Uuid,
    pub kind: DiscoveryKind,
    pub canonical_key: String,
    pub target_url: String,
    pub state: DiscoveryState,
    pub depth: u32,
    pub priority: f64,
    pub confidence: f64,
    pub relevance: f64,
    pub novelty: f64,
    pub claimed_by: Option<String>,
    pub workflow_run_id: Option<String>,
    pub attempt_count: u32,
    pub last_error: Option<String>,
    pub discovered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub claim_expires_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryEvidence {
    pub id: Uuid,
    pub target_id: Uuid,
    pub source_video_id: Option<Uuid>,
    pub parent_target_id: Option<Uuid>,
    pub method: DiscoveryMethod,
    pub evidence: Value,
    pub confidence: f64,
    pub relevance: f64,
    pub novelty: f64,
    pub depth: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryWorkflowInput {
    pub discovery_id: Uuid,
    pub source_kind: DiscoveryKind,
    pub source_url: String,
    pub depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AdmissionDecision {
    pub state: DiscoveryState,
    pub priority: f64,
}

pub(crate) fn admission_decision(
    request: &EnqueueDiscoveryRequest,
    policy: &DiscoveryPolicy,
) -> AdmissionDecision {
    let depth_penalty = (f64::from(request.depth) * 0.05).min(0.25);
    let priority = (request.confidence * 0.30 + request.relevance * 0.50 + request.novelty * 0.20
        - depth_penalty)
        .clamp(0.0, 1.0);
    let admitted = request.depth <= policy.max_depth
        && request.confidence >= policy.min_confidence
        && request.relevance >= policy.min_relevance
        && priority >= policy.min_priority;
    AdmissionDecision {
        state: if admitted {
            DiscoveryState::Pending
        } else {
            DiscoveryState::Deferred
        },
        priority,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        admission_decision, DiscoveryKind, DiscoveryMethod, DiscoveryPolicy, DiscoveryState,
        EnqueueDiscoveryRequest,
    };

    #[test]
    fn admission_policy_bounds_recursive_growth() {
        let mut request = EnqueueDiscoveryRequest {
            kind: DiscoveryKind::Video,
            canonical_key: "youtube:video:child".to_string(),
            target_url: "https://www.youtube.com/watch?v=child".to_string(),
            source_video_id: None,
            parent_target_id: None,
            method: DiscoveryMethod::Manual,
            evidence: json!({}),
            depth: 1,
            confidence: 0.9,
            relevance: 0.8,
            novelty: 0.8,
        };
        let policy = DiscoveryPolicy::default();
        let admitted = admission_decision(&request, &policy);
        assert_eq!(admitted.state, DiscoveryState::Pending);
        assert!(admitted.priority > policy.min_priority);

        request.depth = policy.max_depth + 1;
        assert_eq!(
            admission_decision(&request, &policy).state,
            DiscoveryState::Deferred
        );
    }
}
