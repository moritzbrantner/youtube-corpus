mod ingest;
mod store;
mod types;
mod youtube;

pub use ingest::{record_ingest_discoveries, workflow_handoff};
pub use store::{
    claim_next_discovery, claim_next_discovery_with_lease, complete_discovery,
    discovery_schema_available, enqueue_discovery, fail_discovery, list_discovery_evidence,
    list_frontier,
};
pub use types::{
    DiscoveredYouTubeTarget, DiscoveryEvidence, DiscoveryKind, DiscoveryMethod, DiscoveryPolicy,
    DiscoveryState, DiscoveryTarget, DiscoveryWorkflowInput, EnqueueDiscoveryRequest,
    WorkflowRunHandoff, DEFAULT_CLAIM_LEASE_SECONDS, DISCOVERY_WORKFLOW_ID,
};
pub use youtube::{canonicalize_youtube_target, extract_youtube_targets};
