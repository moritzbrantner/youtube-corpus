import {
  apiFetch,
  type IngestReport,
  type SearchMode,
  type SearchReport,
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
  itemsSeen: number;
  itemsIndexed: number;
  itemsFailed: number;
};

export type CorpusVideo = {
  id: string;
  youtubeId: string | null;
  sourceUrl: string;
  title: string | null;
  channel: string | null;
  thumbnailUrl: string | null;
  durationSeconds: number | null;
  uploadDate: string | null;
  transcriptStreams: number;
  transcriptSegments: number;
  transcriptSourceKinds: string[];
  processingRevision: number | null;
  lastRetrievedAt: string | null;
  updatedAt: string;
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
};

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

export function setCorpusSourceEnabled(corpusId: string, sourceId: string, enabled: boolean) {
  return jsonPost<void>(`/api/corpora/${corpusId}/sources/${sourceId}/enabled`, { enabled });
}

export function listCorpusVideos(corpusId: string, limit = 100) {
  const query = new URLSearchParams({ limit: String(limit) });
  return apiFetch<CorpusVideo[]>(`/api/corpora/${corpusId}/videos?${query}`);
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
