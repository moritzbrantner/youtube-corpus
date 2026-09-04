pub mod annotations;
pub mod api_surface;
pub mod api_types;
pub mod asr;
pub mod benchmark;
pub mod captions;
pub mod cli;
pub mod config;
pub mod corpora;
pub mod db;
pub mod diagnostics;
pub mod discovery;
pub mod evaluation;
pub mod ingest;
pub mod multimodal;
pub mod provenance;
pub mod reprocessing;
pub mod research_quality_web;
pub mod research_web;
pub mod search;
pub mod status;
pub mod subscriptions;
pub mod transcript_quality;
pub mod web;
pub mod youtube;
pub mod yt_dlp;

pub use annotations::{
    create_annotation, list_annotations, AnnotationSourceKind, CreateAnnotationRequest,
    ResearchAnnotation,
};
pub use config::{
    AppConfig, BrowserCookieSource, CaptionConfig, CorpusSource, SearchMode, SourceKind,
    YtDlpConfig,
};
pub use discovery::{
    canonicalize_youtube_target, claim_next_discovery, claim_next_discovery_with_lease,
    complete_discovery, discovery_schema_available, enqueue_discovery, extract_youtube_targets,
    fail_discovery, list_discovery_evidence, list_frontier, record_ingest_discoveries,
    workflow_handoff, DiscoveredYouTubeTarget, DiscoveryEvidence, DiscoveryKind, DiscoveryMethod,
    DiscoveryPolicy, DiscoveryState, DiscoveryTarget, DiscoveryWorkflowInput,
    EnqueueDiscoveryRequest, WorkflowRunHandoff, DEFAULT_CLAIM_LEASE_SECONDS,
    DISCOVERY_WORKFLOW_ID,
};
pub use evaluation::{
    list_cases as list_evaluation_cases, record_judgment, run_evaluation, EvaluationCase,
    EvaluationRunReport, EvaluationTarget, RecordJudgmentRequest,
};
pub use ingest::{ingest_corpus, IngestReport};
pub use multimodal::{
    begin_processing_run, link_face_observation_to_track, link_voice_observation_to_track,
    list_face_observations, list_voice_observations, upsert_face_observation, upsert_face_track,
    upsert_voice_observation, upsert_voice_track, BeginProcessingRunRequest, BoundingBox,
    FaceObservation, FaceTrack, MediaModality, MediaProcessingRun, ProcessingProvenance,
    UpsertFaceObservationRequest, UpsertFaceTrackRequest, UpsertVoiceObservationRequest,
    UpsertVoiceTrackRequest, VoiceObservation, VoiceTrack,
};
pub use reprocessing::{reprocess_video, ReprocessReport, ReprocessRequest, ReprocessStage};
pub use search::{search_corpus, SearchReport, SearchRequest};
pub use status::{corpus_status, list_videos, CorpusStatusReport, VideoStatus};
pub use subscriptions::{
    add_subscription, check_subscriptions, list_subscriptions, CheckSubscriptionsReport,
    Subscription,
};
pub use transcript_quality::{
    list_corpus_quality, list_video_quality, refresh_corpus_quality, refresh_video_quality,
    TranscriptQualitySummary,
};
