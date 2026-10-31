import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Badge, Button, NativeSelect } from "@moritzbrantner/ui";
import {
  correctFaceMatches,
  getFaceMatches,
  runFaceMatches,
  type FaceCorrection,
  type FaceEvidence,
  type FaceMatchReview,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { timestampUrl } from "./evidence";

export function FaceMatchingReview({
  corpusId,
  onSelectPerson,
}: {
  corpusId: string;
  onSelectPerson: (personId: string, label: string) => void;
}) {
  const client = useQueryClient();
  const key = ["corpora", corpusId, "entities", "face-matches"] as const;
  const [selected, setSelected] = useState<string[]>([]);
  const [target, setTarget] = useState("");
  const review = useQuery({ queryKey: key, queryFn: () => getFaceMatches(corpusId) });
  async function updated(report: FaceMatchReview) {
    client.setQueryData(key, report);
    setSelected([]);
    await client.invalidateQueries({ queryKey: ["corpora", corpusId, "entities"] });
  }
  const matching = useMutation({ mutationFn: () => runFaceMatches(corpusId), onSuccess: updated });
  const correction = useMutation({
    mutationFn: (input: FaceCorrection) => correctFaceMatches(corpusId, input),
    onSuccess: updated,
  });
  const pending = matching.isPending || correction.isPending;
  function toggle(id: string) {
    setSelected((current) =>
      current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
    );
  }
  const report = review.data;
  const evidence = (track: FaceEvidence) => (
    <FaceTrack
      key={track.trackId}
      track={track}
      selected={selected.includes(track.trackId)}
      disabled={pending}
      onToggle={() => toggle(track.trackId)}
    />
  );
  return (
    <section
      className="grid gap-3 rounded-md border border-border p-3"
      aria-label="Face matching review"
    >
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">Face matches</h3>
        <Button variant="outline" size="sm" disabled={pending} onClick={() => matching.mutate()}>
          Recheck face matches
        </Button>
      </header>
      <p className="text-sm text-muted-foreground">
        High-confidence face matches are grouped automatically after face processing. These are
        inferred corpus people, not verified real-world identities. Weak, incompatible or ambiguous
        evidence stays unresolved. Face-to-voice links are suggested under Voice matches.
      </p>
      <p className="text-sm text-muted-foreground">
        Unlink rejects the current association; a different supported person may still match
        automatically. Split creates a separate inferred person. Corrections survive unchanged
        reruns and preserve the source evidence.
      </p>
      {review.isPending ? <p role="status">Loading face evidence…</p> : null}
      {[review.error, matching.error, correction.error].filter(Boolean).map((error, index) => (
        <p key={index} role="alert" className="text-sm text-destructive">
          {errorMessage(error)}
        </p>
      ))}
      {pending ? <p role="status">Updating face associations…</p> : null}
      <div className="flex flex-wrap items-center gap-2" aria-label="Correct selected face tracks">
        <span className="text-sm">{selected.length} selected</span>
        <Button
          size="sm"
          variant="outline"
          disabled={pending || selected.length === 0}
          onClick={() => correction.mutate({ action: "unlink", trackIds: selected })}
        >
          Unlink selected
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={pending || selected.length === 0}
          onClick={() => correction.mutate({ action: "split", trackIds: selected })}
        >
          Split selected into new person
        </Button>
        <NativeSelect
          aria-label="Person to link selected face tracks to"
          value={target}
          disabled={pending}
          onChange={(event) => setTarget(event.target.value)}
        >
          <option value="">Choose a corpus person…</option>
          {report?.groups.map((group) => (
            <option key={group.personId} value={group.personId}>
              {group.label}
            </option>
          ))}
        </NativeSelect>
        <Button
          size="sm"
          disabled={pending || selected.length === 0 || !target}
          onClick={() =>
            correction.mutate({ action: "link", trackIds: selected, personId: target })
          }
        >
          Link selected
        </Button>
      </div>
      {report?.groups.map((group) => (
        <article
          key={group.personId}
          className="grid gap-2 rounded-md border border-border p-3"
          aria-label={group.label}
        >
          <header className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{group.label}</span>
            <Badge variant="outline">
              {group.identitySource?.includes("user-confirmed")
                ? "User-confirmed corpus association"
                : "Inferred · not verified"}
            </Badge>
            <span className="text-sm">
              {new Set(group.tracks.map((track) => track.videoId)).size} videos ·{" "}
              {group.tracks.length} face tracks
            </span>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onSelectPerson(group.personId, group.label)}
            >
              Open person occurrences
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={pending || group.tracks.length === 0}
              onClick={() =>
                correction.mutate({
                  action: "confirm",
                  personId: group.personId,
                  trackIds: group.tracks.map((track) => track.trackId),
                })
              }
            >
              Confirm current group
            </Button>
          </header>
          {group.tracks.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No current face associations. You can link evidence back explicitly.
            </p>
          ) : null}
          <ul className="grid gap-2">{group.tracks.map(evidence)}</ul>
        </article>
      ))}
      {report ? (
        <section className="grid gap-2" aria-label="Unresolved face tracks">
          <h4 className="font-medium">Unresolved face tracks ({report.unresolved.length})</h4>
          <ul className="grid gap-2">{report.unresolved.map(evidence)}</ul>
          {report.groups.length === 0 && report.unresolved.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No supported face evidence yet. Enable and run the faces detector on retained video.
            </p>
          ) : null}
        </section>
      ) : null}
    </section>
  );
}

