import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  Button,
  ErrorState,
  Input,
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

import {
  extractCorpusEntities,
  getEntityOccurrences,
  getPersonOccurrences,
  lookupEntities,
  type EntityCoverage,
  type EntityOccurrence,
  type EntitySummary,
  type PersonSummary,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { timestampUrl } from "./CorpusSearch";
import { corpusKeys } from "./query-keys";

type Selection =
  | { kind: "entity"; entityType: string; entityKey: string; name: string }
  | { kind: "person"; personId: string; name: string };

/** Find people, places and other entities and inspect where they occur (Atlas 4.1-4.3). */
export function CorpusEntities({ corpusId }: { corpusId: string }) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState("");
  const [query, setQuery] = React.useState<string | null>(null);
  const [selection, setSelection] = React.useState<Selection | null>(null);
  const lookupQuery = useQuery({
    queryKey: corpusKeys.entities(corpusId, query ?? ""),
    queryFn: () => lookupEntities(corpusId, query ?? ""),
    enabled: query !== null,
  });
  const extractMutation = useMutation({
    mutationFn: () => extractCorpusEntities(corpusId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["corpora", corpusId, "entities"] }),
  });

  function submit(event: React.FormEvent) {
    event.preventDefault();
    setSelection(null);
    setQuery(draft.trim());
  }

  const report = lookupQuery.data;
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Entities</SurfaceTitle>
        <SurfaceDescription>
          People, places and other names mentioned in speech or on-screen text, and labeled people
          shown or heard in videos. A mention is not proof that the person appears, and matched
          people are corpus-local inferences, not verified real-world identities.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        <form className="flex flex-wrap items-end gap-2" onSubmit={submit}>
          <label className="grid min-w-56 flex-1 gap-2">
            <span className="text-sm font-medium">Entity</span>
            <Input
              aria-label="Entity name"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="Ada Lovelace, Lisbon, ... (empty lists all)"
            />
          </label>
          <Button type="submit">Find entities</Button>
          <Button
            type="button"
            variant="outline"
            disabled={extractMutation.isPending}
            onClick={() => extractMutation.mutate()}
          >
            {extractMutation.isPending ? "Extracting..." : "Extract mentions"}
          </Button>
        </form>
        {extractMutation.isPending ? (
          <p className="text-sm text-muted-foreground">
            Running the nlp-stack bert-base-NER model over transcripts and visible text. The first
            run downloads the model (about 400 MB).
          </p>
        ) : null}
        {extractMutation.data ? (
          <p className="text-sm text-muted-foreground">
            Extracted {extractMutation.data.mentions} mentions from{" "}
            {extractMutation.data.videosProcessed} videos ({extractMutation.data.segmentsAnalyzed}{" "}
            transcript segments, {extractMutation.data.visualTextTracksAnalyzed} visible-text
            tracks); {extractMutation.data.videosWithoutText} videos had no text to analyze.
          </p>
        ) : null}
        {extractMutation.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(extractMutation.error)}
          </p>
        ) : null}

        {lookupQuery.isFetching ? <LoadingState label="Looking up entities" /> : null}
        {lookupQuery.error ? (
          <ErrorState>
            <StateViewTitle>Entity lookup failed</StateViewTitle>
            <StateViewDescription>{errorMessage(lookupQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {report ? <CoverageNote coverage={report.coverage} /> : null}
        {report && report.entities.length === 0 && report.people.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No matching entities</StateViewTitle>
            <StateViewDescription>
              Nothing matched in the analyzed material. Videos without extracted mentions or face
              and voice processing may still contain this entity.
            </StateViewDescription>
          </StateView>
        ) : null}
        {report && report.entities.length > 0 ? (
          <section className="grid gap-2" aria-label="Mentioned entities">
            <h3 className="text-sm font-semibold">Mentioned</h3>
            <ul className="grid gap-2">
              {report.entities.map((entity) => (
                <EntityRow
                  key={`${entity.entityType}:${entity.entityKey}`}
                  entity={entity}
                  selected={
                    selection?.kind === "entity" &&
                    selection.entityType === entity.entityType &&
                    selection.entityKey === entity.entityKey
                  }
                  onSelect={() =>
                    setSelection({
                      kind: "entity",
                      entityType: entity.entityType,
                      entityKey: entity.entityKey,
                      name: entity.displayName,
                    })
                  }
                />
              ))}
            </ul>
          </section>
        ) : null}
        {report && report.people.length > 0 ? (
          <section className="grid gap-2" aria-label="People shown or heard">
            <h3 className="text-sm font-semibold">Shown or heard</h3>
            <ul className="grid gap-2">
              {report.people.map((person) => (
                <PersonRow
                  key={person.personId}
                  person={person}
                  selected={selection?.kind === "person" && selection.personId === person.personId}
                  onSelect={() =>
                    setSelection({ kind: "person", personId: person.personId, name: person.label })
                  }
                />
              ))}
            </ul>
          </section>
        ) : null}
        {selection?.kind === "entity" ? (
          <EntityOccurrences corpusId={corpusId} selection={selection} />
        ) : null}
        {selection?.kind === "person" ? (
          <PersonOccurrences corpusId={corpusId} personId={selection.personId} />
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

function CoverageNote({ coverage }: { coverage: EntityCoverage }) {
  const unanalyzed = coverage.corpusVideos - coverage.entityExtractedVideos;
  return (
    <p className="text-sm text-muted-foreground">
      Mentions extracted for {coverage.entityExtractedVideos} of {coverage.corpusVideos} videos
      {unanalyzed > 0 ? ` (${unanalyzed} not analyzed, so absence there is unknown)` : ""}. Face
      tracks processed for {coverage.faceProcessedVideos} videos, voice tracks for{" "}
      {coverage.voiceProcessedVideos}; {coverage.people} corpus people identities.
    </p>
  );
}

function EntityRow({
  entity,
  selected,
  onSelect,
}: {
  entity: EntitySummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2">
      <span className="font-medium">{entity.displayName}</span>
      <Badge variant="outline">{entity.entityType}</Badge>
      {entity.spokenMentions > 0 ? (
        <Badge variant="secondary">Spoken ×{entity.spokenMentions}</Badge>
      ) : null}
      {entity.visibleTextMentions > 0 ? (
        <Badge variant="secondary">On-screen text ×{entity.visibleTextMentions}</Badge>
      ) : null}
      <span className="text-sm text-muted-foreground">
        {entity.videoCount} {entity.videoCount === 1 ? "video" : "videos"}
      </span>
      <Button
        className="ml-auto"
        size="sm"
        variant={selected ? "default" : "outline"}
        onClick={onSelect}
      >
        Show occurrences
      </Button>
    </li>
  );
}

function identityLabel(person: PersonSummary) {
  const source = person.identitySource?.toLowerCase() ?? "";
  if (source.includes("user") || source.includes("confirm")) {
    return "User-confirmed corpus identity";
  }
  return "Inferred identity (not verified)";
}

function PersonRow({
  person,
  selected,
  onSelect,
}: {
  person: PersonSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2">
      <span className="font-medium">{person.label}</span>
      <Badge variant="outline">{identityLabel(person)}</Badge>
      {person.identityConfidence !== null ? (
        <Badge variant="outline">confidence {person.identityConfidence.toFixed(2)}</Badge>
      ) : null}
      <Badge variant="secondary">Shown ×{person.shownTracks}</Badge>
      <Badge variant="secondary">Heard ×{person.heardTracks}</Badge>
      <span className="text-sm text-muted-foreground">
        {person.videoCount} {person.videoCount === 1 ? "video" : "videos"}
      </span>
      <Button
        className="ml-auto"
        size="sm"
        variant={selected ? "default" : "outline"}
        onClick={onSelect}
      >
        Show occurrences
      </Button>
    </li>
  );
}

function EntityOccurrences({
  corpusId,
  selection,
}: {
  corpusId: string;
  selection: Extract<Selection, { kind: "entity" }>;
}) {
  const occurrencesQuery = useQuery({
    queryKey: corpusKeys.entityOccurrences(corpusId, selection.entityType, selection.entityKey),
    queryFn: () => getEntityOccurrences(corpusId, selection.entityType, selection.entityKey),
  });
  return (
    <section className="grid gap-3" aria-label={`Occurrences of ${selection.name}`}>
      <h3 className="text-sm font-semibold">Where “{selection.name}” is mentioned</h3>
      {occurrencesQuery.isPending ? <LoadingState label="Loading occurrences" /> : null}
      {occurrencesQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(occurrencesQuery.error)}
        </p>
      ) : null}
      {occurrencesQuery.data?.videos.map((video) => (
        <article key={video.videoId} className="grid gap-2 rounded-md border border-border p-3">
          <header className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{video.title ?? video.sourceUrl}</span>
            <Badge variant="secondary">Spoken ×{video.spokenMentions}</Badge>
            <Badge variant="secondary">On-screen text ×{video.visibleTextMentions}</Badge>
          </header>
          <ul className="grid gap-2">
            {video.occurrences.map((occurrence) => (
              <MentionItem key={occurrence.id} sourceUrl={video.sourceUrl} mention={occurrence} />
            ))}
          </ul>
        </article>
      ))}
    </section>
  );
}

function MentionItem({ sourceUrl, mention }: { sourceUrl: string; mention: EntityOccurrence }) {
  const [before, match, after] = splitByBytes(mention.context, mention.byteStart, mention.byteEnd);
  return (
    <li className="grid gap-1 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="outline">
          {mention.modality === "spoken"
            ? "Spoken"
            : `On-screen text${mention.visualTextRole ? ` (${mention.visualTextRole})` : ""}`}
        </Badge>
        <a
          className="underline"
          href={timestampUrl(sourceUrl, mention.startSeconds)}
          target="_blank"
          rel="noreferrer"
        >
          {formatTime(mention.startSeconds)}
        </a>
        {mention.confidence !== null ? (
          <span className="text-muted-foreground">confidence {mention.confidence.toFixed(2)}</span>
        ) : null}
        <span className="text-muted-foreground">
          {mention.producer} · {mention.model}
        </span>
      </div>
      <p>
        {before}
        <mark>{match}</mark>
        {after}
      </p>
    </li>
  );
}

function PersonOccurrences({ corpusId, personId }: { corpusId: string; personId: string }) {
  const occurrencesQuery = useQuery({
    queryKey: corpusKeys.personOccurrences(corpusId, personId),
    queryFn: () => getPersonOccurrences(corpusId, personId),
  });
  const report = occurrencesQuery.data;
  return (
    <section className="grid gap-3" aria-label="Person occurrences">
      {occurrencesQuery.isPending ? <LoadingState label="Loading occurrences" /> : null}
      {occurrencesQuery.error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorMessage(occurrencesQuery.error)}
        </p>
      ) : null}
      {report ? (
        <h3 className="text-sm font-semibold">
          Where “{report.person.label}” is shown or heard ({identityLabel(report.person)})
        </h3>
      ) : null}
      {report?.videos.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No face or voice tracks of this identity in the corpus videos.
        </p>
      ) : null}
      {report?.videos.map((video) => (
        <article key={video.videoId} className="grid gap-2 rounded-md border border-border p-3">
          <header className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{video.title ?? video.sourceUrl}</span>
            <Badge variant="secondary">Shown ×{video.shownTracks}</Badge>
            <Badge variant="secondary">Heard ×{video.heardTracks}</Badge>
          </header>
          <ul className="grid gap-1 text-sm">
            {video.occurrences.map((occurrence) => (
              <li
                key={`${occurrence.kind}:${occurrence.trackId}:${occurrence.clusterId}`}
                className="flex flex-wrap items-center gap-2"
              >
                <Badge variant="outline">{occurrence.kind === "shown" ? "Shown" : "Heard"}</Badge>
                <a
                  className="underline"
                  href={timestampUrl(video.sourceUrl, occurrence.startSeconds)}
                  target="_blank"
                  rel="noreferrer"
                >
                  {formatTime(occurrence.startSeconds)}–{formatTime(occurrence.endSeconds)}
                </a>
                {occurrence.linkConfidence !== null ? (
                  <span className="text-muted-foreground">
                    link confidence {occurrence.linkConfidence.toFixed(2)}
                  </span>
                ) : null}
                <span className="text-muted-foreground">
                  {occurrence.linkSource ?? "unknown link source"} · {occurrence.processor}{" "}
                  {occurrence.model}@{occurrence.modelVersion}
                </span>
              </li>
            ))}
          </ul>
        </article>
      ))}
    </section>
  );
}

/** Splits text at UTF-8 byte offsets (the producer's span unit). */
function splitByBytes(text: string, start: number, end: number): [string, string, string] {
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  return [
    decoder.decode(bytes.slice(0, start)),
    decoder.decode(bytes.slice(start, end)),
    decoder.decode(bytes.slice(end)),
  ];
}

function formatTime(seconds: number | null) {
  if (seconds === null) {
    return "unknown time";
  }
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  return `${minutes}:${String(total % 60).padStart(2, "0")}`;
}
