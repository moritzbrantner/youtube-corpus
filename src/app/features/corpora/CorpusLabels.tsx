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
} from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import {
  createCorpusLabel,
  deleteCorpusLabel,
  listCorpusLabels,
  renameCorpusLabel,
  type CorpusLabel,
} from "./api";
import { corpusKeys } from "./query-keys";

/** Shared query for the corpus' user-defined labels. */
export function useCorpusLabels(corpusId: string) {
  return useQuery({
    queryKey: corpusKeys.labels(corpusId),
    queryFn: () => listCorpusLabels(corpusId),
  });
}

/** Invalidates every view that shows labels or label filters. */
export function useInvalidateLabels(corpusId: string) {
  const queryClient = useQueryClient();
  return React.useCallback(
    () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: corpusKeys.labels(corpusId) }),
        queryClient.invalidateQueries({ queryKey: corpusKeys.sources(corpusId) }),
        queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "videos"] }),
      ]),
    [corpusId, queryClient],
  );
}

type CorpusLabelsProps = {
  corpusId: string;
};

/** Create, rename and delete the corpus' own labels (Atlas 1.3). */
export function CorpusLabels({ corpusId }: CorpusLabelsProps) {
  const labelsQuery = useCorpusLabels(corpusId);
  const invalidate = useInvalidateLabels(corpusId);
  const [name, setName] = React.useState("");
  const createMutation = useMutation({
    mutationFn: (labelName: string) => createCorpusLabel(corpusId, { name: labelName }),
    onSuccess: async () => {
      setName("");
      await invalidate();
    },
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    createMutation.mutate(name);
  }

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Labels</SurfaceTitle>
        <SurfaceDescription>
          Your own labels for organizing sources and videos. They are kept separate from the
          categories and tags supplied by YouTube.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        <form className="flex flex-wrap items-end gap-2" onSubmit={submit}>
          <label className="grid min-w-56 flex-1 gap-2">
            <span className="text-sm font-medium">New label</span>
            <Input
              aria-label="New label name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Theology"
            />
          </label>
          <Button type="submit" disabled={createMutation.isPending}>
            {createMutation.isPending ? "Creating..." : "Create label"}
          </Button>
        </form>
        {createMutation.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(createMutation.error)}
          </p>
        ) : null}
        {labelsQuery.isPending ? <LoadingState label="Loading labels" /> : null}
        {labelsQuery.error ? (
          <ErrorState>
            <StateViewTitle>Labels unavailable</StateViewTitle>
            <StateViewDescription>{errorMessage(labelsQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {labelsQuery.data?.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No labels yet</StateViewTitle>
            <StateViewDescription>
              Create a label, then assign it to sources or videos below.
            </StateViewDescription>
          </StateView>
        ) : null}
        <ul className="grid gap-2">
          {labelsQuery.data?.map((label) => (
            <CorpusLabelRow key={label.id} corpusId={corpusId} label={label} />
          ))}
        </ul>
      </SurfaceContent>
    </Surface>
  );
}

function CorpusLabelRow({ corpusId, label }: { corpusId: string; label: CorpusLabel }) {
  const invalidate = useInvalidateLabels(corpusId);
  const [draft, setDraft] = React.useState(label.name);
  React.useEffect(() => setDraft(label.name), [label.name]);
  const renameMutation = useMutation({
    mutationFn: (name: string) => renameCorpusLabel(corpusId, label.id, name),
    onSuccess: invalidate,
  });
  const deleteMutation = useMutation({
    mutationFn: () => deleteCorpusLabel(corpusId, label.id),
    onSuccess: invalidate,
  });
  const error = renameMutation.error ?? deleteMutation.error;

  return (
    <li className="grid gap-2 rounded-md border border-border px-3 py-2">
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          renameMutation.mutate(draft);
        }}
      >
        <Badge variant="default">{label.name}</Badge>
        <Input
          aria-label={`Rename label ${label.name}`}
          className="min-w-40 flex-1"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <Button
          type="submit"
          variant="outline"
          size="sm"
          disabled={renameMutation.isPending || draft === label.name}
        >
          Rename
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-label={`Delete label ${label.name}`}
          disabled={deleteMutation.isPending}
          onClick={() => deleteMutation.mutate()}
        >
          Delete
        </Button>
      </form>
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(error)}
        </p>
      ) : null}
    </li>
  );
}

type LabelAssignmentsProps = {
  /** Accessible name of the labeled item, e.g. a video title. */
  itemName: string;
  assigned: CorpusLabel[];
  available: CorpusLabel[];
  pending: boolean;
  error: unknown;
  onAssign: (labelId: string) => void;
  onRemove: (labelId: string) => void;
};

/** User labels on one source or video, with add/remove controls. */
export function LabelAssignments({
  itemName,
  assigned,
  available,
  pending,
  error,
  onAssign,
  onRemove,
}: LabelAssignmentsProps) {
  const assignedIds = new Set(assigned.map((label) => label.id));
  const candidates = available.filter((label) => !assignedIds.has(label.id));

  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-center gap-2" aria-label={`Your labels on ${itemName}`}>
        <span className="text-xs font-medium text-muted-foreground">Your labels:</span>
        {assigned.length === 0 ? <span className="text-xs text-muted-foreground">none</span> : null}
        {assigned.map((label) => (
          <span key={label.id} className="inline-flex items-center gap-1">
            <Badge variant="default">{label.name}</Badge>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={`Remove label ${label.name} from ${itemName}`}
              disabled={pending}
              onClick={() => onRemove(label.id)}
            >
              ×
            </Button>
          </span>
        ))}
        {candidates.length > 0 ? (
          <NativeSelect
            aria-label={`Add label to ${itemName}`}
            className="min-w-36"
            value=""
            disabled={pending}
            onChange={(event) => {
              if (event.target.value) {
                onAssign(event.target.value);
              }
            }}
          >
            <option value="">Add label…</option>
            {candidates.map((label) => (
              <option key={label.id} value={label.id}>
                {label.name}
              </option>
            ))}
          </NativeSelect>
        ) : null}
      </div>
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(error)}
        </p>
      ) : null}
    </div>
  );
}

export function errorMessage(error: unknown) {
  // fetch() rejects with a TypeError when the local server is unreachable.
  if (error instanceof TypeError) {
    return "Could not reach the corpus server. Check that it is running, then try again.";
  }
  return error instanceof Error ? error.message : String(error);
}
