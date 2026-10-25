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
  Switch,
} from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import { LabelAssignments, useCorpusLabels, useInvalidateLabels } from "./CorpusLabels";
import {
  addCorpusSource,
  assignSourceLabel,
  approveSourcePending,
  checkCorpusSource,
  listSourcePending,
  setSourcePolicy,
  type ProcessingPolicy,
  removeSourceLabel,
  listCorpusSources,
  setCorpusSourceEnabled,
  setCorpusSourceInterval,
  controlBackfill,
  listBackfillEntries,
  type AddCorpusSourceInput,
  type BackfillAction,
  type BackfillProgress,
  type CorpusSource,
  type HistoryScopeInput,
  type HistoryScopeKind,
} from "./api";
import { corpusKeys } from "./query-keys";

type CorpusSourcesProps = {
  corpusId: string;
};

export function CorpusSources({ corpusId }: CorpusSourcesProps) {
  const queryClient = useQueryClient();
  const [sourceKind, setSourceKind] = React.useState<"channel" | "playlist">("channel");
  const [sourceUrl, setSourceUrl] = React.useState("");
  const [name, setName] = React.useState("");
  const [languages, setLanguages] = React.useState("en");
  const [maxItems, setMaxItems] = React.useState("20");
  const [monitor, setMonitor] = React.useState(true);
  // Full available history is preselected (Atlas 1.12); nothing runs until the source is added.
  const [historyScope, setHistoryScope] = React.useState<HistoryScopeKind>("full");
  const [recentDays, setRecentDays] = React.useState("30");
  const [limitCount, setLimitCount] = React.useState("20");
  const [asrEnabled, setAsrEnabled] = React.useState(false);
  const [policy, setPolicy] = React.useState<ProcessingPolicy>("automatic");
  const [checkInterval, setCheckInterval] = React.useState<IntervalValue>(
    intervalValue(DEFAULT_CHECK_INTERVAL_SECONDS),
  );

  const sourcesQuery = useQuery({
    queryKey: corpusKeys.sources(corpusId),
    queryFn: () => listCorpusSources(corpusId),
    // Background work and scheduled checks continue server-side; refresh their progress
    // quickly while any is active and slowly otherwise, so scheduled check results appear.
    refetchInterval: (query) => (query.state.data?.some(hasActiveWork) ? 3000 : 30000),
  });
  const labelsQuery = useCorpusLabels(corpusId);
  const invalidateLabels = useInvalidateLabels(corpusId);
  const labelMutation = useMutation({
    mutationFn: ({
      sourceId,
      labelId,
      assigned,
    }: {
      sourceId: string;
      labelId: string;
      assigned: boolean;
    }) =>
      assigned
        ? assignSourceLabel(corpusId, sourceId, labelId)
        : removeSourceLabel(corpusId, sourceId, labelId),
    onSettled: invalidateLabels,
  });

  async function invalidateCorpus() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: corpusKeys.all }),
      queryClient.invalidateQueries({ queryKey: corpusKeys.sources(corpusId) }),
      queryClient.invalidateQueries({
        queryKey: ["corpora", corpusId, "videos"],
      }),
      queryClient.invalidateQueries({
        queryKey: ["corpora", corpusId, "search"],
      }),
    ]);
  }

  const addMutation = useMutation({
    mutationFn: (input: AddCorpusSourceInput) => addCorpusSource(corpusId, input),
    onSuccess: async () => {
      setSourceUrl("");
      setName("");
      await invalidateCorpus();
    },
  });
  const checkMutation = useMutation({
    mutationFn: (sourceId: string) => checkCorpusSource(corpusId, sourceId),
    onSuccess: invalidateCorpus,
  });
  const enabledMutation = useMutation({
    mutationFn: ({ sourceId, enabled }: { sourceId: string; enabled: boolean }) =>
      setCorpusSourceEnabled(corpusId, sourceId, enabled),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: corpusKeys.all }),
        queryClient.invalidateQueries({
          queryKey: corpusKeys.sources(corpusId),
        }),
      ]);
    },
  });

  const history = historyInput(historyScope, recentDays, limitCount);
  const intervalSeconds = intervalToSeconds(checkInterval);

  function submit(event: React.FormEvent) {
    event.preventDefault();
    const url = sourceUrl.trim();
    if (!url || (monitor && (!history || intervalSeconds === null))) {
      return;
    }
    const parsedMaxItems = Number(maxItems);
    addMutation.mutate({
      sourceKind,
      sourceUrl: url,
      name: name.trim() || null,
      monitor,
      ingestNow: true,
      history: monitor && history ? history : undefined,
      captionLanguages: languages
        .split(",")
        .map((language) => language.trim())
        .filter(Boolean),
      autoCaptionsEnabled: true,
      asrEnabled,
      maxItems: Number.isFinite(parsedMaxItems) && parsedMaxItems > 0 ? parsedMaxItems : null,
      processingPolicy: policy,
      checkIntervalSeconds: monitor ? intervalSeconds : null,
    });
  }

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Sources</SurfaceTitle>
        <SurfaceDescription>
          Add a channel or playlist once, or monitor it so the running server checks it for newly
          published videos on its own schedule.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-6">
        <form className="grid gap-4" onSubmit={submit}>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="grid gap-2">
              <span className="text-sm font-medium">Source type</span>
              <NativeSelect
                value={sourceKind}
                onChange={(event) => setSourceKind(event.target.value as "channel" | "playlist")}
              >
                <option value="channel">Channel</option>
                <option value="playlist">Playlist</option>
              </NativeSelect>
            </label>
            <label className="grid gap-2">
              <span className="text-sm font-medium">
                {monitor ? "Max new videos per check" : "Max videos to ingest"}
              </span>
              <Input
                inputMode="numeric"
                value={maxItems}
                onChange={(event) => setMaxItems(event.target.value)}
              />
            </label>
          </div>
          <label className="grid gap-2">
            <span className="text-sm font-medium">YouTube URL</span>
            <Input
              value={sourceUrl}
              onChange={(event) => setSourceUrl(event.target.value)}
              placeholder="https://www.youtube.com/@channel/videos"
              spellCheck={false}
            />
          </label>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="grid gap-2">
              <span className="text-sm font-medium">Display name</span>
              <Input value={name} onChange={(event) => setName(event.target.value)} />
            </label>
            <label className="grid gap-2">
              <span className="text-sm font-medium">Caption languages</span>
              <Input
                value={languages}
                onChange={(event) => setLanguages(event.target.value)}
                placeholder="en, de"
                spellCheck={false}
              />
            </label>
          </div>
          <div className="grid gap-3 rounded-md border border-border px-4 py-3">
            <label className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Monitor for new videos</span>
              <Switch checked={monitor} onCheckedChange={setMonitor} />
            </label>
            <label className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">New videos</span>
              <NativeSelect
                aria-label="New video processing"
                value={policy}
                onChange={(event) => setPolicy(event.target.value as ProcessingPolicy)}
              >
                <option value="automatic">Analyze automatically</option>
                <option value="review_first">Hold for review first</option>
              </NativeSelect>
            </label>
            <label className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Run ASR fallback</span>
              <Switch checked={asrEnabled} onCheckedChange={setAsrEnabled} />
            </label>
          </div>
          {monitor ? (
            <label className="grid gap-2">
              <span className="text-sm font-medium">Check for new videos every</span>
              <IntervalInput
                label="New source check interval"
                value={checkInterval}
                onChange={setCheckInterval}
              />
              {intervalSeconds === null ? (
                <span className="text-xs text-destructive">
                  Choose an interval between 1 minute and 90 days.
                </span>
              ) : (
                <span className="text-xs text-muted-foreground">
                  The running server checks this source automatically; Check now stays available.
                </span>
              )}
            </label>
          ) : null}
          {monitor ? (
            <HistoryScopeFields
              sourceKind={sourceKind}
              scope={historyScope}
              onScopeChange={setHistoryScope}
              recentDays={recentDays}
              onRecentDaysChange={setRecentDays}
              limitCount={limitCount}
              onLimitCountChange={setLimitCount}
              valid={history !== null}
            />
          ) : (
            <p className="text-sm text-muted-foreground">
              Ingests up to the maximum number of listed videos once, without monitoring.
            </p>
          )}
          {addMutation.error ? (
            <p className="text-sm text-destructive">{String(addMutation.error)}</p>
          ) : null}
          <Button
            type="submit"
            disabled={
              !sourceUrl.trim() ||
              (monitor && (!history || intervalSeconds === null)) ||
              addMutation.isPending
            }
          >
            {addMutation.isPending
              ? "Adding source..."
              : monitor
                ? "Add monitored source"
                : "Ingest once"}
          </Button>
        </form>

        <div className="grid gap-3 border-t border-border pt-5">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-sm font-semibold">Monitored sources</h2>
            <Badge variant="outline">{sourcesQuery.data?.length ?? 0}</Badge>
          </div>
          {sourcesQuery.isPending ? <LoadingState label="Loading sources" /> : null}
          {sourcesQuery.error ? (
            <ErrorState>
              <StateViewTitle>Sources unavailable</StateViewTitle>
              <StateViewDescription>{String(sourcesQuery.error)}</StateViewDescription>
            </ErrorState>
          ) : null}
          {sourcesQuery.data?.length === 0 ? (
            <StateView variant="empty">
              <StateViewTitle>No monitored sources</StateViewTitle>
              <StateViewDescription>
                You can still use one-time ingestion without creating a subscription.
              </StateViewDescription>
            </StateView>
          ) : null}
          {sourcesQuery.data?.map((source) => (
            <article
              key={source.id}
              className="grid gap-3 rounded-md border border-border px-4 py-4"
            >
              <div className="flex min-w-0 flex-wrap items-start gap-3">
                <div className="min-w-0 flex-1">
                  <h3 className="truncate text-sm font-semibold">
                    {source.name ?? source.sourceUrl}
                  </h3>
                  <p className="mt-1 truncate text-xs text-muted-foreground">{source.sourceUrl}</p>
                </div>
                <Badge variant={source.lastCheckStatus === "failed" ? "destructive" : "outline"}>
                  {source.checkInProgress ? "checking" : (source.lastCheckStatus ?? "not checked")}
                </Badge>
              </div>
              <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                <span>{source.itemsSeen} seen</span>
                <span>{source.itemsIndexed} indexed</span>
                {source.itemsFailed > 0 ? <span>{source.itemsFailed} failed</span> : null}
                {source.itemsAwaitingReview > 0 ? (
                  <span>{source.itemsAwaitingReview} awaiting review</span>
                ) : null}
                {source.lastCheckedAt ? (
                  <span>checked {formatDate(source.lastCheckedAt)}</span>
                ) : null}
              </div>
              {source.lastCheckMessage ? (
                <p className="text-sm text-destructive">{source.lastCheckMessage}</p>
              ) : null}
              <SourceSchedule corpusId={corpusId} source={source} onChanged={invalidateCorpus} />
              <QueueActivity source={source} />
              {source.backfill ? (
                <BackfillPanel
                  corpusId={corpusId}
                  sourceId={source.id}
                  backfill={source.backfill}
                  onChanged={invalidateCorpus}
                />
              ) : null}
              <LabelAssignments
                itemName={source.name ?? source.sourceUrl}
                assigned={source.labels}
                available={labelsQuery.data ?? []}
                pending={labelMutation.isPending}
                error={labelMutation.variables?.sourceId === source.id ? labelMutation.error : null}
                onAssign={(labelId) =>
                  labelMutation.mutate({
                    sourceId: source.id,
                    labelId,
                    assigned: true,
                  })
                }
                onRemove={(labelId) =>
                  labelMutation.mutate({
                    sourceId: source.id,
                    labelId,
                    assigned: false,
                  })
                }
              />
              <SourceReview
                corpusId={corpusId}
                sourceId={source.id}
                policy={source.processingPolicy}
                awaiting={source.itemsAwaitingReview}
                onChanged={invalidateCorpus}
              />
              <div className="flex flex-wrap items-center justify-between gap-3">
                <label className="flex items-center gap-2 text-sm">
                  <Switch
                    checked={source.enabled}
                    disabled={enabledMutation.isPending}
                    onCheckedChange={(enabled) =>
                      enabledMutation.mutate({ sourceId: source.id, enabled })
                    }
                  />
                  Enabled
                </label>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={checkMutation.isPending || source.checkInProgress}
                  onClick={() => checkMutation.mutate(source.id)}
                >
                  {source.checkInProgress ||
                  (checkMutation.isPending && checkMutation.variables === source.id)
                    ? "Checking..."
                    : "Check now"}
                </Button>
              </div>
            </article>
          ))}
          {checkMutation.error ? (
            <p className="text-sm text-destructive">{String(checkMutation.error)}</p>
          ) : null}
        </div>
      </SurfaceContent>
    </Surface>
  );
}

