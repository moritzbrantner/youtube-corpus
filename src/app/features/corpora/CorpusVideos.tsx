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
  listCorpusVideos,
  reprocessCorpusVideo,
  type CorpusVideo,
  type ReprocessStage,
} from "./api";
import { corpusKeys } from "./query-keys";

type CorpusVideosProps = {
  corpusId: string;
};

export function CorpusVideos({ corpusId }: CorpusVideosProps) {
  const videosQuery = useQuery({
    queryKey: corpusKeys.videos(corpusId, 100),
    queryFn: () => listCorpusVideos(corpusId, 100),
  });

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Videos</SurfaceTitle>
        <SurfaceDescription>
          Browse corpus contents and selectively refresh metadata, transcripts, ASR, or embeddings.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        {videosQuery.isPending ? <LoadingState label="Loading corpus videos" /> : null}
        {videosQuery.error ? (
          <ErrorState>
            <StateViewTitle>Videos unavailable</StateViewTitle>
            <StateViewDescription>{String(videosQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {videosQuery.data?.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No videos in this corpus</StateViewTitle>
            <StateViewDescription>Add a channel or playlist to begin.</StateViewDescription>
          </StateView>
        ) : null}
        {videosQuery.data?.map((video) => (
          <CorpusVideoRow key={video.id} corpusId={corpusId} video={video} />
        ))}
      </SurfaceContent>
    </Surface>
  );
}

type CorpusVideoRowProps = {
  corpusId: string;
  video: CorpusVideo;
};

function CorpusVideoRow({ corpusId, video }: CorpusVideoRowProps) {
  const queryClient = useQueryClient();
  const [stage, setStage] = React.useState<ReprocessStage>("metadata");
  const mutation = useMutation({
    mutationFn: () => reprocessCorpusVideo(corpusId, video.id, { stage }),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: corpusKeys.all }),
        queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "videos"] }),
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
            {video.channel ?? video.sourceUrl}
          </p>
        </div>
        <Button asChild variant="ghost" size="sm">
          <a href={video.sourceUrl} target="_blank" rel="noreferrer">
            Open source
          </a>
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">{video.transcriptStreams} streams</Badge>
        <Badge variant="outline">{video.transcriptSegments} segments</Badge>
        {video.transcriptSourceKinds.map((kind) => (
          <Badge key={kind} variant="secondary">
            {kind}
          </Badge>
        ))}
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
          <span className="text-xs text-muted-foreground">
            {reprocessSummary(mutation.data)}
          </span>
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
