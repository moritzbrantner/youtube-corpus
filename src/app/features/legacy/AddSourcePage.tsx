import { Clock, FileText, RotateCcw } from "lucide-react";

import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Input,
  MetricStrip,
  Spinner,
  Switch,
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

import type { AddSourceKind, AddSourceReport, IngestRunStatus } from "../../api";
import {
  addSourceKindLabel,
  addSourceKindOptions,
  formatDate,
  ingestReportFromRun,
  ingestRunMessage,
  segmentsMetric,
  statusBadgeVariant,
  subscriptionsMetric,
  videosMetric,
} from "./format";
import { SearchState } from "./SearchState";
import type { AddSourceForm } from "./use-add-source-form";
import type { CorpusConnection } from "./use-corpus-connection";

function AddReport({
  isPending,
  error,
  report,
  run,
}: {
  isPending: boolean;
  error: unknown;
  report: AddSourceReport | null;
  run: IngestRunStatus | null;
}) {
  if (isPending) {
    return <SearchState variant="loading" title="Starting ingest..." />;
  }

  if (error) {
    return <SearchState variant="error" title="Add failed" description={String(error)} />;
  }

  if (!report) {
    return (
      <SearchState
        title="No source added yet"
        description="Submit a YouTube video, channel, or playlist to create corpus entries."
      />
    );
  }

  const ingest = report.ingest ?? ingestReportFromRun(run);
  const runMessage = ingestRunMessage(run);
  return (
    <div className="grid gap-4">
      <div className="grid gap-3 rounded-md border border-border bg-background px-4 py-4">
        <div className="flex min-w-0 flex-wrap items-center gap-3">
          <Badge variant="outline">{addSourceKindLabel(report.sourceKind)}</Badge>
          {report.subscription ? <Badge variant="secondary">Subscription saved</Badge> : null}
          {ingest ? <Badge variant="default">Ingest completed</Badge> : null}
          {run?.status === "running" ? <Badge variant="secondary">Ingest running</Badge> : null}
          {run?.status === "failed" ? <Badge variant="destructive">Ingest failed</Badge> : null}
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

      {run ? (
        <MetricStrip
          items={[
            {
              id: "run-status",
              label: "Run status",
              value: run.status,
              delta: run.createdAt,
            },
            {
              id: "run-indexed",
              label: "Indexed",
              value: run.videosIndexed.toLocaleString(),
              delta: "Videos",
            },
            {
              id: "run-segments",
              label: "Segments",
              value: run.segmentsIndexed.toLocaleString(),
              delta: "Parsed",
            },
          ]}
        />
      ) : null}

      {run?.status === "running" ? (
        <SearchState variant="loading" title="Ingest running..." />
      ) : null}

      {run?.status === "failed" ? (
        <SearchState variant="error" title="Ingest failed" description={runMessage} />
      ) : null}

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
                    <p className="mt-1 truncate text-xs text-muted-foreground">{item.sourceUrl}</p>
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
      ) : run ? null : (
        <SearchState title="Subscription saved" description="No ingest run was requested." />
      )}
    </div>
  );
}

function RecentRuns({
  connection,
  recentIngestRuns,
}: {
  connection: CorpusConnection;
  recentIngestRuns: AddSourceForm["recentIngestRuns"];
}) {
  const { corpusStatus, connectionReady, statusMessage } = connection;
  if (!connectionReady && !corpusStatus.isLoading) {
    return (
      <SearchState
        variant="error"
        title="Runs unavailable"
        description={statusMessage ?? "Corpus database is not ready."}
      />
    );
  }

  if (recentIngestRuns.isPending) {
    return <SearchState variant="loading" title="Loading runs..." />;
  }

  if (recentIngestRuns.error) {
    return (
      <SearchState
        variant="error"
        title="Runs unavailable"
        description={String(recentIngestRuns.error)}
      />
    );
  }

  const runs = recentIngestRuns.data ?? [];
  if (runs.length === 0) {
    return <SearchState title="No ingest runs" description="Background runs will appear here." />;
  }

  return (
    <div className="grid gap-3">
      <div className="flex justify-end">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void recentIngestRuns.refetch()}
        >
          <RotateCcw className="size-4" aria-hidden="true" />
          Refresh
        </Button>
      </div>
      {runs.map((run) => {
        const status = run.job?.status ?? run.status;
        const ingest = ingestReportFromRun(run);
        return (
          <article
            key={run.id}
            className="grid gap-3 rounded-md border border-border bg-background px-4 py-4"
          >
            <div className="flex min-w-0 flex-wrap items-start gap-3">
              <Clock className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <h2 className="truncate text-sm font-semibold">
                  {run.sourceUrl ?? run.job?.sourceUrl ?? run.id}
                </h2>
                <p className="mt-1 text-xs text-muted-foreground">{formatDate(run.createdAt)}</p>
              </div>
              <Badge variant={statusBadgeVariant(status)}>{status}</Badge>
            </div>
            <div className="flex min-w-0 flex-wrap items-center gap-3 text-sm text-muted-foreground">
              <span>{run.videosIndexed.toLocaleString()} videos</span>
              <span>{run.segmentsIndexed.toLocaleString()} segments</span>
              {ingest ? <span>{ingest.videosSeen.toLocaleString()} seen</span> : null}
              {run.job?.failure?.message ? (
                <span className="text-destructive">{run.job.failure.message}</span>
              ) : null}
            </div>
          </article>
        );
      })}
    </div>
  );
}

