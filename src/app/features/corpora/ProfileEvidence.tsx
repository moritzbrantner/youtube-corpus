import * as React from "react";

import { Badge, Button, Input, NativeSelect } from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import type { IdentityLocalUnit, IdentityProfile } from "./api";
import { MentionVideos, PublicationDate, TrackVideos } from "./CorpusEntities";
import { formatInterval, timestampUrl } from "./evidence";
import {
  axisTicks,
  dateMs,
  evidenceKindLabels,
  filterMentions,
  filterTracks,
  layoutMarkers,
  timelineDomain,
  timelineVideos,
  zoomDomain,
  type TimelineVideo,
} from "./publication-timeline";
import {
  corpusPageUrl,
  evidenceKinds,
  navigateInApp,
  patchCorpusUrl,
  videoUrl,
  type CorpusUrlState,
  type EvidenceKind,
} from "./url-state";

type Props = {
  profile: IdentityProfile;
  corpusId: string;
  corpusName: string;
  state: CorpusUrlState;
  onUpdate: (patch: Partial<CorpusUrlState>, history?: "push" | "replace") => void;
};

/**
 * A profile's evidence as a filterable list or a publication timeline (#105). Both views share
 * the scope, evidence type and publication range filters in the URL.
 */
export function ProfileEvidence({ profile, corpusId, corpusName, state, onUpdate }: Props) {
  const filter = { kind: state.evidenceKind, from: state.publishedFrom, to: state.publishedTo };
  const evidence = profile.evidence.map((unit) => ({
    ...unit,
    mentions: unit.mentions ? filterMentions(unit.mentions, filter) : null,
    tracks: unit.tracks ? filterTracks(unit.tracks, filter) : null,
  }));
  const unitsByKey = new Map(profile.units.map((unit) => [unit.unitKey, unit]));
  const corpusNames = new Map(profile.units.map((unit) => [unit.corpusId, unit.corpusName]));
  const videos = timelineVideos(evidence, corpusNames);
  const occurrences = videos.reduce((sum, video) => sum + video.moments.length, 0);
  const undated = videos.filter((video) => !video.publicationDate).length;
  const scopedToCorpus = state.identityScope !== "all";
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Evidence</SurfaceTitle>
        <SurfaceDescription>
          A text mention is never a verified appearance or speaker identification; shown and heard
          tracks belong to an inferred face/voice identity.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        <div className="flex flex-wrap items-end gap-2" role="group" aria-label="Evidence filters">
          <label className="grid gap-1 text-sm">
            Corpus
            <NativeSelect
              aria-label="Corpus"
              value={scopedToCorpus ? corpusId : "all"}
              onChange={(event) => {
                const value = event.target.value;
                onUpdate(
                  value === "all"
                    ? { identityScope: "all" }
                    : { corpusId: value, identityScope: "" },
                );
              }}
            >
              <option value="all">All corpora</option>
              {[
                { id: corpusId, name: corpusName },
                ...profile.identity.corpora.filter((corpus) => corpus.id !== corpusId),
              ].map((corpus) => (
                <option key={corpus.id} value={corpus.id}>
                  {corpus.name}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label className="grid gap-1 text-sm">
            Evidence type
            <NativeSelect
              aria-label="Evidence type"
              value={state.evidenceKind}
              onChange={(event) => onUpdate({ evidenceKind: event.target.value as EvidenceKind })}
            >
              <option value="">All evidence</option>
              {evidenceKinds.map((kind) => (
                <option key={kind} value={kind}>
                  {evidenceKindLabels[kind]}
                </option>
              ))}
            </NativeSelect>
          </label>
          <label className="grid gap-1 text-sm">
            Published from
            <Input
              type="date"
              aria-label="Published from"
              value={state.publishedFrom}
              onChange={(event) => onUpdate({ publishedFrom: event.target.value }, "replace")}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Published to
            <Input
              type="date"
              aria-label="Published to"
              value={state.publishedTo}
              onChange={(event) => onUpdate({ publishedTo: event.target.value }, "replace")}
            />
          </label>
          <div className="flex gap-1" role="group" aria-label="Evidence view">
            <Button
              type="button"
              variant={state.profileView === "" ? "default" : "outline"}
              aria-pressed={state.profileView === ""}
              onClick={() => onUpdate({ profileView: "" })}
            >
              Evidence list
            </Button>
            <Button
              type="button"
              variant={state.profileView === "timeline" ? "default" : "outline"}
              aria-pressed={state.profileView === "timeline"}
              onClick={() => onUpdate({ profileView: "timeline" })}
            >
              Publication timeline
            </Button>
          </div>
        </div>
        <p
          className="text-sm text-muted-foreground"
          aria-live="polite"
          data-testid="evidence-count"
        >
          {occurrences} {occurrences === 1 ? "occurrence" : "occurrences"} in {videos.length}{" "}
          {videos.length === 1 ? "video" : "videos"}
          {undated > 0 ? `; ${undated} without a known publication date` : ""}
          {state.publishedFrom || state.publishedTo
            ? ". Videos with unknown publication dates are outside any date range."
            : "."}
        </p>
        {state.profileView === "timeline" ? (
          <PublicationTimeline videos={videos} state={state} onUpdate={onUpdate} />
        ) : (
          <EvidenceList evidence={evidence} unitsByKey={unitsByKey} />
        )}
      </SurfaceContent>
    </Surface>
  );
}

function EvidenceList({
  evidence,
  unitsByKey,
}: {
  evidence: IdentityProfile["evidence"];
  unitsByKey: Map<string, IdentityLocalUnit>;
}) {
  const shown = evidence.filter(
    (unit) => (unit.mentions?.videos.length ?? 0) + (unit.tracks?.videos.length ?? 0) > 0,
  );
  if (shown.length === 0) {
    return <p className="text-sm text-muted-foreground">No evidence matches these filters.</p>;
  }
  return (
    <>
      {shown.map((unit) => {
        const local = unitsByKey.get(unit.unitKey);
        return (
          <section key={unit.unitKey} className="grid gap-2">
            <h3 className="text-sm font-semibold">
              {local?.label || "Unnamed person"} in {local?.corpusName}
              {unit.tracks ? " (shown or heard, inferred identity)" : " (mentioned)"}
            </h3>
            {unit.mentions ? (
              <MentionVideos corpusId={unit.corpusId} videos={unit.mentions.videos} />
            ) : null}
            {unit.tracks ? (
              <TrackVideos corpusId={unit.corpusId} videos={unit.tracks.videos} />
            ) : null}
          </section>
        );
      })}
    </>
  );
}

function useElementWidth<T extends HTMLElement>(): [React.RefObject<T | null>, number] {
  const ref = React.useRef<T>(null);
  const [width, setWidth] = React.useState(0);
  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    setWidth(element.clientWidth);
    const observer = new ResizeObserver(() => setWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

const LANE_HEIGHT = 26;
const AXIS_HEIGHT = 28;

function markerLabel(video: TimelineVideo) {
  const kinds = evidenceKinds
    .filter((kind) => video.counts[kind] > 0)
    .map((kind) => `${evidenceKindLabels[kind]} ×${video.counts[kind]}`)
    .join(", ");
  return `${video.publicationDate}: ${video.title ?? video.sourceUrl}; ${kinds}${
    video.inferred ? "; inferred identity" : ""
  }`;
}

function PublicationTimeline({
  videos,
  state,
  onUpdate,
}: {
  videos: TimelineVideo[];
  state: CorpusUrlState;
  onUpdate: Props["onUpdate"];
}) {
  const [trackRef, width] = useElementWidth<HTMLDivElement>();
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const dated = videos.filter((video) => video.publicationDate);
  const domain = timelineDomain(
    dated.map((video) => video.publicationDate as string),
    state.publishedFrom,
    state.publishedTo,
  );
  const markers = domain && width > 0 ? layoutMarkers(dated, domain, width) : [];
  const lanes = markers.reduce((max, marker) => Math.max(max, marker.lane + 1), 1);
  const ticks = domain && width > 0 ? axisTicks(domain, width) : [];
  const byId = new Map(videos.map((video) => [video.videoId, video]));
  const selected = selectedId ? (byId.get(selectedId) ?? null) : null;
  const zoom = (factor: number) => {
    if (!domain) return;
    const center =
      selected?.publicationDate && factor < 1 ? dateMs(selected.publicationDate) : undefined;
    const range = zoomDomain(domain, factor, center);
    onUpdate({ publishedFrom: range.from, publishedTo: range.to });
  };
  return (
    <section className="grid gap-3" aria-label="Publication timeline">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!domain}
          onClick={() => zoom(0.5)}
        >
          Zoom in
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!domain}
          onClick={() => zoom(2)}
        >
          Zoom out
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!state.publishedFrom && !state.publishedTo}
          onClick={() => onUpdate({ publishedFrom: "", publishedTo: "" })}
        >
          Full range
        </Button>
        <span className="flex items-center gap-1 text-muted-foreground">
          <span className="inline-block size-3 rounded-full bg-primary" aria-hidden /> Mentions
        </span>
        <span className="flex items-center gap-1 text-muted-foreground">
          <span
            className="inline-block size-3 rounded-full border-2 border-dashed border-primary"
            aria-hidden
          />{" "}
          Inferred face/voice identity
        </span>
        <a
          className="ml-auto underline"
          href={corpusPageUrl(new URL(window.location.href), "events").href}
          onClick={navigateInApp}
        >
          Historical event dates are on Events
        </a>
      </div>
      <div
        ref={trackRef}
        className="relative w-full overflow-hidden rounded-md border border-border bg-muted/30"
        style={{ height: lanes * LANE_HEIGHT + AXIS_HEIGHT + 12 }}
        role="group"
        aria-label={
          domain
            ? `Videos by publication date, ${new Date(domain.start).toISOString().slice(0, 10)} to ${new Date(domain.end).toISOString().slice(0, 10)}`
            : "Videos by publication date"
        }
        data-testid="timeline-track"
      >
        {!domain ? (
          <p className="p-3 text-sm text-muted-foreground">
            No matching video has a known publication date.
          </p>
        ) : null}
        <div
          className="absolute inset-x-0 border-t border-border"
          style={{ bottom: AXIS_HEIGHT }}
          aria-hidden
        />
        {ticks.map((tick) => (
          <span
            key={`${tick.label}:${tick.x}`}
            className="absolute -translate-x-1/2 text-xs text-muted-foreground"
            style={{ left: tick.x, bottom: 6 }}
            aria-hidden
          >
            {tick.label}
          </span>
        ))}
        {markers.map((marker) => {
          const video = byId.get(marker.videoId) as TimelineVideo;
          const size = Math.min(20, 12 + Math.round(Math.log2(video.moments.length + 1) * 2));
          const isSelected = marker.videoId === selectedId;
          return (
            <button
              key={marker.videoId}
              type="button"
              data-testid="timeline-marker"
              data-video-id={marker.videoId}
              data-date={video.publicationDate}
              aria-label={markerLabel(video)}
              aria-pressed={isSelected}
              title={markerLabel(video)}
              className={`absolute -translate-x-1/2 rounded-full border-2 border-primary focus-visible:outline-2 focus-visible:outline-offset-2 ${
                video.inferred ? "border-dashed bg-background" : "bg-primary"
              } ${isSelected ? "ring-2 ring-offset-2 ring-ring" : ""}`}
              style={{
                left: marker.x,
                bottom: AXIS_HEIGHT + 6 + marker.lane * LANE_HEIGHT,
                width: size,
                height: size,
              }}
              onClick={() => setSelectedId(isSelected ? null : marker.videoId)}
            />
          );
        })}
      </div>
      {selected ? <SelectedVideo video={selected} /> : null}
      <TimelineTable videos={videos} selectedId={selectedId} onSelect={setSelectedId} />
    </section>
  );
}

function SelectedVideo({ video }: { video: TimelineVideo }) {
  const current = new URL(window.location.href);
  return (
    <article
      className="grid gap-2 rounded-md border border-border p-3"
      aria-label="Selected video"
      data-testid="timeline-selection"
    >
      <header className="flex flex-wrap items-center gap-2">
        <a
          className="font-medium underline-offset-4 hover:underline"
          href={
            videoUrl(patchCorpusUrl(current, { corpusId: video.corpora[0].id }), video.videoId).href
          }
          onClick={navigateInApp}
        >
          {video.title ?? video.sourceUrl}
        </a>
        <PublicationDate date={video.publicationDate} />
        <span className="text-sm text-muted-foreground">
          {video.corpora.map((corpus) => corpus.name).join(", ")}
        </span>
      </header>
      <ul className="grid gap-1 text-sm">
        {video.moments.map((moment) => (
          <li key={moment.id} className="flex flex-wrap items-center gap-2">
            <Badge
              variant={moment.kind === "shown" || moment.kind === "heard" ? "outline" : "secondary"}
            >
              {evidenceKindLabels[moment.kind]}
            </Badge>
            {moment.kind === "shown" || moment.kind === "heard" ? (
              <Badge variant="outline">Inferred identity</Badge>
            ) : null}
            {moment.moment ? (
              <a
                className="underline"
                href={
                  videoUrl(
                    patchCorpusUrl(current, { corpusId: moment.corpusId }),
                    video.videoId,
                    moment.moment,
                  ).href
                }
                onClick={navigateInApp}
              >
                Open moment {formatInterval(moment.startSeconds, moment.endSeconds)}
              </a>
            ) : null}
            <a
              className="underline"
              href={timestampUrl(video.sourceUrl, moment.startSeconds)}
              target="_blank"
              rel="noreferrer"
            >
              {moment.moment
                ? "Source"
                : `Source ${formatInterval(moment.startSeconds, moment.endSeconds)}`}
            </a>
            {moment.text ? <span>“{moment.text}”</span> : null}
            <span className="text-muted-foreground">{moment.provenance}</span>
          </li>
        ))}
      </ul>
    </article>
  );
}

function TimelineTable({
  videos,
  selectedId,
  onSelect,
}: {
  videos: TimelineVideo[];
  selectedId: string | null;
  onSelect: (videoId: string | null) => void;
}) {
  if (videos.length === 0) {
    return <p className="text-sm text-muted-foreground">No evidence matches these filters.</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm" aria-label="Publication timeline table">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1 pr-3 font-medium">Published</th>
            <th className="py-1 pr-3 font-medium">Video</th>
            <th className="py-1 pr-3 font-medium">Evidence</th>
            <th className="hidden py-1 pr-3 font-medium sm:table-cell">Corpus</th>
            <th className="py-1 font-medium">
              <span className="sr-only">Select</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {videos.map((video) => (
            <tr key={video.videoId} className="border-t border-border align-top">
              <td className="py-1 pr-3 whitespace-nowrap">
                {video.publicationDate ? (
                  <time dateTime={video.publicationDate}>{video.publicationDate}</time>
                ) : (
                  "Unknown"
                )}
              </td>
              <td className="py-1 pr-3">{video.title ?? video.sourceUrl}</td>
              <td className="py-1 pr-3">
                {evidenceKinds
                  .filter((kind) => video.counts[kind] > 0)
                  .map((kind) => `${evidenceKindLabels[kind]} ×${video.counts[kind]}`)
                  .join(", ")}
              </td>
              <td className="hidden py-1 pr-3 sm:table-cell">
                {video.corpora.map((corpus) => corpus.name).join(", ")}
              </td>
              <td className="py-1">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  aria-pressed={video.videoId === selectedId}
                  onClick={() => onSelect(video.videoId === selectedId ? null : video.videoId)}
                >
                  {video.videoId === selectedId ? "Selected" : "Select"}
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
