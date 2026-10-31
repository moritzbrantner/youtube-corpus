import { describe, expect, test } from "bun:test";

import type { EventClaim, HistoricalTimeline } from "../../src/app/features/corpora/api";
import {
  astronomical,
  dateSpan,
  formatEventDate,
  historicalDomain,
  historicalTicks,
  layoutHistorical,
  splitSource,
  timelineDates,
} from "../../src/app/features/corpora/historical-timeline";
import { readUrlState } from "../../src/app/features/corpora/url-state";

function claim(id: string, date: EventClaim["date"], reviewStatus: EventClaim["reviewStatus"]) {
  return { id, date, reviewStatus } as EventClaim;
}

describe("historical timeline", () => {
  test("BC years have no year 0 and spans keep their precision", () => {
    expect(astronomical(-1)).toBe(0);
    expect(astronomical(1)).toBe(1);
    expect(dateSpan({ startYear: 1066, approximate: false })).toEqual({ start: 1066, end: 1067 });
    const day = dateSpan({ startYear: 1066, startMonth: 10, startDay: 14, approximate: false });
    expect(day.end - day.start).toBeCloseTo(1 / 365);
    // 5th century BC: 500 BC through 401 BC.
    expect(dateSpan({ startYear: -500, endYear: -401, approximate: false })).toEqual({
      start: -499,
      end: -399,
    });
    expect(
      formatEventDate({ startYear: -44, startMonth: 3, startDay: 15, approximate: true }),
    ).toBe("c. 15 Mar 44 BC");
  });

  test("disagreeing readings share their event row; rejected ones hide unless asked", () => {
    const timeline: HistoricalTimeline = {
      truncated: false,
      events: [
        {
          event: { id: "e1", label: "Battle", disputed: true } as never,
          claims: [
            claim("a", { startYear: 1066, approximate: false }, "confirmed"),
            claim("b", { startYear: 1067, approximate: true }, "unverified"),
            claim("c", { startYear: 1070, approximate: false }, "rejected"),
            claim("d", null, "unverified"),
          ],
        },
      ],
    };
    const domain = historicalDomain(timelineDates(timeline, false))!;
    const [row] = layoutHistorical(timeline, domain, 800, false);
    expect(row.bars.map((bar) => bar.claim.id)).toEqual(["a", "b"]);
    expect(row.bars[0].x).toBeLessThan(row.bars[1].x);
    expect(row.undated.map((c) => c.id)).toEqual(["d"]);
    expect(row.hiddenRejected).toBe(1);
    const [all] = layoutHistorical(timeline, domain, 800, true);
    expect(all.bars).toHaveLength(3);
  });

  test("overlapping readings stack into lanes", () => {
    const timeline: HistoricalTimeline = {
      truncated: false,
      events: [
        {
          event: { id: "e1" } as never,
          claims: [
            claim("century", { startYear: 1001, endYear: 1100, approximate: false }, "unverified"),
            claim("year", { startYear: 1066, approximate: false }, "unverified"),
          ],
        },
      ],
    };
    const domain = historicalDomain(timelineDates(timeline, false))!;
    const [row] = layoutHistorical(timeline, domain, 800, false);
    expect(row.bars.map((bar) => [bar.claim.id, bar.lane])).toEqual([
      ["century", 0],
      ["year", 1],
    ]);
    expect(row.lanes).toBe(2);
  });

  test("ticks label BC years and step through round years", () => {
    const ticks = historicalTicks({ start: -520, end: 120 }, 800);
    const labels = ticks.map((tick) => tick.label);
    expect(labels).toContain("500 BC");
    expect(labels).toContain("1");
    expect(labels).toContain("100");
    expect(labels).not.toContain("0");
  });

  test("the matched source text is highlighted by UTF-8 byte offsets", () => {
    const text = "Café: the battle was on 14 October 1066.";
    const start = new TextEncoder().encode("Café: the battle was on ").length;
    expect(splitSource(text, start, start + "14 October 1066".length)).toEqual([
      "Café: the battle was on ",
      "14 October 1066",
      ".",
    ]);
  });

  test("the historical view round-trips through the URL", () => {
    expect(readUrlState(new URL("http://localhost/events?view=historical")).profileView).toBe(
      "historical",
    );
    expect(readUrlState(new URL("http://localhost/events?view=bogus")).profileView).toBe("");
  });
});
