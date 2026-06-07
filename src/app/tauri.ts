import { invoke } from "@tauri-apps/api/core";

export type SearchMode = "hybrid" | "fts" | "semantic";
export type SourceKind = "caption_manual" | "caption_auto" | "asr";

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

export function getDatabaseStatus() {
  return invoke<DatabaseStatus>("database_status");
}

export function getCorpusStatus(input: CorpusStatusInput) {
  return invoke<CorpusStatus>("corpus_status", { input });
}

export function searchTranscripts(input: SearchTranscriptsInput) {
  return invoke<SearchReport>("search_transcripts", { input });
}

export function getTranscriptContext(input: TranscriptContextInput) {
  return invoke<TranscriptContextReport>("transcript_context", { input });
}
