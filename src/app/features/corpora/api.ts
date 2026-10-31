import {
  apiFetch,
  type IngestReport,
  type SearchMode,
  type SearchReport,
  type SearchResult,
  type SearchTranscriptsInput,
  type SourceKind,
  type TranscriptContextReport,
  getTranscriptContext,
} from "../../api";

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
  /** Automatic check interval while the source is enabled (Atlas 1.18). */
  checkIntervalSeconds: number;
  /** When the running server checks this source next; overdue while in the past. */
  nextCheckAt: string | null;
  /** A check of this source is running. */
  checkInProgress: boolean;
  itemsSeen: number;
  itemsIndexed: number;
  itemsFailed: number;
  processingPolicy: ProcessingPolicy;
  itemsAwaitingReview: number;
  /** User labels assigned to this tracked source. */
  labels: CorpusLabel[];
  /** Background work of this source: newly published uploads vs history (Atlas 1.16). */
  queue: SourceQueueActivity;
  /** Most recent initial-history backfill (Atlas 1.10, 1.14). */
  backfill: BackfillProgress | null;
};

export type SourceQueueActivity = {
  newQueued: number;
  newRunning: number;
  historyQueued: number;
  historyRunning: number;
};

/** Which already-published videos to process when a source is added (Atlas 1.10, 1.12). */
export type HistoryScopeKind = "none" | "recent" | "limited" | "full";

export type HistoryScopeInput = {
  scope: HistoryScopeKind;
  recentDays?: number | null;
  limitCount?: number | null;
};

export type BackfillState =
  | "discovering"
  | "running"
  | "paused"
  | "canceled"
  | "awaiting_review"
  | "completed"
  | "partial"
  | "failed";

export type BackfillProgress = {
  id: string;
  subscriptionId: string;
  corpusId: string | null;
  scope: HistoryScopeKind;
  recentDays: number | null;
  limitCount: number | null;
  state: BackfillState;
  /** Videos listed by the source; null until the listing finished. */
  discoveredCount: number | null;
  reportedCount: number | null;
  discoveryMessage: string | null;
  selected: number;
  notSelected: { reason: string; count: number }[];
  queued: number;
  running: number;
  indexed: number;
  skipped: number;
  failed: number;
  awaitingReview: number;
  filtered: number;
  canceled: number;
  createdAt: string;
  discoveredAt: string | null;
  finishedAt: string | null;
};

export type BackfillEntry = {
  position: number;
  sourceUrl: string;
  youtubeId: string | null;
  title: string | null;
  uploadDate: string | null;
  selected: boolean;
  reason: string | null;
  status: string | null;
};

export type BackfillAction = "pause" | "resume" | "cancel";

export function controlBackfill(
  corpusId: string,
  sourceId: string,
  backfillId: string,
  action: BackfillAction,
) {
  return jsonPost<BackfillProgress>(
    `/api/corpora/${corpusId}/sources/${sourceId}/backfills/${backfillId}/${action}`,
    {},
  );
}

export function listBackfillEntries(corpusId: string, sourceId: string, backfillId: string) {
  return apiFetch<BackfillEntry[]>(
    `/api/corpora/${corpusId}/sources/${sourceId}/backfills/${backfillId}/entries`,
  );
}

/** A user-defined label owned by one corpus; distinct from YouTube categories/tags. */
export type CorpusLabel = {
  id: string;
  name: string;
};

export type CorpusVideoSource = {
  id: string;
  sourceKind: "channel" | "playlist";
  name: string | null;
};

/** A YouTube channel recognized from corpus video metadata (its YouTube channel id). */
export type CorpusChannel = {
  channelId: string;
  name: string | null;
  channelUrl: string | null;
  videoCount: number;
  latestUploadDate: string | null;
  /** Tracked sources of this corpus that discovered the channel's videos; empty = untracked. */
  sources: CorpusVideoSource[];
};

export type CorpusChannelDirectory = {
  channels: CorpusChannel[];
  /** Corpus videos whose metadata names no channel yet. */
  videosWithoutChannel: number;
};

