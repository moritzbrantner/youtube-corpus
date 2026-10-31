/**
 * Geometry for claimed historical dates (#107, Atlas 5.5, 7.1). Historical years have no year 0
 * and run into BC, so this axis is fractional astronomical years, never the publication-date axis.
 */
import type { EventClaim, EventDate, HistoricalTimeline } from "./api";
import { TRACK_INSET, stackLanes, type Domain } from "./publication-timeline";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function formatYear(year: number) {
  return year < 0 ? `${-year} BC` : `${year}`;
}

function formatPoint(year: number, month?: number | null, day?: number | null) {
  return [day ?? null, month ? MONTHS[month - 1] : null, formatYear(year)]
    .filter((part) => part !== null)
    .join(" ");
}

/** `14 Oct 1066`, `c. 1067`, `44 BC – 43 BC`; a missing date stays visibly unknown. */
export function formatEventDate(date: EventDate | null) {
  if (!date) return "Date unknown";
  const start = formatPoint(date.startYear, date.startMonth, date.startDay);
  const text =
    date.endYear != null
      ? `${start} – ${formatPoint(date.endYear, date.endMonth, date.endDay)}`
      : start;
  return date.approximate ? `c. ${text}` : text;
}

/** Historical year (no year 0, negative is BC) to astronomical year (1 BC is 0). */
export function astronomical(year: number) {
  return year < 0 ? year + 1 : year;
}

function at(year: number, month?: number | null, day?: number | null) {
  return astronomical(year) + (month ? (month - 1) / 12 : 0) + (month && day ? (day - 1) / 365 : 0);
}

/**
 * A claimed date as a half-open span of fractional astronomical years at its own precision:
 * `1066` covers the whole year, `Oct 1066` the month, a range runs to the end of its last unit.
 */
export function dateSpan(date: EventDate): { start: number; end: number } {
  const ranged = date.endYear != null;
  const endYear = ranged ? (date.endYear as number) : date.startYear;
  const endMonth = ranged ? date.endMonth : date.startMonth;
  const endDay = ranged ? date.endDay : date.startDay;
  const unit = endMonth && endDay ? 1 / 365 : endMonth ? 1 / 12 : 1;
  return {
    start: at(date.startYear, date.startMonth, date.startDay),
    end: at(endYear, endMonth, endDay) + unit,
  };
}

/** The year span holding every dated claim, padded; `null` when no claim has a date. */
export function historicalDomain(dates: EventDate[]): Domain | null {
  if (dates.length === 0) return null;
  const spans = dates.map(dateSpan);
  let start = Math.min(...spans.map((s) => s.start));
  let end = Math.max(...spans.map((s) => s.end));
  if (end - start < 10) {
    const middle = (start + end) / 2;
    start = middle - 5;
    end = middle + 5;
  } else {
    const pad = (end - start) * 0.04;
    start -= pad;
    end += pad;
  }
  return { start, end };
}

export function yearX(value: number, domain: Domain, width: number): number {
  const span = Math.max(1e-6, domain.end - domain.start);
  return TRACK_INSET + ((value - domain.start) / span) * (width - 2 * TRACK_INSET);
}

const STEPS = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 5000];

/** Year labels at a round step leaving roughly `width / 90` ticks; BC years are labeled BC. */
export function historicalTicks(domain: Domain, width: number): { label: string; x: number }[] {
  const target = Math.max(2, Math.floor(width / 90));
  const span = domain.end - domain.start;
  const step = STEPS.find((s) => span / s <= target) ?? 10_000;
  const ticks: { label: string; x: number }[] = [];
  // Historical tick years k·step; year 0 does not exist, so the AD 1 tick stands in for it.
  const first = Math.floor(domain.start / step) - 1;
  const last = Math.ceil(domain.end / step) + 1;
  for (let k = first; k <= last; k += 1) {
    const year = k === 0 ? 1 : k * step;
    const value = astronomical(year);
    if (value >= domain.start && value <= domain.end) {
      ticks.push({ label: formatYear(year), x: yearX(value, domain, width) });
    }
  }
  return ticks;
}

/** One date claim placed on its event's row; bars closer than the gap stack into lanes. */
export type HistoricalBar = { claim: EventClaim; x: number; x1: number; lane: number };

export const MIN_BAR_WIDTH = 10;

export type HistoricalRow = {
  event: HistoricalTimeline["events"][number]["event"];
  bars: HistoricalBar[];
  lanes: number;
  /** Date claims of this event without a date (the source leaves it unknown). */
  undated: EventClaim[];
  /** Rejected readings hidden from this row. */
  hiddenRejected: number;
};

export function layoutHistorical(
  timeline: HistoricalTimeline,
  domain: Domain,
  width: number,
  showRejected: boolean,
): HistoricalRow[] {
  return timeline.events.map(({ event, claims }) => {
    const shown = claims.filter((claim) => showRejected || claim.reviewStatus !== "rejected");
    const bars = stackLanes(
      shown
        .filter((claim): claim is EventClaim & { date: EventDate } => claim.date !== null)
        .map((claim) => {
          const span = dateSpan(claim.date);
          const x = yearX(span.start, domain, width);
          return { claim, x, x1: Math.max(x + MIN_BAR_WIDTH, yearX(span.end, domain, width)) };
        })
        .sort((a, b) => a.x - b.x || a.claim.id.localeCompare(b.claim.id)),
      4,
    );
    return {
      event,
      bars,
      lanes: bars.reduce((max, bar) => Math.max(max, bar.lane + 1), 1),
      undated: shown.filter((claim) => claim.date === null),
      hiddenRejected: claims.length - shown.length,
    };
  });
}

/** Every dated, non-hidden claim of the timeline, for the domain. */
export function timelineDates(timeline: HistoricalTimeline, showRejected: boolean): EventDate[] {
  return timeline.events.flatMap(({ claims }) =>
    claims
      .filter((claim) => showRejected || claim.reviewStatus !== "rejected")
      .flatMap((claim) => (claim.date ? [claim.date] : [])),
  );
}

/** Splits evidence text around the matched expression (UTF-8 byte offsets) for highlighting. */
export function splitSource(
  text: string,
  byteStart: number,
  byteEnd: number,
): [string, string, string] | null {
  const bytes = new TextEncoder().encode(text);
  if (byteStart < 0 || byteEnd > bytes.length || byteStart >= byteEnd) return null;
  const decode = (from: number, to: number) => new TextDecoder().decode(bytes.slice(from, to));
  return [decode(0, byteStart), decode(byteStart, byteEnd), decode(byteEnd, bytes.length)];
}
