import { ChevronDown, Filter, RotateCcw, Search } from "lucide-react";

import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Input,
  MetricStrip,
  NativeSelect,
  Spinner,
  Tabs,
  TabsList,
  TabsTrigger,
  Textarea,
} from "@moritzbrantner/ui";
import {
  PageContent,
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import type { SearchMode, SearchResult, SourceKind } from "../../api";
import {
  metricValue,
  modeOptions,
  scoreLabel,
  segmentsMetric,
  sourceOptions,
  subscriptionsMetric,
  videosMetric,
} from "./format";
import { SearchMatches } from "./SearchMatches";
import type { CorpusConnection } from "./use-corpus-connection";
import type { TranscriptSearch } from "./use-transcript-search";

function SearchFilterFields({ search }: { search: TranscriptSearch }) {
  const {
    languageFilter,
    setLanguageFilter,
    titleQuery,
    setTitleQuery,
    uploadDateFrom,
    setUploadDateFrom,
    uploadDateTo,
    setUploadDateTo,
    transcriptStartMin,
    setTranscriptStartMin,
    transcriptStartMax,
    setTranscriptStartMax,
    searchDurationMin,
    setSearchDurationMin,
    searchDurationMax,
    setSearchDurationMax,
    channelQuery,
    setChannelQuery,
    categoryQuery,
    setCategoryQuery,
    tagQuery,
    setTagQuery,
    metadataQuery,
    setMetadataQuery,
    viewCountMin,
    setViewCountMin,
    viewCountMax,
    setViewCountMax,
    clearSearchFilters,
  } = search;
  return (
    <div className="grid gap-4 border-t border-border px-4 py-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <label className="grid gap-2">
          <span className="text-sm font-medium">Language</span>
          <Input
            value={languageFilter}
            onChange={(event) => setLanguageFilter(event.target.value)}
            placeholder="en"
            spellCheck={false}
          />
        </label>
        <label className="grid gap-2">
          <span className="text-sm font-medium">Title contains</span>
          <Input value={titleQuery} onChange={(event) => setTitleQuery(event.target.value)} />
        </label>
      </div>

      <div className="grid gap-3">
        <span className="text-sm font-medium">Time span</span>
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Uploaded from</span>
            <Input
              type="date"
              value={uploadDateFrom}
              onChange={(event) => setUploadDateFrom(event.target.value)}
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Uploaded to</span>
            <Input
              type="date"
              value={uploadDateTo}
              onChange={(event) => setUploadDateTo(event.target.value)}
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Transcript start min</span>
            <Input
              value={transcriptStartMin}
              onChange={(event) => setTranscriptStartMin(event.target.value)}
              inputMode="decimal"
              placeholder="0"
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Transcript start max</span>
            <Input
              value={transcriptStartMax}
              onChange={(event) => setTranscriptStartMax(event.target.value)}
              inputMode="decimal"
              placeholder="300"
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Duration min</span>
            <Input
              value={searchDurationMin}
              onChange={(event) => setSearchDurationMin(event.target.value)}
              inputMode="decimal"
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Duration max</span>
            <Input
              value={searchDurationMax}
              onChange={(event) => setSearchDurationMax(event.target.value)}
              inputMode="decimal"
            />
          </label>
        </div>
      </div>

      <div className="grid gap-3">
        <span className="text-sm font-medium">Video metadata</span>
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Channel or uploader</span>
            <Input
              value={channelQuery}
              onChange={(event) => setChannelQuery(event.target.value)}
              spellCheck={false}
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Category</span>
            <Input
              value={categoryQuery}
              onChange={(event) => setCategoryQuery(event.target.value)}
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Tag</span>
            <Input value={tagQuery} onChange={(event) => setTagQuery(event.target.value)} />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Metadata contains</span>
            <Input
              value={metadataQuery}
              onChange={(event) => setMetadataQuery(event.target.value)}
              spellCheck={false}
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Views min</span>
            <Input
              value={viewCountMin}
              onChange={(event) => setViewCountMin(event.target.value)}
              inputMode="numeric"
            />
          </label>
          <label className="grid gap-2">
            <span className="text-xs font-medium text-muted-foreground">Views max</span>
            <Input
              value={viewCountMax}
              onChange={(event) => setViewCountMax(event.target.value)}
              inputMode="numeric"
            />
          </label>
        </div>
      </div>

      <div className="flex justify-end">
        <Button type="button" variant="ghost" size="sm" onClick={clearSearchFilters}>
          <RotateCcw className="size-4" aria-hidden="true" />
          Reset filters
        </Button>
      </div>
    </div>
  );
}

function SearchControls({
  search,
  connection,
}: {
  search: TranscriptSearch;
  connection: CorpusConnection;
}) {
  const {
    query,
    setQuery,
    mode,
    setMode,
    topK,
    setTopK,
    sourceKind,
    setSourceKind,
    filterPanelOpen,
    setFilterPanelOpen,
    activeSearchFilterCount,
    searchMutation,
    submitSearch,
  } = search;
  const { corpusStatus, connectionReady, statusMessage } = connection;
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Search controls</SurfaceTitle>
        <SurfaceDescription>Configure the query sent to the local HTTP API.</SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent>
        <form className="mt-5 grid gap-5" onSubmit={submitSearch}>
          {!connectionReady && !corpusStatus.isLoading ? (
            <Alert variant="destructive">
              <AlertDescription>
                {statusMessage ?? "Corpus database is not ready for search."}
              </AlertDescription>
            </Alert>
          ) : null}

          <label className="grid gap-2">
            <span className="text-sm font-medium">Query</span>
            <Textarea value={query} onChange={(event) => setQuery(event.target.value)} rows={4} />
          </label>

          <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_112px]">
            <div className="grid gap-2">
              <span className="text-sm font-medium">Mode</span>
              <Tabs value={mode} onValueChange={(value) => setMode(value as SearchMode)}>
                <TabsList className="grid w-full grid-cols-3">
                  {modeOptions.map((option) => (
                    <TabsTrigger key={option.value} value={option.value}>
                      {option.label}
                    </TabsTrigger>
                  ))}
                </TabsList>
              </Tabs>
            </div>
            <label className="grid gap-2">
              <span className="text-sm font-medium">Top K</span>
              <NativeSelect
                value={String(topK)}
                onChange={(event) => setTopK(Number(event.target.value))}
              >
                {[3, 5, 10, 20].map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </NativeSelect>
            </label>
          </div>

          <label className="grid gap-2">
            <span className="text-sm font-medium">Source</span>
            <NativeSelect
              value={sourceKind}
              onChange={(event) => setSourceKind(event.target.value as SourceKind | "all")}
            >
              {sourceOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </NativeSelect>
          </label>

          <div className="overflow-hidden rounded-md border border-border bg-background">
            <button
              type="button"
              className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left"
              aria-expanded={filterPanelOpen}
              onClick={() => setFilterPanelOpen((open) => !open)}
            >
              <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
                <Filter className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                Filters
              </span>
              <span className="flex shrink-0 items-center gap-2">
                {activeSearchFilterCount > 0 ? (
                  <Badge variant="secondary">{activeSearchFilterCount} active</Badge>
                ) : (
                  <Badge variant="outline">None</Badge>
                )}
                <ChevronDown
                  className={`size-4 text-muted-foreground transition-transform ${
                    filterPanelOpen ? "rotate-180" : ""
                  }`}
                  aria-hidden="true"
                />
              </span>
            </button>

            {filterPanelOpen ? <SearchFilterFields search={search} /> : null}
          </div>

          <Button type="submit" disabled={searchMutation.isPending || query.trim() === ""}>
            {searchMutation.isPending ? <Spinner className="mr-2 size-4" /> : <Search />}
            Search transcripts
          </Button>
        </form>
      </SurfaceContent>
    </Surface>
  );
}

export function SearchPage({
  search,
  connection,
  onOpenSettings,
  onOpenContext,
}: {
  search: TranscriptSearch;
  connection: CorpusConnection;
  onOpenSettings: () => void;
  onOpenContext: (result: SearchResult) => void;
}) {
  const { report, resultCount, topScore } = search;
  const { stats, connectionReady } = connection;
  return (
    <PageContent className="grid gap-4 lg:grid-cols-[minmax(360px,440px)_minmax(0,1fr)]">
      <SearchControls search={search} connection={connection} />

      <div className="grid gap-4">
        <MetricStrip
          items={[
            videosMetric(stats, connectionReady),
            segmentsMetric(stats, connectionReady),
            {
              id: "streams",
              label: "Streams",
              value: metricValue(stats?.streams),
              delta: connectionReady ? "Transcript sources" : "Unavailable",
            },
            subscriptionsMetric(stats),
            {
              id: "results",
              label: "Results",
              value: report ? String(resultCount) : "-",
              delta: report
                ? report.provider === "tantivy_literal"
                  ? "literal substring (Tantivy pilot, unranked)"
                  : report.mode
                : "No query",
            },
            {
              id: "top-score",
              label: "Top score",
              value: topScore === null ? "-" : scoreLabel(topScore),
              delta: report?.query ?? "Awaiting search",
            },
          ]}
        />

        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Matches</SurfaceTitle>
            <SurfaceDescription>
              Ranked transcript segments returned by the Rust search pipeline.
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="mt-5">
            <SearchMatches
              search={search}
              connection={connection}
              onOpenSettings={onOpenSettings}
              onOpenContext={onOpenContext}
            />
          </SurfaceContent>
        </Surface>
      </div>
    </PageContent>
  );
}
