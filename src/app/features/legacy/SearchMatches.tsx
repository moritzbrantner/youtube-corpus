import {
  Clock,
  Copy as CopyIcon,
  ExternalLink,
  FileText,
  PanelRightOpen,
  RotateCcw,
} from "lucide-react";
import * as React from "react";

import { Badge, Button, CopyButton } from "@moritzbrantner/ui";

import type { SearchResult } from "../../api";
import {
  type ResultGroup,
  buildTimestampUrl,
  formatTimeRange,
  scoreLabel,
  sourceLabel,
} from "./format";
import { ConnectionStatusButton, SearchState } from "./SearchState";
import type { CorpusConnection } from "./use-corpus-connection";
import type { TranscriptSearch } from "./use-transcript-search";

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function queryTerms(query: string) {
  return Array.from(
    new Set(
      query
        .toLowerCase()
        .split(/\s+/)
        .map((term) => term.replace(/^[^\w]+|[^\w]+$/g, ""))
        .filter((term) => term.length >= 2),
    ),
  );
}

function highlightQueryText(text: string, query: string) {
  const terms = queryTerms(query).sort((left, right) => right.length - left.length);
  if (terms.length === 0) {
    return text;
  }

  const pattern = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "gi");
  const nodes: React.ReactNode[] = [];
  let lastIndex = 0;
  for (const match of text.matchAll(pattern)) {
    const index = match.index ?? 0;
    if (index > lastIndex) {
      nodes.push(text.slice(lastIndex, index));
    }
    nodes.push(
      <mark key={`${index}-${match[0]}`} className="rounded bg-primary/15 px-0.5 text-foreground">
        {match[0]}
      </mark>,
    );
    lastIndex = index + match[0].length;
  }
  if (lastIndex < text.length) {
    nodes.push(text.slice(lastIndex));
  }
  return nodes;
}

