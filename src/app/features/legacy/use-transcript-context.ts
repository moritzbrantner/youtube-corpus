import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import { type SearchResult, getTranscriptContext } from "../../api";
import { formatTimeRange, sourceLabel } from "./format";

export function useTranscriptContext() {
  const [selectedResult, setSelectedResult] = React.useState<SearchResult | null>(null);
  const [contextOpen, setContextOpen] = React.useState(false);

  const transcriptContext = useQuery({
    queryKey: ["transcript-context", selectedResult?.segmentId],
    queryFn: () =>
      getTranscriptContext({
        segmentId: selectedResult!.segmentId,
        before: 4,
        after: 6,
      }),
    enabled: contextOpen && selectedResult !== null,
  });

  const contextMatch = transcriptContext.data?.match ?? selectedResult;
  const contextTimeRange = contextMatch
    ? formatTimeRange(contextMatch.startSeconds, contextMatch.endSeconds)
    : null;
  const contextDescription = contextMatch
    ? [sourceLabel(contextMatch.sourceKind), contextMatch.language, contextTimeRange]
        .filter(Boolean)
        .join(" / ")
    : "Transcript context";

  const openContext = React.useCallback((result: SearchResult) => {
    setSelectedResult(result);
    setContextOpen(true);
  }, []);

  return {
    selectedResult,
    contextOpen,
    setContextOpen,
    transcriptContext,
    contextMatch,
    contextDescription,
    openContext,
  };
}

export type TranscriptContext = ReturnType<typeof useTranscriptContext>;
