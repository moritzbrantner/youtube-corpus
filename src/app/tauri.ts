import { invoke } from "@tauri-apps/api/core";

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
  databaseUrl?: string;
  query: string;
  mode: SearchMode;
  topK: number;
  sourceKind?: SourceKind | null;
}

export interface TranscriptContextInput {
  databaseUrl?: string;
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
  databaseUrl?: string;
  downloadedOnly?: boolean;
  parsedOnly?: boolean;
  limit?: number;
}

export interface DatabaseStatus {
  configured: boolean;
  databaseUrl: string | null;
}

export interface CorpusStatusInput {
  databaseUrl?: string;
}

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
  databaseUrl?: string;
  sourceKind: AddSourceKind;
  sourceUrl: string;
  name?: string | null;
  workDir?: string;
  captionLanguages?: string[];
  captionsEnabled?: boolean;
  autoCaptionsEnabled?: boolean;
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

function tauriInvoke<T>(command: string, args?: Record<string, unknown>) {
  if (typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window)) {
    throw new Error("Tauri backend is unavailable. Start the desktop app with `bun dev`.");
  }
  return invoke<T>(command, args);
}

export function getDatabaseStatus() {
  return tauriInvoke<DatabaseStatus>("database_status");
}

export function getCorpusStatus(input: CorpusStatusInput) {
  return tauriInvoke<CorpusStatus>("corpus_status", { input });
}

export function searchTranscripts(input: SearchTranscriptsInput) {
  return tauriInvoke<SearchReport>("search_transcripts", { input });
}

export function getTranscriptContext(input: TranscriptContextInput) {
  return tauriInvoke<TranscriptContextReport>("transcript_context", { input });
}

export function getDownloadedFiles(input: DownloadedFilesInput) {
  return tauriInvoke<DownloadedFile[]>("downloaded_files", { input });
}

export function addSource(input: AddSourceInput) {
  return tauriInvoke<AddSourceReport>("add_source", { input });
}