const SearchResultList = React.memo(function SearchResultList({
  groupedResults,
  filterLabels,
  query,
  onOpenContext,
}: {
  groupedResults: ResultGroup[];
  filterLabels: string[];
  query: string;
  onOpenContext: (result: SearchResult) => void;
}) {
  return (
    <div className="grid gap-4">
      {filterLabels.length > 0 ? (
        <div className="flex flex-wrap items-center gap-2">
          {filterLabels.map((label) => (
            <Badge key={label} variant="outline">
              {label}
            </Badge>
          ))}
        </div>
      ) : null}
      {groupedResults.map((group) => (
        <article
          key={group.videoId}
          className="overflow-hidden rounded-md border border-border bg-background"
        >
          <div className="flex min-w-0 flex-wrap items-center gap-3 border-b border-border bg-muted/25 px-4 py-3">
            <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-sm font-semibold">{group.title ?? group.videoId}</h2>
              <p className="text-xs text-muted-foreground">
                {group.results.length} {group.results.length === 1 ? "hit" : "hits"}
              </p>
            </div>
            <Button asChild variant="ghost" size="sm">
              <a href={group.sourceUrl} target="_blank" rel="noreferrer">
                <ExternalLink className="size-4" aria-hidden="true" />
                Source
              </a>
            </Button>
          </div>

          <div className="divide-y divide-border">
            {group.results.map((result) => {
              const timeRange = formatTimeRange(result.startSeconds, result.endSeconds);
              const timestampUrl = buildTimestampUrl(result.sourceUrl, result.startSeconds);
              return (
                <div key={result.segmentId} className="grid gap-3 px-4 py-4">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <Badge variant="secondary">{sourceLabel(result.sourceKind)}</Badge>
                    {result.language ? <Badge variant="outline">{result.language}</Badge> : null}
                    {timeRange ? (
                      <a
                        className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-0.5 text-xs font-medium text-muted-foreground hover:text-foreground"
                        href={timestampUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <Clock className="size-3" aria-hidden="true" />
                        {timeRange}
                      </a>
                    ) : null}
                    <div className="ml-auto flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <span>Score {scoreLabel(result.score)}</span>
                      <span>FTS {scoreLabel(result.ftsScore)}</span>
                      <span>Semantic {scoreLabel(result.semanticScore)}</span>
                    </div>
                  </div>

                  <p className="text-sm leading-6 text-muted-foreground">
                    {highlightQueryText(result.text, query)}
                  </p>

                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => onOpenContext(result)}
                    >
                      <PanelRightOpen className="size-4" aria-hidden="true" />
                      Context
                    </Button>
                    <Button asChild variant="ghost" size="sm">
                      <a href={timestampUrl} target="_blank" rel="noreferrer">
                        <ExternalLink className="size-4" aria-hidden="true" />
                        Open
                      </a>
                    </Button>
                    <CopyButton
                      value={result.text}
                      variant="ghost"
                      size="sm"
                      idleLabel={
                        <>
                          <CopyIcon className="size-4" aria-hidden="true" />
                          Copy
                        </>
                      }
                      copiedLabel="Copied"
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </article>
      ))}
    </div>
  );
});

export function SearchMatches({
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
  const {
    searchMutation,
    report,
    groupedResults,
    lastSearchSourceKind,
    lastSearchFilterLabels,
    searchAllSources,
    clearSearchFilters,
  } = search;
  const { corpusStatus, connectionReady, stats } = connection;
  if (searchMutation.isPending) {
    return <SearchState variant="loading" title="Searching transcripts..." />;
  }

  if (corpusStatus.data && !corpusStatus.data.configured) {
    return (
      <SearchState
        variant="error"
        title="Database URL required"
        description="Set DATABASE_URL before starting the local web UI."
        actions={<ConnectionStatusButton onClick={onOpenSettings} />}
      />
    );
  }

  if (corpusStatus.data?.configured && !corpusStatus.data.reachable) {
    return (
      <SearchState
        variant="error"
        title="Database unavailable"
        description={corpusStatus.data.message}
        actions={<ConnectionStatusButton onClick={onOpenSettings} />}
      />
    );
  }

  if (corpusStatus.data?.reachable && !corpusStatus.data.schemaReady) {
    return (
      <SearchState
        variant="error"
        title="Migrations required"
        description="The database is reachable, but corpus tables are missing."
      />
    );
  }

  if (connectionReady && stats?.segments === 0) {
    return (
      <SearchState
        title="No transcripts indexed"
        description="Ingest videos or subscriptions before searching."
      />
    );
  }

  if (searchMutation.error) {
    return (
      <SearchState
        variant="error"
        title="Search failed"
        description={String(searchMutation.error)}
      />
    );
  }

  if (!report) {
    return (
      <SearchState
        title="Run a search"
        description="Enter a query to search indexed transcript segments."
      />
    );
  }

  if (report.results.length === 0) {
    const sourceDescription =
      lastSearchSourceKind === "all"
        ? "all sources"
        : sourceLabel(lastSearchSourceKind).toLowerCase();
    return (
      <SearchState
        title="No matches"
        description={
          lastSearchFilterLabels.length === 0
            ? `No results for "${report.query}" in ${sourceDescription}.`
            : `No results for "${report.query}" with the selected filters.`
        }
        actions={
          <div className="flex flex-wrap items-center justify-center gap-2">
            {lastSearchSourceKind === "all" ? null : (
              <Button type="button" variant="outline" onClick={searchAllSources}>
                Search all sources
              </Button>
            )}
            {lastSearchFilterLabels.length === 0 ? null : (
              <Button type="button" variant="ghost" onClick={clearSearchFilters}>
                <RotateCcw className="size-4" aria-hidden="true" />
                Clear filters
              </Button>
            )}
          </div>
        }
      />
    );
  }

  return (
    <SearchResultList
      groupedResults={groupedResults}
      filterLabels={lastSearchFilterLabels}
      query={report.query}
      onOpenContext={onOpenContext}
    />
  );
}
