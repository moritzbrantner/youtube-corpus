import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Button, ErrorState, Input, LoadingState, StateViewTitle } from "@moritzbrantner/ui";
import {
  WorkflowWorkbench,
  workflowEditorControlFlowNodeTemplates,
} from "@moritzbrantner/workflow-editor";
import {
  WorkflowEditorCompileError,
  compileWorkflowEditorDocument,
} from "@moritzbrantner/workflow-editor/compiler";
import type {
  WorkflowEditorDocument,
  WorkflowEditorNode,
  WorkflowEditorNodeTemplate,
} from "@moritzbrantner/workflow-editor/core";

import {
  detectorNodeKindPrefix,
  getProcessingWorkflow,
  saveProcessingWorkflow,
  type DetectorInfo,
} from "./api";
import { useDetectorProfiles } from "./CorpusDetectors";
import { errorMessage } from "./CorpusLabels";
import { navigateInApp, processingWorkflowUrl, workflowEditorUrl } from "./url-state";

const anyPort = (id: string, label: string) => ({ id, label, type: { kind: "any" as const } });

/** One palette entry per corpus detector stage; the corpus host runs exactly these kinds. */
function stageTemplates(detectors: DetectorInfo[]): WorkflowEditorNodeTemplate[] {
  const structural = workflowEditorControlFlowNodeTemplates.filter(
    (template) => template.kind === "control.start" || template.kind === "control.end",
  );
  return [
    ...structural,
    ...detectors.map((detector): WorkflowEditorNodeTemplate => ({
      id: `youtube-corpus-${detector.id}`,
      label: detector.label,
      description: [detector.producer, detector.note].filter(Boolean).join(". "),
      kind: `${detectorNodeKindPrefix}${detector.id}`,
      category: "Corpus stages",
      inputs: [anyPort("in", "In")],
      outputs: [anyPort("out", "Out")],
      inspectorFields: detector.takesLanguages
        ? [
            {
              path: "targetLanguages",
              label: "Target languages",
              kind: "text",
              placeholder: "de, fr",
              description: "Language codes, comma separated. A corpus can override them.",
            },
          ]
        : undefined,
    })),
  ];
}

function node(
  id: string,
  label: string,
  kind: string,
  x: number,
  ports: Pick<WorkflowEditorNode, "inputs" | "outputs">,
): WorkflowEditorNode {
  return { id, label, kind, x, y: 120, data: {}, ...ports };
}

/** A new definition starts as Start → YouTube captions → End. */
function starterDocument(): WorkflowEditorDocument {
  return {
    nodes: [
      node("start", "Start", "control.start", 40, { outputs: [anyPort("out", "Out")] }),
      node("captions", "YouTube captions", `${detectorNodeKindPrefix}captions`, 340, {
        inputs: [anyPort("in", "In")],
        outputs: [anyPort("out", "Out")],
      }),
      node("end", "End", "control.end", 640, { inputs: [anyPort("in", "In")] }),
    ],
    edges: [edge("start", "captions"), edge("captions", "end")],
  };
}

function edge(source: string, target: string) {
  return {
    id: `${source}-${target}`,
    sourceNodeId: source,
    sourcePortId: "out",
    targetNodeId: target,
    targetPortId: "in",
  };
}

function compileMessages(error: unknown): string[] {
  if (error instanceof WorkflowEditorCompileError) {
    return error.diagnostics.map((diagnostic) => `${diagnostic.path}: ${diagnostic.message}`);
  }
  return [errorMessage(error)];
}

/**
 * `/processing/workflows/:id`: the shared Workflow Editor on a reusable processing workflow
 * definition (#108). Saving compiles it with the editor's own compiler; the corpus host then
 * runs the compiled stages through its detector plan.
 */