/** Automatic / review-first policy and the approval queue of one source (Atlas 1.6, 1.8). */
function SourceReview({
  corpusId,
  sourceId,
  policy,
  awaiting,
  onChanged,
}: {
  corpusId: string;
  sourceId: string;
  policy: ProcessingPolicy;
  awaiting: number;
  onChanged: () => Promise<void>;
}) {
  const [selected, setSelected] = React.useState<string[]>([]);
  const pendingQuery = useQuery({
    queryKey: ["corpora", corpusId, "sources", sourceId, "pending"],
    queryFn: () => listSourcePending(corpusId, sourceId),
    enabled: awaiting > 0,
  });
  const policyMutation = useMutation({
    mutationFn: (next: ProcessingPolicy) => setSourcePolicy(corpusId, sourceId, next),
    onSuccess: onChanged,
  });
  const approveMutation = useMutation({
    mutationFn: (urls: string[]) => approveSourcePending(corpusId, sourceId, urls),
    onSuccess: async () => {
      setSelected([]);
      await Promise.all([onChanged(), pendingQuery.refetch()]);
    },
  });
  const pending = pendingQuery.data ?? [];
  return (
    <div className="grid gap-2">
      <label className="flex flex-wrap items-center gap-2 text-sm">
        New videos
        <NativeSelect
          aria-label="Processing policy"
          value={policy}
          disabled={policyMutation.isPending}
          onChange={(event) => policyMutation.mutate(event.target.value as ProcessingPolicy)}
        >
          <option value="automatic">Analyze automatically</option>
          <option value="review_first">Hold for review first</option>
        </NativeSelect>
      </label>
      {policy === "review_first" ? (
        <p className="text-xs text-muted-foreground">
          Videos also tracked by an automatic source are analyzed anyway.
        </p>
      ) : null}
      {pending.length > 0 ? (
        <div
          className="grid gap-2 rounded-md border border-border p-3"
          aria-label="Awaiting review"
        >
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">Awaiting review</span>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() =>
                setSelected(
                  selected.length === pending.length ? [] : pending.map((item) => item.sourceUrl),
                )
              }
            >
              {selected.length === pending.length ? "Clear" : "Select all"}
            </Button>
            <Button
              type="button"
              size="sm"
              disabled={selected.length === 0 || approveMutation.isPending}
              onClick={() => approveMutation.mutate(selected)}
            >
              {approveMutation.isPending ? "Analyzing..." : `Approve ${selected.length}`}
            </Button>
          </div>
          <ul className="grid gap-1 text-sm">
            {pending.map((item) => (
              <li key={item.sourceUrl} className="flex flex-wrap items-center gap-2">
                <input
                  type="checkbox"
                  aria-label={`Select ${item.title ?? item.sourceUrl}`}
                  checked={selected.includes(item.sourceUrl)}
                  onChange={(event) =>
                    setSelected(
                      event.target.checked
                        ? [...selected, item.sourceUrl]
                        : selected.filter((url) => url !== item.sourceUrl),
                    )
                  }
                />
                <a className="underline" href={item.sourceUrl} target="_blank" rel="noreferrer">
                  {item.title ?? item.sourceUrl}
                </a>
                <span className="text-xs text-muted-foreground">
                  {[
                    item.channel,
                    item.uploadDate,
                    item.durationSeconds ? `${Math.round(item.durationSeconds / 60)} min` : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {policyMutation.error || approveMutation.error || pendingQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {String(policyMutation.error ?? approveMutation.error ?? pendingQuery.error)}
        </p>
      ) : null}
    </div>
  );
}

function historyInput(
  scope: HistoryScopeKind,
  recentDays: string,
  limitCount: string,
): HistoryScopeInput | null {
  if (scope === "recent") {
    const days = Number(recentDays);
    return Number.isInteger(days) && days > 0 ? { scope, recentDays: days } : null;
  }
  if (scope === "limited") {
    const count = Number(limitCount);
    return Number.isInteger(count) && count > 0 ? { scope, limitCount: count } : null;
  }
  return { scope };
}

const ACTIVE_BACKFILL_STATES = new Set(["discovering", "running", "paused"]);

function hasActiveWork(source: CorpusSource) {
  const { queue, backfill } = source;
  return (
    source.checkInProgress ||
    queue.newQueued + queue.newRunning + queue.historyQueued + queue.historyRunning > 0 ||
    (backfill !== null && (ACTIVE_BACKFILL_STATES.has(backfill.state) || backfill.running > 0))
  );
}

/** Initial history selection for a monitored source (Atlas 1.10, 1.12; #67.Q1). */
function HistoryScopeFields({
  sourceKind,
  scope,
  onScopeChange,
  recentDays,
  onRecentDaysChange,
  limitCount,
  onLimitCountChange,
  valid,
}: {
  sourceKind: "channel" | "playlist";
  scope: HistoryScopeKind;
  onScopeChange: (scope: HistoryScopeKind) => void;
  recentDays: string;
  onRecentDaysChange: (value: string) => void;
  limitCount: string;
  onLimitCountChange: (value: string) => void;
  valid: boolean;
}) {
  const order = sourceKind === "playlist" ? "playlist order" : "the channel's listed order";
  const option = (value: HistoryScopeKind, label: React.ReactNode) => (
    <label className="flex flex-wrap items-center gap-2 text-sm">
      <input
        type="radio"
        name="history-scope"
        value={value}
        checked={scope === value}
        onChange={() => onScopeChange(value)}
      />
      {label}
    </label>
  );
  return (
    <fieldset className="grid gap-2 rounded-md border border-border px-4 py-3">
      <legend className="px-1 text-sm font-medium">Already published videos</legend>
      {option("full", "All available history")}
      {option(
        "recent",
        <>
          Published in the last
          <Input
            aria-label="Recent history days"
            className="h-8 w-20"
            inputMode="numeric"
            value={recentDays}
            onFocus={() => onScopeChange("recent")}
            onChange={(event) => onRecentDaysChange(event.target.value)}
          />
          days
        </>,
      )}
      {option(
        "limited",
        <>
          First
          <Input
            aria-label="Number of history videos"
            className="h-8 w-20"
            inputMode="numeric"
            value={limitCount}
            onFocus={() => onScopeChange("limited")}
            onChange={(event) => onLimitCountChange(event.target.value)}
          />
          videos in {order}
        </>,
      )}
      {option("none", "None, only videos published from now on")}
      <p className="text-xs text-muted-foreground" aria-live="polite">
        {!valid
          ? "Enter a positive whole number."
          : scope === "full"
            ? `Adding the source lists every video it currently shows and processes all of them in the background, without the per-check maximum. This can be a large amount of work; you can pause or cancel it.`
            : scope === "recent"
              ? `Adding the source processes videos published in the last ${recentDays} days in the background. Where YouTube lists only a relative age, the date is approximate; videos without a known date are not selected and are reported.`
              : scope === "limited"
                ? `Adding the source processes the first ${limitCount} videos as listed in ${order} at that moment, in the background. This may differ from the newest publication dates.`
                : "Adding the source records its existing videos without processing them; only later uploads are analyzed."}{" "}
        New uploads are checked separately and follow the processing choice below.
      </p>
    </fieldset>
  );
}

function QueueActivity({ source }: { source: CorpusSource }) {
  const { queue } = source;
  if (queue.newQueued + queue.newRunning + queue.historyQueued + queue.historyRunning === 0) {
    return null;
  }
  return (
    <p className="text-xs text-muted-foreground" aria-label="Processing activity">
      New uploads: {queue.newRunning} running, {queue.newQueued} queued · History:{" "}
      {queue.historyRunning} running, {queue.historyQueued} queued
    </p>
  );
}

const SCOPE_LABEL: Record<string, string> = {
  full: "All available history",
  recent: "Recent history",
  limited: "First videos in listed order",
  none: "No history",
};

const REASON_LABEL: Record<string, string> = {
  scope_none: "not requested",
  beyond_limit: "beyond the requested count",
  outside_recent_window: "published before the window",
  publish_date_unknown: "publish date unknown",
};

function backfillStateLabel(backfill: BackfillProgress) {
  const finishing = backfill.running > 0;
  switch (backfill.state) {
    case "discovering":
      return "listing videos";
    case "running":
      return "running";
    case "paused":
      return finishing ? "pausing" : "paused";
    case "canceled":
      return finishing ? "canceling" : "canceled";
    case "awaiting_review":
      return "waiting for review";
    case "completed":
      return "completed";
    case "partial":
      return "finished with gaps";
    case "failed":
      return "listing failed";
  }
}

/** Progress and pause/resume/cancel of a source's history backfill (Atlas 1.14). */
function BackfillPanel({
  corpusId,
  sourceId,
  backfill,
  onChanged,
}: {
  corpusId: string;
  sourceId: string;
  backfill: BackfillProgress;
  onChanged: () => Promise<void>;
}) {
  const [showEntries, setShowEntries] = React.useState(false);
  const controlMutation = useMutation({
    mutationFn: (action: BackfillAction) =>
      controlBackfill(corpusId, sourceId, backfill.id, action),
    onSuccess: onChanged,
  });
  const entriesQuery = useQuery({
    queryKey: ["corpora", corpusId, "sources", sourceId, "backfills", backfill.id, "entries"],
    queryFn: () => listBackfillEntries(corpusId, sourceId, backfill.id),
    enabled: showEntries,
  });
  const scope =
    backfill.scope === "recent"
      ? `Published in the last ${backfill.recentDays} days`
      : backfill.scope === "limited"
        ? `First ${backfill.limitCount} videos in listed order`
        : SCOPE_LABEL[backfill.scope];
  const finished = backfill.indexed + backfill.skipped;
  const remaining = backfill.queued + backfill.running;
  const state = backfillStateLabel(backfill);
  return (
    <section
      className="grid gap-2 rounded-md border border-border p-3"
      aria-label="History backfill"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium">History: {scope}</span>
        <Badge
          variant={
            backfill.state === "failed" || backfill.state === "partial" ? "destructive" : "outline"
          }
        >
          {state}
        </Badge>
      </div>
      <p className="text-xs text-muted-foreground">
        {backfill.discoveredCount === null
          ? "Listed videos: not yet enumerated"
          : `${backfill.discoveredCount} listed${
              backfill.discoveredAt ? ` on ${formatDate(backfill.discoveredAt)}` : ""
            } · ${backfill.selected} selected`}
        {backfill.discoveredCount !== null
          ? ` · ${finished} finished · ${remaining} remaining (${backfill.running} running)`
          : null}
        {backfill.failed > 0 ? ` · ${backfill.failed} failed` : null}
        {backfill.awaitingReview > 0 ? ` · ${backfill.awaitingReview} awaiting review` : null}
        {backfill.filtered > 0 ? ` · ${backfill.filtered} filtered` : null}
        {backfill.canceled > 0 ? ` · ${backfill.canceled} canceled` : null}
      </p>
      {backfill.notSelected.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          Not selected:{" "}
          {backfill.notSelected
            .map((entry) => `${entry.count} ${REASON_LABEL[entry.reason] ?? entry.reason}`)
            .join(", ")}
        </p>
      ) : null}
      {backfill.discoveryMessage ? (
        <p className="text-sm text-destructive">{backfill.discoveryMessage}</p>
      ) : null}
      <div className="flex flex-wrap gap-2">
        {backfill.state === "running" ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={controlMutation.isPending}
            onClick={() => controlMutation.mutate("pause")}
          >
            Pause
          </Button>
        ) : null}
        {backfill.state === "paused" ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={controlMutation.isPending}
            onClick={() => controlMutation.mutate("resume")}
          >
            Resume
          </Button>
        ) : null}
        {["discovering", "running", "paused", "awaiting_review"].includes(backfill.state) ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={controlMutation.isPending}
            onClick={() => controlMutation.mutate("cancel")}
          >
            Cancel
          </Button>
        ) : null}
        {backfill.discoveredCount !== null ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => setShowEntries(!showEntries)}
          >
            {showEntries ? "Hide listed videos" : "Show listed videos"}
          </Button>
        ) : null}
      </div>
      {showEntries && entriesQuery.data ? (
        <ol className="grid max-h-64 gap-1 overflow-y-auto text-xs" aria-label="Listed videos">
          {entriesQuery.data.map((entry) => (
            <li key={entry.position} className="flex flex-wrap gap-2">
              <span className="text-muted-foreground">{entry.position}.</span>
              <a className="underline" href={entry.sourceUrl} target="_blank" rel="noreferrer">
                {entry.title ?? entry.sourceUrl}
              </a>
              {entry.uploadDate ? (
                <span className="text-muted-foreground">{entry.uploadDate}</span>
              ) : null}
              <span className="text-muted-foreground">
                {entry.selected
                  ? (entry.status ?? "selected")
                  : `not selected (${REASON_LABEL[entry.reason ?? ""] ?? entry.reason})`}
              </span>
            </li>
          ))}
        </ol>
      ) : null}
      {controlMutation.error || entriesQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {String(controlMutation.error ?? entriesQuery.error)}
        </p>
      ) : null}
    </section>
  );
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

