import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Badge, Button, Input, LoadingState, NativeSelect } from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import {
  getCorpusWorkflow,
  listProcessingWorkflows,
  setCorpusWorkflowOverrides,
  startCorpusWorkflowRun,
  type CorpusWorkflow,
  type DetectorInfo,
  type StageOverride,
  type WorkflowRun,
} from "./api";
import { useDetectorProfiles } from "./CorpusDetectors";
import { errorMessage } from "./CorpusLabels";
import {
  navigateInApp,
  processingUrl,
  processingWorkflowUrl,
  workflowEditorUrl,
} from "./url-state";

type Draft = Record<string, { enabled: boolean; languages: string }>;

function toDraft(workflow: CorpusWorkflow): Draft {
  return Object.fromEntries(
    workflow.stages.map((stage) => [
      stage.nodeId,
      { enabled: stage.enabled, languages: stage.targetLanguages.join(", ") },
    ]),
  );
}

function parseLanguages(text: string) {
  return text
    .split(/[\s,]+/)
    .map((code) => code.trim().toLowerCase())
    .filter(Boolean);
}

/** Only what differs from the definition is stored as this corpus's override. */
function fromDraft(workflow: CorpusWorkflow, draft: Draft): StageOverride[] {
  return workflow.workflow.stages.flatMap((stage) => {
    const entry = draft[stage.nodeId];
    if (!entry) return [];
    const languages = parseLanguages(entry.languages);
    const override: StageOverride = { nodeId: stage.nodeId };
    if (!entry.enabled) override.enabled = false;
    if (languages.join(",") !== stage.targetLanguages.join(",")) {
      override.targetLanguages = languages;
    }
    return Object.keys(override).length > 1 ? [override] : [];
  });
}

/**
 * The shared processing workflow this corpus runs (#108): a reusable definition authored in the
 * Workflow Editor, this corpus's stage enablement and parameters, and this corpus's runs.
 */