export type CorpusVideo = {
  id: string;
  youtubeId: string | null;
  sourceUrl: string;
  title: string | null;
  channel: string | null;
  /** YouTube channel id; null while the channel metadata is unknown. */
  channelId: string | null;
  channelUrl: string | null;
  thumbnailUrl: string | null;
  durationSeconds: number | null;
  uploadDate: string | null;
  transcriptStreams: number;
  transcriptSegments: number;
  transcriptSourceKinds: string[];
  processingRevision: number | null;
  lastRetrievedAt: string | null;
  updatedAt: string;
  /** User labels assigned directly to the video. */
  labels: CorpusLabel[];
  /** Tracked sources of this corpus that discovered the video. */
  sources: CorpusVideoSource[];
  addedDirectly: boolean;
  youtubeCategories: string[];
  youtubeTags: string[];
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

export type AnnotationSourceKind = "user" | "processor";

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

export type CreateAnnotationInput = {
  videoId: string;
  streamId?: string | null;
  segmentId?: string | null;
  kind: string;
  startSeconds?: number | null;
  endSeconds?: number | null;
  label?: string | null;
  text?: string | null;
  payload?: Record<string, unknown>;
  sourceKind?: AnnotationSourceKind;
  processor?: string | null;
  processorVersion?: string | null;
  processingConfig?: Record<string, unknown>;
};

export type EvaluationTarget = {
  id: string;
  videoId: string | null;
  segmentId: string | null;
  visualTextTrackId?: string | null;
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

export type RecordEvaluationJudgmentInput = {
  query: string;
  mode: SearchMode;
  topK: number;
  videoId?: string | null;
  segmentId?: string | null;
  visualTextTrackId?: string | null;
  sourceUrl?: string | null;
  startSeconds?: number | null;
  endSeconds?: number | null;
  relevance?: number;
  notes?: string | null;
};

export type EvaluationCaseReport = {
  caseId: string;
  query: string;
  topK: number;
  relevantTargets: number;
  hits: number;
  recallAtK: number;
  reciprocalRank: number;
  ndcgAtK: number;
  latencyMs: number;
  returnedSegmentIds: string[];
};

export type EvaluationRunReport = {
  id: string;
  corpusId: string;
  casesCount: number;
  recallAtK: number;
  meanReciprocalRank: number;
  ndcgAtK: number;
  meanLatencyMs: number;
  cases: EvaluationCaseReport[];
  createdAt: string;
};

export type CreateCorpusInput = {
  name: string;
  slug?: string | null;
  description?: string | null;
};

export type AddCorpusSourceInput = {
  sourceKind: "channel" | "playlist";
  sourceUrl: string;
  name?: string | null;
  monitor: boolean;
  ingestNow: boolean;
  captionLanguages?: string[];
  autoCaptionsEnabled?: boolean;
  asrEnabled?: boolean;
  transcriberCommand?: string | null;
  transcriberArgs?: string[];
  transcriberTimeoutSeconds?: number | null;
  maxItems?: number | null;
  processingPolicy?: ProcessingPolicy;
  /** Initial history of a monitored source; the server applies full history when omitted. */
  history?: HistoryScopeInput;
  /** Automatic check interval of a monitored source; the server default when omitted. */
  checkIntervalSeconds?: number | null;
};

/** Whether newly discovered videos are analyzed automatically or held for review (Atlas 1.6). */
export type ProcessingPolicy = "automatic" | "review_first";

export type PendingItem = {
  sourceUrl: string;
  youtubeId: string | null;
  title: string | null;
  channel: string | null;
  durationSeconds: number | null;
  uploadDate: string | null;
  firstSeenAt: string;
};

export function setSourcePolicy(corpusId: string, sourceId: string, policy: ProcessingPolicy) {
  return apiFetch<void>(`/api/corpora/${corpusId}/sources/${sourceId}/policy`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ policy }),
  });
}

export function listSourcePending(corpusId: string, sourceId: string) {
  return apiFetch<PendingItem[]>(`/api/corpora/${corpusId}/sources/${sourceId}/pending`);
}

export function approveSourcePending(corpusId: string, sourceId: string, sourceUrls: string[]) {
  return apiFetch<IngestReport>(`/api/corpora/${corpusId}/sources/${sourceId}/pending/approve`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sourceUrls }),
  });
}

export type SubscriptionCheckItem = {
  subscription: {
    id: string;
    sourceKind: "channel" | "playlist";
    sourceUrl: string;
    name: string | null;
  };
  status: string;
  videosDiscovered: number;
  newVideos: number;
  videosIndexed: number;
  segmentsIndexed: number;
  awaitingReview?: number;
  /** Newly queued for background processing by this check. */
  queued?: number;
  ingest: IngestReport | null;
  message: string | null;
};

