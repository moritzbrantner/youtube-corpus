import {
  apiFetch,
  type IngestReport,
  type SearchReport,
  type SearchTranscriptsInput,
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

export type ReprocessStage =
  | "metadata"
  | "captions"
  | "asr"
  | "segments"
  | "embeddings"
  | "all";

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

export function searchWithinCorpus(corpusId: string, input: SearchTranscriptsInput) {
  return jsonPost<SearchReport>(`/api/corpora/${corpusId}/search`, input);
}

export function reprocessCorpusVideo(
  corpusId: string,
  videoId: string,
  input: ReprocessVideoInput,
) {
  return jsonPost<ReprocessReport>(`/api/corpora/${corpusId}/videos/${videoId}/reprocess`, input);
}

export function loadTranscriptContext(segmentId: string): Promise<TranscriptContextReport> {
  return getTranscriptContext({ segmentId, before: 4, after: 6 });
}
