import { invoke } from "@tauri-apps/api/core";

export type SearchMode = "hybrid" | "fts" | "semantic";

export interface SearchResult {
  segmentId: string;
  videoId: string;
  streamId: string;
  sourceKind: string;
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
}

export interface DatabaseStatus {
  configured: boolean;
  databaseUrl: string | null;
}

export function getDatabaseStatus() {
  return invoke<DatabaseStatus>("database_status");
}

export function searchTranscripts(input: SearchTranscriptsInput) {
  return invoke<SearchReport>("search_transcripts", { input });
}
