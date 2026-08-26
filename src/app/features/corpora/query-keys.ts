import type { SearchTranscriptsInput } from "../../api";

export const corpusKeys = {
  all: ["corpora"] as const,
  sources: (corpusId: string) => ["corpora", corpusId, "sources"] as const,
  videos: (corpusId: string, limit: number) => ["corpora", corpusId, "videos", { limit }] as const,
  search: (corpusId: string, input: SearchTranscriptsInput) =>
    ["corpora", corpusId, "search", input] as const,
  context: (segmentId: string) =>
    ["transcript-context", segmentId, { before: 4, after: 6 }] as const,
};
