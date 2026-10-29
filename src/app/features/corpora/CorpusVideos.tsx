import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  Button,
  ErrorState,
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

import {
  LabelAssignments,
  errorMessage,
  useCorpusLabels,
  useInvalidateLabels,
} from "./CorpusLabels";
import {
  assignVideoLabel,
  listCorpusVideos,
  removeVideoLabel,
  listTranscriptQuality,
  refreshTranscriptQuality,
  reprocessCorpusVideo,
  type CorpusLabel,
  type CorpusVideo,
  type ReprocessStage,
  type TranscriptQuality,
} from "./api";
import { VideoProcessing } from "./VideoProcessing";
import { corpusKeys } from "./query-keys";
import { directoryUrl, navigateInApp } from "./url-state";

type CorpusVideosProps = {
  corpusId: string;
  /** Narrows the list to one tracked source or one YouTube channel. */
  filter?: { sourceId?: string; channelId?: string };
  title?: string;
};

export function CorpusVideos({ corpusId, filter = {}, title = "Videos" }: CorpusVideosProps) {
  const scoped = Boolean(filter.sourceId || filter.channelId);
  const queryClient = useQueryClient();
  const [labelFilter, setLabelFilter] = React.useState<string | null>(null);
  const labelsQuery = useCorpusLabels(corpusId);
  const labels = labelsQuery.data ?? [];
  const activeLabel = labels.find((label) => label.id === labelFilter) ?? null;
  React.useEffect(() => {
    // A deleted label can no longer filter the catalog.
    if (labelFilter && labelsQuery.data && !activeLabel) {
      setLabelFilter(null);
    }
  }, [activeLabel, labelFilter, labelsQuery.data]);
  const videosQuery = useQuery({
    queryKey: corpusKeys.videos(corpusId, 100, { ...filter, labelId: labelFilter }),
    queryFn: () => listCorpusVideos(corpusId, 100, { ...filter, labelId: labelFilter }),
  });
  const qualityQuery = useQuery({
    queryKey: corpusKeys.quality(corpusId),
    queryFn: () => listTranscriptQuality(corpusId),
  });
  const refreshMutation = useMutation({
    mutationFn: () => refreshTranscriptQuality(corpusId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: corpusKeys.quality(corpusId) }),
        queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "search"] }),
      ]);
    },
  });
  const qualityByVideo = React.useMemo(() => {
    const grouped = new Map<string, TranscriptQuality[]>();
    for (const quality of qualityQuery.data ?? []) {
      const items = grouped.get(quality.videoId) ?? [];
      items.push(quality);
      grouped.set(quality.videoId, items);
    }
    return grouped;
  }, [qualityQuery.data]);

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>{title}</SurfaceTitle>
        <SurfaceDescription>
          Browse corpus contents, inspect the selected research transcript, and selectively
          reprocess metadata, transcripts, ASR, or embeddings.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            Search defaults to the highest-quality transcript stream for each video while retaining
            every original stream.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={refreshMutation.isPending}
            onClick={() => refreshMutation.mutate()}
          >
            {refreshMutation.isPending ? "Scoring transcripts..." : "Refresh transcript quality"}
          </Button>
        </div>
        <label className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">Filter by your label</span>
          <NativeSelect
            aria-label="Filter videos by label"
            className="min-w-44"
            value={labelFilter ?? ""}
            onChange={(event) => setLabelFilter(event.target.value || null)}
          >
            <option value="">All videos</option>
            {labels.map((label) => (
              <option key={label.id} value={label.id}>
                {label.name}
              </option>
            ))}
          </NativeSelect>
          {activeLabel ? (
            <Button type="button" variant="ghost" size="sm" onClick={() => setLabelFilter(null)}>
              Clear filter
            </Button>
          ) : null}
        </label>
        {refreshMutation.error ? (
          <p className="text-sm text-destructive">{String(refreshMutation.error)}</p>
        ) : null}
        {videosQuery.isPending ? <LoadingState label="Loading corpus videos" /> : null}
        {videosQuery.error ? (
          <ErrorState>
            <StateViewTitle>Videos unavailable</StateViewTitle>
            <StateViewDescription>{errorMessage(videosQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {qualityQuery.error ? (
          <ErrorState>
            <StateViewTitle>Transcript quality unavailable</StateViewTitle>
            <StateViewDescription>{String(qualityQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {videosQuery.data?.length === 0 && activeLabel ? (
          <StateView variant="empty">
            <StateViewTitle>No videos with the label “{activeLabel.name}”</StateViewTitle>
            <StateViewDescription>
              Assign this label to a video or to one of its sources, or clear the filter.
            </StateViewDescription>
          </StateView>
        ) : null}
        {videosQuery.data?.length === 0 && !activeLabel ? (
          <StateView variant="empty">
            <StateViewTitle>
              {scoped ? "No corpus videos here yet" : "No videos in this corpus"}
            </StateViewTitle>
            <StateViewDescription>
              {scoped
                ? "Videos appear once a check discovers and ingests them."
                : "Add a channel or playlist to begin."}
            </StateViewDescription>
          </StateView>
        ) : null}
        {videosQuery.data?.map((video) => (
          <CorpusVideoRow
            key={video.id}
            corpusId={corpusId}
            video={video}
            labels={labels}
            quality={qualityByVideo.get(video.id) ?? []}
          />
        ))}
      </SurfaceContent>
    </Surface>
  );
}

type CorpusVideoRowProps = {
  corpusId: string;
  video: CorpusVideo;
  labels: CorpusLabel[];
  quality: TranscriptQuality[];
};

function CorpusVideoRow({ corpusId, video, labels, quality }: CorpusVideoRowProps) {
  const queryClient = useQueryClient();
  const invalidateLabels = useInvalidateLabels(corpusId);
  const labelMutation = useMutation({
    mutationFn: ({ labelId, assigned }: { labelId: string; assigned: boolean }) =>
      assigned
        ? assignVideoLabel(corpusId, video.id, labelId)
        : removeVideoLabel(corpusId, video.id, labelId),
    onSettled: invalidateLabels,
  });
  const videoName = video.title ?? video.sourceUrl;
  const [stage, setStage] = React.useState<ReprocessStage>("metadata");
  const preferred = quality.find((item) => item.isPreferred) ?? null;
  const mutation = useMutation({
    mutationFn: async () => {
      const report = await reprocessCorpusVideo(corpusId, video.id, { stage });
      await refreshTranscriptQuality(corpusId);
      return report;
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: corpusKeys.all }),
        queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "videos"] }),
        queryClient.invalidateQueries({ queryKey: corpusKeys.quality(corpusId) }),
        queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "search"] }),
      ]);
    },
  });

  return (
    <article className="grid gap-3 rounded-md border border-border px-4 py-4">
      <div className="flex min-w-0 flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold">{video.title ?? video.sourceUrl}</h3>
          <p className="mt-1 truncate text-xs text-muted-foreground">
            {video.channelId ? (
              <a
                className="underline-offset-4 hover:underline"
                href={
                  directoryUrl(new URL(window.location.href), { channelId: video.channelId }).href
                }
                onClick={navigateInApp}
              >
                {video.channel ?? video.channelId}
              </a>
            ) : (
              (video.channel ?? "Channel unknown")
            )}
          </p>
        </div>
        <Button asChild variant="ghost" size="sm">
          <a href={video.sourceUrl} target="_blank" rel="noreferrer">
            Open source
          </a>
        </Button>
      </div>

      <p className="text-xs text-muted-foreground">
        {video.sources.length > 0 ? (
          <>
            Tracked sources:{" "}
            {video.sources.map((source, index) => (
              <React.Fragment key={source.id}>
                {index > 0 ? ", " : null}
                <a
                  className="underline-offset-4 hover:underline"
                  href={directoryUrl(new URL(window.location.href), { sourceId: source.id }).href}
                  onClick={navigateInApp}
                >
                  {source.name ?? source.sourceKind}
                </a>{" "}
                ({source.sourceKind})
              </React.Fragment>
            ))}
          </>
        ) : (
          "No tracked source"
        )}
        {video.addedDirectly ? " · Added directly" : ""}
      </p>

      <LabelAssignments
        itemName={videoName}
        assigned={video.labels}
        available={labels}
        pending={labelMutation.isPending}
        error={labelMutation.error}
        onAssign={(labelId) => labelMutation.mutate({ labelId, assigned: true })}
        onRemove={(labelId) => labelMutation.mutate({ labelId, assigned: false })}
      />

      {video.youtubeCategories.length > 0 || video.youtubeTags.length > 0 ? (
        <div
          className="flex flex-wrap items-center gap-2"
          aria-label={`YouTube categories and tags of ${videoName}`}
        >
          <span className="text-xs font-medium text-muted-foreground">From YouTube:</span>
          {video.youtubeCategories.map((category) => (
            <Badge key={`category:${category}`} variant="outline">
              category: {category}
            </Badge>
          ))}
          {video.youtubeTags.map((tag) => (
            <Badge key={`tag:${tag}`} variant="outline">
              #{tag}
            </Badge>
          ))}
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">{video.transcriptStreams} streams</Badge>
        <Badge variant="outline">{video.transcriptSegments} segments</Badge>
        {video.transcriptSourceKinds.map((kind) => (
          <Badge key={kind} variant="secondary">
            {kind}
          </Badge>
        ))}
        {preferred ? (
          <>
            <Badge variant="secondary">preferred: {preferred.sourceKind}</Badge>
            <Badge variant="outline">quality {formatPercent(preferred.score)}</Badge>
            <Badge variant="outline">coverage {formatPercent(preferred.coverageRatio)}</Badge>
          </>
        ) : (
          <Badge variant="outline">no preferred transcript</Badge>
        )}
        {video.processingRevision !== null ? (
          <Badge variant="outline">revision {video.processingRevision}</Badge>
        ) : null}
      </div>

      <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-3">
        <span>{video.uploadDate ? `Uploaded ${video.uploadDate}` : "Upload date unknown"}</span>
        <span>{formatDuration(video.durationSeconds)}</span>
        <span>
          {video.lastRetrievedAt
            ? `Retrieved ${formatDate(video.lastRetrievedAt)}`
            : "No provenance yet"}
        </span>
      </div>

      <VideoProcessing corpusId={corpusId} videoId={video.id} />
      <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3">
        <NativeSelect
          aria-label={`Reprocess ${video.title ?? video.sourceUrl}`}
          className="min-w-44"
          value={stage}
          onChange={(event) => setStage(event.target.value as ReprocessStage)}
        >
          <option value="metadata">Refresh metadata</option>
          <option value="captions">Redownload captions</option>
          <option value="asr">Rerun ASR</option>
          <option value="segments">Re-segment local captions</option>
          <option value="embeddings">Re-embed transcript</option>
          <option value="all">Reprocess all</option>
        </NativeSelect>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={mutation.isPending}
          onClick={() => mutation.mutate()}
        >
          {mutation.isPending ? "Reprocessing..." : "Run"}
        </Button>
        {mutation.data ? (
          <span className="text-xs text-muted-foreground">{reprocessSummary(mutation.data)}</span>
        ) : null}
      </div>
      {mutation.error ? <p className="text-sm text-destructive">{String(mutation.error)}</p> : null}
    </article>
  );
}

function reprocessSummary(report: {
  segmentsResegmented: number;
  segmentsReembedded: number;
  metadataProcessingRevision: number;
}) {
  if (report.segmentsResegmented > 0) {
    return `${report.segmentsResegmented} segments rebuilt`;
  }
  if (report.segmentsReembedded > 0) {
    return `${report.segmentsReembedded} segments re-embedded`;
  }
  return `metadata revision ${report.metadataProcessingRevision}`;
}

function formatPercent(value: number) {
  return `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%`;
}

function formatDuration(seconds: number | null) {
  if (seconds === null) {
    return "Duration unknown";
  }
  const total = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