export default function ProcessingWorkflowEditor({
  corpusId,
  workflowId,
}: {
  corpusId: string;
  workflowId: string;
}) {
  const isNew = workflowId === "new";
  const profiles = useDetectorProfiles(corpusId);
  const definition = useQuery({
    queryKey: ["processing-workflows", workflowId],
    queryFn: () => getProcessingWorkflow(workflowId),
    enabled: !isNew,
  });
  if (profiles.isPending || (!isNew && definition.isPending)) {
    return <LoadingState label="Loading the Workflow Editor" />;
  }
  const error = profiles.error ?? definition.error;
  if (error || !profiles.data) {
    return (
      <ErrorState>
        <StateViewTitle>{errorMessage(error)}</StateViewTitle>
      </ErrorState>
    );
  }
  return (
    <EditorSurface
      key={`${workflowId}:${definition.data?.version ?? 0}`}
      workflowId={isNew ? null : workflowId}
      initialName={definition.data?.name ?? "New processing workflow"}
      initialDocument={
        (definition.data?.document as WorkflowEditorDocument | undefined) ?? starterDocument()
      }
      version={definition.data?.version ?? null}
      templates={stageTemplates(profiles.data.detectors)}
    />
  );
}

function EditorSurface({
  workflowId,
  initialName,
  initialDocument,
  version,
  templates,
}: {
  workflowId: string | null;
  initialName: string;
  initialDocument: WorkflowEditorDocument;
  version: number | null;
  templates: WorkflowEditorNodeTemplate[];
}) {
  const client = useQueryClient();
  const [name, setName] = React.useState(initialName);
  const [document, setDocument] = React.useState(initialDocument);
  const [dirty, setDirty] = React.useState(false);
  const [problems, setProblems] = React.useState<string[]>([]);
  const save = useMutation({
    mutationFn: () => {
      let compiled: unknown;
      try {
        compiled = compileWorkflowEditorDocument(document);
      } catch (error) {
        setProblems(compileMessages(error));
        throw error;
      }
      setProblems([]);
      return saveProcessingWorkflow(workflowId, { name, document, compiled });
    },
    onSuccess: (saved) => {
      setDirty(false);
      void client.invalidateQueries({ queryKey: ["processing-workflows"] });
      void client.invalidateQueries({ queryKey: ["corpora"] });
      if (!workflowId) {
        window.history.replaceState(
          null,
          "",
          workflowEditorUrl(new URL(window.location.href), saved.id),
        );
        window.dispatchEvent(new PopStateEvent("popstate"));
      }
    },
  });
  const back = processingWorkflowUrl(new URL(window.location.href), workflowId ?? "");
  if (!workflowId) back.searchParams.delete("workflow");
  return (
    <section className="grid gap-3" aria-label="Workflow Editor">
      <div className="flex flex-wrap items-end gap-2">
        <label className="grid min-w-64 flex-1 gap-1 text-sm">
          Workflow name
          <Input
            value={name}
            onChange={(event) => {
              setName(event.target.value);
              setDirty(true);
            }}
          />
        </label>
        <span className="pb-2 text-sm text-muted-foreground">
          {version ? `Version ${version}` : "Not saved yet"}
          {dirty ? " · unsaved changes" : ""}
        </span>
        <Button disabled={save.isPending || !name.trim()} onClick={() => save.mutate()}>
          {save.isPending ? "Saving..." : "Compile and save"}
        </Button>
        <Button asChild variant="outline">
          <a href={back.href} onClick={navigateInApp}>
            Back to Processing
          </a>
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        Add corpus stages from the palette and connect them. Saving compiles the graph with the
        Workflow Editor compiler; every corpus that uses this definition runs its stages through its
        own detector plan, with its own enabled stages and parameters.
      </p>
      {problems.length > 0 ||
      (save.error && !(save.error instanceof WorkflowEditorCompileError)) ? (
        <ul role="alert" className="grid gap-1 text-sm text-destructive">
          {(problems.length > 0 ? problems : [errorMessage(save.error)]).map((problem) => (
            <li key={problem}>{problem}</li>
          ))}
        </ul>
      ) : null}
      <div className="h-[640px] overflow-hidden rounded-md border border-border">
        <WorkflowWorkbench
          className="h-full"
          document={document}
          nodeTemplates={templates}
          onDocumentChange={(next) => {
            setDocument(next);
            setDirty(true);
          }}
        />
      </div>
    </section>
  );
}
