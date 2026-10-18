import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  Button,
  ErrorState,
  Input,
  LoadingState,
  NativeSelect,
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
  getDetectorProfiles,
  getVideoDetectorPlan,
  listCorpusVideos,
  runVideoDetectorPlan,
  setCorpusDetectors,
  setSourceDetectors,
  setVideoDetectorOverrides,
  type DetectorInfo,
  type DetectorSetting,
  type PlanContributor,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { corpusKeys } from "./query-keys";

type Choice = "inherit" | "on" | "off";
type Draft = Record<string, { choice: Choice; languages: string }>;

function toDraft(settings: DetectorSetting[]): Draft {
  return Object.fromEntries(
    settings.map((setting) => [
      setting.detector,
      {
        choice: setting.enabled ? "on" : "off",
        languages: setting.targetLanguages.join(", "),
      },
    ]),
  );
}

function fromDraft(draft: Draft): DetectorSetting[] {
  return Object.entries(draft)
    .filter(([, value]) => value.choice !== "inherit")
    .map(([detector, value]) => ({
      detector,
      enabled: value.choice === "on",
      targetLanguages:
        value.choice === "on"
          ? value.languages
              .split(",")
              .map((language) => language.trim())
              .filter(Boolean)
          : [],
    }));
}

/** Corpus detector profile, tracked-source overrides and per-video effective plans (#63). */
export function CorpusDetectors({ corpusId }: { corpusId: string }) {
  const profilesQuery = useQuery({
    queryKey: corpusKeys.detectorProfiles(corpusId),
    queryFn: () => getDetectorProfiles(corpusId),
  });
  const profiles = profilesQuery.data;
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Detectors</SurfaceTitle>
        <SurfaceDescription>
          Choose which analyses run. The corpus sets the default; a tracked source can switch single
          detectors on or off; a video can be overridden within this corpus only. A video found
          through several sources gets every detector any of them requests, run once.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-6">
        {profilesQuery.isPending ? <LoadingState label="Loading detector profiles" /> : null}
        {profilesQuery.error ? (
          <ErrorState>
            <StateViewTitle>Detector profiles unavailable</StateViewTitle>
            <StateViewDescription>{errorMessage(profilesQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {profiles ? (
          <>
            <ScopeEditor
              title="Corpus default"
              corpusId={corpusId}
              detectors={profiles.detectors}
              settings={profiles.corpus}
              allowInherit={false}
              save={(settings) => setCorpusDetectors(corpusId, settings)}
            />
            {profiles.sources.map((source) => (
              <ScopeEditor
                key={source.sourceId}
                title={`Source: ${source.name ?? source.sourceUrl}`}
                corpusId={corpusId}
                detectors={profiles.detectors}
                settings={source.settings}
                allowInherit
                save={(settings) => setSourceDetectors(corpusId, source.sourceId, settings)}
              />
            ))}
            <VideoPlan corpusId={corpusId} detectors={profiles.detectors} />
          </>
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

function ScopeEditor({
  title,
  corpusId,
  detectors,
  settings,
  allowInherit,
  save,
}: {
  title: string;
  corpusId: string;
  detectors: DetectorInfo[];
  settings: DetectorSetting[];
  allowInherit: boolean;
  save: (settings: DetectorSetting[]) => Promise<void>;
}) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState<Draft>(() => toDraft(settings));
  React.useEffect(() => setDraft(toDraft(settings)), [settings]);
  const mutation = useMutation({
    mutationFn: () => save(fromDraft(draft)),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: corpusKeys.detectorProfiles(corpusId) }),
  });
  return (
    <section className="grid gap-2" aria-label={title}>
      <h3 className="text-sm font-semibold">{title}</h3>
      <DetectorTable
        detectors={detectors}
        draft={draft}
        allowInherit={allowInherit}
        onChange={setDraft}
      />
      <div className="flex items-center gap-2">
        <Button size="sm" disabled={mutation.isPending} onClick={() => mutation.mutate()}>
          {mutation.isPending ? "Saving..." : "Save"}
        </Button>
        {mutation.isSuccess ? <span className="text-sm text-muted-foreground">Saved</span> : null}
        {mutation.error ? (
          <span role="alert" className="text-sm text-destructive">
            {errorMessage(mutation.error)}
          </span>
        ) : null}
      </div>
    </section>
  );
}

function DetectorTable({
  detectors,
  draft,
  allowInherit,
  onChange,
  plan,
}: {
  detectors: DetectorInfo[];
  draft: Draft;
  allowInherit: boolean;
  onChange: (draft: Draft) => void;
  plan?: Record<string, { enabled: boolean; contributors: PlanContributor[]; languages: string[] }>;
}) {
  return (
    <ul className="grid gap-2">
      {detectors.map((detector) => {
        const value = draft[detector.id] ?? {
          choice: allowInherit ? "inherit" : "off",
          languages: "",
        };
        const entry = plan?.[detector.id];
        return (
          <li
            key={detector.id}
            className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2"
          >
            <span className="min-w-48 font-medium">{detector.label}</span>
            {detector.available ? null : <Badge variant="outline">Unavailable</Badge>}
            <NativeSelect
              aria-label={`${detector.label} setting`}
              value={value.choice}
              onChange={(event) =>
                onChange({
                  ...draft,
                  [detector.id]: { ...value, choice: event.target.value as Choice },
                })
              }
            >
              {allowInherit ? <option value="inherit">Inherit</option> : null}
              <option value="on">On</option>
              <option value="off">Off</option>
            </NativeSelect>
            {detector.takesLanguages && value.choice === "on" ? (
              <Input
                aria-label={`${detector.label} target languages`}
                className="w-40"
                value={value.languages}
                placeholder="de, es"
                onChange={(event) =>
                  onChange({ ...draft, [detector.id]: { ...value, languages: event.target.value } })
                }
              />
            ) : null}
            {entry ? (
              <>
                <Badge variant={entry.enabled ? "default" : "secondary"}>
                  {entry.enabled ? "Will run" : "Off"}
                </Badge>
                {entry.languages.length > 0 ? (
                  <Badge variant="outline">→ {entry.languages.join(", ")}</Badge>
                ) : null}
                {entry.contributors.length > 0 ? (
                  <span className="text-sm text-muted-foreground">
                    requested by{" "}
                    {entry.contributors
                      .map(
                        (contributor) =>
                          `${contributor.name ?? contributor.kind}${contributor.explicit ? "" : " (corpus default)"}`,
                      )
                      .join(", ")}
                  </span>
                ) : null}
              </>
            ) : null}
            {detector.note ? (
              <span className="basis-full text-xs text-muted-foreground">{detector.note}</span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}

function VideoPlan({ corpusId, detectors }: { corpusId: string; detectors: DetectorInfo[] }) {
  const queryClient = useQueryClient();
  const videosQuery = useQuery({
    queryKey: corpusKeys.videos(corpusId, 200),
    queryFn: () => listCorpusVideos(corpusId, 200),
  });
  const [videoId, setVideoId] = React.useState<string>("");
  const planQuery = useQuery({
    queryKey: corpusKeys.detectorPlan(corpusId, videoId),
    queryFn: () => getVideoDetectorPlan(corpusId, videoId),
    enabled: videoId !== "",
  });
  const [draft, setDraft] = React.useState<Draft>({});
  React.useEffect(() => {
    setDraft(planQuery.data ? toDraft(planQuery.data.videoOverrides) : {});
  }, [planQuery.data]);
  const [refresh, setRefresh] = React.useState(false);
  const saveMutation = useMutation({
    mutationFn: () => setVideoDetectorOverrides(corpusId, videoId, fromDraft(draft)),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: corpusKeys.detectorPlan(corpusId, videoId) }),
  });
  const runMutation = useMutation({
    mutationFn: () => runVideoDetectorPlan(corpusId, videoId, refresh),
  });

  const plan = planQuery.data
    ? Object.fromEntries(
        planQuery.data.detectors.map((entry) => [
          entry.detector,
          {
            enabled: entry.enabled,
            contributors: entry.contributors,
            languages: entry.targetLanguages,
          },
        ]),
      )
    : undefined;

  return (
    <section className="grid gap-2" aria-label="Video detector plan">
      <h3 className="text-sm font-semibold">Effective plan for a video</h3>
      <NativeSelect
        aria-label="Video"
        value={videoId}
        onChange={(event) => {
          setVideoId(event.target.value);
          runMutation.reset();
        }}
      >
        <option value="">Choose a video</option>
        {videosQuery.data?.map((video) => (
          <option key={video.id} value={video.id}>
            {video.title ?? video.sourceUrl}
          </option>
        ))}
      </NativeSelect>
      {planQuery.isFetching ? <LoadingState label="Resolving plan" /> : null}
      {planQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(planQuery.error)}
        </p>
      ) : null}
      {planQuery.data ? (
        <>
          <p className="text-sm text-muted-foreground">
            {planQuery.data.addedDirectly ? "Added directly. " : ""}
            {planQuery.data.sources.length > 0
              ? `Tracked through ${planQuery.data.sources.map((source) => source.name ?? source.id).join(", ")}.`
              : "Not tracked through a source; the corpus default applies."}{" "}
            Overrides here apply only in this corpus.
          </p>
          <DetectorTable
            detectors={detectors}
            draft={draft}
            allowInherit
            onChange={setDraft}
            plan={plan}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={saveMutation.isPending}
              onClick={() => saveMutation.mutate()}
            >
              {saveMutation.isPending ? "Saving..." : "Save video overrides"}
            </Button>
            <label className="flex items-center gap-1 text-sm">
              <input
                type="checkbox"
                checked={refresh}
                onChange={(event) => setRefresh(event.target.checked)}
              />
              Re-run even if output exists
            </label>
            <Button size="sm" disabled={runMutation.isPending} onClick={() => runMutation.mutate()}>
              {runMutation.isPending ? "Running..." : "Run plan"}
            </Button>
          </div>
          {saveMutation.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(saveMutation.error)}
            </p>
          ) : null}
          {runMutation.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(runMutation.error)}
            </p>
          ) : null}
          {runMutation.data ? (
            <ul className="grid gap-1 text-sm" aria-label="Plan run result">
              {runMutation.data.stages.length === 0 ? <li>No detector is enabled.</li> : null}
              {runMutation.data.stages.map((stage) => (
                <li key={stage.detector} className="flex flex-wrap gap-2">
                  <span className="font-medium">{stage.detector}</span>
                  <Badge variant={stage.outcome === "failed" ? "destructive" : "outline"}>
                    {stage.outcome}
                  </Badge>
                  {stage.message ? (
                    <span className="text-muted-foreground">{stage.message}</span>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
