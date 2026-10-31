import { useQuery } from "@tanstack/react-query";
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
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import type { SearchResult } from "../../api";
import { parseYouTubeVideoUrl } from "../analyzer/youtube-url";
import {
  getVideoInspection,
  listTranscriptQuality,
  loadTranscriptContext,
  type TranscriptStreamEvidence,
  type VideoInspection,
} from "./api";
import { errorMessage, useCorpusLabels } from "./CorpusLabels";
import { VideoChannel, VideoOverview, VideoReprocessControl } from "./CorpusVideos";
import {
  OcrEvidenceDetails,
  formatInterval,
  formatTimestamp,
  sourceLabel,
  timestampUrl,
} from "./evidence";
import { corpusKeys } from "./query-keys";
import {
  type MomentLocator,
  type MomentModality,
  type VideoRoute,
  corpusPageUrl,
  eventsForMomentUrl,
  momentModality,
  navigateInApp,
  navigateInPlace,
  processingUrl,
  readUrlState,
  videoUrl,
} from "./url-state";
import { VideoProcessing } from "./VideoProcessing";

type CorpusVideoPageProps = { corpusId: string; route: VideoRoute };

/** `/videos/:id` and its deep-linked moments: one video's metadata, evidence and processing. */
export function CorpusVideoPage({ corpusId, route }: CorpusVideoPageProps) {
  const { videoId, moment } = route;
  const inspection = useQuery({
    queryKey: corpusKeys.video(corpusId, videoId),
    queryFn: () => getVideoInspection(corpusId, videoId),
  });
  const momentQuery = useQuery({
    queryKey: corpusKeys.context(moment?.evidenceId ?? ""),
    queryFn: () => loadTranscriptContext(moment?.evidenceId ?? ""),
    enabled: Boolean(moment),
  });
  const resolved = moment && momentQuery.data ? momentQuery.data : null;
  const momentProblem = resolved && moment ? locatorMismatch(moment, resolved.match) : null;
  const activeMatch = resolved && !momentProblem ? resolved.match : null;

  const player = useYouTubePlayer();
  // Hold the embed back until a deep-linked moment resolved, so it opens at that time; once
  // shown it stays mounted and later moments seek in place.
  // `seekedEvidence` is the moment the player is at: the one it opened with, then each one it
  // sought to.
  const playerShown = React.useRef(false);
  const seekedEvidence = React.useRef<string | null>(null);
  if (!playerShown.current && (!moment || !momentQuery.isPending)) {
    playerShown.current = true;
    seekedEvidence.current = activeMatch?.segmentId ?? null;
  }
  const playerReady = playerShown.current;
  React.useEffect(() => {
    if (!playerShown.current || !activeMatch || seekedEvidence.current === activeMatch.segmentId) {
      return;
    }
    player.seek(activeMatch.startSeconds ?? 0);
    seekedEvidence.current = activeMatch.segmentId;
  }, [activeMatch, player]);

  if (inspection.isPending) {
    return <LoadingState label="Loading video" />;
  }
  if (inspection.error) {
    return (
      <ErrorState>
        <StateViewTitle>Video unavailable</StateViewTitle>
        <StateViewDescription>{errorMessage(inspection.error)}</StateViewDescription>
        <Button asChild variant="outline">
          <a href={corpusPageUrl(new URL(window.location.href), "videos").href}>All videos</a>
        </Button>
      </ErrorState>
    );
  }
  const data = inspection.data;

  return (
    <div className="grid gap-4">
      <VideoHeader corpusId={corpusId} data={data} />
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(360px,0.9fr)]">
        <div className="grid content-start gap-4">
          <Surface>
            <SurfaceContent className="grid gap-3 pt-4">
              {playerReady ? (
                <VideoPlayer
                  sourceUrl={data.video.sourceUrl}
                  start={activeMatch?.startSeconds ?? null}
                  frameRef={player.frameRef}
                  onLoad={player.onLoad}
                />
              ) : (
                <LoadingState label="Resolving moment" />
              )}
            </SurfaceContent>
          </Surface>
          {moment ? (
            <MomentPanel
              data={data}
              moment={moment}
              pending={momentQuery.isPending}
              error={momentQuery.error}
              resolved={resolved}
              problem={momentProblem}
              onPlay={(seconds) => player.seek(seconds)}
            />
          ) : null}
        </div>
        <EvidencePanel data={data} active={activeMatch} />
      </div>
      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Processing</SurfaceTitle>
          <SurfaceDescription>
            Detector stages and reprocessing for this video only.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-3">
          <VideoProcessing corpusId={corpusId} videoId={videoId} />
          <VideoReprocessControl corpusId={corpusId} video={data.video} />
          <a
            className="text-sm underline-offset-2 hover:underline"
            href={processingUrl(new URL(window.location.href), videoId).href}
            onClick={navigateInApp}
          >
            Plan, overrides and attempt history in Processing
          </a>
        </SurfaceContent>
      </Surface>
    </div>
  );
}

