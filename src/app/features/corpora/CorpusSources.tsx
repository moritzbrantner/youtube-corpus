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

import {
  addCorpusSource,
  checkCorpusSource,
  listCorpusSources,
  setCorpusSourceEnabled,
  type AddCorpusSourceInput,
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
  const [ingestNow, setIngestNow] = React.useState(true);
  const [asrEnabled, setAsrEnabled] = React.useState(false);

  const sourcesQuery = useQuery({
    queryKey: corpusKeys.sources(corpusId),
    queryFn: () => listCorpusSources(corpusId),
  });

  async function invalidateCorpus() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: corpusKeys.all }),
      queryClient.invalidateQueries({ queryKey: corpusKeys.sources(corpusId) }),
      queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "videos"] }),
      queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "search"] }),
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
        queryClient.invalidateQueries({ queryKey: corpusKeys.sources(corpusId) }),
      ]);
    },
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    const url = sourceUrl.trim();
    if (!url || (!monitor && !ingestNow)) {
      return;
    }
    const parsedMaxItems = Number(maxItems);
    addMutation.mutate({
      sourceKind,
      sourceUrl: url,
      name: name.trim() || null,
      monitor,
      ingestNow,
      captionLanguages: languages
        .split(",")
        .map((language) => language.trim())
        .filter(Boolean),
      autoCaptionsEnabled: true,
      asrEnabled,
      maxItems: Number.isFinite(parsedMaxItems) && parsedMaxItems > 0 ? parsedMaxItems : null,
    });
  }

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Sources</SurfaceTitle>
        <SurfaceDescription>
          Add a channel or playlist once, or monitor it and check for newly published videos later.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-6">
        <form className="grid gap-4" onSubmit={submit}>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="grid gap-2">
              <span className="text-sm font-medium">Source type</span>
              <NativeSelect
                value={sourceKind}
                onChange={(event) =>
                  setSourceKind(event.target.value as "channel" | "playlist")
                }
              >
                <option value="channel">Channel</option>
                <option value="playlist">Playlist</option>
              </NativeSelect>
            </label>
            <label className="grid gap-2">
              <span className="text-sm font-medium">Max videos per check</span>
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
              <span className="text-sm font-medium">Ingest now</span>
              <Switch checked={ingestNow} onCheckedChange={setIngestNow} />
            </label>
            <label className="flex items-center justify-between gap-3">
              <span className="text-sm font-medium">Run ASR fallback</span>
              <Switch checked={asrEnabled} onCheckedChange={setAsrEnabled} />
            </label>
          </div>
          {!monitor && !ingestNow ? (
            <p className="text-sm text-destructive">
              A one-time source must be ingested now so its videos can be attached to this corpus.
            </p>
          ) : null}
          {addMutation.error ? (
            <p className="text-sm text-destructive">{String(addMutation.error)}</p>
          ) : null}
          <Button
            type="submit"
            disabled={!sourceUrl.trim() || (!monitor && !ingestNow) || addMutation.isPending}
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
                  <p className="mt-1 truncate text-xs text-muted-foreground">
                    {source.sourceUrl}
                  </p>
                </div>
                <Badge
                  variant={source.lastCheckStatus === "failed" ? "destructive" : "outline"}
                >
                  {source.lastCheckStatus ?? "not checked"}
                </Badge>
              </div>
              <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                <span>{source.itemsSeen} seen</span>
                <span>{source.itemsIndexed} indexed</span>
                {source.itemsFailed > 0 ? <span>{source.itemsFailed} failed</span> : null}
                {source.lastCheckedAt ? (
                  <span>checked {formatDate(source.lastCheckedAt)}</span>
                ) : null}
              </div>
              {source.lastCheckMessage ? (
                <p className="text-sm text-destructive">{source.lastCheckMessage}</p>
              ) : null}
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
                  disabled={checkMutation.isPending}
                  onClick={() => checkMutation.mutate(source.id)}
                >
                  {checkMutation.isPending && checkMutation.variables === source.id
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

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