export function ProcessingWorkflows({ corpusId }: { corpusId: string }) {
  const workflows = useQuery({
    queryKey: ["processing-workflows"],
    queryFn: listProcessingWorkflows,
  });
  const [selected, setSelected] = React.useState(
    () => new URL(window.location.href).searchParams.get("workflow") ?? "",
  );
  const workflowId =
    workflows.data?.find((workflow) => workflow.id === selected)?.id ?? workflows.data?.[0]?.id;
  const here = new URL(window.location.href);
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Processing workflow</SurfaceTitle>
        <SurfaceDescription>
          A reusable definition authored in the shared Workflow Editor. Starting a run makes its
          enabled stages this corpus's default detector plan and runs every video's plan; source and
          video overrides still apply and unchanged output is reused, also across corpora.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        {workflows.isPending ? <LoadingState label="Loading workflows" /> : null}
        {workflows.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(workflows.error)}
          </p>
        ) : null}
        <div className="flex flex-wrap items-end gap-2">
          {workflows.data && workflows.data.length > 0 ? (
            <label className="grid min-w-64 gap-1 text-sm">
              Workflow
              <NativeSelect
                aria-label="Workflow"
                value={workflowId ?? ""}
                onChange={(event) => {
                  setSelected(event.target.value);
                  window.history.replaceState(
                    null,
                    "",
                    processingWorkflowUrl(new URL(window.location.href), event.target.value),
                  );
                }}
              >
                {workflows.data.map((workflow) => (
                  <option key={workflow.id} value={workflow.id}>
                    {workflow.name} (v{workflow.version})
                  </option>
                ))}
              </NativeSelect>
            </label>
          ) : null}
          {workflowId ? (
            <Button asChild variant="outline">
              <a href={workflowEditorUrl(here, workflowId).href} onClick={navigateInApp}>
                Edit in Workflow Editor
              </a>
            </Button>
          ) : null}
          <Button asChild variant="ghost">
            <a href={workflowEditorUrl(here, "new").href} onClick={navigateInApp}>
              New workflow
            </a>
          </Button>
        </div>
        {workflows.data && workflows.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No processing workflow is defined yet. Create one in the Workflow Editor; until then the
            corpus detector plan below is the pipeline.
          </p>
        ) : null}
        {workflowId ? (
          <CorpusWorkflowPanel key={workflowId} corpusId={corpusId} workflowId={workflowId} />
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

function CorpusWorkflowPanel({ corpusId, workflowId }: { corpusId: string; workflowId: string }) {
  const client = useQueryClient();
  const detectors = useDetectorProfiles(corpusId).data?.detectors ?? [];
  const view = useQuery({
    queryKey: ["corpora", corpusId, "processing-workflows", workflowId],
    queryFn: () => getCorpusWorkflow(corpusId, workflowId),
    refetchInterval: (query) =>
      query.state.data?.runs.some((run) => run.state === "running") ? 2000 : false,
  });
  const [draft, setDraft] = React.useState<Draft | null>(null);
  const [refresh, setRefresh] = React.useState(false);
  const running = view.data?.runs.some((run) => run.state === "running") ?? false;
  const wasRunning = React.useRef(running);
  React.useEffect(() => {
    // A run just ended: stage attempts, statuses and the corpus plan changed.
    if (wasRunning.current && !running)
      void client.invalidateQueries({ queryKey: ["corpora", corpusId] });
    wasRunning.current = running;
  }, [client, corpusId, running]);
  const save = useMutation({
    mutationFn: (stages: StageOverride[]) =>
      setCorpusWorkflowOverrides(corpusId, workflowId, stages),
    onSuccess: (next) => {
      client.setQueryData(["corpora", corpusId, "processing-workflows", workflowId], next);
      setDraft(null);
    },
  });
  const start = useMutation({
    mutationFn: () => startCorpusWorkflowRun(corpusId, workflowId, refresh),
    onSuccess: () => void client.invalidateQueries({ queryKey: ["corpora", corpusId] }),
  });
  if (view.isPending) return <LoadingState label="Loading workflow" />;
  if (view.error) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {errorMessage(view.error)}
      </p>
    );
  }
  const workflow = view.data;
  const current = draft ?? toDraft(workflow);
  const changed =
    draft !== null &&
    JSON.stringify(fromDraft(workflow, draft)) !== JSON.stringify(workflow.overrides);
  const info = (id: string): DetectorInfo | undefined => detectors.find((d) => d.id === id);
  return (
    <div className="grid gap-4">
      <section className="grid gap-2" aria-label="Workflow stages">
        <h3 className="text-sm font-semibold">
          Stages in this corpus{" "}
          <span className="font-normal text-muted-foreground">
            · definition v{workflow.workflow.version}
          </span>
        </h3>
        <ol className="grid gap-2">
          {workflow.stages.map((stage) => {
            const entry = current[stage.nodeId];
            const detector = info(stage.detector);
            return (
              <li
                key={stage.nodeId}
                className="grid gap-2 rounded-md border border-border px-3 py-2 sm:grid-cols-[minmax(12rem,1fr)_auto] sm:items-center"
              >
                <label className="flex min-w-0 items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={entry?.enabled ?? true}
                    onChange={(event) =>
                      setDraft({
                        ...current,
                        [stage.nodeId]: { ...entry, enabled: event.target.checked },
                      })
                    }
                  />
                  <span className="grid min-w-0">
                    <span className="font-medium">{stage.label}</span>
                    <span className="text-xs text-muted-foreground">
                      {detector?.producer ?? stage.detector}
                    </span>
                  </span>
                </label>
                <div className="flex flex-wrap items-center gap-2 text-sm">
                  {detector && !detector.available ? (
                    <Badge variant="outline">Unavailable</Badge>
                  ) : null}
                  {stage.overridden ? <Badge variant="secondary">Corpus override</Badge> : null}
                  {detector?.takesLanguages ? (
                    <Input
                      aria-label={`${stage.label} target languages`}
                      className="h-8 w-36"
                      placeholder="de, fr"
                      value={entry?.languages ?? ""}
                      onChange={(event) =>
                        setDraft({
                          ...current,
                          [stage.nodeId]: { ...entry, languages: event.target.value },
                        })
                      }
                    />
                  ) : null}
                </div>
              </li>
            );
          })}
        </ol>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={!changed || save.isPending}
            onClick={() => draft && save.mutate(fromDraft(workflow, draft))}
          >
            {save.isPending ? "Saving..." : "Save corpus stage settings"}
          </Button>
          {save.error ? (
            <span role="alert" className="text-sm text-destructive">
              {errorMessage(save.error)}
            </span>
          ) : null}
        </div>
      </section>
      <div className="flex flex-wrap items-center gap-3">
        <Button disabled={running || changed || start.isPending} onClick={() => start.mutate()}>
          {running ? "Running..." : "Start run"}
        </Button>
        <label className="flex items-center gap-1 text-sm">
          <input
            type="checkbox"
            checked={refresh}
            onChange={(event) => setRefresh(event.target.checked)}
          />
          Refresh stages that already have output
        </label>
        {changed ? (
          <span className="text-sm text-muted-foreground">Save the stage settings first.</span>
        ) : null}
        {start.error ? (
          <span role="alert" className="text-sm text-destructive">
            {errorMessage(start.error)}
          </span>
        ) : null}
      </div>
      <section className="grid gap-2" aria-label="Workflow runs">
        <h3 className="text-sm font-semibold">Runs in this corpus</h3>
        {workflow.runs.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            This corpus has not run this workflow yet.
          </p>
        ) : (
          <ul className="grid gap-2">
            {workflow.runs.map((run, index) => (
              <RunEntry key={run.id} run={run} defaultOpen={index === 0} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

const outcomeVariant = {
  completed: "default",
  reused: "secondary",
  skipped: "outline",
  unavailable: "outline",
  failed: "destructive",
} as const;

function RunEntry({ run, defaultOpen }: { run: WorkflowRun; defaultOpen: boolean }) {
  const here = new URL(window.location.href);
  const enabled = run.stages.filter((stage) => stage.enabled).map((stage) => stage.label);
  return (
    <li className="rounded-md border border-border px-3 py-2 text-sm">
      <details open={defaultOpen}>
        <summary className="flex cursor-pointer flex-wrap items-center gap-2">
          <Badge
            variant={
              run.state === "failed" || run.state === "interrupted" ? "destructive" : "secondary"
            }
          >
            {run.state}
          </Badge>
          <span>{new Date(run.startedAt).toLocaleString()}</span>
          <span className="text-muted-foreground">v{run.workflowVersion}</span>
          {run.report ? (
            <span className="text-muted-foreground">
              {run.report.videos.length} videos · {run.report.failures} failures
            </span>
          ) : null}
          {run.refresh ? <Badge variant="outline">refresh</Badge> : null}
        </summary>
        <div className="mt-2 grid gap-2">
          <span className="text-xs text-muted-foreground">Stages: {enabled.join(", ")}</span>
          {run.message ? <span className="text-xs text-destructive">{run.message}</span> : null}
          {run.report?.videos.map((video) => (
            <div key={video.videoId} className="grid gap-1">
              <a
                className="min-w-0 truncate underline-offset-2 hover:underline"
                href={processingUrl(here, video.videoId).href}
                onClick={navigateInApp}
              >
                {video.title ?? video.videoId}
              </a>
              <div className="flex flex-wrap gap-1">
                {video.stages.map((stage) => (
                  <Badge
                    key={stage.detector}
                    variant={outcomeVariant[stage.outcome]}
                    title={stage.message ?? undefined}
                  >
                    {stage.detector}: {stage.outcome}
                  </Badge>
                ))}
              </div>
              {video.error ? <span className="text-xs text-destructive">{video.error}</span> : null}
            </div>
          ))}
        </div>
      </details>
    </li>
  );
}
