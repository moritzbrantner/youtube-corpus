import * as React from "react";

import { Badge, Button } from "@moritzbrantner/ui";

import type { EventClaim, HistoricalTimeline as Timeline } from "./api";
import { formatInterval, timestampUrl } from "./evidence";
import {
  formatEventDate,
  historicalDomain,
  historicalTicks,
  layoutHistorical,
  splitSource,
  timelineDates,
  type HistoricalRow,
} from "./historical-timeline";
import { useElementWidth } from "./ProfileEvidence";
import { eventUrl, navigateInApp, patchCorpusUrl, videoUrl } from "./url-state";

const LANE_HEIGHT = 18;
const AXIS_HEIGHT = 28;

export const reviewLabels: Record<EventClaim["reviewStatus"], string> = {
  unverified: "Unverified",
  confirmed: "Confirmed",
  rejected: "Rejected",
};

function barLabel(row: HistoricalRow, claim: EventClaim) {
  const source = claim.modality === "visible" ? "on-screen text" : "spoken";
  return `${formatEventDate(claim.date)}: ${row.event.label}; ${reviewLabels[claim.reviewStatus]}${
    claim.extraction ? ", suggested automatically" : ""
  }${claim.polarity === "denies" ? "; denies this date" : ""}; ${source} in ${
    claim.videoTitle ?? "video"
  }`;
}

function barClass(claim: EventClaim) {
  const status =
    claim.reviewStatus === "confirmed"
      ? "border-primary bg-primary"
      : claim.reviewStatus === "rejected"
        ? "border-dashed border-muted-foreground bg-muted opacity-60"
        : "border-dashed border-primary bg-background";
  return `${status} ${claim.polarity === "denies" ? "border-destructive" : ""} ${
    claim.date?.approximate ? "opacity-70" : ""
  } ${claim.preferred ? "ring-2 ring-primary ring-offset-1" : ""}`;
}

/**
 * Claimed historical dates by event (#107): every reading of an event sits on its row, so
 * disagreeing sources show side by side. Unreviewed suggestions stay visibly unverified, and
 * every bar opens the video moment the claim cites. Publication dates never appear here.
 */
