import type { SearchTranscriptsInput } from "../../api";

import type { CorpusVideoFilter, DetectorAttemptFilter } from "./api";

export const corpusKeys = {
  all: ["corpora"] as const,
  sources: (corpusId: string) => ["corpora", corpusId, "sources"] as const,
  videos: (corpusId: string, limit: number, filter: CorpusVideoFilter = {}) =>
    [
      "corpora",
      corpusId,
      "videos",
      {
        limit,
        labelId: filter.labelId ?? null,
        sourceId: filter.sourceId ?? null,
        channelId: filter.channelId ?? null,
      },
    ] as const,
  video: (corpusId: string, videoId: string) =>
    ["corpora", corpusId, "videos", "detail", videoId] as const,
  /** Channels are derived from video metadata, so they live under the videos key. */
  channels: (corpusId: string) => ["corpora", corpusId, "videos", "channels"] as const,
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
  attempts: (corpusId: string, filter: DetectorAttemptFilter) =>
    ["corpora", corpusId, "attempts", filter] as const,
  attempt: (corpusId: string, attemptId: string) =>
    ["corpora", corpusId, "attempts", "detail", attemptId] as const,
  context: (segmentId: string) =>
    ["transcript-context", segmentId, { before: 4, after: 6 }] as const,
};