export type SubscriptionCheckReport = {
  checkedAt: string;
  subscriptionsChecked: number;
  videosDiscovered: number;
  newVideos: number;
  videosIndexed: number;
  segmentsIndexed: number;
  items: SubscriptionCheckItem[];
};

export type AddCorpusSourceReport = {
  source: CorpusSource | null;
  ingest: IngestReport | null;
  check: SubscriptionCheckReport | null;
  backfill: BackfillProgress | null;
};

export type ReprocessStage = "metadata" | "captions" | "asr" | "segments" | "embeddings" | "all";

export type ReprocessReport = {
  videoId: string;
  stage: ReprocessStage;
  ingest: IngestReport | null;
  segmentsResegmented: number;
  segmentsReembedded: number;
  metadataProcessingRevision: number;
  streamProcessingRevision: number | null;
};

export type ReprocessVideoInput = {
  stage: ReprocessStage;
  captionLanguages?: string[];
  transcriberCommand?: string | null;
  transcriberArgs?: string[];
  transcriberTimeoutSeconds?: number | null;
};

function jsonPost<T>(path: string, body: unknown) {
  return apiFetch<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function listCorpora() {
  return apiFetch<Corpus[]>("/api/corpora");
}

export function createCorpus(input: CreateCorpusInput) {
  return jsonPost<Corpus>("/api/corpora", input);
}

export function listCorpusSources(corpusId: string) {
  return apiFetch<CorpusSource[]>(`/api/corpora/${corpusId}/sources`);
}

export function addCorpusSource(corpusId: string, input: AddCorpusSourceInput) {
  return jsonPost<AddCorpusSourceReport>(`/api/corpora/${corpusId}/sources`, input);
}

export function checkCorpusSource(corpusId: string, sourceId: string) {
  return jsonPost<SubscriptionCheckReport>(
    `/api/corpora/${corpusId}/sources/${sourceId}/check`,
    {},
  );
}

export function setCorpusSourceInterval(
  corpusId: string,
  sourceId: string,
  intervalSeconds: number,
) {
  return jsonPost<void>(`/api/corpora/${corpusId}/sources/${sourceId}/interval`, {
    intervalSeconds,
  });
}

export function setCorpusSourceEnabled(corpusId: string, sourceId: string, enabled: boolean) {
  return jsonPost<void>(`/api/corpora/${corpusId}/sources/${sourceId}/enabled`, { enabled });
}

export type CorpusVideoFilter = {
  labelId?: string | null;
  sourceId?: string | null;
  channelId?: string | null;
};

export function listCorpusVideos(corpusId: string, limit = 100, filter: CorpusVideoFilter = {}) {
  const query = new URLSearchParams({ limit: String(limit) });
  for (const key of ["labelId", "sourceId", "channelId"] as const) {
    const value = filter[key];
    if (value) {
      query.set(key, value);
    }
  }
  return apiFetch<CorpusVideo[]>(`/api/corpora/${corpusId}/videos?${query}`);
}

/** One retained transcript stream of a video with all its timed segments. */
export type TranscriptStreamEvidence = {
  streamId: string;
  sourceKind: SourceKind;
  language: string | null;
  status: string;
  isPreferred: boolean;
  processor: string;
  processorVersion: string;
  retrievedAt: string;
  processingRevision: number;
  segments: {
    segmentId: string;
    segmentIndex: number;
    startSeconds: number | null;
    endSeconds: number | null;
    text: string;
  }[];
};

/** A video's own page: catalog metadata plus separate speech and on-screen text evidence. */
export type VideoInspection = {
  video: CorpusVideo;
  description: string | null;
  transcriptStreams: TranscriptStreamEvidence[];
  ocrStatus: "unprocessed" | "attempted_without_tracks" | "indexed";
  /** OCR tracks in time order with producer/run/observation provenance. */
  visualText: SearchResult[];
};

export function getVideoInspection(corpusId: string, videoId: string) {
  return apiFetch<VideoInspection>(`/api/corpora/${corpusId}/videos/${videoId}`);
}

export function listCorpusChannels(corpusId: string) {
  return apiFetch<CorpusChannelDirectory>(`/api/corpora/${corpusId}/channels`);
}

export function listCorpusLabels(corpusId: string) {
  return apiFetch<CorpusLabel[]>(`/api/corpora/${corpusId}/labels`);
}

export function createCorpusLabel(corpusId: string, input: { name: string }) {
  return jsonPost<CorpusLabel>(`/api/corpora/${corpusId}/labels`, input);
}

export function renameCorpusLabel(corpusId: string, labelId: string, name: string) {
  return apiFetch<CorpusLabel>(`/api/corpora/${corpusId}/labels/${labelId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name }),
  });
}

export function deleteCorpusLabel(corpusId: string, labelId: string) {
  return apiFetch<void>(`/api/corpora/${corpusId}/labels/${labelId}`, {
    method: "DELETE",
  });
}

function labelAssignment(
  corpusId: string,
  target: "sources" | "videos",
  targetId: string,
  labelId: string,
  method: "PUT" | "DELETE",
) {
  return apiFetch<void>(`/api/corpora/${corpusId}/${target}/${targetId}/labels/${labelId}`, {
    method,
  });
}

export function assignSourceLabel(corpusId: string, sourceId: string, labelId: string) {
  return labelAssignment(corpusId, "sources", sourceId, labelId, "PUT");
}

export function removeSourceLabel(corpusId: string, sourceId: string, labelId: string) {
  return labelAssignment(corpusId, "sources", sourceId, labelId, "DELETE");
}

export function assignVideoLabel(corpusId: string, videoId: string, labelId: string) {
  return labelAssignment(corpusId, "videos", videoId, labelId, "PUT");
}

export function removeVideoLabel(corpusId: string, videoId: string, labelId: string) {
  return labelAssignment(corpusId, "videos", videoId, labelId, "DELETE");
}

export function listTranscriptQuality(corpusId: string) {
  return apiFetch<TranscriptQuality[]>(`/api/corpora/${corpusId}/transcript-quality`);
}

export function refreshTranscriptQuality(corpusId: string) {
  return jsonPost<TranscriptQuality[]>(`/api/corpora/${corpusId}/transcript-quality/refresh`, {});
}

export function searchWithinCorpus(corpusId: string, input: SearchTranscriptsInput) {
  return jsonPost<SearchReport>(`/api/corpora/${corpusId}/preferred-search`, input);
}

export function reprocessCorpusVideo(
  corpusId: string,
  videoId: string,
  input: ReprocessVideoInput,
) {
  return jsonPost<ReprocessReport>(`/api/corpora/${corpusId}/videos/${videoId}/reprocess`, input);
}

export function listAnnotations(
  corpusId: string,
  options: { kind?: string; videoId?: string; limit?: number } = {},
) {
  const query = new URLSearchParams();
  if (options.kind) query.set("kind", options.kind);
  if (options.videoId) query.set("videoId", options.videoId);
  if (options.limit) query.set("limit", String(options.limit));
  const suffix = query.size > 0 ? `?${query}` : "";
  return apiFetch<ResearchAnnotation[]>(`/api/corpora/${corpusId}/annotations${suffix}`);
}

export function createAnnotation(corpusId: string, input: CreateAnnotationInput) {
  return jsonPost<ResearchAnnotation>(`/api/corpora/${corpusId}/annotations`, input);
}

export function listEvaluationCases(corpusId: string) {
  return apiFetch<EvaluationCase[]>(`/api/corpora/${corpusId}/evaluation/cases`);
}

export function recordEvaluationJudgment(corpusId: string, input: RecordEvaluationJudgmentInput) {
  return jsonPost<EvaluationCase>(`/api/corpora/${corpusId}/evaluation/judgments`, input);
}

export function runEvaluation(corpusId: string) {
  return jsonPost<EvaluationRunReport>(`/api/corpora/${corpusId}/evaluation/run`, {});
}

export function loadTranscriptContext(segmentId: string): Promise<TranscriptContextReport> {
  return getTranscriptContext({ segmentId, before: 4, after: 6 });
}

/** A mentioned entity grouped by type and normalized name (Atlas 4.1, 4.2). */
export type EntitySummary = {
  entityType: string;
  entityKey: string;
  displayName: string;
  spokenMentions: number;
  visibleTextMentions: number;
  videoCount: number;
  /** Global canonical identity of this corpus-local group (#104). */
  identityId: string | null;
};

/** A labeled corpus-local person identity from face/voice matching; never a verified identity. */
export type PersonSummary = {
  personId: string;
  label: string;
  identitySource: string | null;
  identityConfidence: number | null;
  /** Evidence-grounded name (#65); `label` stays the placeholder identity label. */
  name: string | null;
  nameStatus: PersonNameStatus | null;
  /** Global canonical identity of this corpus-local person (#104). */
  identityId: string | null;
  shownTracks: number;
  heardTracks: number;
  videoCount: number;
};

export type EntityCoverage = {
  corpusVideos: number;
  entityExtractedVideos: number;
  faceProcessedVideos: number;
  voiceProcessedVideos: number;
  people: number;
};

export type EntityLookupReport = {
  query: string;
  entities: EntitySummary[];
  people: PersonSummary[];
  coverage: EntityCoverage;
};

export type EntityOccurrence = {
  id: string;
  modality: "spoken" | "visible_text";
  surfaceText: string;
  startSeconds: number | null;
  endSeconds: number | null;
  context: string;
  byteStart: number;
  byteEnd: number;
  confidence: number | null;
  transcriptSegmentId: string | null;
  visualTextTrackId: string | null;
  visualTextRole: string | null;
  runId: string;
  producer: string;
  producerVersion: string;
  model: string;
  configHash: string;
};

export type EntityOccurrencesReport = {
  entityType: string;
  entityKey: string;
  videos: {
    videoId: string;
    title: string | null;
    sourceUrl: string;
    /** `YYYY-MM-DD` from the video's metadata; `null` when unknown. */
    publicationDate: string | null;
    spokenMentions: number;
    visibleTextMentions: number;
    occurrences: EntityOccurrence[];
  }[];
};

export type PersonOccurrencesReport = {
  person: PersonSummary;
  videos: {
    videoId: string;
    title: string | null;
    sourceUrl: string;
    /** `YYYY-MM-DD` from the video's metadata; `null` when unknown. */
    publicationDate: string | null;
    shownTracks: number;
    heardTracks: number;
    occurrences: {
      kind: "shown" | "heard";
      trackId: string;
      clusterId: string;
      startSeconds: number;
      endSeconds: number;
      linkConfidence: number | null;
      linkSource: string | null;
      clusterSimilarity: number | null;
      processor: string;
      processorVersion: string;
      model: string;
      modelVersion: string;
    }[];
  }[];
};

export type EntityExtractionReport = {
  producer: string;
  producerVersion: string;
  model: string;
  videosProcessed: number;
  videosWithoutText: number;
  segmentsAnalyzed: number;
  visualTextTracksAnalyzed: number;
  mentions: number;
};

export function lookupEntities(corpusId: string, query: string) {
  const params = new URLSearchParams({ q: query });
  return apiFetch<EntityLookupReport>(`/api/corpora/${corpusId}/entities?${params}`);
}

export function getEntityOccurrences(corpusId: string, entityType: string, entityKey: string) {
  const params = new URLSearchParams({ entityType, entityKey });
  return apiFetch<EntityOccurrencesReport>(`/api/corpora/${corpusId}/entity-occurrences?${params}`);
}

export function getPersonOccurrences(corpusId: string, personId: string) {
  return apiFetch<PersonOccurrencesReport>(
    `/api/corpora/${corpusId}/people/${personId}/occurrences`,
  );
}

export function extractCorpusEntities(corpusId: string) {
  return apiFetch<EntityExtractionReport>(`/api/corpora/${corpusId}/entities/extract`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ allowDownload: true }),
  });
}

/** A detector this corpus can request (Atlas 2.3); unavailable ones explain why. */
export type DetectorInfo = {
  id: string;
  label: string;
  producer: string;
  available: boolean;
  note: string | null;
  takesLanguages: boolean;
};

export type DetectorSetting = {
  detector: string;
  enabled: boolean;
  targetLanguages: string[];
};

export type CorpusDetectorProfiles = {
  detectors: DetectorInfo[];
  corpus: DetectorSetting[];
  sources: {
    sourceId: string;
    name: string | null;
    sourceUrl: string;
    settings: DetectorSetting[];
  }[];
};

export type PlanContributor = {
  kind: "corpus" | "source";
  id: string;
  name: string | null;
  explicit: boolean;
};

export type VideoDetectorPlan = {
  corpusId: string;
  videoId: string;
  addedDirectly: boolean;
  sources: PlanContributor[];
  detectors: {
    detector: string;
    label: string;
    available: boolean;
    note: string | null;
    inherited: boolean;
    videoOverride: boolean | null;
    enabled: boolean;
    contributors: PlanContributor[];
    targetLanguages: string[];
  }[];
  videoOverrides: DetectorSetting[];
};

export type PlanRunReport = {
  videoId: string;
  stages: {
    detector: string;
    outcome: "reused" | "completed" | "skipped" | "failed" | "unavailable";
    message: string | null;
  }[];
};

export function getDetectorProfiles(corpusId: string) {
  return apiFetch<CorpusDetectorProfiles>(`/api/corpora/${corpusId}/detector-profiles`);
}

function putSettings(path: string, settings: DetectorSetting[]) {
  return apiFetch<void>(path, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ settings }),
  });
}

export function setCorpusDetectors(corpusId: string, settings: DetectorSetting[]) {
  return putSettings(`/api/corpora/${corpusId}/detector-profiles/corpus`, settings);
}

export function setSourceDetectors(
  corpusId: string,
  sourceId: string,
  settings: DetectorSetting[],
) {
  return putSettings(`/api/corpora/${corpusId}/detector-profiles/sources/${sourceId}`, settings);
}

export function setVideoDetectorOverrides(
  corpusId: string,
  videoId: string,
  settings: DetectorSetting[],
) {
  return putSettings(`/api/corpora/${corpusId}/videos/${videoId}/detector-overrides`, settings);
}

export function getVideoDetectorPlan(corpusId: string, videoId: string) {
  return apiFetch<VideoDetectorPlan>(`/api/corpora/${corpusId}/videos/${videoId}/detector-plan`);
}

export function runVideoDetectorPlan(
  corpusId: string,
  videoId: string,
  refresh: boolean,
  detectors?: string[],
) {
  return apiFetch<PlanRunReport>(`/api/corpora/${corpusId}/videos/${videoId}/detector-plan/run`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ refresh, detectors }),
  });
}

export type VideoProcessingStatus = {
  videoId: string;
  stages: {
    detector: string;
    label: string;
    targetLanguage: string | null;
    sourceLanguages: string[];
    state:
      | "disabled"
      | "awaiting_review"
      | "missing"
      | "missing_source"
      | "running"
      | "completed"
      | "skipped"
      | "failed"
      | "unavailable"
      | "interrupted";
    outputCount: number;
    message: string | null;
    startedAt: string | null;
    finishedAt: string | null;
  }[];
};

export function getVideoProcessingStatus(corpusId: string, videoId: string) {
  return apiFetch<VideoProcessingStatus>(
    `/api/corpora/${corpusId}/videos/${videoId}/processing-status`,
  );
}

export type StageState = VideoProcessingStatus["stages"][number]["state"];

/** States a recorded attempt can be in; `interrupted` is a running row without a live owner. */
export const attemptStates = [
  "running",
  "interrupted",
  "completed",
  "skipped",
  "failed",
  "unavailable",
] as const;
export type AttemptState = (typeof attemptStates)[number];

export type DetectorAttempt = {
  id: string;
  videoId: string;
  videoTitle: string | null;
  videoSourceUrl: string;
  detector: string;
  label: string;
  state: AttemptState;
  message: string | null;
  startedAt: string;
  finishedAt: string | null;
  /** Newest attempt of its video stage; older attempts are superseded. */
  latest: boolean;
};

export type DetectorAttemptDetail = DetectorAttempt & {
  identity: unknown;
  history: DetectorAttempt[];
};

export type DetectorAttemptFilter = {
  videoId?: string;
  detector?: string;
  state?: string;
  latest?: boolean;
  limit?: number;
};

export function listDetectorAttempts(corpusId: string, filter: DetectorAttemptFilter = {}) {
  const params = new URLSearchParams();
  if (filter.videoId) params.set("videoId", filter.videoId);
  if (filter.detector) params.set("detector", filter.detector);
  if (filter.state) params.set("state", filter.state);
  if (filter.latest) params.set("latest", "true");
  if (filter.limit) params.set("limit", String(filter.limit));
  const query = params.toString();
  return apiFetch<DetectorAttempt[]>(
    `/api/corpora/${corpusId}/detector-attempts${query ? `?${query}` : ""}`,
  );
}

export function getDetectorAttempt(corpusId: string, attemptId: string) {
  return apiFetch<DetectorAttemptDetail>(
    `/api/corpora/${corpusId}/detector-attempts/${encodeURIComponent(attemptId)}`,
  );
}

export type FaceEvidence = {
  trackId: string;
  videoId: string;
  title: string | null;
  sourceUrl: string;
  startSeconds: number;
  endSeconds: number;
  observations: number;
  detectionScore: number | null;
  processor: string;
  processorVersion: string;
  model: string;
  modelVersion: string;
  inputHash: string;
  producerConfig: unknown;
  similarity: number | null;
  assignmentSource: string | null;
  matchingAlgorithm: string | null;
  matchingVersion: string | null;
  matchingConfiguration: unknown;
  protected: boolean;
  reason: string | null;
};
export type FacePersonGroup = {
  personId: string;
  label: string;
  identitySource: string | null;
  tracks: FaceEvidence[];
};
export type FaceMatchReview = {
  configuration: unknown;
  groups: FacePersonGroup[];
  unresolved: FaceEvidence[];
};
export type FaceCorrection = {
  action: "unlink" | "split" | "link" | "confirm";
  trackIds: string[];
  personId?: string;
};
export function getFaceMatches(corpusId: string) {
  return apiFetch<FaceMatchReview>(`/api/corpora/${corpusId}/face-matches`);
}
export function runFaceMatches(corpusId: string) {
  return apiFetch<FaceMatchReview>(`/api/corpora/${corpusId}/face-matches/run`, { method: "POST" });
}
export function correctFaceMatches(corpusId: string, correction: FaceCorrection) {
  return apiFetch<FaceMatchReview>(`/api/corpora/${corpusId}/face-matches/corrections`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(correction),
  });
}

export type PersonNameStatus = "inferred" | "confirmed" | "user_provided";
export type NameEvidence = {
  videoId: string;
  title: string | null;
  sourceUrl: string;
  modality: "visible_text";
  startSeconds: number;
  endSeconds: number;
  surfaceText: string;
  sourceText: string;
  byteStart: number;
  byteEnd: number;
  confidence: number | null;
  entityMentionId: string;
  visualTextTrackId: string;
  faceTrackId: string;
  provenance: unknown;
};
export type PersonNameReview = {
  personId: string;
  label: string;
  name: {
    name: string;
    status: PersonNameStatus;
    confidence: number | null;
    algorithm: string;
    algorithmVersion: string;
    configuration: unknown;
    evidence: NameEvidence[];
  } | null;
  unresolved: { reason: string; candidates: string[] } | null;
  rejectedNames: string[];
};
export type PersonNamesReport = {
  configuration: unknown;
  people: PersonNameReview[];
};
export type PersonNameCorrection = {
  personId: string;
  action: "confirm" | "reject" | "replace";
  name?: string;
};
export function getPersonNames(corpusId: string) {
  return apiFetch<PersonNamesReport>(`/api/corpora/${corpusId}/person-names`);
}
export function runPersonNames(corpusId: string) {
  return apiFetch<PersonNamesReport>(`/api/corpora/${corpusId}/person-names/run`, {
    method: "POST",
  });
}
export function correctPersonName(corpusId: string, correction: PersonNameCorrection) {
  return apiFetch<PersonNamesReport>(`/api/corpora/${corpusId}/person-names/corrections`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(correction),
  });
}

/** Global canonical people and entities over corpus-local evidence (#104, Atlas 5.2-5.3). */
export type IdentityKind = "person" | "entity";
export type IdentitySummary = {
  id: string;
  kind: IdentityKind;
  entityType: string;
  displayName: string;
  /** `researcher`, `mentions`, `inferred_name` or `unnamed`. */
  nameSource: "researcher" | "mentions" | "inferred_name" | "unnamed";
  corpora: { id: string; name: string }[];
  localUnits: number;
  hasConfirmedLinks: boolean;
  spokenMentions: number;
  visibleTextMentions: number;
  shownTracks: number;
  heardTracks: number;
  videoCount: number;
};
export type IdentityLocalUnit = {
  unitKey: string;
  unitKind: "entity_group" | "inferred_person";
  corpusId: string;
  corpusName: string;
  label: string;
  entityType: string | null;
  entityKey: string | null;
  personId: string | null;
  nameStatus: PersonNameStatus | null;
  status: "auto" | "confirmed";
  basis: "local" | "shared_evidence" | "name_evidence" | "researcher";
  inScope: boolean;
};
export type IdentityProfile = {
  requestedId: string;
  identity: IdentitySummary;
  units: IdentityLocalUnit[];
  evidence: {
    unitKey: string;
    corpusId: string;
    mentions: EntityOccurrencesReport | null;
    tracks: PersonOccurrencesReport | null;
  }[];
  suggestions: { identity: IdentitySummary; sharedKey: string }[];
  keptApart: IdentitySummary[];
};
export type IdentityCorrection = {
  action: "confirm" | "merge" | "split" | "reject" | "unreject";
  otherIdentityId?: string;
  unitKeys?: string[];
};
export type IdentityListFilter = {
  kind?: IdentityKind;
  entityType?: string;
  /** Restricts to one corpus; omitted lists all corpora. */
  corpusId?: string | null;
  q?: string;
};
export function listIdentities(filter: IdentityListFilter) {
  const params = new URLSearchParams();
  if (filter.kind) params.set("kind", filter.kind);
  if (filter.entityType) params.set("entityType", filter.entityType);
  if (filter.corpusId) params.set("corpusId", filter.corpusId);
  if (filter.q) params.set("q", filter.q);
  return apiFetch<IdentitySummary[]>(`/api/identities?${params}`);
}
export function getIdentityProfile(identityId: string, corpusId?: string | null) {
  const params = new URLSearchParams();
  if (corpusId) params.set("corpusId", corpusId);
  return apiFetch<IdentityProfile>(`/api/identities/${identityId}?${params}`);
}
export function resolveIdentities() {
  return apiFetch<{ units: number; identities: number; linkedUnits: number }>(
    "/api/identities/resolve",
    { method: "POST" },
  );
}
export function correctIdentity(identityId: string, correction: IdentityCorrection) {
  return apiFetch<{ identityId: string; createdIdentityId: string | null }>(
    `/api/identities/${identityId}/corrections`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(correction),
    },
  );
}

/** Historical events and their separate source-backed claims (#106, Atlas 7.2-7.3). */
export type EventDate = {
  /** Historical year: negative is BC; there is no year 0. */
  startYear: number;
  startMonth?: number | null;
  startDay?: number | null;
  endYear?: number | null;
  endMonth?: number | null;
  endDay?: number | null;
  approximate: boolean;
};
export type ClaimPredicate = "date" | "participant" | "place";
export type EventSummary = {
  id: string;
  label: string;
  claimCount: number;
  videoCount: number;
  dateReadings: number;
  disputed: boolean;
  earliestYear: number | null;
  latestYear: number | null;
  preferredDate: EventDate | null;
  corpora: { id: string; name: string }[];
};
export type EventClaim = {
  id: string;
  videoId: string;
  videoTitle: string | null;
  modality: "spoken" | "visible";
  evidenceId: string;
  evidenceText: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
  predicate: ClaimPredicate;
  date: EventDate | null;
  identityId: string | null;
  identity: IdentitySummary | null;
  polarity: "affirms" | "denies";
  certainty: "stated" | "hedged" | "inferred";
  qualification: string;
  reviewStatus: "unverified" | "confirmed" | "rejected";
  preferred: boolean;
};
export type EventProfile = {
  requestedId: string;
  event: EventSummary;
  claims: EventClaim[];
  claimsOutsideScope: number;
};
export type ClaimInput = {
  videoId: string;
  modality: "spoken" | "visible";
  evidenceId: string;
  predicate: ClaimPredicate;
  date?: EventDate | null;
  identityId?: string | null;
  polarity?: "affirms" | "denies";
  certainty?: "stated" | "hedged" | "inferred";
  qualification?: string;
};
export type EventCorrection = {
  action: "review" | "prefer" | "unprefer" | "rename" | "merge" | "split";
  claimIds?: string[];
  reviewStatus?: EventClaim["reviewStatus"];
  otherEventId?: string;
  label?: string;
};
export function listEvents(filter: { corpusId?: string | null; q?: string }) {
  const params = new URLSearchParams();
  if (filter.corpusId) params.set("corpusId", filter.corpusId);
  if (filter.q) params.set("q", filter.q);
  return apiFetch<EventSummary[]>(`/api/events?${params}`);
}
export function getEventProfile(eventId: string, corpusId?: string | null) {
  const params = new URLSearchParams();
  if (corpusId) params.set("corpusId", corpusId);
  return apiFetch<EventProfile>(`/api/events/${eventId}?${params}`);
}
function postJson<T>(path: string, body: unknown) {
  return apiFetch<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}
export function createEvent(label: string, claim: ClaimInput) {
  return postJson<{ id: string }>("/api/events", { label, claim });
}
export function addEventClaim(eventId: string, claim: ClaimInput) {
  return postJson<{ id: string }>(`/api/events/${eventId}/claims`, claim);
}
export function correctEvent(eventId: string, correction: EventCorrection) {
  return postJson<{ eventId: string; createdEventId: string | null }>(
    `/api/events/${eventId}/corrections`,
    correction,
  );
}
