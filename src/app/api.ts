export type SearchMode = "hybrid" | "fts" | "semantic";
export type SourceKind = "caption_manual" | "caption_auto" | "asr";
export type AddSourceKind = "video" | "channel" | "playlist";

export interface SearchResult {
  segmentId: string;
  videoId: string;
  streamId: string;
  sourceKind: SourceKind;
  language: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
  sourceUrl: string;
  title: string | null;
  score: number;
  ftsScore: number;
  semanticScore: number;
}

export interface SearchReport {
  query: string;
  mode: SearchMode;
  results: SearchResult[];
}

export interface SearchTranscriptsInput {
  query: string;
  mode: SearchMode;
  topK: number;
  sourceKind?: SourceKind | null;
  videoId?: string | null;
  language?: string | null;
  transcriptStartMin?: number | null;
  transcriptStartMax?: number | null;
  uploadDateFrom?: string | null;
  uploadDateTo?: string | null;
  durationMin?: number | null;
  durationMax?: number | null;
  channelQuery?: string | null;
  titleQuery?: string | null;
  categoryQuery?: string | null;
  tagQuery?: string | null;
  metadataQuery?: string | null;
  viewCountMin?: number | null;
  viewCountMax?: number | null;
}

export interface TranscriptContextInput {
  segmentId: string;
  before?: number;
  after?: number;
}

export interface TranscriptContextSegment {
  segmentId: string;
  videoId: string;
  streamId: string;
  segmentIndex: number;
  sourceKind: SourceKind;
  language: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
  isMatch: boolean;
}

export interface TranscriptContextReport {
  match: SearchResult;
  segments: TranscriptContextSegment[];
}

export interface DownloadedFile {
  id: string;
  youtubeId: string | null;
  sourceUrl: string;
  title: string | null;
  mediaDownloaded: boolean;
  localVideoPath: string | null;
  captionFilesDownloaded: boolean;
  parsed: boolean;
  transcriptStreams: number;
  transcriptSegments: number;
  transcriptSourceKinds: string[];
  subscriptionNames: string[];
  subscriptionSourceUrls: string[];
  durationSeconds: number | null;
  uploadDate: string | null;
  description: string | null;
  channel: string | null;
  channelId: string | null;
  channelUrl: string | null;
  uploader: string | null;
  uploaderId: string | null;
  uploaderUrl: string | null;
  thumbnailUrl: string | null;
  durationString: string | null;
  timestamp: number | null;
  releaseTimestamp: number | null;
  viewCount: number | null;
  likeCount: number | null;
  commentCount: number | null;
  liveStatus: string | null;
  availability: string | null;
  ageLimit: number | null;
  categories: string[];
  tags: string[];
  metadata: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export interface DownloadedFilesInput {
  downloadedOnly?: boolean;
  parsedOnly?: boolean;
  limit?: number;
}

export interface DatabaseStatus {
  configured: boolean;
  databaseUrl: string | null;
}

export interface CorpusStatusInput {}

export interface CorpusStats {
  videos: number;
  streams: number;
  segments: number;
  subscriptions: number;
  enabledSubscriptions: number;
  sourceKinds: Array<{ sourceKind: SourceKind; streams: number; segments: number }>;
  languages: Array<{ language: string; segments: number }>;
  lastIngestRun: {
    id: string;
    sourceUrl: string | null;
    status: string;
    videosIndexed: number;
    segmentsIndexed: number;
    createdAt: string;
  } | null;
}

export interface CorpusStatus {
  configured: boolean;
  databaseUrl: string | null;
  reachable: boolean;
  schemaReady: boolean;
  message: string | null;
  stats: CorpusStats | null;
}

export interface IngestItemReport {
  videoId: string | null;
  sourceUrl: string;
  title: string | null;
  status: string;
  streamsIndexed: number;
  segmentsIndexed: number;
  message: string | null;
}

export interface IngestReport {
  workflow: string;
  runId: string;
  videosSeen: number;
  videosIndexed: number;
  segmentsIndexed: number;
  items: IngestItemReport[];
}

export interface Subscription {
  id: string;
  sourceKind: "channel" | "playlist";
  sourceUrl: string;
  name: string | null;
  enabled: boolean;
  workDir: string;
  maxItems: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface AddSourceInput {
  sourceKind: AddSourceKind;
  sourceUrl: string;
  name?: string | null;
  workDir?: string;
  captionLanguages?: string[];
  captionsEnabled?: boolean;
  autoCaptionsEnabled?: boolean;
  ytDlpArgs?: string[];
  asrEnabled?: boolean;
  transcriberCommand?: string | null;
  transcriberArgs?: string[];
  maxItems?: number | null;
  titleContains?: string | null;
  titleExcludes?: string[];
  durationMin?: number | null;
  durationMax?: number | null;
  migrate?: boolean;
  subscribe?: boolean;
  ingestNow?: boolean;
}

export interface AddSourceReport {
  sourceKind: AddSourceKind;
  sourceUrl: string;
  subscription: Subscription | null;
  ingest: IngestReport | null;
}

interface ApiErrorEnvelope {
  error?: {
    message?: string;
  };
}

export async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init);
  const contentType = response.headers.get("content-type") ?? "";
  const hasJson = contentType.includes("application/json");
  const data = hasJson ? await response.json() : await response.text();

  if (!response.ok) {
    const message =
      hasJson && typeof data === "object" && data !== null
        ? ((data as ApiErrorEnvelope).error?.message ?? response.statusText)
        : String(data || response.statusText);
    throw new Error(message);
  }

  return data as T;
}

function jsonPost<T>(path: string, body: unknown) {
  return apiFetch<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

function queryString(input: DownloadedFilesInput) {
  const params = new URLSearchParams();
  if (input.downloadedOnly !== undefined) {
    params.set("downloadedOnly", String(input.downloadedOnly));
  }
  if (input.parsedOnly !== undefined) {
    params.set("parsedOnly", String(input.parsedOnly));
  }
  if (input.limit !== undefined) {
    params.set("limit", String(input.limit));
  }
  const value = params.toString();
  return value ? `?${value}` : "";
}

export function getDatabaseStatus() {
  return apiFetch<DatabaseStatus>("/api/database-status");
}

export function getCorpusStatus(_input: CorpusStatusInput = {}) {
  return apiFetch<CorpusStatus>("/api/corpus-status");
}

export function searchTranscripts(input: SearchTranscriptsInput) {
  return jsonPost<SearchReport>("/api/search", input);
}

export function getTranscriptContext(input: TranscriptContextInput) {
  return jsonPost<TranscriptContextReport>("/api/transcript-context", input);
}

export function getDownloadedFiles(input: DownloadedFilesInput) {
  return apiFetch<DownloadedFile[]>(`/api/downloaded-files${queryString(input)}`);
}

export function addSource(input: AddSourceInput) {
  return jsonPost<AddSourceReport>("/api/sources", input);
}
