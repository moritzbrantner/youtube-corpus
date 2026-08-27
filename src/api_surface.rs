use runtime_core::{
    surface_operation, surface_operation_with_execution_plan, OperationId, PackageSurface,
    RuntimeCapabilities, RuntimeRequirement, SurfaceExecutionMode, SurfaceExecutionPlan,
    SurfaceSideEffect,
};

/// Returns the public web API surface for transport-neutral clients and docs.
pub fn package_surface() -> PackageSurface {
    PackageSurface {
        library: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        capabilities: RuntimeCapabilities {
            native: true,
            server: true,
            wasm: false,
            mobile: runtime_core::MobileCapability::ApiOnly,
            requirements: vec![postgres_requirement()],
            max_recommended_input_bytes: Some(1_048_576),
        },
        operations: vec![
            surface_operation(
                "corpus.databaseStatus",
                "Database status",
                "Reports whether the server has a configured database URL.",
                serde_json::json!({}),
            ),
            surface_operation(
                "corpus.status",
                "Corpus status",
                "Reports database reachability, migration readiness, and corpus counts.",
                serde_json::json!({}),
            ),
            surface_operation(
                "corpus.search",
                "Search transcripts",
                "Searches transcript segments using Postgres FTS, pgvector semantic search, or a hybrid merge.",
                serde_json::json!({
                    "query": "attention mechanism",
                    "mode": "hybrid",
                    "topK": 10,
                    "sourceKind": "caption_manual",
                    "language": "en"
                }),
            ),
            surface_operation(
                "corpus.transcriptContext",
                "Transcript context",
                "Returns neighboring transcript segments around a matching segment.",
                serde_json::json!({
                    "segmentId": "00000000-0000-0000-0000-000000000000",
                    "before": 4,
                    "after": 6
                }),
            ),
            surface_operation(
                "corpus.downloadedFiles",
                "Downloaded files",
                "Lists videos and local download/parse status.",
                serde_json::json!({
                    "downloadedOnly": true,
                    "parsedOnly": false,
                    "limit": 20
                }),
            ),
            surface_operation_with_execution_plan(
                "corpus.addSource",
                "Add source",
                "Adds a video, playlist, or channel source and optionally starts ingest.",
                serde_json::json!({
                    "sourceKind": "video",
                    "sourceUrl": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                    "captionLanguages": ["en"],
                    "captionsEnabled": true,
                    "autoCaptionsEnabled": true,
                    "asrEnabled": false,
                    "ingestNow": true,
                    "async": true
                }),
                add_source_plan(),
            ),
            surface_operation(
                "corpus.ingestRuns.list",
                "List ingest runs",
                "Lists recent background ingest runs.",
                serde_json::json!({ "limit": 20 }),
            ),
            surface_operation(
                "corpus.ingestRuns.get",
                "Get ingest run",
                "Returns one background ingest run by id.",
                serde_json::json!({
                    "id": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.list",
                "List research corpora",
                "Lists named research corpora and their source/video counts.",
                serde_json::json!({}),
            ),
            surface_operation(
                "corpus.collections.create",
                "Create research corpus",
                "Creates a named corpus used to scope sources, videos, search, and reprocessing.",
                serde_json::json!({
                    "name": "Church history research",
                    "description": "Primary sources and lectures"
                }),
            ),
            surface_operation(
                "corpus.collections.sources.list",
                "List corpus sources",
                "Lists monitored channel and playlist sources attached to a named corpus.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.sources.add",
                "Add corpus source",
                "Adds a channel or playlist to a corpus for one-time ingestion or ongoing monitoring.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "sourceKind": "channel",
                    "sourceUrl": "https://www.youtube.com/@Distinguo/videos",
                    "monitor": true,
                    "ingestNow": true,
                    "captionLanguages": ["en"]
                }),
            ),
            surface_operation(
                "corpus.collections.sources.check",
                "Check corpus source",
                "Checks one monitored source for newly published videos and ingests eligible additions.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "sourceId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.sources.enabled",
                "Set corpus source enabled state",
                "Enables or disables future checks for a monitored source.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "sourceId": "00000000-0000-0000-0000-000000000000",
                    "enabled": true
                }),
            ),
            surface_operation(
                "corpus.collections.videos.list",
                "List corpus videos",
                "Lists videos attached directly or through monitored sources, including transcript provenance.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "limit": 100
                }),
            ),
            surface_operation(
                "corpus.collections.search",
                "Search a research corpus",
                "Searches transcript passages while returning only videos belonging to the selected corpus.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "query": "church history",
                    "mode": "hybrid",
                    "topK": 10
                }),
            ),
            surface_operation(
                "corpus.collections.searchPreferred",
                "Search preferred transcripts",
                "Searches the corpus while keeping only the selected highest-quality transcript stream for each video.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "query": "church history",
                    "mode": "hybrid",
                    "topK": 10
                }),
            ),
            surface_operation(
                "corpus.collections.transcriptQuality.list",
                "List transcript quality",
                "Lists deterministic stream quality measurements and the preferred stream for each corpus video.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.transcriptQuality.refresh",
                "Refresh transcript quality",
                "Recomputes transcript quality from source type, text presence, timestamp density, and duration coverage.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.annotations.list",
                "List research annotations",
                "Lists provenance-aware user or processor annotations scoped to a research corpus.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "kind": "claim",
                    "limit": 100
                }),
            ),
            surface_operation(
                "corpus.collections.annotations.create",
                "Create research annotation",
                "Creates an open-kind annotation anchored to a video, transcript stream, segment, or timestamp range.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "videoId": "00000000-0000-0000-0000-000000000000",
                    "segmentId": "00000000-0000-0000-0000-000000000000",
                    "kind": "claim",
                    "text": "Research note"
                }),
            ),
            surface_operation(
                "corpus.collections.evaluation.cases",
                "List retrieval evaluation cases",
                "Lists durable search queries and relevance judgments used for retrieval evaluation.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.evaluation.judgments.record",
                "Record retrieval relevance",
                "Records a relevant segment or timestamp target for a corpus query and retrieval mode.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "query": "church history",
                    "mode": "hybrid",
                    "topK": 10,
                    "segmentId": "00000000-0000-0000-0000-000000000000",
                    "relevance": 3
                }),
            ),
            surface_operation(
                "corpus.collections.evaluation.run",
                "Run retrieval evaluation",
                "Runs the recorded relevance set and persists Recall@K, MRR, NDCG@K, and mean latency.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000"
                }),
            ),
            surface_operation(
                "corpus.collections.videos.reprocess",
                "Reprocess corpus video",
                "Refreshes metadata, captions, ASR, local segmentation, embeddings, or the complete processing pipeline for one corpus video.",
                serde_json::json!({
                    "corpusId": "00000000-0000-0000-0000-000000000000",
                    "videoId": "00000000-0000-0000-0000-000000000000",
                    "stage": "captions",
                    "captionLanguages": ["en"]
                }),
            ),
        ],
    }
}