function time(seconds: number) {
  return `${Math.floor(seconds / 60)}:${Math.floor(seconds % 60)
    .toString()
    .padStart(2, "0")}`;
}
function FaceTrack({
  track,
  selected,
  disabled,
  onToggle,
}: {
  track: FaceEvidence;
  selected: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <li className="grid gap-1 rounded border border-border p-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="checkbox"
          aria-label={`Select ${track.title ?? "video"} at ${time(track.startSeconds)} (${track.trackId.slice(0, 8)})`}
          checked={selected}
          disabled={disabled}
          onChange={onToggle}
        />
        <a
          className="underline"
          href={timestampUrl(track.sourceUrl, track.startSeconds)}
          target="_blank"
          rel="noreferrer"
        >
          {track.title ?? track.sourceUrl} · {time(track.startSeconds)}–{time(track.endSeconds)}
        </a>
        {track.similarity !== null ? (
          <span>minimum cosine similarity {track.similarity.toFixed(3)}</span>
        ) : null}
        {track.protected ? <Badge variant="outline">Researcher correction preserved</Badge> : null}
      </div>
      <p className="text-muted-foreground">
        {track.observations} sampled observations
        {track.detectionScore === null
          ? ""
          : ` · detector score ${track.detectionScore.toFixed(3)}`}
      </p>
      {track.reason ? <p>{track.reason}</p> : null}
      <details>
        <summary className="cursor-pointer">Evidence and provenance</summary>
        <dl className="grid gap-1 break-all">
          <dt>Producer</dt>
          <dd>
            {track.processor}@{track.processorVersion}
          </dd>
          <dt>Model</dt>
          <dd>
            {track.model}@{track.modelVersion}
          </dd>
          <dt>Input identity</dt>
          <dd>{track.inputHash}</dd>
          <dt>Face processing configuration</dt>
          <dd>
            <pre className="whitespace-pre-wrap">
              {JSON.stringify(track.producerConfig, null, 2)}
            </pre>
          </dd>
          {track.assignmentSource ? (
            <>
              <dt>Association source</dt>
              <dd>{track.assignmentSource}</dd>
            </>
          ) : null}
          {track.matchingAlgorithm ? (
            <>
              <dt>Matching algorithm</dt>
              <dd>
                {track.matchingAlgorithm}@{track.matchingVersion}
              </dd>
              <dt>Matching configuration</dt>
              <dd>
                <pre className="whitespace-pre-wrap">
                  {JSON.stringify(track.matchingConfiguration, null, 2)}
                </pre>
              </dd>
            </>
          ) : null}
        </dl>
      </details>
    </li>
  );
}
