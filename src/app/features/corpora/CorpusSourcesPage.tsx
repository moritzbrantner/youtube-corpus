import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import {
  Badge,
  Button,
  ErrorState,
  LoadingState,
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

import { errorMessage } from "./CorpusLabels";
import {
  AddSourceForm,
  SourceControls,
  SourceCounts,
  SourceStatusBadge,
  SourcesLoading,
  SourcesUnavailable,
  useCorpusSources,
} from "./CorpusSources";
import { CorpusVideos } from "./CorpusVideos";
import { listCorpusChannels, type CorpusChannel, type CorpusVideoSource } from "./api";
import { corpusKeys } from "./query-keys";
import { directoryUrl, navigateInApp } from "./url-state";

type CorpusSourcesPageProps = {
  corpusId: string;
  sourceId: string | null;
  channelId: string | null;
};

/** Channels & Sources: directory, one tracked source, or one recognized channel (Atlas 5.1). */
export function CorpusSourcesPage({ corpusId, sourceId, channelId }: CorpusSourcesPageProps) {
  if (sourceId) {
    return <SourceDetail key={sourceId} corpusId={corpusId} sourceId={sourceId} />;
  }
  if (channelId) {
    return <ChannelDetail key={channelId} corpusId={corpusId} channelId={channelId} />;
  }
  return <SourceDirectory corpusId={corpusId} />;
}

function useCorpusChannels(corpusId: string) {
  return useQuery({
    queryKey: corpusKeys.channels(corpusId),
    queryFn: () => listCorpusChannels(corpusId),
  });
}

function InAppLink({ href, children }: { href: URL; children: ReactNode }) {
  return (
    <a
      className="rounded-sm font-medium text-foreground underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
      href={href.href}
      onClick={navigateInApp}
    >
      {children}
    </a>
  );
}

function sourceHref(sourceId: string) {
  return directoryUrl(new URL(window.location.href), { sourceId });
}

function channelHref(channelId: string) {
  return directoryUrl(new URL(window.location.href), { channelId });
}

function BackToDirectory() {
  return (
    <Button asChild variant="ghost" size="sm" className="justify-self-start">
      <a href={directoryUrl(new URL(window.location.href)).href} onClick={navigateInApp}>
        ← All channels & sources
      </a>
    </Button>
  );
}

function sourceLabel(source: CorpusVideoSource) {
  return source.name ?? `Unnamed ${source.sourceKind}`;
}

function SourceDirectory({ corpusId }: { corpusId: string }) {
  const sourcesQuery = useCorpusSources(corpusId);
  const channelsQuery = useCorpusChannels(corpusId);

  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
      <div className="grid min-w-0 gap-4">
        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Tracked sources</SurfaceTitle>
            <SurfaceDescription>
              Channels and playlists added to this corpus. Open one to check it, change its policy
              or approve held videos.
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="grid gap-2">
            {sourcesQuery.isPending ? <SourcesLoading /> : null}
            {sourcesQuery.error ? <SourcesUnavailable error={sourcesQuery.error} /> : null}
            {sourcesQuery.data?.length === 0 ? (
              <StateView variant="empty">
                <StateViewTitle>No tracked sources</StateViewTitle>
                <StateViewDescription>
                  Add a channel or playlist. One-time ingestion stays available without monitoring.
                </StateViewDescription>
              </StateView>
            ) : null}
            {sourcesQuery.data?.length ? (
              <ul className="grid gap-2" aria-label="Tracked sources">
                {sourcesQuery.data.map((source) => (
                  <li
                    key={source.id}
                    className="grid gap-2 rounded-md border border-border px-4 py-3"
                  >
                    <div className="flex min-w-0 flex-wrap items-start gap-2">
                      <div className="min-w-0 flex-1">
                        <h3 className="truncate text-sm">
                          <InAppLink href={sourceHref(source.id)}>
                            {source.name ?? source.sourceUrl}
                          </InAppLink>
                        </h3>
                        <p className="mt-1 truncate text-xs text-muted-foreground">
                          {source.sourceUrl}
                        </p>
                      </div>
                      <Badge variant="secondary">{source.sourceKind}</Badge>
                      <Badge variant="outline">
                        {source.processingPolicy === "review_first" ? "review first" : "automatic"}
                      </Badge>
                      <SourceStatusBadge source={source} />
                    </div>
                    <SourceCounts source={source} />
                  </li>
                ))}
              </ul>
            ) : null}
          </SurfaceContent>
        </Surface>

        <Surface>
          <SurfaceHeader>
            <SurfaceTitle>Channels in this corpus</SurfaceTitle>
            <SurfaceDescription>
              YouTube channels recognized from the metadata of corpus videos, whether or not a
              tracked source monitors them.
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="grid gap-2">
            {channelsQuery.isPending ? <LoadingState label="Loading channels" /> : null}
            {channelsQuery.error ? (
              <ErrorState>
                <StateViewTitle>Channels unavailable</StateViewTitle>
                <StateViewDescription>{errorMessage(channelsQuery.error)}</StateViewDescription>
              </ErrorState>
            ) : null}
            {channelsQuery.data?.channels.length === 0 ? (
              <StateView variant="empty">
                <StateViewTitle>No channels recognized yet</StateViewTitle>
                <StateViewDescription>
                  Channels appear once ingested videos carry YouTube channel metadata.
                </StateViewDescription>
              </StateView>
            ) : null}
            {channelsQuery.data?.channels.length ? (
              <ul className="grid gap-2" aria-label="Channels">
                {channelsQuery.data.channels.map((channel) => (
                  <ChannelRow key={channel.channelId} channel={channel} />
                ))}
              </ul>
            ) : null}
            {channelsQuery.data?.videosWithoutChannel ? (
              <p className="text-xs text-muted-foreground">
                {channelsQuery.data.videosWithoutChannel} corpus{" "}
                {channelsQuery.data.videosWithoutChannel === 1 ? "video has" : "videos have"} no
                channel metadata yet; refresh their metadata from the corpus video list.
              </p>
            ) : null}
          </SurfaceContent>
        </Surface>
      </div>

      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Add source</SurfaceTitle>
          <SurfaceDescription>
            Add a channel or playlist once, or monitor it so the running server checks it for newly
            published videos on its own schedule.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent>
          <AddSourceForm corpusId={corpusId} />
        </SurfaceContent>
      </Surface>
    </div>
  );
}