function VideoHeader({ corpusId, data }: { corpusId: string; data: VideoInspection }) {
  const { video } = data;
  const labels = useCorpusLabels(corpusId).data ?? [];
  const quality = useQuery({
    queryKey: corpusKeys.quality(corpusId),
    queryFn: () => listTranscriptQuality(corpusId),
  });
  const current = new URL(window.location.href);
  const query = readUrlState(current).query;
  return (
    <Surface>
      <SurfaceHeader>
        <div className="flex min-w-0 flex-wrap items-start gap-3">
          <div className="min-w-0 flex-1">
            <SurfaceTitle className="break-words">{video.title ?? video.sourceUrl}</SurfaceTitle>
            <VideoChannel video={video} />
          </div>
          <div className="flex flex-wrap gap-2">
            {query ? (
              <Button asChild size="sm">
                <a href={corpusPageUrl(current, "search").href} onClick={navigateInApp}>
                  Back to search “{query}”
                </a>
              </Button>
            ) : null}
            <Button asChild variant="outline" size="sm">
              <a href={corpusPageUrl(current, "videos").href} onClick={navigateInApp}>
                All videos
              </a>
            </Button>
            <Button asChild variant="ghost" size="sm">
              <a href={video.sourceUrl} target="_blank" rel="noreferrer">
                Open source
              </a>
            </Button>
          </div>
        </div>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        <VideoOverview
          corpusId={corpusId}
          video={video}
          labels={labels}
          quality={(quality.data ?? []).filter((item) => item.videoId === video.id)}
        />
        {data.description ? (
          <details className="text-sm">
            <summary className="cursor-pointer font-medium">Description</summary>
            <p className="mt-2 whitespace-pre-line text-muted-foreground">{data.description}</p>
          </details>
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

/** Seeks an embedded YouTube player through its documented iframe postMessage API. */
function useYouTubePlayer() {
  const frameRef = React.useRef<HTMLIFrameElement>(null);
  const post = React.useCallback((message: object) => {
    frameRef.current?.contentWindow?.postMessage(JSON.stringify(message), "*");
  }, []);
  return React.useMemo(
    () => ({
      frameRef,
      onLoad: () => post({ event: "listening", id: 1 }),
      seek: (seconds: number) => {
        post({ event: "command", func: "seekTo", args: [Math.max(0, seconds), true] });
        post({ event: "command", func: "playVideo", args: [] });
      },
    }),
    [post],
  );
}

function VideoPlayer({
  sourceUrl,
  start,
  frameRef,
  onLoad,
}: {
  sourceUrl: string;
  start: number | null;
  frameRef: React.RefObject<HTMLIFrameElement | null>;
  onLoad: () => void;
}) {
  // The embed opens at the time it first rendered with; later moments seek in place.
  const [initialStart] = React.useState(start);
  const parsed = parseYouTubeVideoUrl(sourceUrl);
  if (!parsed) {
    return (
      <StateView variant="empty">
        <StateViewTitle>No embedded player for this source</StateViewTitle>
        <StateViewDescription>
          <a
            className="underline"
            href={timestampUrl(sourceUrl, start)}
            target="_blank"
            rel="noreferrer"
          >
            Open the original source
          </a>
          {start !== null ? ` at ${formatTimestamp(start)}` : ""}.
        </StateViewDescription>
      </StateView>
    );
  }
  const embed = new URL(parsed.embedUrl);
  embed.searchParams.set("enablejsapi", "1");
  embed.searchParams.set("playsinline", "1");
  embed.searchParams.set("rel", "0");
  embed.searchParams.set("origin", window.location.origin);
  if (initialStart !== null) {
    embed.searchParams.set("start", String(Math.floor(initialStart)));
  }
  return (
    <div className="aspect-video w-full overflow-hidden rounded-md bg-black">
      <iframe
        ref={frameRef}
        className="h-full w-full"
        src={embed.href}
        title="Video player"
        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
        allowFullScreen
        onLoad={onLoad}
      />
    </div>
  );
}

/** Why a resolved evidence record does not match the moment URL that named it. */
function locatorMismatch(locator: MomentLocator, match: SearchResult) {
  if (match.videoId !== locator.videoId) {
    return "This evidence belongs to another video.";
  }
  if (momentModality(match.sourceKind) !== locator.modality) {
    return locator.modality === "spoken"
      ? "This evidence is on-screen text, not speech."
      : "This evidence is speech, not on-screen text.";
  }
  return null;
}

function MomentPanel({
  data,
  moment,
  pending,
  error,
  resolved,
  problem,
  onPlay,
}: {
  data: VideoInspection;
  moment: MomentLocator;
  pending: boolean;
  error: unknown;
  resolved: Awaited<ReturnType<typeof loadTranscriptContext>> | null;
  problem: string | null;
  onPlay: (seconds: number) => void;
}) {
  const current = new URL(window.location.href);
  if (pending) {
    return <LoadingState label="Loading moment" />;
  }
  if (error) {
    return (
      <ErrorState>
        <StateViewTitle>Moment not found</StateViewTitle>
        <StateViewDescription>
          {errorMessage(error)} The evidence may have been reprocessed or removed.
        </StateViewDescription>
        <Button asChild variant="outline">
          <a href={videoUrl(current, moment.videoId).href} onClick={navigateInApp}>
            Show the whole video
          </a>
        </Button>
      </ErrorState>
    );
  }
  if (!resolved) {
    return null;
  }
  const match = resolved.match;
  if (problem) {
    return (
      <ErrorState>
        <StateViewTitle>Moment link does not match its evidence</StateViewTitle>
        <StateViewDescription>{problem}</StateViewDescription>
        <Button asChild variant="outline">
          <a
            href={
              videoUrl(current, match.videoId, {
                modality: momentModality(match.sourceKind),
                evidenceId: match.segmentId,
              }).href
            }
            onClick={navigateInApp}
          >
            Open the evidence where it belongs
          </a>
        </Button>
      </ErrorState>
    );
  }
  const visible = match.sourceKind === "visual_ocr";
  const stream = visible
    ? null
    : (data.transcriptStreams.find((item) => item.streamId === match.streamId) ?? null);
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Moment {formatInterval(match.startSeconds, match.endSeconds)}</SurfaceTitle>
        <SurfaceDescription>
          {visible ? "On-screen text" : "Spoken"} · {sourceLabel(match.sourceKind)}
          {match.language ? ` · ${match.language}` : ""}
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        <blockquote className="rounded-md border border-border bg-muted px-3 py-3 text-sm leading-6">
          {match.text}
        </blockquote>
        <div className="flex flex-wrap gap-2">
          <Button type="button" size="sm" onClick={() => onPlay(match.startSeconds ?? 0)}>
            Play from {formatTimestamp(match.startSeconds)}
          </Button>
          <Button asChild variant="outline" size="sm">
            <a
              href={timestampUrl(match.sourceUrl, match.startSeconds)}
              target="_blank"
              rel="noreferrer"
            >
              Open in YouTube
            </a>
          </Button>
          <Button asChild variant="outline" size="sm">
            <a href={eventsForMomentUrl(current, moment).href} onClick={navigateInApp}>
              Record event claim
            </a>
          </Button>
        </div>
        <div className="grid gap-1 text-xs text-muted-foreground">
          <p className="break-all">
            Evidence {visible ? "OCR track" : "transcript segment"} {match.segmentId}
          </p>
          {stream ? (
            <p>
              Stream {stream.streamId} · {stream.processor} {stream.processorVersion} · revision{" "}
              {stream.processingRevision} · retrieved{" "}
              {new Date(stream.retrievedAt).toLocaleString()}
              {stream.isPreferred ? " · preferred research stream" : ""}
            </p>
          ) : null}
          {match.ocrEvidence ? <OcrEvidenceDetails evidence={match.ocrEvidence} open /> : null}
        </div>
        <div className="grid gap-1 border-t border-border pt-3">
          <p className="text-xs font-medium text-muted-foreground">
            {visible ? "Nearby on-screen text" : "Surrounding transcript"}
          </p>
          {resolved.segments.map((segment) => (
            <EvidenceLink
              key={segment.segmentId}
              videoId={match.videoId}
              modality={momentModality(segment.sourceKind)}
              evidenceId={segment.segmentId}
              start={segment.startSeconds}
              end={segment.endSeconds}
              text={segment.text}
              active={segment.isMatch}
            />
          ))}
        </div>
      </SurfaceContent>
    </Surface>
  );
}

function EvidencePanel({ data, active }: { data: VideoInspection; active: SearchResult | null }) {
  const activeModality = active ? momentModality(active.sourceKind) : null;
  const [tab, setTab] = React.useState<MomentModality>(activeModality ?? "spoken");
  React.useEffect(() => {
    if (activeModality) setTab(activeModality);
  }, [activeModality]);
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Evidence</SurfaceTitle>
        <SurfaceDescription>
          Speech and on-screen text stay separate. Select a passage to open it as a moment.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent>
        <Tabs value={tab} onValueChange={(value) => setTab(value as MomentModality)}>
          <TabsList>
            <TabsTrigger value="spoken">Transcript</TabsTrigger>
            <TabsTrigger value="visible">On-screen text ({data.visualText.length})</TabsTrigger>
          </TabsList>
          <TabsContent value="spoken" className="pt-3">
            <TranscriptEvidence data={data} active={activeModality === "spoken" ? active : null} />
          </TabsContent>
          <TabsContent value="visible" className="pt-3">
            <VisibleTextEvidence
              data={data}
              active={activeModality === "visible" ? active : null}
            />
          </TabsContent>
        </Tabs>
      </SurfaceContent>
    </Surface>
  );
}

function TranscriptEvidence({
  data,
  active,
}: {
  data: VideoInspection;
  active: SearchResult | null;
}) {
  const streams = data.transcriptStreams;
  const [streamId, setStreamId] = React.useState<string | null>(
    active?.streamId ?? streams[0]?.streamId ?? null,
  );
  React.useEffect(() => {
    if (active) setStreamId(active.streamId);
  }, [active]);
  const stream: TranscriptStreamEvidence | undefined =
    streams.find((item) => item.streamId === streamId) ?? streams[0];
  if (!stream) {
    return (
      <StateView variant="empty">
        <StateViewTitle>No transcript yet</StateViewTitle>
        <StateViewDescription>
          Redownload captions or rerun ASR under Processing to add speech evidence.
        </StateViewDescription>
      </StateView>
    );
  }
  return (
    <div className="grid gap-2">
      {streams.length > 1 ? (
        <NativeSelect
          aria-label="Transcript stream"
          value={stream.streamId}
          onChange={(event) => setStreamId(event.target.value)}
        >
          {streams.map((item) => (
            <option key={item.streamId} value={item.streamId}>
              {sourceLabel(item.sourceKind)}
              {item.language ? ` (${item.language})` : ""}
              {item.isPreferred ? " · preferred" : ""}
            </option>
          ))}
        </NativeSelect>
      ) : null}
      <p className="text-xs text-muted-foreground">
        {sourceLabel(stream.sourceKind)}
        {stream.language ? ` · ${stream.language}` : ""} · {stream.segments.length} segments ·{" "}
        {stream.processor} {stream.processorVersion} · revision {stream.processingRevision}
        {stream.isPreferred ? " · preferred research stream" : ""}
      </p>
      {stream.segments.length === 0 ? (
        <StateView variant="empty">
          <StateViewTitle>No segments in this stream</StateViewTitle>
          <StateViewDescription>Stream status: {stream.status}.</StateViewDescription>
        </StateView>
      ) : (
        <div data-evidence-scroll className="relative grid max-h-[70vh] gap-1 overflow-y-auto pr-1">
          {stream.segments.map((segment) => (
            <EvidenceLink
              key={segment.segmentId}
              videoId={data.video.id}
              modality="spoken"
              evidenceId={segment.segmentId}
              start={segment.startSeconds}
              end={segment.endSeconds}
              text={segment.text}
              active={active?.segmentId === segment.segmentId}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function VisibleTextEvidence({
  data,
  active,
}: {
  data: VideoInspection;
  active: SearchResult | null;
}) {
  if (data.ocrStatus === "unprocessed") {
    return (
      <StateView variant="empty">
        <StateViewTitle>On-screen text not processed</StateViewTitle>
        <StateViewDescription>
          Run OCR from Processing stages. Until then this video's visible text is unknown.
        </StateViewDescription>
      </StateView>
    );
  }
  if (data.ocrStatus === "attempted_without_tracks") {
    return (
      <StateView variant="empty">
        <StateViewTitle>No on-screen text found</StateViewTitle>
        <StateViewDescription>
          OCR ran on this video but produced no text tracks.
        </StateViewDescription>
      </StateView>
    );
  }
  return (
    <div data-evidence-scroll className="relative grid max-h-[70vh] gap-2 overflow-y-auto pr-1">
      {data.visualText.map((track) => (
        <div key={track.segmentId} className="grid gap-1">
          <EvidenceLink
            videoId={data.video.id}
            modality="visible"
            evidenceId={track.segmentId}
            start={track.startSeconds}
            end={track.endSeconds}
            text={track.text}
            detail={track.ocrEvidence?.role.replaceAll("_", " ")}
            active={active?.segmentId === track.segmentId}
          />
          <div className="px-3">
            <OcrEvidenceDetails evidence={track.ocrEvidence} />
          </div>
        </div>
      ))}
    </div>
  );
}

/** One timestamped evidence record; following it opens that moment of the video. */
function EvidenceLink({
  videoId,
  modality,
  evidenceId,
  start,
  end,
  text,
  detail,
  active,
}: {
  videoId: string;
  modality: MomentModality;
  evidenceId: string;
  start: number | null;
  end: number | null;
  text: string;
  detail?: string;
  active: boolean;
}) {
  const ref = React.useRef<HTMLAnchorElement>(null);
  React.useEffect(() => {
    // Scroll only the evidence list, never the page, to the active record.
    const node = ref.current;
    const list = node?.closest<HTMLElement>("[data-evidence-scroll]");
    if (active && node && list) {
      list.scrollTop = node.offsetTop - list.clientHeight / 3;
    }
  }, [active]);
  return (
    <a
      ref={ref}
      href={videoUrl(new URL(window.location.href), videoId, { modality, evidenceId }).href}
      onClick={navigateInPlace}
      aria-current={active ? "true" : undefined}
      className={
        active
          ? "grid gap-1 rounded-md border border-primary bg-muted px-3 py-2"
          : "grid gap-1 rounded-md px-3 py-2 hover:bg-muted focus-visible:bg-muted"
      }
    >
      <span className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {formatInterval(start, end)}
        {detail ? <Badge variant="outline">{detail}</Badge> : null}
      </span>
      <span className="text-sm leading-6">{text}</span>
    </a>
  );
}