/** Matches the server default for a new monitored source (6 hours). */
const DEFAULT_CHECK_INTERVAL_SECONDS = 6 * 60 * 60;
const MIN_CHECK_INTERVAL_SECONDS = 60;
const MAX_CHECK_INTERVAL_SECONDS = 90 * 24 * 60 * 60;

type IntervalUnit = "minutes" | "hours" | "days";
type IntervalValue = { amount: string; unit: IntervalUnit };

const UNIT_SECONDS: Record<IntervalUnit, number> = {
  minutes: 60,
  hours: 60 * 60,
  days: 24 * 60 * 60,
};

/** Largest whole unit that represents the interval exactly. */
function intervalValue(seconds: number): IntervalValue {
  for (const unit of ["days", "hours", "minutes"] as const) {
    if (seconds % UNIT_SECONDS[unit] === 0) {
      return { amount: String(seconds / UNIT_SECONDS[unit]), unit };
    }
  }
  return {
    amount: String(Math.max(1, Math.round(seconds / 60))),
    unit: "minutes",
  };
}

/** Seconds of a valid interval, or null outside 1 minute to 90 days. */
function intervalToSeconds(value: IntervalValue): number | null {
  const amount = Number(value.amount);
  if (!value.amount.trim() || !Number.isFinite(amount) || amount <= 0) {
    return null;
  }
  const seconds = Math.round(amount * UNIT_SECONDS[value.unit]);
  return seconds >= MIN_CHECK_INTERVAL_SECONDS && seconds <= MAX_CHECK_INTERVAL_SECONDS
    ? seconds
    : null;
}

