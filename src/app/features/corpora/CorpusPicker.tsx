import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { Badge, Button, Input, NativeSelect, Textarea } from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import { createCorpus, type Corpus } from "./api";
import { corpusKeys } from "./query-keys";

type CorpusPickerProps = {
  corpora: Corpus[];
  selectedCorpusId: string | null;
  onSelect: (corpusId: string) => void;
};

export function CorpusPicker({ corpora, selectedCorpusId, onSelect }: CorpusPickerProps) {
  const queryClient = useQueryClient();
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const createMutation = useMutation({
    mutationFn: createCorpus,
    onSuccess: async (corpus) => {
      await queryClient.invalidateQueries({ queryKey: corpusKeys.all });
      setName("");
      setDescription("");
      onSelect(corpus.id);
    },
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    const trimmedName = name.trim();
    if (!trimmedName) {
      return;
    }
    createMutation.mutate({
      name: trimmedName,
      description: description.trim() || null,
    });
  }

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Research corpus</SurfaceTitle>
        <SurfaceDescription>
          Scope sources, videos, search, and reprocessing to a named collection.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-5">
        <label className="grid gap-2">
          <span className="text-sm font-medium">Active corpus</span>
          <NativeSelect
            value={selectedCorpusId ?? ""}
            onChange={(event) => onSelect(event.target.value)}
          >
            <option value="" disabled>
              Select a corpus
            </option>
            {corpora.map((corpus) => (
              <option key={corpus.id} value={corpus.id}>
                {corpus.name}
              </option>
            ))}
          </NativeSelect>
        </label>

        {selectedCorpusId ? (
          <div className="flex flex-wrap gap-2">
            {corpora
              .filter((corpus) => corpus.id === selectedCorpusId)
              .map((corpus) => (
                <React.Fragment key={corpus.id}>
                  {corpus.isDefault ? <Badge variant="secondary">Default</Badge> : null}
                  <Badge variant="outline">{corpus.sourceCount} sources</Badge>
                  <Badge variant="outline">{corpus.videoCount} videos</Badge>
                </React.Fragment>
              ))}
          </div>
        ) : null}

        <form className="grid gap-3 border-t border-border pt-4" onSubmit={submit}>
          <div className="grid gap-1">
            <span className="text-sm font-medium">Create corpus</span>
            <span className="text-xs text-muted-foreground">
              The slug is generated from the name and stays stable for the collection.
            </span>
          </div>
          <Input
            aria-label="Corpus name"
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Church history research"
          />
          <Textarea
            aria-label="Corpus description"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            placeholder="Optional scope or research note"
            rows={2}
          />
          {createMutation.error ? (
            <p className="text-sm text-destructive">{String(createMutation.error)}</p>
          ) : null}
          <Button type="submit" disabled={!name.trim() || createMutation.isPending}>
            {createMutation.isPending ? "Creating..." : "Create corpus"}
          </Button>
        </form>
      </SurfaceContent>
    </Surface>
  );
}