fn add_source_plan() -> SurfaceExecutionPlan {
    SurfaceExecutionPlan {
        operation: OperationId::new("corpus.addSource"),
        mode: SurfaceExecutionMode::BackgroundJob,
        side_effects: vec![
            SurfaceSideEffect::WritesFiles,
            SurfaceSideEffect::Network,
            SurfaceSideEffect::ExternalProcess,
        ],
        cancellable: false,
        progress_unit: Some("videos".to_string()),
        expected_artifacts: Vec::new(),
        requirements: vec![
            postgres_requirement(),
            RuntimeRequirement {
                name: "yt-dlp".to_string(),
                description: Some(
                    "Required for YouTube metadata, media, and captions.".to_string(),
                ),
                required: true,
            },
        ],
        max_recommended_input_bytes: Some(1_048_576),
    }
}

fn postgres_requirement() -> RuntimeRequirement {
    RuntimeRequirement {
        name: "postgres".to_string(),
        description: Some("Required for corpus storage and search endpoints.".to_string()),
        required: true,
    }
}

pub fn typescript_declarations() -> &'static str {
    r#"export type SearchMode = "hybrid" | "fts" | "semantic";
export type SourceKind = "caption_manual" | "caption_auto" | "asr";
export type AddSourceKind = "video" | "channel" | "playlist";
export type JobStatus = "queued" | "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type ReprocessStage = "metadata" | "captions" | "asr" | "segments" | "embeddings" | "all";
export type AnnotationSourceKind = "user" | "processor";

