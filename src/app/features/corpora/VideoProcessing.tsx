import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Badge, Button } from "@moritzbrantner/ui";
import { getVideoProcessingStatus, runVideoDetectorPlan, type StageState } from "./api";
import { corpusKeys } from "./query-keys";

/**
 * Only `completed` reads as success. Failures are destructive, running work is secondary, and
 * every other state (skipped, unavailable, missing or incomplete evidence, held for review,
 * disabled) is an outlined badge naming the state.
 */
export function StageStateBadge({ state }: { state: StageState }) {
  const variant =
    state === "completed"
      ? "default"
      : state === "failed" || state === "interrupted"
        ? "destructive"
        : state === "running"
          ? "secondary"
          : "outline";
  return <Badge variant={variant}>{state.replaceAll("_", " ")}</Badge>;
}

export function VideoProcessing({
  corpusId,
  videoId,
  defaultOpen = false,
}: {
  corpusId: string;
  videoId: string;
  defaultOpen?: boolean;
}) {
  const client = useQueryClient();
  const [expanded, setExpanded] = useState(defaultOpen);
  const retry = useMutation({
    mutationFn: ({ detector, refresh }: { detector: string; refresh: boolean }) =>
      runVideoDetectorPlan(corpusId, videoId, refresh, [detector]),
    onSettled: async () => {
      await client.invalidateQueries({ queryKey: ["corpora", corpusId] });
    },
  });
  const status = useQuery({
    enabled: expanded,
    queryKey: corpusKeys.processingStatus(corpusId, videoId),
    queryFn: () => getVideoProcessingStatus(corpusId, videoId),
    // Poll only while an actual stage is running, or this view owns a pending request.
    refetchInterval: (query) =>
      retry.isPending || query.state.data?.stages.some((stage) => stage.state === "running")
        ? 2000
        : false,
  });
  return (
    <details
      open={defaultOpen || undefined}
      onToggle={(event) => {
        setExpanded(event.currentTarget.open);
      }}
    >
      <summary className="cursor-pointer text-sm font-medium">Processing stages</summary>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          void status.refetch();
        }}
      >
        Refresh processing status
      </Button>
      {status.isPending ? <p>Loading processing status…</p> : null}
      {status.error ? <p role="alert">{String(status.error)}</p> : null}
      <ul className="mt-2 grid gap-2">
        {status.data?.stages.map((stage) => (
          <li
            key={`${stage.detector}:${stage.targetLanguage ?? ""}`}
            className="flex flex-wrap items-center gap-2 text-sm"
          >
            <span>
              {stage.label}
              {stage.sourceLanguages.length > 0 ? ` (${stage.sourceLanguages.join(", ")})` : ""}
              {stage.targetLanguage ? ` → ${stage.targetLanguage}` : ""}
            </span>
            <StageStateBadge state={stage.state} />
            <span className="text-xs text-muted-foreground">
              {stage.outputCount} retained outputs
            </span>
            {stage.message ? (
              <span className="text-xs text-muted-foreground">{stage.message}</span>
            ) : null}
            {!["disabled", "awaiting_review", "unavailable", "missing_source", "running"].includes(
              stage.state,
            ) ? (
              <Button
                size="sm"
                variant="outline"
                disabled={retry.isPending}
                onClick={() => retry.mutate({ detector: stage.detector, refresh: false })}
              >
                Run {stage.label}
              </Button>
            ) : null}
            {stage.state === "completed" ? (
              <Button
                size="sm"
                variant="ghost"
                disabled={retry.isPending}
                onClick={() => retry.mutate({ detector: stage.detector, refresh: true })}
              >
                Refresh {stage.label}
              </Button>
            ) : null}
          </li>
        ))}
      </ul>
      {retry.isPending ? <p role="status">Processing selected stage…</p> : null}
      {retry.error ? <p role="alert">{String(retry.error)}</p> : null}
      {retry.data?.stages.map((stage) => (
        <p key={stage.detector} role="status">
          {stage.detector}: {stage.outcome}
          {stage.message ? ` — ${stage.message}` : ""}
        </p>
      ))}
    </details>
  );
}
