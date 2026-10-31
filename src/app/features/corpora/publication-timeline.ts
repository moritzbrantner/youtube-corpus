/**
 * Filtering and geometry for profile evidence over video publication dates (#105, Atlas 5.4-5.5).
 * Publication dates come from video metadata only; an unknown date is never placed on the axis.
 */
import type { EntityOccurrencesReport, IdentityProfile, PersonOccurrencesReport } from "./api";
import type { EvidenceKind, MomentModality } from "./url-state";

export type EvidenceFilter = { kind: "" | EvidenceKind; from: string; to: string };

export const evidenceKindLabels: Record<EvidenceKind, string> = {
  spoken: "Spoken mention",
  ocr: "On-screen text mention",
  shown: "Shown (face track)",
  heard: "Heard (voice track)",
};

/** Whether a known publication date falls in the inclusive range; no range keeps every video. */
export function inPublicationRange(date: string | null, from: string, to: string): boolean {
  if (!from && !to) return true;
  if (!date) return false;
  return (!from || date >= from) && (!to || date <= to);
}

export function filterMentions(
  report: EntityOccurrencesReport,
  filter: EvidenceFilter,
): EntityOccurrencesReport {
  const videos = report.videos
    .filter((video) => inPublicationRange(video.publicationDate, filter.from, filter.to))
    .map((video) => {
      const occurrences = video.occurrences.filter(
        (occurrence) => !filter.kind || mentionKind(occurrence.modality) === filter.kind,
      );
      return {
        ...video,
        occurrences,
        spokenMentions: occurrences.filter((o) => o.modality === "spoken").length,
        visibleTextMentions: occurrences.filter((o) => o.modality === "visible_text").length,
      };
    })
    .filter((video) => video.occurrences.length > 0);
  return { ...report, videos };
}

export function filterTracks(
  report: PersonOccurrencesReport,
  filter: EvidenceFilter,
): PersonOccurrencesReport {
  const videos = report.videos
    .filter((video) => inPublicationRange(video.publicationDate, filter.from, filter.to))
    .map((video) => {
      const occurrences = video.occurrences.filter(
        (occurrence) => !filter.kind || occurrence.kind === filter.kind,
      );
      const tracks = (kind: "shown" | "heard") =>
        new Set(occurrences.filter((o) => o.kind === kind).map((o) => o.trackId)).size;
      return { ...video, occurrences, shownTracks: tracks("shown"), heardTracks: tracks("heard") };
    })
    .filter((video) => video.occurrences.length > 0);
  return { ...report, videos };
}

function mentionKind(modality: "spoken" | "visible_text"): EvidenceKind {
  return modality === "spoken" ? "spoken" : "ocr";
}

/** One evidence record on the timeline, with what it takes to open its source moment. */
export type TimelineMoment = {
  id: string;
  kind: EvidenceKind;
  corpusId: string;
  startSeconds: number | null;
  endSeconds: number | null;
  /** Mentions open as an exact in-app moment; face/voice tracks only at a source timestamp. */
  moment: { modality: MomentModality; evidenceId: string } | null;
  text: string | null;
  provenance: string;
};

/** Every filtered evidence record of one video, placed at the video's publication date. */
export type TimelineVideo = {
  videoId: string;
  title: string | null;
  sourceUrl: string;
  publicationDate: string | null;
  corpora: { id: string; name: string }[];
  counts: Record<EvidenceKind, number>;
  /** Face or voice tracks attributed to an inferred identity, not a text mention. */
  inferred: boolean;
  moments: TimelineMoment[];
};

/** Filtered profile evidence by video: dated videos by date, then undated ones. */
export function timelineVideos(
  evidence: IdentityProfile["evidence"],
  corpusNames: Map<string, string>,
): TimelineVideo[] {
  const byVideo = new Map<string, TimelineVideo>();
  const entry = (
    corpusId: string,
    video: { videoId: string; title: string | null; sourceUrl: string },
    publicationDate: string | null,
  ) => {
    let found = byVideo.get(video.videoId);
    if (!found) {
      found = {
        videoId: video.videoId,
        title: video.title,
        sourceUrl: video.sourceUrl,
        publicationDate,
        corpora: [],
        counts: { spoken: 0, ocr: 0, shown: 0, heard: 0 },
        inferred: false,
        moments: [],
      };
      byVideo.set(video.videoId, found);
    }
    if (!found.corpora.some((corpus) => corpus.id === corpusId)) {
      found.corpora.push({ id: corpusId, name: corpusNames.get(corpusId) ?? corpusId });
    }
    return found;
  };
  for (const unit of evidence) {
    for (const video of unit.mentions?.videos ?? []) {
      const target = entry(unit.corpusId, video, video.publicationDate);
      for (const o of video.occurrences) {
        const kind = mentionKind(o.modality);
        const evidenceId = o.modality === "spoken" ? o.transcriptSegmentId : o.visualTextTrackId;
        target.counts[kind] += 1;
        target.moments.push({
          id: o.id,
          kind,
          corpusId: unit.corpusId,
          startSeconds: o.startSeconds,
          endSeconds: o.endSeconds,
          moment: evidenceId
            ? { modality: o.modality === "spoken" ? "spoken" : "visible", evidenceId }
            : null,
          text: o.surfaceText,
          provenance: `${o.producer} · ${o.model}`,
        });
      }
    }
    for (const video of unit.tracks?.videos ?? []) {
      const target = entry(unit.corpusId, video, video.publicationDate);
      target.inferred = true;
      for (const o of video.occurrences) {
        target.counts[o.kind] += 1;
        target.moments.push({
          id: `${o.kind}:${o.trackId}:${o.clusterId}`,
          kind: o.kind,
          corpusId: unit.corpusId,
          startSeconds: o.startSeconds,
          endSeconds: o.endSeconds,
          moment: null,
          text: null,
          provenance: `${o.linkSource ?? "unknown link source"} · ${o.processor} ${o.model}@${o.modelVersion}`,
        });
      }
    }
  }
  for (const video of byVideo.values()) {
    video.moments.sort((a, b) => (a.startSeconds ?? Infinity) - (b.startSeconds ?? Infinity));
  }
  return [...byVideo.values()].sort(
    (a, b) =>
      (a.publicationDate === null ? 1 : 0) - (b.publicationDate === null ? 1 : 0) ||
      (a.publicationDate ?? "").localeCompare(b.publicationDate ?? "") ||
      a.videoId.localeCompare(b.videoId),
  );
}

