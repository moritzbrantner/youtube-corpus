import { useMutation, useQuery } from "@tanstack/react-query";

import {
  Badge,
  Button,
  ErrorState,
  LoadingState,
  StateView,
  StateViewDescription,
  StateViewTitle,
} from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import { listEvaluationCases, runEvaluation } from "./api";
import { corpusKeys } from "./query-keys";

type CorpusEvaluationProps = {
  corpusId: string;
};

export function CorpusEvaluation({ corpusId }: CorpusEvaluationProps) {
  const casesQuery = useQuery({
    queryKey: corpusKeys.evaluationCases(corpusId),
    queryFn: () => listEvaluationCases(corpusId),
  });
  const runMutation = useMutation({
    mutationFn: () => runEvaluation(corpusId),
  });
  const targetCount =
    casesQuery.data?.reduce((total, evaluationCase) => total + evaluationCase.targets.length, 0) ?? 0;

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Retrieval evaluation</SurfaceTitle>
        <SurfaceDescription>
          Turn marked search results into a durable relevance set, then measure preferred-transcript
          retrieval with Recall@K, MRR, NDCG@K, and latency.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        {casesQuery.isPending ? <LoadingState label="Loading evaluation cases" /> : null}
        {casesQuery.error ? (
          <ErrorState>
            <StateViewTitle>Evaluation cases unavailable</StateViewTitle>
            <StateViewDescription>{String(casesQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {casesQuery.data ? (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">{casesQuery.data.length} queries</Badge>
              <Badge variant="outline">{targetCount} relevance judgments</Badge>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={targetCount === 0 || runMutation.isPending}
                onClick={() => runMutation.mutate()}
              >
                {runMutation.isPending ? "Running evaluation..." : "Run evaluation"}
              </Button>
            </div>
            {casesQuery.data.length === 0 ? (
              <StateView variant="empty">
                <StateViewTitle>No benchmark judgments yet</StateViewTitle>
                <StateViewDescription>
                  Search this corpus and use Mark relevant on passages that should rank highly. Each
                  judgment becomes part of the repeatable benchmark.
                </StateViewDescription>
              </StateView>
            ) : (
              <div className="grid gap-2">
                {casesQuery.data.slice(0, 8).map((evaluationCase) => (
                  <div
                    key={evaluationCase.id}
                    className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border px-3 py-2"
                  >
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">{evaluationCase.query}</p>
                      <p className="text-xs text-muted-foreground">
                        {evaluationCase.mode} · top {evaluationCase.topK}
                      </p>
                    </div>
                    <Badge variant="secondary">
                      {evaluationCase.targets.length} relevant target
                      {evaluationCase.targets.length === 1 ? "" : "s"}
                    </Badge>
                  </div>
                ))}
              </div>
            )}
          </>
        ) : null}
        {runMutation.data ? (
          <div className="grid gap-2 rounded-md border border-border px-4 py-4 sm:grid-cols-4">
            <Metric label="Recall@K" value={formatMetric(runMutation.data.recallAtK)} />
            <Metric label="MRR" value={formatMetric(runMutation.data.meanReciprocalRank)} />
            <Metric label="NDCG@K" value={formatMetric(runMutation.data.ndcgAtK)} />
            <Metric label="Mean latency" value={`${Math.round(runMutation.data.meanLatencyMs)} ms`} />
          </div>
        ) : null}
        {runMutation.error ? (
          <ErrorState>
            <StateViewTitle>Evaluation failed</StateViewTitle>
            <StateViewDescription>{String(runMutation.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

type MetricProps = {
  label: string;
  value: string;
};

function Metric({ label, value }: MetricProps) {
  return (
    <div>
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 text-lg font-semibold tabular-nums">{value}</p>
    </div>
  );
}

function formatMetric(value: number) {
  return value.toFixed(3);
}