export function HistoricalTimeline({
  timeline,
  corpusId,
  empty,
}: {
  timeline: Timeline;
  corpusId: string;
  empty: string;
}) {
  const [trackRef, width] = useElementWidth<HTMLDivElement>();
  const [showRejected, setShowRejected] = React.useState(false);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const domain = historicalDomain(timelineDates(timeline, showRejected));
  const rows = domain && width > 0 ? layoutHistorical(timeline, domain, width, showRejected) : [];
  const ticks = domain && width > 0 ? historicalTicks(domain, width) : [];
  const claims = timeline.events.flatMap(({ event, claims }) =>
    claims.map((claim) => ({ event, claim })),
  );
  const rejected = claims.filter(({ claim }) => claim.reviewStatus === "rejected").length;
  const unverified = claims.filter(({ claim }) => claim.reviewStatus === "unverified").length;
  const selected = claims.find(({ claim }) => claim.id === selectedId) ?? null;
  if (timeline.events.length === 0) {
    return <p className="text-sm text-muted-foreground">{empty}</p>;
  }
  return (
    <section className="grid gap-3" aria-label="Historical timeline">
      <p className="text-sm text-muted-foreground" data-testid="historical-count">
        {claims.length} date {claims.length === 1 ? "claim" : "claims"} on {timeline.events.length}{" "}
        {timeline.events.length === 1 ? "event" : "events"}; {unverified} unverified. Dates are what
        the sources claim, not established facts, and not when the videos were published.
        {timeline.truncated ? " Only the first 200 events are shown." : ""}
      </p>
      <div className="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-5 rounded-sm border-2 border-primary bg-primary" />
          Confirmed
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-5 rounded-sm border-2 border-dashed border-primary bg-background" />
          Unverified
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-5 rounded-sm border-2 border-primary bg-primary opacity-70" />
          Approximate (c.)
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-3 w-5 rounded-sm border-2 border-primary bg-primary ring-2 ring-primary ring-offset-1" />
          Preferred reading
        </span>
        <label className="ml-auto flex items-center gap-1">
          <input
            type="checkbox"
            checked={showRejected}
            disabled={rejected === 0}
            onChange={(event) => setShowRejected(event.target.checked)}
          />
          Show rejected ({rejected})
        </label>
      </div>
      <div
        ref={trackRef}
        className="relative w-full overflow-hidden rounded-md border border-border bg-muted/30"
        role="group"
        aria-label="Event dates by historical year"
        data-testid="historical-track"
      >
        {!domain ? (
          <p className="p-3 text-sm text-muted-foreground">No claim in view has a known date.</p>
        ) : null}
        {ticks.map((tick) => (
          <span
            key={`grid:${tick.label}`}
            className="absolute inset-y-0 border-l border-border/60"
            style={{ left: tick.x }}
            aria-hidden
          />
        ))}
        {rows.map((row) => (
          <div
            key={row.event.id}
            className="relative grid gap-1 px-2 pt-2"
            data-testid="historical-row"
          >
            <div className="relative z-10 flex flex-wrap items-center gap-2 text-sm">
              <a
                className="font-medium underline-offset-4 hover:underline"
                href={eventUrl(new URL(window.location.href), row.event.id).href}
                onClick={navigateInApp}
              >
                {row.event.label}
              </a>
              {row.event.disputed ? <Badge variant="destructive">Disputed</Badge> : null}
              {row.undated.length > 0 ? (
                <span className="text-xs text-muted-foreground">
                  + {row.undated.length} without a date
                </span>
              ) : null}
            </div>
            <div className="relative -mx-2" style={{ height: row.lanes * LANE_HEIGHT }}>
              {row.bars.map((bar) => {
                const isSelected = bar.claim.id === selectedId;
                return (
                  <button
                    key={bar.claim.id}
                    type="button"
                    data-testid="historical-bar"
                    data-claim-id={bar.claim.id}
                    data-review={bar.claim.reviewStatus}
                    data-start-year={bar.claim.date?.startYear}
                    aria-label={barLabel(row, bar.claim)}
                    aria-pressed={isSelected}
                    title={barLabel(row, bar.claim)}
                    className={`absolute rounded-sm border-2 focus-visible:outline-2 focus-visible:outline-offset-2 ${barClass(
                      bar.claim,
                    )} ${isSelected ? "outline-2 outline-offset-2 outline-ring" : ""}`}
                    style={{
                      left: bar.x,
                      width: bar.x1 - bar.x,
                      top: bar.lane * LANE_HEIGHT + 2,
                      height: LANE_HEIGHT - 6,
                    }}
                    onClick={() => setSelectedId(isSelected ? null : bar.claim.id)}
                  />
                );
              })}
            </div>
          </div>
        ))}
        <div className="relative border-t border-border" style={{ height: AXIS_HEIGHT }}>
          {ticks.map((tick) => (
            <span
              key={`${tick.label}:${tick.x}`}
              className="absolute -translate-x-1/2 text-xs whitespace-nowrap text-muted-foreground"
              style={{ left: tick.x, bottom: 6 }}
              aria-hidden
            >
              {tick.label}
            </span>
          ))}
        </div>
      </div>
      {selected ? (
        <SelectedClaim claim={selected.claim} eventId={selected.event.id} corpusId={corpusId} />
      ) : null}
      <HistoricalTable
        rows={timeline.events}
        showRejected={showRejected}
        selectedId={selectedId}
        onSelect={setSelectedId}
      />
    </section>
  );
}

/** The cited source text, with the matched expression of a suggestion highlighted. */
export function SourceQuote({ claim }: { claim: EventClaim }) {
  if (!claim.evidenceText) {
    return <span>The original evidence was reprocessed or removed.</span>;
  }
  const parts = claim.extraction
    ? splitSource(
        claim.evidenceText,
        claim.extraction.sourceByteStart,
        claim.extraction.sourceByteEnd,
      )
    : null;
  if (!parts || parts[1] !== claim.extraction?.sourceText) {
    return <span>{claim.evidenceText}</span>;
  }
  return (
    <span>
      {parts[0]}
      <mark className="rounded-sm bg-primary/20 px-0.5 text-foreground">{parts[1]}</mark>
      {parts[2]}
    </span>
  );
}

/** Who suggested a claim; nothing for a claim a researcher recorded. */
export function ExtractionNote({ claim }: { claim: EventClaim }) {
  if (!claim.extraction) return null;
  return (
    <span className="text-xs text-muted-foreground" data-testid="claim-provenance">
      Suggested automatically · {claim.extraction.producer}@{claim.extraction.producerVersion} ·
      confidence {claim.extraction.confidence.toFixed(2)}
    </span>
  );
}

