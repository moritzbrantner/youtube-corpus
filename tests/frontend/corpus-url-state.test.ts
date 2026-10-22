import { describe, expect, test } from "bun:test";

import {
  corpusPageUrl,
  patchCorpusUrl,
  readUrlState,
} from "../../src/app/features/corpora/url-state";

describe("corpus navigation state", () => {
  test("page changes preserve query, all facets and selected original evidence", () => {
    const current = new URL(
      "http://localhost/search?corpus=one&q=helios&segment=original&mode=fts&modality=visible&sourceKind=visual_ocr&language=en&channel=Source&title=Slide&category=Education&tag=history&from=2025-01-01&to=2026-12-31",
    );
    const corpora = corpusPageUrl(current, "corpora");
    expect(corpora.pathname).toBe("/corpora");
    expect(readUrlState(corpora)).toEqual(readUrlState(current));
    expect(corpusPageUrl(corpora, "search").href).toBe(current.href);
  });

  test("switching corpus preserves page and filters but clears stale moment", () => {
    const original = new URL(
      "http://localhost/search?corpus=one&q=helios&segment=original&mode=fts&language=en",
    );
    const switched = patchCorpusUrl(original, { corpusId: "two" });
    expect(switched.pathname).toBe("/search");
    expect(readUrlState(switched)).toEqual({
      ...readUrlState(original),
      corpusId: "two",
      segmentId: null,
    });
    expect(readUrlState(original).segmentId).toBe("original");
  });

  test("one submission applies query and facets atomically", () => {
    const url = patchCorpusUrl(new URL("http://localhost/search?corpus=one&segment=old"), {
      query: "helios",
      language: "en",
      mode: "fts",
    });
    expect(readUrlState(url)).toMatchObject({
      corpusId: "one",
      query: "helios",
      language: "en",
      mode: "fts",
      segmentId: null,
    });
    const selected = patchCorpusUrl(url, { segmentId: "evidence" });
    expect(readUrlState(selected)).toEqual({ ...readUrlState(url), segmentId: "evidence" });
  });

  test("invalid enums use real supported modes and unknown corpus stays explicit", () => {
    expect(
      readUrlState(
        new URL(
          "http://localhost/search?corpus=deleted&mode=invalid&modality=invalid&sourceKind=invalid",
        ),
      ),
    ).toMatchObject({ corpusId: "deleted", mode: "hybrid", modality: "all", sourceKind: "all" });
  });
});
