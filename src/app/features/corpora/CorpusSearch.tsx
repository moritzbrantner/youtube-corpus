import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  Button,
  ErrorState,
  Input,
  LoadingState,
  NativeSelect,
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

import type { SearchMode, SearchResult, SearchTranscriptsInput } from "../../api";
import {
  createAnnotation,
  loadTranscriptContext,
  recordEvaluationJudgment,
  searchWithinCorpus,
} from "./api";
import { corpusKeys } from "./query-keys";

type CorpusSearchProps = {
  corpusId: string;
  submittedQuery: string;
  selectedSegmentId: string | null;
  onSubmitQuery: (query: string) => void;
  onSelectSegment: (segmentId: string | null) => void;
};

export function CorpusSearch({
  corpusId,
  submittedQuery,
  selectedSegmentId,
  onSubmitQuery,
  onSelectSegment,
}: CorpusSearchProps) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState(submittedQuery);
  const [mode, setMode] = React.useState<SearchMode>("hybrid");
  const [annotationKind, setAnnotationKind] = React.useState("finding");
  const [annotationText, setAnnotationText] = React.useState("");
  const searchInput = React.useMemo<SearchTranscriptsInput>(
    () => ({
      query: submittedQuery,
      mode,
      topK: 10,
    }),
    [mode, submittedQuery],
  );
  const searchQuery = useQuery({
    queryKey: corpusKeys.search(corpusId, searchInput),
    queryFn: () => searchWithinCorpus(corpusId, searchInput),
    enabled: submittedQuery.trim().length > 0,
  });
  const contextQuery = useQuery({
    queryKey: corpusKeys.context(selectedSegmentId ?? ""),
    queryFn: () => loadTranscriptContext(selectedSegmentId ?? ""),
    enabled: Boolean(selectedSegmentId),
  });
  const judgmentMutation = useMutation({
    mutationFn: (result: SearchResult) =>
      recordEvaluationJudgment(corpusId, {
        query: submittedQuery,
        mode,
        topK: searchInput.topK,
        videoId: result.videoId,
        segmentId: result.segmentId,
        sourceUrl: result.sourceUrl,
        startSeconds: result.startSeconds,
        endSeconds: result.endSeconds,
        relevance: 3,
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: corpusKeys.evaluationCases(corpusId) });
    },
  });
  const annotationMutation = useMutation({
    mutationFn: async () => {
      const match = contextQuery.data?.match;
      if (!match) {
        throw new Error("Select a transcript passage first.");
      }
      return createAnnotation(corpusId, {
        videoId: match.videoId,
        streamId: match.streamId,
        segmentId: match.segmentId,
        kind: annotationKind,
        startSeconds: match.startSeconds,
        endSeconds: match.endSeconds,
        text: annotationText.trim() || null,
        payload: submittedQuery ? { researchQuery: submittedQuery } : {},
      });
    },
    onSuccess: async () => {
      setAnnotationText("");
      await queryClient.invalidateQueries({ queryKey: corpusKeys.annotations(corpusId) });
    },
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    onSubmitQuery(draft.trim());
  }

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(340px,0.8fr)]">
      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Search this corpus</SurfaceTitle>
          <SurfaceDescription>
            Search is limited to corpus videos and defaults to each video's preferred transcript
            stream. Mark strong results as relevance judgments to grow the retrieval benchmark.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-4">
          <form className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_140px_auto]" onSubmit={submit}>
            <Input
              aria-label="Search corpus"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="Search transcript passages"
            />
            <NativeSelect
              aria-label="Search mode"
              value={mode}
              onChange={(event) => setMode(event.target.value as SearchMode)}
            >
              <option value="hybrid">Hybrid</option>
              <option value="fts">Full text</option>
              <option value="semantic">Semantic</option>
            </NativeSelect>
            <Button type="submit" disabled={!draft.trim()}>
              Search
            </Button>
          </form>

          {!submittedQuery ? (
            <StateView variant="empty">
              <StateViewTitle>Search a transcript corpus</StateViewTitle>
              <StateViewDescription>
                Results link to the exact YouTube timestamp and can open surrounding transcript
                context.
              </StateViewDescription>
            </StateView>
          ) : null}
          {searchQuery.isPending && submittedQuery ? (
            <LoadingState label="Searching preferred transcripts" />
          ) : null}
          {searchQuery.error ? (
            <ErrorState>
              <StateViewTitle>Search failed</StateViewTitle>
              <StateViewDescription>{String(searchQuery.error)}</StateViewDescription>
            </ErrorState>
          ) : null}
          {searchQuery.data?.results.length === 0 ? (
            <StateView variant="empty">
              <StateViewTitle>No matching passages</StateViewTitle>
              <StateViewDescription>
                Try a broader query or another retrieval mode.
              </StateViewDescription>
            </StateView>
          ) : null}
          <div className="grid gap-3">
            {searchQuery.data?.results.map((result) => {
              const judgmentPending =
                judgmentMutation.isPending && judgmentMutation.variables?.segmentId === result.segmentId;
              const judgmentSaved =
                judgmentMutation.isSuccess && judgmentMutation.variables?.segmentId === result.segmentId;
              return (
                <article
                  key={result.segmentId}
                  className="grid gap-3 rounded-md border border-border px-4 py-4"
                >
                  <div className="flex min-w-0 flex-wrap items-start gap-3">
                    <div className="min-w-0 flex-1">
                      <h3 className="truncate text-sm font-semibold">
                        {result.title ?? result.sourceUrl}
                      </h3>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {formatTimestamp(result.startSeconds)} · {result.sourceKind}
                      </p>
                    </div>
                    <Badge variant="outline">{result.score.toFixed(3)}</Badge>
                  </div>
                  <p className="text-sm leading-6">{result.text}</p>
                  <div className="flex flex-wrap gap-2">
                    <Button asChild variant="outline" size="sm">
                      <a
                        href={timestampUrl(result.sourceUrl, result.startSeconds)}
                        target="_blank"
                        rel="noreferrer"
                      >
                        Open timestamp
                      </a>
                    </Button>
                    <Button
                      type="button"
                      variant={selectedSegmentId === result.segmentId ? "default" : "ghost"}
                      size="sm"
                      onClick={() =>
                        onSelectSegment(
                          selectedSegmentId === result.segmentId ? null : result.segmentId,
                        )
                      }
                    >
                      Transcript context
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={judgmentPending}
                      onClick={() => judgmentMutation.mutate(result)}
                    >
                      {judgmentPending ? "Saving judgment..." : judgmentSaved ? "Relevant saved" : "Mark relevant"}
                    </Button>
                  </div>
                </article>
              );
            })}
          </div>
          {judgmentMutation.error ? (
            <p className="text-sm text-destructive">{String(judgmentMutation.error)}</p>
          ) : null}
        </SurfaceContent>
      </Surface>

      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Transcript context</SurfaceTitle>
          <SurfaceDescription>
            Inspect neighboring segments and attach a provenance-aware research annotation to the
            selected passage.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-3">
          {!selectedSegmentId ? (
            <StateView variant="empty">
              <StateViewTitle>No passage selected</StateViewTitle>
              <StateViewDescription>Choose transcript context from a search result.</StateViewDescription>
            </StateView>
          ) : null}
          {contextQuery.isPending && selectedSegmentId ? (
            <LoadingState label="Loading transcript context" />
          ) : null}
          {contextQuery.error ? (
            <ErrorState>
              <StateViewTitle>Context unavailable</StateViewTitle>
              <StateViewDescription>{String(contextQuery.error)}</StateViewDescription>
            </ErrorState>
          ) : null}
          {contextQuery.data ? (
            <>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <Badge variant="outline">{contextQuery.data.match.sourceKind}</Badge>
                <Button asChild variant="outline" size="sm">
                  <a
                    href={timestampUrl(
                      contextQuery.data.match.sourceUrl,
                      contextQuery.data.match.startSeconds,
                    )}
                    target="_blank"
                    rel="noreferrer"
                  >
                    Open in YouTube
                  </a>
                </Button>
              </div>
              <div className="grid gap-2">
                {contextQuery.data.segments.map((segment) => (
                  <div
                    key={segment.segmentId}
                    className={
                      segment.isMatch
                        ? "rounded-md border border-border bg-muted px-3 py-3"
                        : "rounded-md px-3 py-2"
                    }
                  >
                    <p className="text-xs text-muted-foreground">
                      {formatTimestamp(segment.startSeconds)}
                    </p>
                    <p className="mt-1 text-sm leading-6">{segment.text}</p>
                  </div>
                ))}
              </div>
              <div className="grid gap-2 border-t border-border pt-3">
                <div className="grid gap-2 sm:grid-cols-[140px_minmax(0,1fr)]">
                  <NativeSelect
                    aria-label="Annotation kind"
                    value={annotationKind}
                    onChange={(event) => setAnnotationKind(event.target.value)}
                  >
                    <option value="finding">Finding</option>
                    <option value="claim">Claim</option>
                    <option value="quote">Quote</option>
                    <option value="reference">Reference</option>
                    <option value="topic">Topic</option>
                    <option value="note">Note</option>
                  </NativeSelect>
                  <Input
                    aria-label="Annotation text"
                    value={annotationText}
                    onChange={(event) => setAnnotationText(event.target.value)}
                    placeholder="Add a note, label, or interpretation"
                  />
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={annotationMutation.isPending}
                    onClick={() => annotationMutation.mutate()}
                  >
                    {annotationMutation.isPending ? "Saving annotation..." : "Save annotation"}
                  </Button>
                  {annotationMutation.isSuccess ? (
                    <span className="text-xs text-muted-foreground">Annotation saved with provenance.</span>
                  ) : null}
                </div>
                {annotationMutation.error ? (
                  <p className="text-sm text-destructive">{String(annotationMutation.error)}</p>
                ) : null}
              </div>
            </>
          ) : null}
        </SurfaceContent>
      </Surface>
    </div>
  );
}

function formatTimestamp(seconds: number | null) {
  if (seconds === null) {
    return "No timestamp";
  }
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${rest.toString().padStart(2, "0")}`
    : `${minutes}:${rest.toString().padStart(2, "0")}`;
}

function timestampUrl(sourceUrl: string, seconds: number | null) {
  if (seconds === null) {
    return sourceUrl;
  }
  try {
    const url = new URL(sourceUrl);
    url.searchParams.set("t", String(Math.max(0, Math.floor(seconds))));
    return url.toString();
  } catch {
    return sourceUrl;
  }
}
