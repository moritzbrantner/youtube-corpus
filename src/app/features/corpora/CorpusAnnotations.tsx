import { useQuery } from "@tanstack/react-query";

import {
  Badge,
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

import { listAnnotations } from "./api";
import { corpusKeys } from "./query-keys";

type CorpusAnnotationsProps = {
  corpusId: string;
};

export function CorpusAnnotations({ corpusId }: CorpusAnnotationsProps) {
  const annotationsQuery = useQuery({
    queryKey: corpusKeys.annotations(corpusId),
    queryFn: () => listAnnotations(corpusId, { limit: 50 }),
  });

  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Research annotations</SurfaceTitle>
        <SurfaceDescription>
          Findings are stored independently from transcript streams so user notes and future NLP or
          visual-analysis enrichments share one provenance-aware model.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-3">
        {annotationsQuery.isPending ? <LoadingState label="Loading annotations" /> : null}
        {annotationsQuery.error ? (
          <ErrorState>
            <StateViewTitle>Annotations unavailable</StateViewTitle>
            <StateViewDescription>{String(annotationsQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {annotationsQuery.data?.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No research annotations yet</StateViewTitle>
            <StateViewDescription>
              Select transcript context from a search result and save a finding, claim, quote,
              reference, topic, or note.
            </StateViewDescription>
          </StateView>
        ) : null}
        {annotationsQuery.data?.map((annotation) => (
          <article
            key={annotation.id}
            className="grid gap-2 rounded-md border border-border px-3 py-3"
          >
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="secondary">{annotation.kind}</Badge>
              <Badge variant="outline">{annotation.sourceKind}</Badge>
              {annotation.startSeconds !== null ? (
                <Badge variant="outline">{formatTimestamp(annotation.startSeconds)}</Badge>
              ) : null}
              <span className="ml-auto text-xs text-muted-foreground">
                revision {annotation.revision}
              </span>
            </div>
            <p className="text-sm leading-6">
              {annotation.text ?? annotation.label ?? "Structured annotation"}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              video {annotation.videoId} · checksum {annotation.contentChecksum.slice(0, 10)}
            </p>
          </article>
        ))}
      </SurfaceContent>
    </Surface>
  );
}

function formatTimestamp(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}
