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

export type JobStatus = "queued" | "running" | "cancelling" | "succeeded" | "failed" | "cancelled";

export interface JobProgress {
  completed: number;
  total: number | null;
  unit: string;
  message: string | null;
}

export interface JobFailure {
  message: string;
}

export interface IngestJob {
  id: string;
  status: JobStatus;
  progress: JobProgress | null;
  failure: JobFailure | null;
  ingest: IngestReport | null;
  sourceUrl: string | null;
  createdAt: string;
}

export interface IngestRunStatus {
  id: string;
  sourceUrl: string | null;
  status: string;
  videosSeen: number;
  videosIndexed: number;
  segmentsIndexed: number;
  report: Record<string, unknown>;
  createdAt: string;
  job: IngestJob | null;
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
  ytDlpTimeoutSeconds?: number | null;
  asrEnabled?: boolean;
  transcriberCommand?: string | null;
  transcriberArgs?: string[];
  transcriberTimeoutSeconds?: number | null;
  maxItems?: number | null;
  titleContains?: string | null;
  titleExcludes?: string[];
  durationMin?: number | null;
  durationMax?: number | null;
  migrate?: boolean;
  subscribe?: boolean;
  ingestNow?: boolean;
  async?: boolean;
}

export interface AddSourceReport {
  sourceKind: AddSourceKind;
  sourceUrl: string;
  subscription: Subscription | null;
  ingest: IngestReport | null;
  jobId: string | null;
  ingestRun: IngestRunStatus | null;
}

export interface VideoAnalysisStream {
  streamId: string;
  sourceKind: SourceKind;
  language: string | null;
  status: string;
  segmentCount: number;
  message: string | null;
}

export interface VideoAnalysisSegment {
  segmentId: string;
  streamId: string;
  segmentIndex: number;
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
  language: string | null;
}

export interface VideoAnalysisCoverage {
  metadata: boolean;
  transcript: boolean;
  lexical: boolean;
  mediaRetained: boolean;
  visualTimeline: boolean;
  audioFeatures: boolean;
}

export interface VideoAnalysisReport {
  video: DownloadedFile;
  streams: VideoAnalysisStream[];
  primaryStreamId: string | null;
  segments: VideoAnalysisSegment[];
  lexicalAnalysis: Record<string, unknown> | null;
  coverage: VideoAnalysisCoverage;
}

interface ApiErrorEnvelope {
  error?: {
    message?: string;
  };
}

let apiBaseUrl = "";

export function configureApiBaseUrl(value: string | null | undefined) {
  apiBaseUrl = (value ?? "").trim().replace(/\/+$/, "");
}

export function getConfiguredApiBaseUrl() {
  return apiBaseUrl;
}

export async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(resolveApiPath(path), init);
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

function resolveApiPath(path: string) {
  return apiBaseUrl ? `${apiBaseUrl}${path}` : path;
}

function jsonPost<T>(path: string, body: unknown) {
  return apiFetch<T>(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

function queryString(input: object) {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(input)) {
    if (value !== undefined && value !== null) {
      params.set(key, String(value));
    }
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

export function listIngestRuns(input: { limit?: number } = {}) {
  return apiFetch<IngestRunStatus[]>(`/api/ingest-runs${queryString(input)}`);
}

export function getIngestRun(id: string) {
  return apiFetch<IngestRunStatus>(`/api/ingest-runs/${id}`);
}

export function addSource(input: AddSourceInput) {
  return jsonPost<AddSourceReport>("/api/sources", input);
}

export function getVideoAnalysis(sourceUrl: string) {
  return apiFetch<VideoAnalysisReport>(`/api/video-analysis${queryString({ sourceUrl })}`);
}