function formatInterval(seconds: number) {
  const { amount, unit } = intervalValue(seconds);
  return `${amount} ${Number(amount) === 1 ? unit.slice(0, -1) : unit}`;
}

function IntervalInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: IntervalValue;
  onChange: (value: IntervalValue) => void;
}) {
  return (
    <span className="flex items-center gap-2">
      <Input
        aria-label={`${label} amount`}
        className="w-24"
        inputMode="decimal"
        value={value.amount}
        onChange={(event) => onChange({ ...value, amount: event.target.value })}
      />
      <NativeSelect
        aria-label={`${label} unit`}
        value={value.unit}
        onChange={(event) => onChange({ ...value, unit: event.target.value as IntervalUnit })}
      >
        <option value="minutes">minutes</option>
        <option value="hours">hours</option>
        <option value="days">days</option>
      </NativeSelect>
    </span>
  );
}

/** Automatic check cadence of one source: interval, next/overdue check and its editor (1.18). */
function SourceSchedule({
  corpusId,
  source,
  onChanged,
}: {
  corpusId: string;
  source: CorpusSource;
  onChanged: () => Promise<void>;
}) {
  const [draft, setDraft] = React.useState<IntervalValue>(() =>
    intervalValue(source.checkIntervalSeconds),
  );
  React.useEffect(() => {
    setDraft(intervalValue(source.checkIntervalSeconds));
  }, [source.checkIntervalSeconds]);
  const mutation = useMutation({
    mutationFn: (seconds: number) => setCorpusSourceInterval(corpusId, source.id, seconds),
    onSuccess: onChanged,
  });
  const seconds = intervalToSeconds(draft);
  const changed = seconds !== null && seconds !== source.checkIntervalSeconds;
  const next = source.nextCheckAt ? new Date(source.nextCheckAt) : null;
  const overdue = next !== null && next.getTime() <= Date.now();

  let schedule: string;
  if (!source.enabled) {
    schedule = "Disabled: not checked automatically.";
  } else if (source.checkInProgress) {
    schedule = "Checking now.";
  } else if (!next) {
    schedule = "Next check not scheduled yet.";
  } else if (overdue) {
    schedule = `Due since ${formatDate(source.nextCheckAt!)}; checked while the server runs.`;
  } else {
    schedule = `Next check ${formatDate(source.nextCheckAt!)}.`;
  }

  return (
    <section
      aria-label="Check schedule"
      className="grid gap-2 rounded-md border border-border px-3 py-2"
    >
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>Checks every {formatInterval(source.checkIntervalSeconds)}</span>
        <span data-testid="next-check">{schedule}</span>
      </div>
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (seconds !== null && changed) {
            mutation.mutate(seconds);
          }
        }}
      >
        <IntervalInput label="Check interval" value={draft} onChange={setDraft} />
        <Button type="submit" variant="outline" size="sm" disabled={!changed || mutation.isPending}>
          {mutation.isPending ? "Saving..." : "Save interval"}
        </Button>
        {seconds === null ? (
          <span className="text-xs text-destructive">Between 1 minute and 90 days.</span>
        ) : null}
      </form>
      {mutation.error ? (
        <p role="alert" className="text-sm text-destructive">
          {String(mutation.error)}
        </p>
      ) : null}
    </section>
  );
}
