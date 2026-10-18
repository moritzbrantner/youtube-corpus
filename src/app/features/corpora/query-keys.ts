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
  entities: (corpusId: string, query: string) => ["corpora", corpusId, "entities", query] as const,
  entityOccurrences: (corpusId: string, entityType: string, entityKey: string) =>
    ["corpora", corpusId, "entities", "occurrences", entityType, entityKey] as const,
  personOccurrences: (corpusId: string, personId: string) =>
    ["corpora", corpusId, "entities", "people", personId] as const,
  detectorProfiles: (corpusId: string) => ["corpora", corpusId, "detectors"] as const,
  detectorPlan: (corpusId: string, videoId: string) =>
    ["corpora", corpusId, "detectors", "plan", videoId] as const,
  processingStatus: (corpusId: string, videoId: string) =>
    ["corpora", corpusId, "detectors", "plan", videoId, "status"] as const,
  context: (segmentId: string) =>
    ["transcript-context", segmentId, { before: 4, after: 6 }] as const,
};
