import { useMutation, useQuery } from "@tanstack/react-query";
import { Database, ExternalLink, Search } from "lucide-react";
import * as React from "react";

import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  EmptyState,
  Input,
  MetricStrip,
  NativeSelect,
  Spinner,
  Tabs,
  TabsList,
  TabsTrigger,
  Textarea,
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
  getDatabaseStatus,
  searchTranscripts,
} from "./app/tauri";

const defaultQuery = "first uploaded youtube video";
const defaultDatabaseUrl = "postgres://postgres:postgres@localhost:5432/youtube_corpus";

const modeOptions: Array<{ value: SearchMode; label: string }> = [
  { value: "hybrid", label: "Hybrid" },
  { value: "fts", label: "FTS" },
  { value: "semantic", label: "Semantic" },
];

const navigationGroups = [
  {
    id: "corpus",
    label: "Corpus",
    items: [{ id: "search", label: "Search", href: "#search", active: true }],
  },
];

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

function sourceLabel(sourceKind: string) {
  return sourceKind.replaceAll("_", " ");
}

function scoreLabel(score: number) {
  return Number.isFinite(score) ? score.toFixed(3) : "0.000";
}

export default function App() {
  const [databaseUrl, setDatabaseUrl] = React.useState(defaultDatabaseUrl);
  const [query, setQuery] = React.useState(defaultQuery);
  const [mode, setMode] = React.useState<SearchMode>("hybrid");
  const [topK, setTopK] = React.useState(5);
  const [lastReport, setLastReport] = React.useState<SearchReport | null>(null);

  const databaseStatus = useQuery({
    queryKey: ["database-status"],
    queryFn: getDatabaseStatus,
  });

  React.useEffect(() => {
    const envDatabaseUrl = databaseStatus.data?.databaseUrl;
    if (envDatabaseUrl) {
      setDatabaseUrl(envDatabaseUrl);
    }
  }, [databaseStatus.data?.databaseUrl]);

  const searchMutation = useMutation({
    mutationFn: searchTranscripts,
    onSuccess: (report) => {
      setLastReport(report);
    },
  });

  const report = searchMutation.data ?? lastReport;
  const resultCount = report?.results.length ?? 0;
  const topScore = report?.results[0]?.score ?? null;

  function submitSearch(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedQuery = query.trim();
    if (!trimmedQuery) {
      return;
    }
    searchMutation.mutate({
      databaseUrl: databaseUrl.trim() || undefined,
      query: trimmedQuery,
      mode,
      topK,
    });
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
              unreadCount: databaseStatus.data?.configured ? 0 : 1,
              items: databaseStatus.data?.configured
                ? [{ id: "database", title: "DATABASE_URL detected", meta: "Ready" }]
                : [{ id: "database", title: "Using editable database URL", unread: true }],
            }}
            accountMenu={{
              user: { name: "Local Corpus", email: "desktop app", initials: "YC" },
              items: [{ id: "postgres", label: "Postgres corpus" }],
            }}
            themeModeSwitch
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
          <ThemeModeSwitch />
        </PageActions>
      </PageHeader>

      <PageContent className="grid gap-4 lg:grid-cols-[minmax(360px,440px)_minmax(0,1fr)]">
        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Search controls</SurfaceTitle>
            <SurfaceDescription>Configure the query sent to the Tauri backend.</SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent>
            <form className="mt-5 grid gap-5" onSubmit={submitSearch}>
              <label className="grid gap-2">
                <span className="text-sm font-medium">Database URL</span>
                <div className="flex min-w-0 items-center gap-2">
                  <Database className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <Input
                    value={databaseUrl}
                    onChange={(event) => setDatabaseUrl(event.target.value)}
                    spellCheck={false}
                  />
                </div>
              </label>

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

              {searchMutation.error ? (
                <Alert variant="destructive">
                  <AlertDescription>{String(searchMutation.error)}</AlertDescription>
                </Alert>
              ) : null}

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
                id: "results",
                label: "Results",
                value: String(resultCount),
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
            <SurfaceContent className="mt-5">
              {searchMutation.isPending ? (
                <div className="flex min-h-64 items-center justify-center">
                  <Spinner className="size-6" />
                </div>
              ) : report && report.results.length > 0 ? (
                <div className="grid gap-3">
                  {report.results.map((result) => {
                    const timestamp = formatTimestamp(result.startSeconds);
                    const url = timestamp
                      ? `${result.sourceUrl}${result.sourceUrl.includes("?") ? "&" : "?"}t=${Math.floor(
                          result.startSeconds ?? 0,
                        )}`
                      : result.sourceUrl;
                    return (
                      <article
                        key={result.segmentId}
                        className="grid gap-3 rounded-md border border-border bg-background p-4"
                      >
                        <div className="flex min-w-0 flex-wrap items-center gap-2">
                          <Badge variant="secondary">{sourceLabel(result.sourceKind)}</Badge>
                          {result.language ? (
                            <Badge variant="outline">{result.language}</Badge>
                          ) : null}
                          {timestamp ? <Badge variant="outline">{timestamp}</Badge> : null}
                          <span className="ml-auto text-sm text-muted-foreground">
                            {scoreLabel(result.score)}
                          </span>
                        </div>
                        <div className="grid gap-1">
                          <h2 className="text-base font-semibold">
                            {result.title ?? result.videoId}
                          </h2>
                          <p className="text-sm leading-6 text-muted-foreground">{result.text}</p>
                        </div>
                        <a
                          className="inline-flex w-fit items-center gap-2 text-sm font-medium text-primary"
                          href={url}
                          target="_blank"
                          rel="noreferrer"
                        >
                          Open source
                          <ExternalLink className="size-4" aria-hidden="true" />
                        </a>
                      </article>
                    );
                  })}
                </div>
              ) : (
                <EmptyState>No transcript matches yet.</EmptyState>
              )}
            </SurfaceContent>
          </Surface>
        </div>
      </PageContent>
    </PageShell>
  );
}
