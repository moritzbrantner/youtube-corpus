import { useMutation, useQuery } from "@tanstack/react-query";
import {
  Clock,
  Copy as CopyIcon,
  Database,
  ExternalLink,
  FileText,
  PanelRightOpen,
  Search,
  Settings,
} from "lucide-react";
import * as React from "react";

import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  CopyButton,
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
  ErrorState,
  Input,
  LoadingState,
  MetricStrip,
  NativeSelect,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Spinner,
  StateView,
  StateViewActions,
  StateViewDescription,
  StateViewTitle,
  Tabs,
  TabsList,
  TabsTrigger,
  Textarea,
  type ThemeMode,
  ThemeModeSwitch,
} from "@moritzbrantner/ui";
import {
  Navbar,
  NavbarActions,
  PageActions,
  PageContent,
  PageDescription,
  PageHeader,
  PageShell,
  PageTitle,
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";
import {
  type SearchMode,
  type SearchReport,
  type SearchResult,
  type SourceKind,
  getCorpusStatus,
  getTranscriptContext,
  searchTranscripts,
} from "./app/tauri";

const defaultQuery = "first uploaded youtube video";
const themeStorageKey = "youtube-corpus-theme-mode";

const modeOptions: Array<{ value: SearchMode; label: string }> = [
  { value: "hybrid", label: "Hybrid" },
  { value: "fts", label: "FTS" },
  { value: "semantic", label: "Semantic" },
];

const sourceOptions: Array<{ value: SourceKind | "all"; label: string }> = [
  { value: "all", label: "All sources" },
  { value: "caption_manual", label: "Manual captions" },
  { value: "caption_auto", label: "Auto captions" },
  { value: "asr", label: "ASR" },
];

const navigationGroups = [
  {
    id: "corpus",
    label: "Corpus",
    items: [{ id: "search", label: "Search", href: "#search", active: true }],
  },
];

function getInitialThemeMode(): ThemeMode {
  if (typeof window === "undefined") {
    return "light";
  }

  const storedMode = window.localStorage.getItem(themeStorageKey);
  if (storedMode === "light" || storedMode === "dark") {
    return storedMode;
  }

  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyThemeMode(mode: ThemeMode) {
  document.documentElement.classList.toggle("dark", mode === "dark");
  document.documentElement.style.colorScheme = mode;
}

interface ResultGroup {
  videoId: string;
  title: string | null;
  sourceUrl: string;
  results: SearchResult[];
}

function formatTimestamp(seconds: number | null) {
  if (seconds === null) {
    return null;
  }
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const rest = totalSeconds % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${rest.toString().padStart(2, "0")}`
    : `${minutes}:${rest.toString().padStart(2, "0")}`;
}

function formatTimeRange(startSeconds: number | null, endSeconds: number | null) {
  const start = formatTimestamp(startSeconds);
  const end = formatTimestamp(endSeconds);
  if (start && end) {
    return `${start}-${end}`;
  }
  return start ?? null;
}

function buildTimestampUrl(sourceUrl: string, seconds: number | null) {
  if (seconds === null) {
    return sourceUrl;
  }
  const separator = sourceUrl.includes("?") ? "&" : "?";
  return `${sourceUrl}${separator}t=${Math.max(0, Math.floor(seconds))}`;
}

function sourceLabel(sourceKind: string) {
  return sourceOptions.find((option) => option.value === sourceKind)?.label ?? sourceKind;
}

function scoreLabel(score: number) {
  return Number.isFinite(score) ? score.toFixed(3) : "0.000";
}

function metricValue(value: number | undefined) {
  return value === undefined ? "-" : value.toLocaleString();
}

function groupResultsByVideo(results: SearchResult[]) {
  const groups = new Map<string, ResultGroup>();
  for (const result of results) {
    const group = groups.get(result.videoId);
    if (group) {
      group.results.push(result);
    } else {
      groups.set(result.videoId, {
        videoId: result.videoId,
        title: result.title,
        sourceUrl: result.sourceUrl,
        results: [result],
      });
    }
  }
  return Array.from(groups.values());
}

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

function SearchState({
  variant = "empty",
  title,
  description,
  actions,
}: {
  variant?: "empty" | "loading" | "error";
  title: string;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  const content = (
    <>
      <StateViewTitle>{title}</StateViewTitle>
      {description ? <StateViewDescription>{description}</StateViewDescription> : null}
      {actions ? <StateViewActions>{actions}</StateViewActions> : null}
    </>
  );

  if (variant === "loading") {
    return <LoadingState size="lg" label={title} />;
  }
  if (variant === "error") {
    return <ErrorState size="lg">{content}</ErrorState>;
  }
  return (
    <StateView variant="empty" size="lg">
      {content}
    </StateView>
  );
}

export default function App() {
  const [themeMode, setThemeMode] = React.useState<ThemeMode>(getInitialThemeMode);
  const [databaseUrl, setDatabaseUrl] = React.useState("");
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [query, setQuery] = React.useState(defaultQuery);
  const [mode, setMode] = React.useState<SearchMode>("hybrid");
  const [topK, setTopK] = React.useState(5);
  const [sourceKind, setSourceKind] = React.useState<SourceKind | "all">("all");
  const [lastSearchSourceKind, setLastSearchSourceKind] = React.useState<SourceKind | "all">("all");
  const [lastReport, setLastReport] = React.useState<SearchReport | null>(null);
  const [selectedResult, setSelectedResult] = React.useState<SearchResult | null>(null);
  const [contextOpen, setContextOpen] = React.useState(false);
  const hasUserEditedDatabaseUrl = React.useRef(false);

  React.useLayoutEffect(() => {
    applyThemeMode(themeMode);
    window.localStorage.setItem(themeStorageKey, themeMode);
  }, [themeMode]);

  const corpusStatus = useQuery({
    queryKey: ["corpus-status", databaseUrl.trim()],
    queryFn: () => getCorpusStatus({ databaseUrl: databaseUrl.trim() || undefined }),
    refetchOnWindowFocus: false,
  });

  React.useEffect(() => {
    const envDatabaseUrl = corpusStatus.data?.databaseUrl;
    if (!hasUserEditedDatabaseUrl.current && envDatabaseUrl && databaseUrl.trim() === "") {
      setDatabaseUrl(envDatabaseUrl);
    }
  }, [corpusStatus.data?.databaseUrl, databaseUrl]);

  const searchMutation = useMutation({
    mutationFn: searchTranscripts,
    onSuccess: (report) => {
      setLastReport(report);
    },
  });

  const transcriptContext = useQuery({
    queryKey: ["transcript-context", selectedResult?.segmentId, databaseUrl.trim()],
    queryFn: () =>
      getTranscriptContext({
        databaseUrl: databaseUrl.trim() || undefined,
        segmentId: selectedResult!.segmentId,
        before: 4,
        after: 6,
      }),
    enabled: contextOpen && selectedResult !== null,
  });

  const report = searchMutation.data ?? lastReport;
  const groupedResults = React.useMemo(
    () => (report ? groupResultsByVideo(report.results) : []),
    [report],
  );
  const resultCount = report?.results.length ?? 0;
  const topScore = report?.results[0]?.score ?? null;
  const stats = corpusStatus.data?.stats ?? null;
  const statusMessage =
    corpusStatus.data?.message ?? (corpusStatus.error ? String(corpusStatus.error) : null);
  const connectionReady = corpusStatus.data?.reachable && corpusStatus.data.schemaReady;
  const connectionBadge = corpusStatus.isLoading
    ? { label: "Checking", variant: "outline" as const }
    : connectionReady
      ? { label: "Ready", variant: "default" as const }
      : corpusStatus.data?.reachable
        ? { label: "Needs migration", variant: "secondary" as const }
        : { label: "Not connected", variant: "destructive" as const };
  const sourceFilterLabel =
    sourceKind === "all"
      ? null
      : sourceOptions.find((option) => option.value === sourceKind)?.label;
  const contextMatch = transcriptContext.data?.match ?? selectedResult;
  const contextTimeRange = contextMatch
    ? formatTimeRange(contextMatch.startSeconds, contextMatch.endSeconds)
    : null;
  const contextDescription = contextMatch
    ? [sourceLabel(contextMatch.sourceKind), contextMatch.language, contextTimeRange]
        .filter(Boolean)
        .join(" / ")
    : "Transcript context";
  const themeModeSwitchProps = {
    mode: themeMode,
    onModeChange: setThemeMode,
  };

  function runSearch(nextSourceKind: SourceKind | "all" = sourceKind) {
    const trimmedQuery = query.trim();
    if (!trimmedQuery) {
      return;
    }
    setLastSearchSourceKind(nextSourceKind);
    searchMutation.mutate({
      databaseUrl: databaseUrl.trim() || undefined,
      query: trimmedQuery,
      mode,
      topK,
      sourceKind: nextSourceKind === "all" ? null : nextSourceKind,
    });
  }

  function submitSearch(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    runSearch();
  }

  function searchAllSources() {
    setSourceKind("all");
    runSearch("all");
  }

  function openContext(result: SearchResult) {
    setSelectedResult(result);
    setContextOpen(true);
  }

  function updateDatabaseUrl(value: string) {
    hasUserEditedDatabaseUrl.current = true;
    setDatabaseUrl(value);
  }

  function renderMatches() {
    if (searchMutation.isPending) {
      return <SearchState variant="loading" title="Searching transcripts..." />;
    }

    if (corpusStatus.data && !corpusStatus.data.configured) {
      return (
        <SearchState
          variant="error"
          title="Database URL required"
          description="Add a Postgres connection in Connection settings."
          actions={
            <Button type="button" variant="outline" onClick={() => setSettingsOpen(true)}>
              <Settings className="size-4" aria-hidden="true" />
              Connection settings
            </Button>
          }
        />
      );
    }

    if (corpusStatus.data?.configured && !corpusStatus.data.reachable) {
      return (
        <SearchState
          variant="error"
          title="Database unavailable"
          description={corpusStatus.data.message}
          actions={
            <Button type="button" variant="outline" onClick={() => setSettingsOpen(true)}>
              <Settings className="size-4" aria-hidden="true" />
              Connection settings
            </Button>
          }
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
          description={`No results for "${report.query}" in ${sourceDescription}.`}
          actions={
            lastSearchSourceKind === "all" ? null : (
              <Button type="button" variant="outline" onClick={searchAllSources}>
                Search all sources
              </Button>
            )
          }
        />
      );
    }

    return (
      <div className="grid gap-4">
        {sourceFilterLabel ? (
          <div>
            <Badge variant="outline">Source: {sourceFilterLabel}</Badge>
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
                      {highlightQueryText(result.text, report.query)}
                    </p>

                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        onClick={() => openContext(result)}
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
  }

  return (
    <PageShell background="muted" maxWidth="wide">
      <Navbar
        brand={<span className="font-semibold">YouTube Corpus</span>}
        groups={navigationGroups}
        activeItemId="search"
        actionSlot={
          <NavbarActions
            notificationMenu={{
              unreadCount: connectionReady ? 0 : 1,
              items: connectionReady
                ? [{ id: "database", title: "Corpus database ready", meta: "Ready" }]
                : [{ id: "database", title: "Connection needs attention", unread: true }],
            }}
            accountMenu={{
              user: { name: "Local Corpus", email: "desktop app", initials: "YC" },
              items: [{ id: "postgres", label: "Postgres corpus" }],
            }}
            themeModeSwitch={themeModeSwitchProps}
          />
        }
      />

      <PageHeader>
        <div className="grid min-w-0 gap-2">
          <PageTitle>Transcript search</PageTitle>
          <PageDescription>
            Query the local Postgres corpus with full-text, semantic, or hybrid retrieval.
          </PageDescription>
        </div>
        <PageActions>
          <Badge variant={connectionBadge.variant}>{connectionBadge.label}</Badge>
          <Button type="button" variant="outline" onClick={() => setSettingsOpen(true)}>
            <Settings className="size-4" aria-hidden="true" />
            Settings
          </Button>
          <ThemeModeSwitch {...themeModeSwitchProps} />
        </PageActions>
      </PageHeader>

      <Sheet open={settingsOpen} onOpenChange={setSettingsOpen}>
        <SheetContent side="right" className="w-full sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Connection</SheetTitle>
            <SheetDescription>Configure the local Postgres corpus connection.</SheetDescription>
          </SheetHeader>
          <div className="mt-6 grid gap-5">
            <label className="grid gap-2">
              <span className="text-sm font-medium">Database URL</span>
              <div className="flex min-w-0 items-center gap-2">
                <Database className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                <Input
                  value={databaseUrl}
                  onChange={(event) => updateDatabaseUrl(event.target.value)}
                  spellCheck={false}
                />
              </div>
            </label>

            <div className="grid gap-3 rounded-md border border-border p-4 text-sm">
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">Configured</span>
                <Badge variant={corpusStatus.data?.configured ? "default" : "secondary"}>
                  {corpusStatus.data?.configured ? "Yes" : "No"}
                </Badge>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">Reachable</span>
                <Badge variant={corpusStatus.data?.reachable ? "default" : "secondary"}>
                  {corpusStatus.data?.reachable ? "Yes" : "No"}
                </Badge>
              </div>
              <div className="flex items-center justify-between gap-3">
                <span className="text-muted-foreground">Schema ready</span>
                <Badge variant={corpusStatus.data?.schemaReady ? "default" : "secondary"}>
                  {corpusStatus.data?.schemaReady ? "Yes" : "No"}
                </Badge>
              </div>
            </div>

            {statusMessage ? (
              <Alert variant={connectionReady ? "default" : "destructive"}>
                <AlertDescription>{statusMessage}</AlertDescription>
              </Alert>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>

      <PageContent className="grid gap-4 lg:grid-cols-[minmax(360px,440px)_minmax(0,1fr)]">
        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Search controls</SurfaceTitle>
            <SurfaceDescription>Configure the query sent to the Tauri backend.</SurfaceDescription>
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
                <Textarea
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  rows={4}
                />
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

              <Button type="submit" disabled={searchMutation.isPending || query.trim() === ""}>
                {searchMutation.isPending ? <Spinner className="mr-2 size-4" /> : <Search />}
                Search transcripts
              </Button>
            </form>
          </SurfaceContent>
        </Surface>

        <div className="grid gap-4">
          <MetricStrip
            items={[
              {
                id: "videos",
                label: "Videos",
                value: metricValue(stats?.videos),
                delta: connectionReady ? "Indexed" : "Unavailable",
              },
              {
                id: "segments",
                label: "Segments",
                value: metricValue(stats?.segments),
                delta: connectionReady ? "Searchable" : "Unavailable",
              },
              {
                id: "streams",
                label: "Streams",
                value: metricValue(stats?.streams),
                delta: connectionReady ? "Transcript sources" : "Unavailable",
              },
              {
                id: "subscriptions",
                label: "Subscriptions",
                value: metricValue(stats?.subscriptions),
                delta:
                  stats === null
                    ? "Unavailable"
                    : `${stats.enabledSubscriptions.toLocaleString()} enabled`,
              },
              {
                id: "results",
                label: "Results",
                value: report ? String(resultCount) : "-",
                delta: report ? report.mode : "No query",
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
            <SurfaceContent className="mt-5">{renderMatches()}</SurfaceContent>
          </Surface>
        </div>
      </PageContent>

      <Drawer open={contextOpen} onOpenChange={setContextOpen}>
        <DrawerContent className="mx-auto max-h-[85vh] w-full max-w-3xl">
          <DrawerHeader className="border-b border-border text-left">
            <div className="flex min-w-0 flex-wrap items-start gap-3">
              <div className="min-w-0 flex-1">
                <DrawerTitle className="truncate">
                  {contextMatch?.title ?? "Transcript context"}
                </DrawerTitle>
                <DrawerDescription>{contextDescription}</DrawerDescription>
              </div>
              {contextMatch ? (
                <div className="flex flex-wrap items-center gap-2">
                  <Button asChild variant="outline" size="sm">
                    <a
                      href={buildTimestampUrl(contextMatch.sourceUrl, contextMatch.startSeconds)}
                      target="_blank"
                      rel="noreferrer"
                    >
                      <ExternalLink className="size-4" aria-hidden="true" />
                      Open source
                    </a>
                  </Button>
                  <CopyButton
                    value={contextMatch.text}
                    variant="outline"
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
              ) : null}
            </div>
          </DrawerHeader>

          <div className="overflow-y-auto px-4 py-4">
            {transcriptContext.isPending && contextOpen ? (
              <SearchState variant="loading" title="Loading transcript context..." />
            ) : transcriptContext.error ? (
              <SearchState
                variant="error"
                title="Context unavailable"
                description={String(transcriptContext.error)}
              />
            ) : transcriptContext.data ? (
              <ol className="grid gap-2">
                {transcriptContext.data.segments.map((segment) => {
                  const timeRange = formatTimeRange(segment.startSeconds, segment.endSeconds);
                  return (
                    <li
                      key={segment.segmentId}
                      className={[
                        "grid gap-2 rounded-md border px-3 py-3 text-sm",
                        segment.isMatch
                          ? "border-primary/50 bg-primary/5"
                          : "border-border bg-background",
                      ].join(" ")}
                    >
                      <div className="flex min-w-0 flex-wrap items-center gap-2">
                        {timeRange ? (
                          <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
                            <Clock className="size-3" aria-hidden="true" />
                            {timeRange}
                          </span>
                        ) : null}
                        {segment.isMatch ? <Badge variant="default">Match</Badge> : null}
                      </div>
                      <p className="leading-6 text-muted-foreground">{segment.text}</p>
                    </li>
                  );
                })}
              </ol>
            ) : (
              <SearchState
                title="Choose a result"
                description="Open context from a transcript match."
              />
            )}
          </div>
        </DrawerContent>
      </Drawer>
    </PageShell>
  );
}