function trackingBadge(channel: CorpusChannel) {
  if (channel.sources.some((source) => source.sourceKind === "channel")) {
    return <Badge variant="secondary">tracked channel</Badge>;
  }
  if (channel.sources.length > 0) {
    return <Badge variant="outline">via playlist</Badge>;
  }
  return <Badge variant="outline">not tracked</Badge>;
}

function ChannelRow({ channel }: { channel: CorpusChannel }) {
  return (
    <li className="grid gap-1 rounded-md border border-border px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-start gap-2">
        <h3 className="min-w-0 flex-1 truncate text-sm">
          <InAppLink href={channelHref(channel.channelId)}>
            {channel.name ?? channel.channelId}
          </InAppLink>
        </h3>
        {trackingBadge(channel)}
      </div>
      <p className="text-xs text-muted-foreground">
        {channel.videoCount} {channel.videoCount === 1 ? "video" : "videos"}
        {channel.latestUploadDate ? ` · latest upload ${channel.latestUploadDate}` : ""}
        {channel.sources.length > 0
          ? ` · discovered by ${channel.sources.map(sourceLabel).join(", ")}`
          : " · added directly, not monitored"}
      </p>
    </li>
  );
}

function SourceDetail({ corpusId, sourceId }: { corpusId: string; sourceId: string }) {
  const sourcesQuery = useCorpusSources(corpusId);
  const channelsQuery = useCorpusChannels(corpusId);
  const source = sourcesQuery.data?.find((item) => item.id === sourceId) ?? null;
  const channels =
    channelsQuery.data?.channels.filter((channel) =>
      channel.sources.some((item) => item.id === sourceId),
    ) ?? [];

  if (sourcesQuery.isPending) {
    return <SourcesLoading />;
  }
  if (sourcesQuery.error) {
    return <SourcesUnavailable error={sourcesQuery.error} />;
  }
  if (!source) {
    return (
      <div className="grid gap-4">
        <BackToDirectory />
        <ErrorState>
          <StateViewTitle>Source not in this corpus</StateViewTitle>
          <StateViewDescription>
            The selected source was removed or belongs to another corpus.
          </StateViewDescription>
        </ErrorState>
      </div>
    );
  }

  return (
    <div className="grid gap-4">
      <BackToDirectory />
      <Surface>
        <SurfaceHeader>
          <div className="flex min-w-0 flex-wrap items-start gap-2">
            <SurfaceTitle className="min-w-0 flex-1 truncate">
              {source.name ?? source.sourceUrl}
            </SurfaceTitle>
            <Badge variant="secondary">{source.sourceKind}</Badge>
            <SourceStatusBadge source={source} />
          </div>
          <SurfaceDescription>
            <a
              className="break-all underline-offset-4 hover:underline"
              href={source.sourceUrl}
              target="_blank"
              rel="noreferrer"
            >
              {source.sourceUrl}
            </a>
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-4">
          <section className="grid gap-1" aria-label="Channels of this source">
            <h3 className="text-sm font-semibold">
              {source.sourceKind === "playlist" ? "Channels in this playlist" : "Channel"}
            </h3>
            {channelsQuery.isPending ? <LoadingState label="Loading channels" /> : null}
            {channelsQuery.error ? (
              <p className="text-sm text-destructive">{errorMessage(channelsQuery.error)}</p>
            ) : null}
            {channelsQuery.data && channels.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                {source.itemsSeen === 0
                  ? "No videos discovered yet, so the channel is unknown."
                  : "Channel metadata unknown: discovered videos have not reported a channel yet."}
              </p>
            ) : null}
            {channels.length > 0 ? (
              <ul className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
                {channels.map((channel) => (
                  <li key={channel.channelId}>
                    <InAppLink href={channelHref(channel.channelId)}>
                      {channel.name ?? channel.channelId}
                    </InAppLink>
                    <span className="text-xs text-muted-foreground">
                      {" "}
                      ({channel.videoCount} in corpus)
                    </span>
                  </li>
                ))}
              </ul>
            ) : null}
          </section>
          <SourceControls corpusId={corpusId} source={source} />
        </SurfaceContent>
      </Surface>
      <CorpusVideos
        corpusId={corpusId}
        filter={{ sourceId }}
        title={`Videos from ${source.name ?? "this source"}`}
      />
    </div>
  );
}

