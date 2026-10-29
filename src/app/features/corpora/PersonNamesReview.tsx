import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Badge, Button, Input } from "@moritzbrantner/ui";
import {
  correctPersonName,
  getPersonNames,
  runPersonNames,
  type PersonNameCorrection,
  type PersonNameReview,
  type PersonNameStatus,
  type PersonNamesReport,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { timestampUrl } from "./evidence";

export function nameStatusLabel(status: PersonNameStatus) {
  switch (status) {
    case "inferred":
      return "Inferred name · not verified";
    case "confirmed":
      return "Name confirmed by researcher";
    case "user_provided":
      return "Name entered by researcher";
  }
}

/** Review, confirm, replace or reject evidence-grounded person names (Atlas 4.7). */
export function PersonNamesReview({
  corpusId,
  onSelectPerson,
}: {
  corpusId: string;
  onSelectPerson: (personId: string, label: string) => void;
}) {
  const client = useQueryClient();
  const key = ["corpora", corpusId, "entities", "person-names"] as const;
  const review = useQuery({
    queryKey: key,
    queryFn: () => getPersonNames(corpusId),
  });
  async function updated(report: PersonNamesReport) {
    client.setQueryData(key, report);
    await client.invalidateQueries({
      queryKey: ["corpora", corpusId, "entities"],
    });
  }
  const inference = useMutation({
    mutationFn: () => runPersonNames(corpusId),
    onSuccess: updated,
  });
  const correction = useMutation({
    mutationFn: (input: PersonNameCorrection) => correctPersonName(corpusId, input),
    onSuccess: updated,
  });
  const pending = inference.isPending || correction.isPending;
  const people = review.data?.people ?? [];
  const relevant = people.filter(
    (person) => person.name || person.unresolved || person.rejectedNames.length > 0,
  );
  return (
    <section className="grid gap-3 rounded-md border border-border p-3" aria-label="Person names">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">Person names</h3>
        <Button variant="outline" size="sm" disabled={pending} onClick={() => inference.mutate()}>
          Recheck names
        </Button>
      </header>
      <p className="text-sm text-muted-foreground">
        A name is inferred only from an on-screen name tag shown while that person's face is the
        only one visible. Mentions in speech or sentences, end credits, several visible faces,
        conflicting tags and a name supported for several people leave the name unresolved. Inferred
        names are not verified identities. Rejected names are not inferred again. Self-introductions
        in speech are not used yet: voices are not matched to people.
      </p>
      {review.isPending ? <p role="status">Loading person names…</p> : null}
      {[review.error, inference.error, correction.error].filter(Boolean).map((error, index) => (
        <p key={index} role="alert" className="text-sm text-destructive">
          {errorMessage(error)}
        </p>
      ))}
      {review.data && relevant.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No name evidence for the {people.length} corpus people yet.
        </p>
      ) : null}
      <ul className="grid gap-2">
        {relevant.map((person) => (
          <PersonName
            key={person.personId}
            person={person}
            disabled={pending}
            onCorrect={(input) => correction.mutate(input)}
            onOpen={() => onSelectPerson(person.personId, person.name?.name ?? person.label)}
          />
        ))}
      </ul>
    </section>
  );
}

function PersonName({
  person,
  disabled,
  onCorrect,
  onOpen,
}: {
  person: PersonNameReview;
  disabled: boolean;
  onCorrect: (input: PersonNameCorrection) => void;
  onOpen: () => void;
}) {
  const [draft, setDraft] = useState("");
  const name = person.name;
  return (
    <li
      className="grid gap-2 rounded border border-border p-2 text-sm"
      aria-label={name?.name ?? person.label}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{name?.name ?? "Unnamed"}</span>
        <span className="text-muted-foreground">{person.label}</span>
        {name ? (
          <Badge variant={name.status === "inferred" ? "outline" : "secondary"}>
            {nameStatusLabel(name.status)}
          </Badge>
        ) : null}
        {name?.confidence != null ? (
          <span className="text-muted-foreground">confidence {name.confidence.toFixed(2)}</span>
        ) : null}
        <Button size="sm" variant="ghost" onClick={onOpen}>
          Open person occurrences
        </Button>
      </div>
      {person.unresolved ? (
        <p>
          Unresolved: {person.unresolved.reason}
          {person.unresolved.candidates.length > 0
            ? ` (${person.unresolved.candidates.join(", ")})`
            : ""}
        </p>
      ) : null}
      {person.rejectedNames.length > 0 ? (
        <p className="text-muted-foreground">Rejected: {person.rejectedNames.join(", ")}</p>
      ) : null}
      {name && name.evidence.length > 0 ? (
        <ul className="grid gap-1" aria-label="Name evidence">
          {name.evidence.map((item) => (
            <li key={item.entityMentionId} className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">On-screen name tag</Badge>
              <a
                className="underline"
                href={timestampUrl(item.sourceUrl, item.startSeconds)}
                target="_blank"
                rel="noreferrer"
              >
                {item.title ?? item.sourceUrl} · {time(item.startSeconds)}–{time(item.endSeconds)}
              </a>
              <span>“{item.sourceText}”</span>
              <details>
                <summary className="cursor-pointer">Provenance</summary>
                <pre className="whitespace-pre-wrap break-all">
                  {JSON.stringify(
                    {
                      faceTrack: item.faceTrackId,
                      visibleTextTrack: item.visualTextTrackId,
                      mention: item.entityMentionId,
                      confidence: item.confidence,
                      producers: item.provenance,
                      algorithm: `${name.algorithm}@${name.algorithmVersion}`,
                      configuration: name.configuration,
                    },
                    null,
                    2,
                  )}
                </pre>
              </details>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        {name?.status === "inferred" ? (
          <Button
            size="sm"
            variant="outline"
            disabled={disabled}
            onClick={() => onCorrect({ personId: person.personId, action: "confirm" })}
          >
            Confirm name
          </Button>
        ) : null}
        {name ? (
          <Button
            size="sm"
            variant="outline"
            disabled={disabled}
            onClick={() => onCorrect({ personId: person.personId, action: "reject" })}
          >
            Reject name
          </Button>
        ) : null}
        <Input
          aria-label={`Name for ${person.label}`}
          className="w-56"
          value={draft}
          disabled={disabled}
          placeholder={name ? "Replace with…" : "Enter a name…"}
          onChange={(event) => setDraft(event.target.value)}
        />
        <Button
          size="sm"
          disabled={disabled || draft.trim() === ""}
          onClick={() => {
            onCorrect({
              personId: person.personId,
              action: "replace",
              name: draft.trim(),
            });
            setDraft("");
          }}
        >
          {name ? "Replace name" : "Set name"}
        </Button>
      </div>
    </li>
  );
}

function time(seconds: number) {
  return `${Math.floor(seconds / 60)}:${Math.floor(seconds % 60)
    .toString()
    .padStart(2, "0")}`;
}