const DAY = 86_400_000;

export function dateMs(date: string): number {
  return Date.parse(`${date}T00:00:00Z`);
}

export function isoDate(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

export type Domain = { start: number; end: number };

/**
 * The visible date span: the range filter where set, else the dated videos padded so the edge
 * markers are not clipped. `null` when nothing has a known date.
 */
export function timelineDomain(dates: string[], from: string, to: string): Domain | null {
  const known = dates.map(dateMs).filter(Number.isFinite);
  const fromMs = from ? dateMs(from) : NaN;
  const toMs = to ? dateMs(to) : NaN;
  if (known.length === 0 && !(Number.isFinite(fromMs) && Number.isFinite(toMs))) return null;
  let start = Number.isFinite(fromMs) ? fromMs : Math.min(...known);
  let end = Number.isFinite(toMs) ? toMs : Math.max(...known);
  if (end - start < 30 * DAY) {
    const middle = (start + end) / 2;
    start = middle - 15 * DAY;
    end = middle + 15 * DAY;
  } else {
    const pad = (end - start) * 0.04;
    if (!Number.isFinite(fromMs)) start -= pad;
    if (!Number.isFinite(toMs)) end += pad;
  }
  return { start, end };
}

/** Horizontal inset so edge markers stay inside the track. */
export const TRACK_INSET = 16;

export function dateX(date: string, domain: Domain, width: number): number {
  const span = Math.max(1, domain.end - domain.start);
  return TRACK_INSET + ((dateMs(date) - domain.start) / span) * (width - 2 * TRACK_INSET);
}

export type Marker = { videoId: string; x: number; lane: number };

/** Places dated videos left to right by date; markers closer than `minGap` stack into lanes. */
export function layoutMarkers(
  videos: { videoId: string; publicationDate: string | null }[],
  domain: Domain,
  width: number,
  minGap = 24,
): Marker[] {
  const laneEnds: number[] = [];
  return videos
    .filter((video): video is { videoId: string; publicationDate: string } =>
      Boolean(video.publicationDate),
    )
    .map((video) => ({ videoId: video.videoId, x: dateX(video.publicationDate, domain, width) }))
    .sort((a, b) => a.x - b.x || a.videoId.localeCompare(b.videoId))
    .map(({ videoId, x }) => {
      let lane = laneEnds.findIndex((end) => x - end >= minGap);
      if (lane === -1) lane = laneEnds.length;
      laneEnds[lane] = x;
      return { videoId, x, lane };
    });
}

/** Scales the span around `center` (default: its middle); factor < 1 zooms in. */
export function zoomDomain(
  domain: Domain,
  factor: number,
  center?: number,
): { from: string; to: string } {
  const middle = center ?? (domain.start + domain.end) / 2;
  const half = Math.max(15 * DAY, ((domain.end - domain.start) * factor) / 2);
  return { from: isoDate(middle - half), to: isoDate(middle + half) };
}

/** Axis labels at year, month or week steps, whichever leaves roughly `width / 90` ticks. */
export function axisTicks(domain: Domain, width: number): { label: string; x: number }[] {
  const span = domain.end - domain.start;
  const target = Math.max(2, Math.floor(width / 90));
  const start = new Date(domain.start);
  const ticks: { label: string; x: number }[] = [];
  const push = (ms: number, label: string) => {
    if (ms >= domain.start && ms <= domain.end) {
      ticks.push({ label, x: dateX(isoDate(ms), domain, width) });
    }
  };
  const years = span / (365 * DAY);
  if (years >= target * 0.75) {
    const step = Math.max(1, Math.ceil(years / target));
    for (let y = start.getUTCFullYear(); Date.UTC(y, 0, 1) <= domain.end; y += 1) {
      if (y % step === 0) push(Date.UTC(y, 0, 1), String(y));
    }
    return ticks;
  }
  const months = span / (30 * DAY);
  if (months >= target * 0.75) {
    const step = [1, 2, 3, 6].find((s) => months / s <= target) ?? 12;
    for (
      let y = start.getUTCFullYear(), m = start.getUTCMonth();
      Date.UTC(y, m, 1) <= domain.end;
      m += 1
    ) {
      const ms = Date.UTC(y, m, 1);
      const month = new Date(ms).getUTCMonth();
      if (month % step === 0) {
        push(ms, isoDate(ms).slice(0, month === 0 ? 4 : 7));
      }
    }
    return ticks;
  }
  const step = Math.max(1, Math.ceil(span / DAY / target));
  for (let ms = Math.ceil(domain.start / DAY) * DAY; ms <= domain.end; ms += step * DAY) {
    push(ms, isoDate(ms));
  }
  return ticks;
}
