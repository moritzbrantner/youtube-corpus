import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

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
  attemptStates,
  getDetectorAttempt,
  getVideoProcessingStatus,
  listCorpusVideos,
  listDetectorAttempts,
  runVideoDetectorPlan,
  type DetectorAttempt,
  type DetectorAttemptFilter,
} from "./api";
import { PipelineStages, VideoPlan, useDetectorProfiles } from "./CorpusDetectors";
import { errorMessage } from "./CorpusLabels";
import { corpusKeys } from "./query-keys";
import {
  attemptUrl,
  navigateInApp,
  processingUrl,
  type CorpusUrlState,
  videoUrl,
} from "./url-state";
import { StageStateBadge, VideoProcessing } from "./VideoProcessing";

type ProcessingPageProps = {
  corpusId: string;
  attemptId: string | null;
  state: CorpusUrlState;
  onUpdate: (patch: Partial<CorpusUrlState>) => void;
};

/**
 * `/processing`: the corpus pipeline, recorded runs and one selected video's stages;
 * `/processing/attempts/:id`: one recorded attempt (#103, Atlas 6.2).
 */
export function CorpusProcessingPage({
  corpusId,
  attemptId,
  state,
  onUpdate,
}: ProcessingPageProps) {
  if (attemptId) {
    return <AttemptInspector corpusId={corpusId} attemptId={attemptId} />;
  }
  return (
    <div className="grid gap-4">
      <PipelineStages corpusId={corpusId} />
      <div className="grid gap-4 xl:grid-cols-2">
        <RunList corpusId={corpusId} state={state} onUpdate={onUpdate} />
        <SelectedVideo
          corpusId={corpusId}
          videoId={state.processingVideoId}
          onSelect={(processingVideoId) => onUpdate({ processingVideoId })}
        />
      </div>
    </div>
  );
}

function useAttempts(corpusId: string, filter: DetectorAttemptFilter, enabled = true) {
  return useQuery({
    queryKey: corpusKeys.attempts(corpusId, filter),
    queryFn: () => listDetectorAttempts(corpusId, filter),
    enabled,
    // Follow live work only while a recorded attempt is actually running.
    refetchInterval: (query) =>
      query.state.data?.some((attempt) => attempt.state === "running") ? 2000 : false,
  });
}