function SelectedClaim({
  claim,
  eventId,
  corpusId,
}: {
  claim: EventClaim;
  eventId: string;
  corpusId: string;
}) {
  const current = patchCorpusUrl(new URL(window.location.href), { corpusId });
  return (
    <article
      className="grid gap-2 rounded-md border border-border p-3 text-sm"
      aria-label="Selected claim"
      data-testid="historical-selection"
    >
      <header className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{formatEventDate(claim.date)}</span>
        <Badge variant={claim.reviewStatus === "confirmed" ? "secondary" : "outline"}>
          {reviewLabels[claim.reviewStatus]}
        </Badge>
        {claim.reviewStatus === "unverified" ? (
          <span className="text-muted-foreground">Not reviewed; not an established fact.</span>
        ) : null}
        {claim.polarity === "denies" ? <Badge variant="destructive">Denies</Badge> : null}
        {claim.certainty !== "stated" ? <Badge variant="outline">{claim.certainty}</Badge> : null}
        {claim.preferred ? <Badge variant="secondary">Preferred reading</Badge> : null}
      </header>
      <blockquote className="rounded-md bg-muted px-3 py-2">
        <span className="block text-xs text-muted-foreground">
          {claim.videoTitle ?? "Video"} ·{" "}
          {claim.modality === "visible" ? "On-screen text" : "Spoken"} ·{" "}
          {formatInterval(claim.startSeconds, claim.endSeconds)}
          {claim.publicationDate ? ` · video published ${claim.publicationDate}` : ""}
        </span>
        <SourceQuote claim={claim} />
      </blockquote>
      <ExtractionNote claim={claim} />
      <div className="flex flex-wrap gap-3">
        <a
          className="underline"
          href={
            videoUrl(current, claim.videoId, {
              modality: claim.modality,
              evidenceId: claim.evidenceId,
            }).href
          }
          onClick={navigateInApp}
        >
          Open moment {formatInterval(claim.startSeconds, claim.endSeconds)}
        </a>
        <a
          className="underline"
          href={timestampUrl(claim.sourceUrl, claim.startSeconds)}
          target="_blank"
          rel="noreferrer"
        >
          Source
        </a>
        <a
          className="underline"
          href={eventUrl(new URL(window.location.href), eventId).href}
          onClick={navigateInApp}
        >
          Review on the event page
        </a>
      </div>
    </article>
  );
}

function HistoricalTable({
  rows,
  showRejected,
  selectedId,
  onSelect,
}: {
  rows: Timeline["events"];
  showRejected: boolean;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const entries = rows.flatMap(({ event, claims }) =>
    claims
      .filter((claim) => showRejected || claim.reviewStatus !== "rejected")
      .map((claim) => ({ event, claim })),
  );
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm" aria-label="Historical timeline table">
        <thead className="text-muted-foreground">
          <tr>
            <th className="py-1 pr-3 font-medium">Claimed date</th>
            <th className="py-1 pr-3 font-medium">Event</th>
            <th className="py-1 pr-3 font-medium">Review</th>
            <th className="hidden py-1 pr-3 font-medium sm:table-cell">Source</th>
            <th className="py-1 font-medium">
              <span className="sr-only">Select</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {entries.map(({ event, claim }) => (
            <tr key={claim.id} className="border-t border-border align-top">
              <td className="py-1 pr-3 whitespace-nowrap">
                {claim.polarity === "denies" ? "Not " : ""}
                {formatEventDate(claim.date)}
              </td>
              <td className="py-1 pr-3">
                {event.label}
                {event.disputed ? " (disputed)" : ""}
              </td>
              <td className="py-1 pr-3">
                {reviewLabels[claim.reviewStatus]}
                {claim.extraction ? ", suggested" : ""}
              </td>
              <td className="hidden py-1 pr-3 sm:table-cell">
                {claim.modality === "visible" ? "On-screen text" : "Spoken"} in{" "}
                {claim.videoTitle ?? "video"}
              </td>
              <td className="py-1">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  aria-pressed={claim.id === selectedId}
                  onClick={() => onSelect(claim.id === selectedId ? null : claim.id)}
                >
                  {claim.id === selectedId ? "Selected" : "Select"}
                </Button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
