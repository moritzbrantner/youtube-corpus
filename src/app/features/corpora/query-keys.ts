import type { SearchTranscriptsInput } from "../../api";

export const corpusKeys = {
  all: ["corpora"] as const,
  sources: (corpusId: string) => ["corpora", corpusId, "sources"] as const,
  videos: (corpusId: string, limit: number, labelId: string | null = null) =>
    ["corpora", corpusId, "videos", { limit, labelId }] as const,
  labels: (corpusId: string) => ["corpora", corpusId, "labels"] as const,
  quality: (corpusId: string) => ["corpora", corpusId, "transcript-quality"] as const,
  annotations: (corpusId: string) => ["corpora", corpusId, "annotations"] as const,
  evaluationCases: (corpusId: string) => ["corpora", corpusId, "evaluation", "cases"] as const,
  search: (corpusId: string, input: SearchTranscriptsInput) =>
    ["corpora", corpusId, "search", input] as const,
  context: (segmentId: string) =>
    ["transcript-context", segmentId, { before: 4, after: 6 }] as const,
};