function ChannelDetail({ corpusId, channelId }: { corpusId: string; channelId: string }) {
  const channelsQuery = useCorpusChannels(corpusId);
  const channel = channelsQuery.data?.channels.find((item) => item.channelId === channelId) ?? null;

  if (channelsQuery.isPending) {
    return <LoadingState label="Loading channel" />;
  }
  if (channelsQuery.error) {
    return (
      <ErrorState>
        <StateViewTitle>Channel unavailable</StateViewTitle>
        <StateViewDescription>{errorMessage(channelsQuery.error)}</StateViewDescription>
      </ErrorState>
    );
  }
  if (!channel) {
    return (
      <div className="grid gap-4">
        <BackToDirectory />
        <ErrorState>
          <StateViewTitle>Channel not in this corpus</StateViewTitle>
          <StateViewDescription>
            No video of this corpus names the channel {channelId}.
          </StateViewDescription>
        </ErrorState>
      </div>
    );
  }
  const trackedAsChannel = channel.sources.some((source) => source.sourceKind === "channel");

  return (
    <div className="grid gap-4">
      <BackToDirectory />
      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
        <Surface>
          <SurfaceHeader>
            <div className="flex min-w-0 flex-wrap items-start gap-2">
              <SurfaceTitle className="min-w-0 flex-1 truncate">
                {channel.name ?? channel.channelId}
              </SurfaceTitle>
              {trackingBadge(channel)}
            </div>
            <SurfaceDescription>YouTube channel {channel.channelId}</SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="grid gap-3 text-sm">
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1">
              <dt className="text-muted-foreground">Videos in corpus</dt>
              <dd>{channel.videoCount}</dd>
              <dt className="text-muted-foreground">Latest upload</dt>
              <dd>{channel.latestUploadDate ?? "unknown"}</dd>
              <dt className="text-muted-foreground">On YouTube</dt>
              <dd className="min-w-0 break-all">
                {channel.channelUrl ? (
                  <a
                    className="underline-offset-4 hover:underline"
                    href={channel.channelUrl}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {channel.channelUrl}
                  </a>
                ) : (
                  "channel URL unknown"
                )}
              </dd>
            </dl>
            <section className="grid gap-1" aria-label="Tracked sources of this channel">
              <h3 className="font-semibold">Tracked sources</h3>
              {channel.sources.length === 0 ? (
                <p className="text-muted-foreground">
                  Recognized from directly added videos; no tracked source monitors it.
                </p>
              ) : (
                <ul className="flex flex-wrap gap-x-4 gap-y-1">
                  {channel.sources.map((source) => (
                    <li key={source.id}>
                      <InAppLink href={sourceHref(source.id)}>{sourceLabel(source)}</InAppLink>
                      <span className="text-xs text-muted-foreground"> ({source.sourceKind})</span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </SurfaceContent>
        </Surface>
        {trackedAsChannel ? null : (
          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Track this channel</SurfaceTitle>
              <SurfaceDescription>
                Add it as a source of this corpus to ingest or monitor its uploads.
              </SurfaceDescription>
            </SurfaceHeader>
            <SurfaceContent>
              <AddSourceForm
                corpusId={corpusId}
                initialUrl={channel.channelUrl ?? ""}
                initialName={channel.name ?? ""}
              />
            </SurfaceContent>
          </Surface>
        )}
      </div>
      <CorpusVideos
        corpusId={corpusId}
        filter={{ channelId }}
        title={`Videos from ${channel.name ?? channel.channelId}`}
      />
    </div>
  );
}
