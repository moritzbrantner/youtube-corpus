import { describe, expect, test } from "bun:test";

import {
  inPublicationRange,
  layoutMarkers,
  timelineDomain,
  zoomDomain,
} from "../../src/app/features/corpora/publication-timeline";
import { patchCorpusUrl, readUrlState } from "../../src/app/features/corpora/url-state";

describe("publication timeline", () => {
  test("unknown publication dates fall outside any range but stay without one", () => {
    expect(inPublicationRange(null, "", "")).toBe(true);
    expect(inPublicationRange(null, "2020-01-01", "")).toBe(false);
    expect(inPublicationRange("2020-01-01", "2020-01-01", "2020-01-01")).toBe(true);
    expect(inPublicationRange("2021-01-01", "", "2020-12-31")).toBe(false);
  });

  test("markers order by date and stack when closer than the gap", () => {
    const domain = timelineDomain(["2020-01-01", "2020-12-31"], "", "")!;
    const markers = layoutMarkers(
      [
        { videoId: "late", publicationDate: "2020-12-31" },
        { videoId: "undated", publicationDate: null },
        { videoId: "early", publicationDate: "2020-01-01" },
        { videoId: "early2", publicationDate: "2020-01-02" },
      ],
      domain,
      600,
    );
    expect(markers.map((m) => [m.videoId, m.lane])).toEqual([
      ["early", 0],
      ["early2", 1],
      ["late", 0],
    ]);
    expect(markers[0].x).toBeGreaterThan(0);
    expect(markers[2].x).toBeLessThan(600);
  });

  test("zoom halves the span around a center and the range stays inclusive dates", () => {
    const domain = timelineDomain([], "2020-01-01", "2020-12-31")!;
    expect(zoomDomain(domain, 0.5, Date.parse("2020-07-01T00:00:00Z"))).toEqual({
      from: "2020-03-31",
      to: "2020-09-30",
    });
    expect(timelineDomain([], "", "")).toBeNull();
  });

  test("profile view, evidence type and range round-trip through the URL", () => {
    const url = patchCorpusUrl(new URL("http://localhost/people/id?corpus=c&segment=s"), {
      profileView: "timeline",
      evidenceKind: "heard",
      publishedFrom: "2020-01-01",
      publishedTo: "2021-01-01",
    });
    expect(url.search).toBe(
      "?corpus=c&segment=s&view=timeline&evidence=heard&since=2020-01-01&until=2021-01-01",
    );
    expect(readUrlState(url)).toMatchObject({
      profileView: "timeline",
      evidenceKind: "heard",
      publishedFrom: "2020-01-01",
      publishedTo: "2021-01-01",
    });
    expect(readUrlState(new URL("http://localhost/?evidence=bogus")).evidenceKind).toBe("");
  });
});
