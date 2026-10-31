import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Badge, Button, NativeSelect } from "@moritzbrantner/ui";
import {
  correctVoiceMatches,
  getVoiceMatches,
  reviewFaceVoiceLink,
  runVoiceMatches,
  type FaceLinkReview,
  type FaceVoiceLink,
  type VoiceCorrection,
  type VoiceEvidence,
  type VoiceMatchReview,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { timestampUrl } from "./evidence";

/** Unresolved tracks shown before "show all"; most are short diarization fragments. */
const UNRESOLVED_PREVIEW = 20;

export function VoiceMatchingReview({
  corpusId,
  onSelectPerson,
}: {
  corpusId: string;
  onSelectPerson: (personId: string, label: string) => void;
}) {
  const client = useQueryClient();
  const key = ["corpora", corpusId, "entities", "voice-matches"] as const;
  const [selected, setSelected] = useState<string[]>([]);
  const [target, setTarget] = useState("");
  const [allUnresolved, setAllUnresolved] = useState(false);
  const review = useQuery({
    queryKey: key,
    queryFn: () => getVoiceMatches(corpusId),
  });
  async function updated(report: VoiceMatchReview) {
    client.setQueryData(key, report);
    setSelected([]);
    await client.invalidateQueries({
      queryKey: ["corpora", corpusId, "entities"],
    });
  }
  const matching = useMutation({
    mutationFn: () => runVoiceMatches(corpusId),
    onSuccess: updated,
  });
  const correction = useMutation({
    mutationFn: (input: VoiceCorrection) => correctVoiceMatches(corpusId, input),
    onSuccess: updated,
  });
  const linkReview = useMutation({
    mutationFn: (input: FaceLinkReview) => reviewFaceVoiceLink(corpusId, input),
    onSuccess: updated,
  });
  const pending = matching.isPending || correction.isPending || linkReview.isPending;
  function toggle(id: string) {
    setSelected((current) =>
      current.includes(id) ? current.filter((value) => value !== id) : [...current, id],
    );
  }
  const report = review.data;
  const evidence = (track: VoiceEvidence) => (
    <VoiceTrack
      key={track.trackId}
      track={track}
      selected={selected.includes(track.trackId)}
      disabled={pending}
      onToggle={() => toggle(track.trackId)}
    />
  );
  const unresolved = report
    ? allUnresolved
      ? report.unresolved
      : report.unresolved.slice(0, UNRESOLVED_PREVIEW)
    : [];
  return (
    <section
      className="grid gap-3 rounded-md border border-border p-3"
      aria-label="Voice matching review"
    >
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">Voice matches</h3>
        <Button variant="outline" size="sm" disabled={pending} onClick={() => matching.mutate()}>
          Recheck voice matches
        </Button>
      </header>
      <p className="text-sm text-muted-foreground">
        Voices heard in more than one video are suggested as the same corpus person after voice
        processing. Every match is an unverified suggestion until you confirm it; it says that two
        recordings sound alike, not who the speaker is. Tracks with under 2 s of speech or a close
        competing person stay unresolved.
      </p>
      {review.isPending ? <p role="status">Loading voice evidence…</p> : null}
      {[review.error, matching.error, correction.error, linkReview.error]
        .filter(Boolean)
        .map((error, index) => (
          <p key={index} role="alert" className="text-sm text-destructive">
            {errorMessage(error)}
          </p>
        ))}
      {pending ? <p role="status">Updating voice associations…</p> : null}
      {report && report.faceLinks.length > 0 ? (
        <section className="grid gap-2" aria-label="Suggested face and voice links">
          <h4 className="font-medium">Suggested face ↔ voice links ({report.faceLinks.length})</h4>
          <ul className="grid gap-2">
            {report.faceLinks.map((link) => (
              <FaceLinkSuggestion
                key={`${link.facePersonId}:${link.voicePersonId}`}
                link={link}
                disabled={pending}
                onReview={(action) =>
                  linkReview.mutate({
                    action,
                    facePersonId: link.facePersonId,
                    voicePersonId: link.voicePersonId,
                  })
                }
              />
            ))}
          </ul>
        </section>
      ) : null}
      <div className="flex flex-wrap items-center gap-2" aria-label="Correct selected voice tracks">
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
          aria-label="Person to link selected voice tracks to"
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
            correction.mutate({
              action: "link",
              trackIds: selected,
              personId: target,
            })
          }
        >
          Link selected
        </Button>
      </div>
      {report?.groups.map((group) => {
        const confirmed =
          group.tracks.length > 0 && group.tracks.every((track) => track.status === "confirmed");
        return (
          <article
            key={group.personId}
            className="grid gap-2 rounded-md border border-border p-3"
            aria-label={group.label}
          >
            <header className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{group.label}</span>
              <Badge variant="outline">
                {confirmed ? "Confirmed voice matches" : "Suggested · not verified"}
              </Badge>
              <span className="text-sm">
                {new Set(group.tracks.map((track) => track.videoId)).size} videos ·{" "}
                {group.tracks.length} voice tracks ·{" "}
                {Math.round(group.tracks.reduce((total, track) => total + track.speechSeconds, 0))}{" "}
                s speech
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
                disabled={pending || group.tracks.length === 0 || confirmed}
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
                No current voice associations. You can link evidence back explicitly.
              </p>
            ) : null}
            <ul className="grid gap-2">{group.tracks.map(evidence)}</ul>
          </article>
        );
      })}
      {report ? (
        <section className="grid gap-2" aria-label="Unresolved voice tracks">
          <h4 className="font-medium">Unresolved voice tracks ({report.unresolved.length})</h4>
          <ul className="grid gap-2">{unresolved.map(evidence)}</ul>
          {report.unresolved.length > UNRESOLVED_PREVIEW ? (
            <Button size="sm" variant="ghost" onClick={() => setAllUnresolved(!allUnresolved)}>
              {allUnresolved
                ? "Show fewer unresolved tracks"
                : `Show all ${report.unresolved.length} unresolved tracks`}
            </Button>
          ) : null}
          {report.groups.length === 0 && report.unresolved.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No supported voice evidence yet. Enable and run the voices detector on retained media.
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

function FaceLinkSuggestion({
  link,
  disabled,
  onReview,
}: {
  link: FaceVoiceLink;
  disabled: boolean;
  onReview: (action: "confirm" | "reject") => void;
}) {
  return (
    <li className="grid gap-1 rounded border border-border p-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">
          {link.faceLabel} (face) ↔ {link.voiceLabel} (voice)
        </span>
        <Badge variant="outline">Unverified suggestion</Badge>
        <Button size="sm" disabled={disabled} onClick={() => onReview("confirm")}>
          Confirm: same person
        </Button>
        <Button size="sm" variant="outline" disabled={disabled} onClick={() => onReview("reject")}>
          Reject
        </Button>
      </div>
      <p className="text-muted-foreground">
        The face is on screen for {Math.round(link.coverage * 100)}% of this voice's speech (
        {Math.round(link.overlapSeconds)} of {Math.round(link.voiceSeconds)} s) across{" "}
        {link.sharedVideos} videos, {Math.round(link.margin * 100)} points ahead of any other face.
        Confirming merges the voice person into the face person.
      </p>
      <ul className="grid gap-1">
        {link.videos.map((video) => (
          <li key={video.videoId}>
            <a className="underline" href={video.sourceUrl} target="_blank" rel="noreferrer">
              {video.title ?? video.sourceUrl}
            </a>{" "}
            · {Math.round(video.overlapSeconds)} of {Math.round(video.voiceSeconds)} s together
          </li>
        ))}
      </ul>
    </li>
  );
}

function VoiceTrack({
  track,
  selected,
  disabled,
  onToggle,
}: {
  track: VoiceEvidence;
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
          <span>cosine similarity {track.similarity.toFixed(3)}</span>
        ) : null}
        {track.status === "confirmed" ? <Badge variant="outline">Confirmed</Badge> : null}
        {track.status === "suggested" ? <Badge variant="outline">Suggested</Badge> : null}
        {track.protected ? <Badge variant="outline">Researcher correction preserved</Badge> : null}
      </div>
      <p className="text-muted-foreground">
        {track.observations} speech spans · {track.speechSeconds.toFixed(1)} s speech
      </p>
      {track.note ? <p>{track.note}</p> : null}
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
          <dt>Voice processing configuration</dt>
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
          {track.matchingConfiguration ? (
            <>
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
