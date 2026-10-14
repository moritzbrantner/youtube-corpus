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
import type { CorpusSearchFilters } from "./url-state";

type CorpusSearchProps = {
  corpusId: string;
  submittedQuery: string;
  filters: CorpusSearchFilters;
  selectedSegmentId: string | null;
  onSubmitQuery: (query: string) => void;
  onUpdateFilters: (filters: Partial<CorpusSearchFilters>) => void;
  onSelectSegment: (segmentId: string | null) => void;
};

export function CorpusSearch({
  corpusId,
  submittedQuery,
  filters,
  selectedSegmentId,
  onSubmitQuery,
  onUpdateFilters,
  onSelectSegment,
}: CorpusSearchProps) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState(submittedQuery);
  const [draftFilters, setDraftFilters] = React.useState(filters);
  React.useEffect(() => {
    setDraftFilters(filters);
  }, [
    filters.mode, filters.modality, filters.sourceKind, filters.language, filters.channelQuery,
    filters.titleQuery, filters.categoryQuery, filters.tagQuery, filters.uploadDateFrom,
    filters.uploadDateTo,
  ]);
  function changeFilter(patch: Partial<CorpusSearchFilters>) {
    setDraftFilters((current) => ({ ...current, ...patch }));
    onUpdateFilters(patch);
  }
  const [annotationKind, setAnnotationKind] = React.useState("finding");
  const [annotationText, setAnnotationText] = React.useState("");
  const searchInput = React.useMemo<SearchTranscriptsInput>(
    () => ({
      query: submittedQuery,
      mode: filters.mode,
      topK: 30,
      modality: filters.modality === "all" ? null : filters.modality,
      sourceKind: filters.sourceKind === "all" ? null : filters.sourceKind,
      language: filters.language.trim() || null,
      channelQuery: filters.channelQuery.trim() || null,
      titleQuery: filters.titleQuery.trim() || null,
      categoryQuery: filters.categoryQuery.trim() || null,
      tagQuery: filters.tagQuery.trim() || null,
      uploadDateFrom: filters.uploadDateFrom || null,
      uploadDateTo: filters.uploadDateTo || null,
    }),
    [filters, submittedQuery],
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
        mode: filters.mode,
        topK: searchInput.topK,
        videoId: result.videoId,
        segmentId: result.sourceKind === "visual_ocr" ? null : result.segmentId,
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
        throw new Error("Select a search moment first.");
      }
      return createAnnotation(corpusId, {
        videoId: match.videoId,
        streamId: match.sourceKind === "visual_ocr" ? null : match.streamId,
        segmentId: match.sourceKind === "visual_ocr" ? null : match.segmentId,
        kind: annotationKind,
        startSeconds: match.startSeconds,
        endSeconds: match.endSeconds,
        text: annotationText.trim() || null,
        payload: {
          ...(submittedQuery ? { researchQuery: submittedQuery } : {}),
          ...(match.sourceKind === "visual_ocr"
            ? { ocrTrackId: match.segmentId, ocrRunId: match.streamId }
            : {}),
        },
      });
    },
    onSuccess: async () => {
      setAnnotationText("");
      await queryClient.invalidateQueries({ queryKey: corpusKeys.annotations(corpusId) });
    },
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    onUpdateFilters(draftFilters);
    onSubmitQuery(draft.trim());
  }

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(340px,0.8fr)]">
      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Search this corpus</SurfaceTitle>
          <SurfaceDescription>
            Search spoken passages and on-screen OCR tracks together by relevance. Speech uses
            each video's preferred transcript stream; visible text retains its own evidence.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-4">
          <form className="grid gap-3" onSubmit={submit}>
            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_140px_140px_auto]">
              <Input
                aria-label="Search corpus"
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                placeholder="Search spoken and visible text"
              />
              <NativeSelect
                aria-label="Search mode"
                value={filters.mode}
                onChange={(event) => changeFilter({ mode: event.target.value as SearchMode })}
              >
                <option value="hybrid">Hybrid</option>
                <option value="fts">Full text</option>
                <option value="semantic">Semantic</option>
              </NativeSelect>
              <NativeSelect
                aria-label="Search modality"
                value={filters.modality}
                onChange={(event) =>
                  changeFilter({ modality: event.target.value as CorpusSearchFilters["modality"] })
                }
              >
                <option value="all">Spoken + visible</option>
                <option value="spoken">Spoken</option>
                <option value="visible">Visible text</option>
              </NativeSelect>
              <Button type="submit" disabled={!draft.trim()}>
                Search
              </Button>
            </div>
            <details className="rounded-md border border-border px-3 py-2">
              <summary className="cursor-pointer text-sm">Source and metadata filters</summary>
              <div className="mt-3 grid gap-3 sm:grid-cols-2">
                <NativeSelect
                  aria-label="Transcript or OCR source"
                  value={filters.sourceKind}
                  onChange={(event) =>
                    changeFilter({
                      sourceKind: event.target.value as CorpusSearchFilters["sourceKind"],
                    })
                  }
                >
                  <option value="all">All evidence sources</option>
                  <option value="caption_manual">Manual captions</option>
                  <option value="caption_auto">Automatic captions</option>
                  <option value="asr">Speech recognition</option>
                  <option value="visual_ocr">OCR tracks</option>
                </NativeSelect>
                <Input
                  aria-label="Language filter"
                  placeholder="Language (e.g. en)"
                  value={draftFilters.language}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, language: event.target.value }))
                  }
                />
                <Input
                  aria-label="Channel filter"
                  placeholder="Channel or uploader"
                  value={draftFilters.channelQuery}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, channelQuery: event.target.value }))
                  }
                />
                <Input
                  aria-label="Video title filter"
                  placeholder="Video title"
                  value={draftFilters.titleQuery}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, titleQuery: event.target.value }))
                  }
                />
                <Input
                  aria-label="YouTube category filter"
                  placeholder="YouTube category"
                  value={draftFilters.categoryQuery}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, categoryQuery: event.target.value }))
                  }
                />
                <Input
                  aria-label="YouTube tag filter"
                  placeholder="YouTube tag"
                  value={draftFilters.tagQuery}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, tagQuery: event.target.value }))
                  }
                />
                <Input
                  aria-label="Upload date from"
                  type="date"
                  value={draftFilters.uploadDateFrom}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, uploadDateFrom: event.target.value }))
                  }
                />
                <Input
                  aria-label="Upload date to"
                  type="date"
                  value={draftFilters.uploadDateTo}
                  onChange={(event) =>
                    setDraftFilters((current) => ({ ...current, uploadDateTo: event.target.value }))
                  }
                />
              </div>
              <p className="mt-2 text-xs text-muted-foreground">
                Submit Search to apply metadata filters. User labels will use the corpus
                organization feature when it is available.
              </p>
            </details>
          </form>

          {!submittedQuery ? (
            <StateView variant="empty">
              <StateViewTitle>Search corpus evidence</StateViewTitle>
              <StateViewDescription>
                Results link to original video moments with separate spoken and visible-text
                evidence.
              </StateViewDescription>
            </StateView>
          ) : null}
          {searchQuery.isPending && submittedQuery ? (
            <LoadingState label="Searching speech and visible text" />
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
                Try a broader query or another retrieval mode. Missing OCR hits do not imply
                that unprocessed videos contain no visible text.
              </StateViewDescription>
            </StateView>
          ) : null}
          {searchQuery.data?.ocrCoverage ? (
            <p className="text-xs text-muted-foreground" role="status">
              Visual-text index: {searchQuery.data.ocrCoverage.indexedVideos} videos indexed,{" "}
              {searchQuery.data.ocrCoverage.attemptedWithoutTracks} attempted without tracks,{" "}
              {searchQuery.data.ocrCoverage.unprocessedVideos} unprocessed. Unprocessed videos
              have unknown on-screen text coverage.
            </p>
          ) : null}
          <div className="grid gap-3">
            {searchQuery.data?.results.map((result) => {
              const judgmentPending =
                judgmentMutation.isPending &&
                judgmentMutation.variables?.segmentId === result.segmentId;
              const judgmentSaved =
                judgmentMutation.isSuccess &&
                judgmentMutation.variables?.segmentId === result.segmentId;
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
                        {formatTimestamp(result.startSeconds)} ·{" "}
                        {sourceLabel(result.sourceKind)}
                        {result.language ? ` · ${result.language}` : ""}
                      </p>
                    </div>
                    <Badge variant="outline">
                      {result.sourceKind === "visual_ocr" ? "Visible text" : "Spoken"}
                    </Badge>
                    <Badge variant="outline">{result.score.toFixed(3)}</Badge>
                  </div>
                  <p className="text-sm leading-6">{result.text}</p>
                  {result.ocrEvidence ? <OcrEvidenceDetails evidence={result.ocrEvidence} /> : null}
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
                      {result.sourceKind === "visual_ocr" ? "Visual context" : "Transcript context"}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={judgmentPending}
                      onClick={() => judgmentMutation.mutate(result)}
                    >
                      {judgmentPending
                        ? "Saving judgment..."
                        : judgmentSaved
                          ? "Relevant saved"
                          : "Mark relevant"}
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
          <SurfaceTitle>Moment context</SurfaceTitle>
          <SurfaceDescription>
            Inspect adjacent transcript passages or tracked visible text, and annotate the
            selected moment.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-3">
          {!selectedSegmentId ? (
            <StateView variant="empty">
              <StateViewTitle>No passage selected</StateViewTitle>
              <StateViewDescription>
                Select a moment from the ranked results.
              </StateViewDescription>
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
                <Badge variant="outline">
                  {contextQuery.data.match.sourceKind === "visual_ocr" ? "Visible text" : "Spoken"}
                </Badge>
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
                      {formatTimestamp(segment.startSeconds)} · {sourceLabel(segment.sourceKind)}
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
                    <span className="text-xs text-muted-foreground">
                      Annotation saved with provenance.
                    </span>
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