function RunList({
  corpusId,
  state,
  onUpdate,
}: {
  corpusId: string;
  state: CorpusUrlState;
  onUpdate: (patch: Partial<CorpusUrlState>) => void;
}) {
  const profiles = useDetectorProfiles(corpusId);
  const filter: DetectorAttemptFilter = {
    detector: state.stage || undefined,
    state: state.attemptState || undefined,
    latest: state.attempts !== "all",
    limit: 100,
  };
  const attempts = useAttempts(corpusId, filter);
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Runs</SurfaceTitle>
        <SurfaceDescription>
          Recorded stage attempts of this corpus's videos, newest first. Open one to inspect its
          input, failure and history.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        <div className="flex flex-wrap items-end gap-2">
          <label className="grid gap-1 text-sm">
            Stage
            <NativeSelect
              aria-label="Stage"
              value={state.stage}
              onChange={(event) => onUpdate({ stage: event.target.value })}
            >
              <option value="">All stages</option>
              {profiles.data?.detectors.map((detector) => (
                <option key={detector.id} value={detector.id}>
                  {detector.label}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label className="grid gap-1 text-sm">
            State
            <NativeSelect
              aria-label="Attempt state"
              value={state.attemptState}
              onChange={(event) => onUpdate({ attemptState: event.target.value })}
            >
              <option value="">Any state</option>
              {attemptStates.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label className="flex items-center gap-1 pb-2 text-sm">
            <input
              type="checkbox"
              checked={state.attempts === "all"}
              onChange={(event) => onUpdate({ attempts: event.target.checked ? "all" : "" })}
            />
            Include superseded attempts
          </label>
          <Button size="sm" variant="ghost" onClick={() => void attempts.refetch()}>
            Refresh runs
          </Button>
        </div>
        <AttemptList
          query={attempts}
          empty="No recorded attempts match. Attempts are recorded whenever a stage actually runs."
        />
      </SurfaceContent>
    </Surface>
  );
}

function AttemptList({
  query,
  empty,
  currentId,
  showVideo = true,
}: {
  query: ReturnType<typeof useAttempts>;
  empty: string;
  currentId?: string;
  showVideo?: boolean;
}) {
  if (query.isPending) return <LoadingState label="Loading runs" />;
  if (query.error) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {errorMessage(query.error)}
      </p>
    );
  }
  if (query.data.length === 0) {
    return <p className="text-sm text-muted-foreground">{empty}</p>;
  }
  return <AttemptRows attempts={query.data} currentId={currentId} showVideo={showVideo} />;
}

function AttemptRows({
  attempts,
  currentId,
  showVideo,
}: {
  attempts: DetectorAttempt[];
  currentId?: string;
  showVideo: boolean;
}) {
  const here = new URL(window.location.href);
  return (
    <ul className="grid gap-2" aria-label="Attempts">
      {attempts.map((attempt) => (
        <li
          key={attempt.id}
          aria-current={attempt.id === currentId ? "true" : undefined}
          className="grid gap-1 rounded-md border border-border px-3 py-2 text-sm aria-[current]:border-primary"
        >
          <div className="flex flex-wrap items-center gap-2">
            <a
              className="font-medium underline-offset-2 hover:underline"
              href={attemptUrl(here, attempt.id).href}
              onClick={navigateInApp}
            >
              {attempt.label}
            </a>
            <StageStateBadge state={attempt.state} />
            {attempt.latest ? null : <Badge variant="outline">superseded</Badge>}
            <span className="text-xs text-muted-foreground">
              {formatTime(attempt.startedAt)} · {formatDuration(attempt)}
            </span>
          </div>
          {showVideo ? (
            <a
              className="min-w-0 truncate text-muted-foreground underline-offset-2 hover:underline"
              href={processingUrl(here, attempt.videoId).href}
              onClick={navigateInApp}
            >
              {attempt.videoTitle ?? attempt.videoSourceUrl}
            </a>
          ) : null}
          {attempt.message ? (
            <span
              className={`line-clamp-2 text-xs ${attempt.state === "failed" || attempt.state === "interrupted" ? "text-destructive" : "text-muted-foreground"}`}
            >
              {attempt.message}
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

function SelectedVideo({
  corpusId,
  videoId,
  onSelect,
}: {
  corpusId: string;
  videoId: string | null;
  onSelect: (videoId: string | null) => void;
}) {
  const videos = useQuery({
    queryKey: corpusKeys.videos(corpusId, 200),
    queryFn: () => listCorpusVideos(corpusId, 200),
  });
  const profiles = useDetectorProfiles(corpusId);
  const history = useAttempts(corpusId, { videoId: videoId ?? undefined, limit: 50 }, !!videoId);
  const here = new URL(window.location.href);
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Video</SurfaceTitle>
        <SurfaceDescription>
          A video's live stage status, retry and refresh controls, effective plan and attempt
          history.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        <NativeSelect
          aria-label="Video"
          value={videoId ?? ""}
          onChange={(event) => onSelect(event.target.value || null)}
        >
          <option value="">Choose a video</option>
          {videoId && videos.data && !videos.data.some((video) => video.id === videoId) ? (
            <option value={videoId}>{videoId}</option>
          ) : null}
          {videos.data?.map((video) => (
            <option key={video.id} value={video.id}>
              {video.title ?? video.sourceUrl}
            </option>
          ))}
        </NativeSelect>
        {videoId ? (
          <div key={videoId} className="grid gap-4">
            <a
              className="text-sm underline-offset-2 hover:underline"
              href={videoUrl(here, videoId).href}
              onClick={navigateInApp}
            >
              Open video page
            </a>
            <VideoProcessing corpusId={corpusId} videoId={videoId} defaultOpen />
            <section className="grid gap-2" aria-label="Attempt history">
              <h3 className="text-sm font-semibold">Attempt history</h3>
              <AttemptList
                query={history}
                showVideo={false}
                empty="No stage of this video has run yet."
              />
            </section>
            {profiles.data ? (
              <VideoPlan
                corpusId={corpusId}
                videoId={videoId}
                detectors={profiles.data.detectors}
              />
            ) : null}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Choose a video, or open one from the runs list.
          </p>
        )}
      </SurfaceContent>
    </Surface>
  );
}

function AttemptInspector({ corpusId, attemptId }: { corpusId: string; attemptId: string }) {
  const client = useQueryClient();
  const attempt = useQuery({
    queryKey: corpusKeys.attempt(corpusId, attemptId),
    queryFn: () => getDetectorAttempt(corpusId, attemptId),
    refetchInterval: (query) => (query.state.data?.state === "running" ? 2000 : false),
  });
  const videoId = attempt.data?.videoId ?? "";
  const status = useQuery({
    queryKey: corpusKeys.processingStatus(corpusId, videoId),
    queryFn: () => getVideoProcessingStatus(corpusId, videoId),
    enabled: videoId !== "",
  });
  const retry = useMutation({
    mutationFn: ({ refresh }: { refresh: boolean }) =>
      runVideoDetectorPlan(corpusId, videoId, refresh, [attempt.data?.detector ?? ""]),
    onSettled: () => client.invalidateQueries({ queryKey: ["corpora", corpusId] }),
  });
  const here = new URL(window.location.href);
  const back = (
    <Button asChild variant="outline" size="sm">
      <a href={processingUrl(here).href} onClick={navigateInApp}>
        All runs
      </a>
    </Button>
  );

  if (attempt.isPending) return <LoadingState label="Loading attempt" />;
  if (attempt.error) {
    return (
      <ErrorState>
        <StateViewTitle>Attempt unavailable</StateViewTitle>
        <StateViewDescription>{errorMessage(attempt.error)}</StateViewDescription>
        {back}
      </ErrorState>
    );
  }
  const data = attempt.data;
  const newest = data.history[0];
  const stages = status.data?.stages.filter((stage) => stage.detector === data.detector) ?? [];
  const failed = data.state === "failed" || data.state === "interrupted";

  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap gap-2">
        {back}
        <Button asChild variant="outline" size="sm">
          <a href={processingUrl(here, data.videoId).href} onClick={navigateInApp}>
            Video processing
          </a>
        </Button>
      </div>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
        <Surface>
          <SurfaceHeader>
            <SurfaceTitle className="flex flex-wrap items-center gap-2">
              {data.label}
              <StageStateBadge state={data.state} />
              {data.latest ? null : <Badge variant="outline">superseded</Badge>}
            </SurfaceTitle>
            <SurfaceDescription>
              <a href={videoUrl(here, data.videoId).href} onClick={navigateInApp}>
                {data.videoTitle ?? data.videoSourceUrl}
              </a>
            </SurfaceDescription>
          </SurfaceHeader>
          <SurfaceContent className="grid gap-4">
            {!data.latest && newest ? (
              <StateView variant="empty">
                <StateViewTitle>A newer attempt exists</StateViewTitle>
                <StateViewDescription>
                  This attempt no longer describes the stage. The newest attempt is {newest.state}.
                </StateViewDescription>
                <Button asChild size="sm" variant="outline">
                  <a href={attemptUrl(here, newest.id).href} onClick={navigateInApp}>
                    Open newest attempt
                  </a>
                </Button>
              </StateView>
            ) : null}
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
              <dt className="text-muted-foreground">Stage</dt>
              <dd>{data.detector}</dd>
              <dt className="text-muted-foreground">Started</dt>
              <dd>{formatTime(data.startedAt)}</dd>
              <dt className="text-muted-foreground">Finished</dt>
              <dd>{data.finishedAt ? formatTime(data.finishedAt) : "not finished"}</dd>
              <dt className="text-muted-foreground">Duration</dt>
              <dd>{formatDuration(data)}</dd>
            </dl>
            {data.state === "interrupted" ? (
              <p className="text-sm text-destructive">
                Recorded as running, but no process holds this stage any more: the run was
                interrupted before it finished.
              </p>
            ) : null}
            {data.message ? (
              <pre
                className={`whitespace-pre-wrap break-words rounded-md border border-border p-3 text-xs ${failed ? "text-destructive" : ""}`}
                aria-label="Attempt message"
              >
                {data.message}
              </pre>
            ) : null}
            <details>
              <summary className="cursor-pointer text-sm font-medium">
                Recorded input identity
              </summary>
              <pre className="mt-2 overflow-x-auto rounded-md border border-border p-3 text-xs">
                {JSON.stringify(data.identity, null, 2)}
              </pre>
            </details>
          </SurfaceContent>
        </Surface>
        <div className="grid content-start gap-4">
          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Current stage</SurfaceTitle>
              <SurfaceDescription>
                What the stage reports now for this video, from its retained outputs and newest
                attempt.
              </SurfaceDescription>
            </SurfaceHeader>
            <SurfaceContent className="grid gap-3">
              {status.isPending ? <LoadingState label="Loading stage status" /> : null}
              {status.error ? (
                <p role="alert" className="text-sm text-destructive">
                  {errorMessage(status.error)}
                </p>
              ) : null}
              {status.data && stages.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  This stage is no longer part of the corpus plan.
                </p>
              ) : null}
              <ul className="grid gap-1 text-sm">
                {stages.map((stage) => (
                  <li
                    key={stage.targetLanguage ?? ""}
                    className="flex flex-wrap items-center gap-2"
                  >
                    {stage.targetLanguage ? <span>→ {stage.targetLanguage}</span> : null}
                    <StageStateBadge state={stage.state} />
                    <span className="text-xs text-muted-foreground">
                      {stage.outputCount} retained outputs
                    </span>
                  </li>
                ))}
              </ul>
              <div className="flex flex-wrap gap-2">
                {data.state !== "running" && data.state !== "completed" ? (
                  <Button
                    size="sm"
                    disabled={retry.isPending}
                    onClick={() => retry.mutate({ refresh: false })}
                  >
                    Retry {data.label}
                  </Button>
                ) : null}
                {data.state === "completed" ? (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={retry.isPending}
                    onClick={() => retry.mutate({ refresh: true })}
                  >
                    Refresh {data.label}
                  </Button>
                ) : null}
              </div>
              {retry.isPending ? <p role="status">Running {data.label}…</p> : null}
              {retry.error ? (
                <p role="alert" className="text-sm text-destructive">
                  {errorMessage(retry.error)}
                </p>
              ) : null}
              {retry.data?.stages.map((stage) => (
                <p key={stage.detector} role="status" className="text-sm">
                  {stage.detector}: {stage.outcome}
                  {stage.message ? ` — ${stage.message}` : ""}
                </p>
              ))}
            </SurfaceContent>
          </Surface>
          <Surface>
            <SurfaceHeader>
              <SurfaceTitle>Stage history</SurfaceTitle>
            </SurfaceHeader>
            <SurfaceContent>
              <AttemptRows attempts={data.history} currentId={data.id} showVideo={false} />
            </SurfaceContent>
          </Surface>
        </div>
      </div>
    </div>
  );
}

function formatTime(iso: string) {
  return new Date(iso).toLocaleString();
}

function formatDuration(attempt: Pick<DetectorAttempt, "startedAt" | "finishedAt" | "state">) {
  if (!attempt.finishedAt) {
    return attempt.state === "running" ? "running" : "unfinished";
  }
  const seconds = Math.max(
    0,
    (new Date(attempt.finishedAt).getTime() - new Date(attempt.startedAt).getTime()) / 1000,
  );
  return seconds < 60 ? `${seconds.toFixed(1)} s` : `${Math.round(seconds / 60)} min`;
}
