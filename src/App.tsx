import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
  Switch,
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
  type AddSourceKind,
  type AddSourceReport,
  type SearchMode,
  type SearchReport,
  type SearchResult,
  type SourceKind,
  type DownloadedFile,
  addSource,
  getCorpusStatus,
  getDownloadedFiles,
  getTranscriptContext,
  searchTranscripts,
} from "./app/tauri";

const defaultQuery = "first uploaded youtube video";
const themeStorageKey = "youtube-corpus-theme-mode";
type PageId = "add" | "search" | "files";

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

const addSourceKindOptions: Array<{ value: AddSourceKind; label: string }> = [
  { value: "video", label: "Video" },
  { value: "channel", label: "Channel" },
  { value: "playlist", label: "Playlist" },
];

const navigationGroups = [
  {
    id: "corpus",
    label: "Corpus",
    items: [
      { id: "add", label: "Add sources", href: "#add" },
      { id: "search", label: "Search", href: "#search" },
      { id: "files", label: "Downloaded files", href: "#files" },
    ],
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

function getInitialPage(): PageId {
  if (typeof window === "undefined") {
    return "search";
  }
  if (window.location.hash === "#add") {
    return "add";
  }
  return window.location.hash === "#files" ? "files" : "search";
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

function optionalNumber(value: string) {
  const trimmed = value.trim();
  if (trimmed === "") {
    return null;
  }
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function splitList(value: string) {
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function addSourceKindLabel(value: AddSourceKind) {
  return addSourceKindOptions.find((option) => option.value === value)?.label ?? value;
}

function statusBadgeVariant(status: string) {
  if (status === "indexed" || status === "completed") {
    return "default" as const;
  }
  if (status === "failed") {
    return "destructive" as const;
  }
  return "secondary" as const;
}

function formatDuration(seconds: number | null) {
  if (seconds === null) {
    return "Unknown duration";
  }
  return formatTimestamp(seconds) ?? "Unknown duration";
}

function formatDate(value: string | null) {
  if (!value) {
    return "Unknown date";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function videoTitle(file: DownloadedFile) {
  return file.title ?? file.youtubeId ?? file.sourceUrl;
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
  const queryClient = useQueryClient();
  const [activePage, setActivePage] = React.useState<PageId>(getInitialPage);
  const [themeMode, setThemeMode] = React.useState<ThemeMode>(getInitialThemeMode);
  const [databaseUrl, setDatabaseUrl] = React.useState("");
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [addSourceKind, setAddSourceKind] = React.useState<AddSourceKind>("video");
  const [sourceUrl, setSourceUrl] = React.useState("");
  const [sourceName, setSourceName] = React.useState("");
  const [workDir, setWorkDir] = React.useState("use-case-output/youtube-corpus");
  const [captionLanguages, setCaptionLanguages] = React.useState("en");
  const [captionsEnabled, setCaptionsEnabled] = React.useState(true);
  const [autoCaptionsEnabled, setAutoCaptionsEnabled] = React.useState(true);
  const [asrEnabled, setAsrEnabled] = React.useState(false);
  const [transcriberCommand, setTranscriberCommand] = React.useState("");
  const [transcriberArgs, setTranscriberArgs] = React.useState("");
  const [maxItems, setMaxItems] = React.useState("10");
  const [titleContains, setTitleContains] = React.useState("");
  const [titleExcludes, setTitleExcludes] = React.useState("");
  const [durationMin, setDurationMin] = React.useState("");
  const [durationMax, setDurationMax] = React.useState("");
  const [runMigrations, setRunMigrations] = React.useState(false);
  const [saveSubscription, setSaveSubscription] = React.useState(false);
  const [ingestNow, setIngestNow] = React.useState(true);
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

  React.useEffect(() => {
    const syncPage = () => setActivePage(getInitialPage());
    syncPage();
    window.addEventListener("hashchange", syncPage);
    return () => window.removeEventListener("hashchange", syncPage);
  }, []);

  React.useEffect(() => {
    if (addSourceKind === "video") {
      setSaveSubscription(false);
      setIngestNow(true);
      return;
    }
    setSaveSubscription(true);
    setMaxItems((value) => (value.trim() === "" ? "10" : value));
  }, [addSourceKind]);

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

  const addSourceMutation = useMutation({
    mutationFn: addSource,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["corpus-status"] }),
        queryClient.invalidateQueries({ queryKey: ["downloaded-files"] }),
      ]);
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
  const downloadedFiles = useQuery({
    queryKey: ["downloaded-files", databaseUrl.trim()],
    queryFn: () =>
      getDownloadedFiles({
        databaseUrl: databaseUrl.trim() || undefined,
        downloadedOnly: true,
        limit: 100,
      }),
    enabled: activePage === "files" && connectionReady,
    refetchOnWindowFocus: false,
  });
  const addReport = addSourceMutation.data ?? null;
  const canSaveSubscription = addSourceKind !== "video";
  const canSubmitSource =
    sourceUrl.trim() !== "" &&
    !addSourceMutation.isPending &&
    (ingestNow || (canSaveSubscription && saveSubscription));
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

  function submitSource(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canSubmitSource) {
      return;
    }

    const parsedMaxItems = optionalNumber(maxItems);
    addSourceMutation.mutate({
      databaseUrl: databaseUrl.trim() || undefined,
      sourceKind: addSourceKind,
      sourceUrl: sourceUrl.trim(),
      name: sourceName.trim() || null,
      workDir: workDir.trim() || undefined,
      captionLanguages: splitList(captionLanguages),
      captionsEnabled,
      autoCaptionsEnabled,
      asrEnabled,
      transcriberCommand: transcriberCommand.trim() || null,
      transcriberArgs: splitList(transcriberArgs),
      maxItems: parsedMaxItems === null ? null : Math.max(1, Math.floor(parsedMaxItems)),
      titleContains: titleContains.trim() || null,
      titleExcludes: splitList(titleExcludes),
      durationMin: optionalNumber(durationMin),
      durationMax: optionalNumber(durationMax),
      migrate: runMigrations,
      subscribe: canSaveSubscription && saveSubscription,
      ingestNow,
    });
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

  function renderAddReport(report: AddSourceReport | null) {
    if (addSourceMutation.isPending) {
      return <SearchState variant="loading" title="Downloading and parsing..." />;
    }

    if (addSourceMutation.error) {
      return (
        <SearchState
          variant="error"
          title="Add failed"
          description={String(addSourceMutation.error)}
        />
      );
    }

    if (!report) {
      return (
        <SearchState
          title="No source added yet"
          description="Submit a YouTube video, channel, or playlist to create corpus entries."
        />
      );
    }

    const ingest = report.ingest;
    return (
      <div className="grid gap-4">
        <div className="grid gap-3 rounded-md border border-border bg-background px-4 py-4">
          <div className="flex min-w-0 flex-wrap items-center gap-3">
            <Badge variant="outline">{addSourceKindLabel(report.sourceKind)}</Badge>
            {report.subscription ? <Badge variant="secondary">Subscription saved</Badge> : null}
            {ingest ? <Badge variant="default">Ingest completed</Badge> : null}
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{report.sourceUrl}</p>
            {report.subscription ? (
              <p className="mt-1 text-xs text-muted-foreground">
                Subscription {report.subscription.name ?? report.subscription.id}
              </p>
            ) : null}
          </div>
        </div>

        {ingest ? (
          <>
            <MetricStrip
              items={[
                {
                  id: "seen",
                  label: "Seen",
                  value: ingest.videosSeen.toLocaleString(),
                  delta: "Discovered",
                },
                {
                  id: "indexed",
                  label: "Indexed",
                  value: ingest.videosIndexed.toLocaleString(),
                  delta: "Videos",
                },
                {
                  id: "segments",
                  label: "Segments",
                  value: ingest.segmentsIndexed.toLocaleString(),
                  delta: "Parsed",
                },
              ]}
            />
            <div className="grid gap-3">
              {ingest.items.slice(0, 12).map((item) => (
                <article
                  key={item.sourceUrl}
                  className="grid gap-3 rounded-md border border-border bg-background px-4 py-4"
                >
                  <div className="flex min-w-0 flex-wrap items-start gap-3">
                    <FileText
                      className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                      aria-hidden="true"
                    />
                    <div className="min-w-0 flex-1">
                      <h2 className="truncate text-sm font-semibold">
                        {item.title ?? item.sourceUrl}
                      </h2>
                      <p className="mt-1 truncate text-xs text-muted-foreground">
                        {item.sourceUrl}
                      </p>
                    </div>
                    <Badge variant={statusBadgeVariant(item.status)}>{item.status}</Badge>
                  </div>
                  <div className="flex min-w-0 flex-wrap items-center gap-2 text-sm text-muted-foreground">
                    <span>{item.streamsIndexed.toLocaleString()} streams</span>
                    <span>{item.segmentsIndexed.toLocaleString()} segments</span>
                    {item.message ? <span className="text-destructive">{item.message}</span> : null}
                  </div>
                </article>
              ))}
              {ingest.items.length > 12 ? (
                <p className="text-sm text-muted-foreground">
                  {ingest.items.length - 12} more items were included in the run.
                </p>
              ) : null}
            </div>
          </>
        ) : (
          <SearchState title="Subscription saved" description="No ingest run was requested." />
        )}
      </div>
    );
  }

  function renderAddSource() {
    const showCollectionOptions = addSourceKind !== "video";
    return (
      <PageContent className="grid gap-4 lg:grid-cols-[minmax(360px,460px)_minmax(0,1fr)]">
        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Add source</SurfaceTitle>
            <SurfaceDescription>
              Create downloads and parsed transcript rows from YouTube sources.
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent>
            <form className="mt-5 grid gap-5" onSubmit={submitSource}>
              {!connectionReady && !runMigrations && !corpusStatus.isLoading ? (
                <Alert variant="destructive">
                  <AlertDescription>
                    {statusMessage ?? "Corpus database is not ready for ingestion."}
                  </AlertDescription>
                </Alert>
              ) : null}

              <div className="grid gap-2">
                <span className="text-sm font-medium">Source type</span>
                <Tabs
                  value={addSourceKind}
                  onValueChange={(value) => setAddSourceKind(value as AddSourceKind)}
                >
                  <TabsList className="grid w-full grid-cols-3">
                    {addSourceKindOptions.map((option) => (
                      <TabsTrigger key={option.value} value={option.value}>
                        {option.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              </div>

              <label className="grid gap-2">
                <span className="text-sm font-medium">YouTube URL</span>
                <Input
                  value={sourceUrl}
                  onChange={(event) => setSourceUrl(event.target.value)}
                  placeholder="https://www.youtube.com/watch?v=..."
                  spellCheck={false}
                />
              </label>

              {showCollectionOptions ? (
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Name</span>
                  <Input
                    value={sourceName}
                    onChange={(event) => setSourceName(event.target.value)}
                    placeholder="Optional display name"
                  />
                </label>
              ) : null}

              <div className="grid gap-4 sm:grid-cols-2">
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Work directory</span>
                  <Input
                    value={workDir}
                    onChange={(event) => setWorkDir(event.target.value)}
                    spellCheck={false}
                  />
                </label>
                {showCollectionOptions ? (
                  <label className="grid gap-2">
                    <span className="text-sm font-medium">Max items</span>
                    <Input
                      value={maxItems}
                      onChange={(event) => setMaxItems(event.target.value)}
                      inputMode="numeric"
                    />
                  </label>
                ) : null}
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Caption languages</span>
                  <Input
                    value={captionLanguages}
                    onChange={(event) => setCaptionLanguages(event.target.value)}
                    placeholder="en, de"
                    spellCheck={false}
                  />
                </label>
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Transcriber command</span>
                  <Input
                    value={transcriberCommand}
                    onChange={(event) => setTranscriberCommand(event.target.value)}
                    placeholder="whisper"
                    spellCheck={false}
                  />
                </label>
              </div>

              <label className="grid gap-2">
                <span className="text-sm font-medium">Transcriber args</span>
                <Input
                  value={transcriberArgs}
                  onChange={(event) => setTranscriberArgs(event.target.value)}
                  placeholder="--model small"
                  spellCheck={false}
                />
              </label>

              <div className="grid gap-4 sm:grid-cols-2">
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Title contains</span>
                  <Input
                    value={titleContains}
                    onChange={(event) => setTitleContains(event.target.value)}
                  />
                </label>
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Title excludes</span>
                  <Input
                    value={titleExcludes}
                    onChange={(event) => setTitleExcludes(event.target.value)}
                    placeholder="shorts, trailer"
                  />
                </label>
              </div>

              <div className="grid gap-4 sm:grid-cols-2">
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Min duration</span>
                  <Input
                    value={durationMin}
                    onChange={(event) => setDurationMin(event.target.value)}
                    inputMode="decimal"
                  />
                </label>
                <label className="grid gap-2">
                  <span className="text-sm font-medium">Max duration</span>
                  <Input
                    value={durationMax}
                    onChange={(event) => setDurationMax(event.target.value)}
                    inputMode="decimal"
                  />
                </label>
              </div>

              <div className="grid gap-3 rounded-md border border-border p-4">
                {[
                  {
                    id: "captions",
                    label: "Captions",
                    checked: captionsEnabled,
                    onCheckedChange: setCaptionsEnabled,
                  },
                  {
                    id: "auto-captions",
                    label: "Auto captions",
                    checked: autoCaptionsEnabled,
                    onCheckedChange: setAutoCaptionsEnabled,
                  },
                  {
                    id: "asr",
                    label: "ASR",
                    checked: asrEnabled,
                    onCheckedChange: setAsrEnabled,
                  },
                  {
                    id: "migrate",
                    label: "Run migrations",
                    checked: runMigrations,
                    onCheckedChange: setRunMigrations,
                  },
                  ...(showCollectionOptions
                    ? [
                        {
                          id: "subscription",
                          label: "Save subscription",
                          checked: saveSubscription,
                          onCheckedChange: setSaveSubscription,
                        },
                      ]
                    : []),
                  {
                    id: "ingest-now",
                    label: "Download now",
                    checked: ingestNow,
                    onCheckedChange: setIngestNow,
                  },
                ].map((item) => (
                  <label key={item.id} className="flex items-center justify-between gap-3">
                    <span className="text-sm font-medium">{item.label}</span>
                    <Switch
                      checked={item.checked}
                      onCheckedChange={item.onCheckedChange}
                      disabled={item.id === "ingest-now" && addSourceKind === "video"}
                    />
                  </label>
                ))}
              </div>

              <Button type="submit" disabled={!canSubmitSource}>
                {addSourceMutation.isPending ? (
                  <Spinner className="mr-2 size-4" />
                ) : (
                  <FileText className="size-4" aria-hidden="true" />
                )}
                {addSourceMutation.isPending ? "Adding source..." : "Add source"}
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
                id: "subscriptions",
                label: "Subscriptions",
                value: metricValue(stats?.subscriptions),
                delta:
                  stats === null
                    ? "Unavailable"
                    : `${stats.enabledSubscriptions.toLocaleString()} enabled`,
              },
            ]}
          />

          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Latest add</SurfaceTitle>
              <SurfaceDescription>
                Result returned by the Rust ingest and subscription pipeline.
              </SurfaceDescription>
            </SurfaceHeader>
            <SurfaceContent className="mt-5">{renderAddReport(addReport)}</SurfaceContent>
          </Surface>
        </div>
      </PageContent>
    );
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

  function renderDownloadedFiles() {
    if (!connectionReady && !corpusStatus.isLoading) {
      return (
        <SearchState
          variant="error"
          title="Database unavailable"
          description={statusMessage ?? "Corpus database is not ready."}
          actions={
            <Button type="button" variant="outline" onClick={() => setSettingsOpen(true)}>
              <Settings className="size-4" aria-hidden="true" />
              Connection settings
            </Button>
          }
        />
      );
    }

    if (corpusStatus.isLoading || downloadedFiles.isPending) {
      return <SearchState variant="loading" title="Loading downloaded files..." />;
    }

    if (downloadedFiles.error) {
      return (
        <SearchState
          variant="error"
          title="Downloaded files unavailable"
          description={String(downloadedFiles.error)}
        />
      );
    }

    const files = downloadedFiles.data ?? [];
    if (files.length === 0) {
      return (
        <SearchState
          title="No downloaded files"
          description="Ingest videos or subscriptions to create local media or caption files."
        />
      );
    }

    return (
      <div className="grid gap-3">
        {files.map((file) => {
          const transcriptKinds = file.transcriptSourceKinds.map(sourceLabel);
          return (
            <article
              key={file.id}
              className="grid gap-3 rounded-md border border-border bg-background px-4 py-4"
            >
              <div className="flex min-w-0 flex-wrap items-start gap-3">
                <FileText
                  className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                  aria-hidden="true"
                />
                <div className="min-w-0 flex-1">
                  <h2 className="truncate text-sm font-semibold">{videoTitle(file)}</h2>
                  <p className="mt-1 truncate text-xs text-muted-foreground">{file.sourceUrl}</p>
                </div>
                <Button asChild variant="ghost" size="sm">
                  <a href={file.sourceUrl} target="_blank" rel="noreferrer">
                    <ExternalLink className="size-4" aria-hidden="true" />
                    Source
                  </a>
                </Button>
              </div>

              <div className="flex min-w-0 flex-wrap items-center gap-2">
                {file.mediaDownloaded ? <Badge variant="default">Media</Badge> : null}
                {file.captionFilesDownloaded ? <Badge variant="secondary">Captions</Badge> : null}
                {file.parsed ? <Badge variant="outline">Parsed</Badge> : null}
                {transcriptKinds.map((kind) => (
                  <Badge key={kind} variant="outline">
                    {kind}
                  </Badge>
                ))}
              </div>

              <div className="grid gap-2 text-sm text-muted-foreground md:grid-cols-2">
                <div className="min-w-0">
                  <span className="font-medium text-foreground">Local file</span>
                  <p className="mt-1 truncate">{file.localVideoPath ?? "No media file recorded"}</p>
                </div>
                <div className="min-w-0">
                  <span className="font-medium text-foreground">Transcript</span>
                  <p className="mt-1">
                    {file.transcriptStreams.toLocaleString()} streams /{" "}
                    {file.transcriptSegments.toLocaleString()} segments
                  </p>
                </div>
                <div>
                  <span className="font-medium text-foreground">Duration</span>
                  <p className="mt-1">{formatDuration(file.durationSeconds)}</p>
                </div>
                <div>
                  <span className="font-medium text-foreground">Updated</span>
                  <p className="mt-1">{formatDate(file.updatedAt)}</p>
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                {file.localVideoPath ? (
                  <CopyButton
                    value={file.localVideoPath}
                    variant="outline"
                    size="sm"
                    idleLabel={
                      <>
                        <CopyIcon className="size-4" aria-hidden="true" />
                        Copy path
                      </>
                    }
                    copiedLabel="Copied"
                  />
                ) : null}
                {file.subscriptionNames.map((name) => (
                  <Badge key={name} variant="outline">
                    {name}
                  </Badge>
                ))}
              </div>
            </article>
          );
        })}
      </div>
    );
  }

  return (
    <PageShell background="muted" maxWidth="wide">
      <Navbar
        brand={<span className="font-semibold">YouTube Corpus</span>}
        groups={navigationGroups}
        activeItemId={activePage}
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
          <PageTitle>
            {activePage === "add"
              ? "Add sources"
              : activePage === "files"
                ? "Downloaded files"
                : "Transcript search"}
          </PageTitle>
          <PageDescription>
            {activePage === "add"
              ? "Download and parse videos, playlists, and channels into the local corpus."
              : activePage === "files"
                ? "Review local media and caption files recorded in the corpus."
                : "Query the local Postgres corpus with full-text, semantic, or hybrid retrieval."}
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

      {activePage === "add" ? (
        renderAddSource()
      ) : activePage === "search" ? (
        <PageContent className="grid gap-4 lg:grid-cols-[minmax(360px,440px)_minmax(0,1fr)]">
          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Search controls</SurfaceTitle>
              <SurfaceDescription>
                Configure the query sent to the Tauri backend.
              </SurfaceDescription>
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
      ) : (
        <PageContent className="grid gap-4">
          <MetricStrip
            items={[
              {
                id: "downloaded",
                label: "Downloaded",
                value: downloadedFiles.data ? downloadedFiles.data.length.toLocaleString() : "-",
                delta: "Media or captions",
              },
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
                id: "subscriptions",
                label: "Subscriptions",
                value: metricValue(stats?.subscriptions),
                delta:
                  stats === null
                    ? "Unavailable"
                    : `${stats.enabledSubscriptions.toLocaleString()} enabled`,
              },
            ]}
          />

          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Files</SurfaceTitle>
              <SurfaceDescription>
                Videos with local media paths or downloaded transcript source files.
              </SurfaceDescription>
            </SurfaceHeader>
            <SurfaceContent className="mt-5">{renderDownloadedFiles()}</SurfaceContent>
          </Surface>
        </PageContent>
      )}

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