export type IngestItemReport = {
  videoId: string | null;
  sourceUrl: string;
  title: string | null;
  status: string;
  streamsIndexed: number;
  segmentsIndexed: number;
  message: string | null;
};

export type IngestReport = {
  workflow: string;
  runId: string;
  videosSeen: number;
  videosIndexed: number;
  segmentsIndexed: number;
  items: IngestItemReport[];
};

export type JobProgress = {
  completed: number;
  total: number | null;
  unit: string;
  message: string | null;
};

export type JobFailure = {
  message: string;
};

export type IngestJob = {
  id: string;
  status: JobStatus;
  progress: JobProgress | null;
  failure: JobFailure | null;
  ingest: IngestReport | null;
  sourceUrl: string | null;
  createdAt: string;
};

export type IngestRunStatus = {
  id: string;
  sourceUrl: string | null;
  status: string;
  videosSeen: number;
  videosIndexed: number;
  segmentsIndexed: number;
  report: Record<string, unknown>;
  createdAt: string;
  job: IngestJob | null;
};

export type Corpus = {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  isDefault: boolean;
  videoCount: number;
  sourceCount: number;
  createdAt: string;
  updatedAt: string;
};

export type CorpusSource = {
  id: string;
  sourceKind: "channel" | "playlist";
  sourceUrl: string;
  name: string | null;
  enabled: boolean;
  lastCheckedAt: string | null;
  lastIngestedAt: string | null;
  lastCheckStatus: string | null;
  lastCheckMessage: string | null;
  itemsSeen: number;
  itemsIndexed: number;
  itemsFailed: number;
};

export type ReprocessReport = {
  videoId: string;
  stage: ReprocessStage;
  ingest: IngestReport | null;
  segmentsResegmented: number;
  segmentsReembedded: number;
  metadataProcessingRevision: number;
  streamProcessingRevision: number | null;
};

export type TranscriptQuality = {
  streamId: string;
  videoId: string;
  sourceKind: SourceKind;
  language: string | null;
  score: number;
  sourcePriority: number;
  coverageRatio: number;
  timedSegmentRatio: number;
  textCharacters: number;
  segmentCount: number;
  reasons: Record<string, unknown>;
  isPreferred: boolean;
  evaluatedAt: string;
};

export type ResearchAnnotation = {
  id: string;
  corpusId: string;
  videoId: string;
  streamId: string | null;
  segmentId: string | null;
  kind: string;
  startSeconds: number | null;
  endSeconds: number | null;
  label: string | null;
  text: string | null;
  payload: Record<string, unknown>;
  sourceKind: AnnotationSourceKind;
  processor: string | null;
  processorVersion: string | null;
  processingConfig: Record<string, unknown>;
  revision: number;
  contentChecksum: string;
  createdAt: string;
  updatedAt: string;
};

export type EvaluationTarget = {
  id: string;
  videoId: string | null;
  segmentId: string | null;
  sourceUrl: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
  relevance: number;
  notes: string | null;
};

export type EvaluationCase = {
  id: string;
  corpusId: string;
  query: string;
  mode: SearchMode;
  topK: number;
  notes: string | null;
  targets: EvaluationTarget[];
  createdAt: string;
  updatedAt: string;
};

export type EvaluationRunReport = {
  id: string;
  corpusId: string;
  casesCount: number;
  recallAtK: number;
  meanReciprocalRank: number;
  ndcgAtK: number;
  meanLatencyMs: number;
  createdAt: string;
};
"#
}
