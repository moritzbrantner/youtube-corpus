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
pub mod evaluation;
pub mod ingest;
pub mod media_evidence;
pub mod multimodal;
pub mod provenance;
pub mod reprocessing;
pub mod research_quality_web;
pub mod research_web;
pub mod search;
pub mod source_span_export;
pub mod sponsorblock;
pub mod status;
pub mod subscriptions;
pub mod transcript_quality;
pub mod visual_timeline;
pub mod video_analysis_web;
pub mod web;
pub mod youtube;
pub mod yt_dlp;
pub mod yt_dlp_bridge_web;

pub use annotations::{
    create_annotation, list_annotations, AnnotationSourceKind, CreateAnnotationRequest,
    ResearchAnnotation,
};
pub use config::{
    AppConfig, BrowserCookieSource, CaptionConfig, CorpusSource, SearchMode, SourceKind,
    YtDlpConfig,
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
pub use source_span_export::{
    export_source_span_batch, SourceLocatorV1, SourceProducerV1, SourceRecordV1, SourceSpanBatchV1,
    SourceSpanRecordV1,
};
pub use status::{corpus_status, list_videos, CorpusStatusReport, VideoStatus};
pub use subscriptions::{
    add_subscription, check_subscriptions, list_subscriptions, CheckSubscriptionsReport,
    Subscription,
};
pub use transcript_quality::{
    list_corpus_quality, list_video_quality, refresh_corpus_quality, refresh_video_quality,
    TranscriptQualitySummary,
};

pub use visual_timeline::{
    begin_visual_processing_run, list_video_scenes, list_visual_text_tracks, upsert_video_scene,
    upsert_visual_text_observation, upsert_visual_text_track, BeginVisualProcessingRunRequest,
    UpsertVideoSceneRequest, UpsertVisualTextObservationRequest, UpsertVisualTextTrackRequest,
    VideoScene, VisualBoundingBox, VisualProcessingKind, VisualTextObservation, VisualTextRole,
    VisualTextTrack,
};

pub use media_evidence::{
    export_media_evidence_batch, BoundingBoxV1, EvidenceProducerV1, EvidenceVideoV1,
    MediaEvidenceBatchV1, OcrObservationEvidenceV1, OcrTrackEvidenceV1, ProcessingEvidenceV1,
    SceneEvidenceV1, SponsorBlockEvidenceV1, SponsorBlockSegmentEvidenceV1,
    MEDIA_EVIDENCE_SCHEMA, MEDIA_EVIDENCE_VERSION_V1,
};
pub use sponsorblock::{
    fetch_sponsorblock_snapshot, latest_sponsorblock_snapshot, parse_hash_response,
    persist_sponsorblock_snapshot, refresh_sponsorblock_for_video, SponsorBlockSegment,
    SponsorBlockSnapshot, DEFAULT_SPONSORBLOCK_CATEGORIES, SPONSORBLOCK_API_BASE,
    SPONSORBLOCK_ATTRIBUTION, SPONSORBLOCK_DATA_LICENSE,
};