export function AddSourcePage({
  form,
  connection,
}: {
  form: AddSourceForm;
  connection: CorpusConnection;
}) {
  const {
    addSourceKind,
    setAddSourceKind,
    sourceUrl,
    setSourceUrl,
    sourceName,
    setSourceName,
    workDir,
    setWorkDir,
    captionLanguages,
    setCaptionLanguages,
    captionsEnabled,
    setCaptionsEnabled,
    autoCaptionsEnabled,
    setAutoCaptionsEnabled,
    ytDlpArgs,
    setYtDlpArgs,
    ytDlpTimeoutSeconds,
    setYtDlpTimeoutSeconds,
    asrEnabled,
    setAsrEnabled,
    transcriberCommand,
    setTranscriberCommand,
    transcriberArgs,
    setTranscriberArgs,
    transcriberTimeoutSeconds,
    setTranscriberTimeoutSeconds,
    maxItems,
    setMaxItems,
    titleContains,
    setTitleContains,
    titleExcludes,
    setTitleExcludes,
    durationMin,
    setDurationMin,
    durationMax,
    setDurationMax,
    runMigrations,
    setRunMigrations,
    saveSubscription,
    setSaveSubscription,
    ingestNow,
    setIngestNow,
    asyncIngest,
    setAsyncIngest,
    addSourceMutation,
    recentIngestRuns,
    addReport,
    addIngestRun,
    canSubmitSource,
    submitSource,
  } = form;
  const { corpusStatus, stats, statusMessage, connectionReady } = connection;
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
              <span className="text-sm font-medium">yt-dlp args</span>
              <Textarea
                value={ytDlpArgs}
                onChange={(event) => setYtDlpArgs(event.target.value)}
                rows={3}
                placeholder={"--cookies-from-browser\nfirefox"}
                spellCheck={false}
              />
            </label>

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
                <span className="text-sm font-medium">yt-dlp timeout</span>
                <Input
                  value={ytDlpTimeoutSeconds}
                  onChange={(event) => setYtDlpTimeoutSeconds(event.target.value)}
                  inputMode="numeric"
                  placeholder="Seconds"
                />
              </label>
              <label className="grid gap-2">
                <span className="text-sm font-medium">Transcriber timeout</span>
                <Input
                  value={transcriberTimeoutSeconds}
                  onChange={(event) => setTranscriberTimeoutSeconds(event.target.value)}
                  inputMode="numeric"
                  placeholder="Seconds"
                />
              </label>
            </div>

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
                {
                  id: "async-ingest",
                  label: "Run in background",
                  checked: asyncIngest,
                  onCheckedChange: setAsyncIngest,
                },
              ].map((item) => (
                <label key={item.id} className="flex items-center justify-between gap-3">
                  <span className="text-sm font-medium">{item.label}</span>
                  <Switch
                    checked={item.checked}
                    onCheckedChange={item.onCheckedChange}
                    disabled={
                      (item.id === "ingest-now" && addSourceKind === "video") ||
                      (item.id === "async-ingest" && !ingestNow)
                    }
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
            videosMetric(stats, connectionReady),
            segmentsMetric(stats, connectionReady),
            subscriptionsMetric(stats),
          ]}
        />

        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Latest add</SurfaceTitle>
            <SurfaceDescription>
              Result returned by the Rust ingest and subscription pipeline.
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="mt-5">
            <AddReport
              isPending={addSourceMutation.isPending}
              error={addSourceMutation.error}
              report={addReport}
              run={addIngestRun}
            />
          </SurfaceContent>
        </Surface>

        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Recent runs</SurfaceTitle>
            <SurfaceDescription>Latest background ingest activity.</SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="mt-5">
            <RecentRuns connection={connection} recentIngestRuns={recentIngestRuns} />
          </SurfaceContent>
        </Surface>
      </div>
    </PageContent>
  );
}
