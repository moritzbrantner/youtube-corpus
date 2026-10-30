import { describe, expect, test } from "bun:test";

import {
  attemptUrl,
  corpusPageUrl,
  directoryUrl,
  identityUrl,
  momentModality,
  patchCorpusUrl,
  processingUrl,
  readIdentityRoute,
  readProcessingRoute,
  readUrlState,
  readVideoRoute,
  searchMomentUrl,
  videoUrl,
} from "../../src/app/features/corpora/url-state";

describe("channels and sources navigation", () => {
  const search = new URL(
    "http://localhost/search?corpus=one&q=helios&segment=moment&mode=fts&channel=Source",
  );

  test("opening a source or channel keeps the search and its selected moment", () => {
    const source = directoryUrl(search, { sourceId: "src" });
    expect(source.pathname).toBe("/sources");
    expect(readUrlState(source)).toEqual({ ...readUrlState(search), sourceId: "src" });

    const channel = directoryUrl(source, { channelId: "UC1" });
    expect(readUrlState(channel)).toMatchObject({ sourceId: null, channelId: "UC1" });
    expect(readUrlState(channel).segmentId).toBe("moment");

    const back = corpusPageUrl(directoryUrl(channel), "search");
    expect(readUrlState(back)).toEqual(readUrlState(search));
  });

  test("switching corpus drops source and channel selections", () => {
    const detail = directoryUrl(search, { sourceId: "src" });
    const switched = patchCorpusUrl(detail, { corpusId: "two" });
    expect(readUrlState(switched)).toMatchObject({
      corpusId: "two",
      sourceId: null,
      channelId: null,
    });
    expect(readUrlState(patchCorpusUrl(detail, { corpusId: "one" })).sourceId).toBe("src");
  });
});

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

describe("video and moment pages", () => {
  const search = new URL(
    "http://localhost/search?corpus=one&q=helios&mode=fts&modality=visible&channel=Source",
  );

  test("a search result opens its moment and returns to the same search", () => {
    const moment = searchMomentUrl(search, {
      videoId: "video-1",
      modality: "visible",
      evidenceId: "track-9",
    });
    expect(moment.pathname).toBe("/videos/video-1/moments/visible/track-9");
    expect(readVideoRoute(moment)).toEqual({
      videoId: "video-1",
      moment: { videoId: "video-1", modality: "visible", evidenceId: "track-9" },
    });
    expect(readUrlState(moment)).toEqual({ ...readUrlState(search), segmentId: "track-9" });

    // Moment → whole video → another moment keeps the originating search and its selection.
    const video = videoUrl(moment, "video-1");
    expect(readVideoRoute(video)).toEqual({ videoId: "video-1", moment: null });
    const other = videoUrl(video, "video-1", { modality: "spoken", evidenceId: "seg-2" });
    const back = corpusPageUrl(other, "search");
    expect(back.pathname).toBe("/search");
    expect(readUrlState(back)).toEqual({ ...readUrlState(search), segmentId: "track-9" });
  });

  test("only well-formed video paths are video routes", () => {
    expect(readVideoRoute(new URL("http://localhost/videos"))).toBeNull();
    expect(readVideoRoute(new URL("http://localhost/videos/v/moments/heard/x"))).toBeNull();
    expect(readVideoRoute(new URL("http://localhost/videos/v/moments/spoken"))).toBeNull();
    expect(readVideoRoute(new URL("http://localhost/search"))).toBeNull();
    expect(momentModality("visual_ocr")).toBe("visible");
    expect(momentModality("caption_auto")).toBe("spoken");
  });
});

describe("processing page", () => {
  const runs = new URL(
    "http://localhost/processing?corpus=one&q=helios&segment=moment&stage=entities&state=failed&attempts=all",
  );

  test("selecting a video or attempt keeps run filters and the originating search", () => {
    expect(readProcessingRoute(runs)).toEqual({ attemptId: null });
    expect(readUrlState(runs)).toMatchObject({
      stage: "entities",
      attemptState: "failed",
      attempts: "all",
      processingVideoId: null,
    });

    const video = processingUrl(runs, "video-1");
    expect(readUrlState(video)).toEqual({ ...readUrlState(runs), processingVideoId: "video-1" });

    const attempt = attemptUrl(video, "attempt-7");
    expect(attempt.pathname).toBe("/processing/attempts/attempt-7");
    expect(readProcessingRoute(attempt)).toEqual({ attemptId: "attempt-7" });

    const back = processingUrl(attempt);
    expect(back.pathname).toBe("/processing");
    expect(readUrlState(back)).toEqual(readUrlState(video));
    expect(readUrlState(corpusPageUrl(back, "search")).segmentId).toBe("moment");
  });

  test("switching corpus drops the selected video", () => {
    const switched = patchCorpusUrl(processingUrl(runs, "video-1"), { corpusId: "two" });
    expect(readUrlState(switched)).toMatchObject({ corpusId: "two", processingVideoId: null });
  });

  test("only well-formed processing paths are processing routes", () => {
    expect(readProcessingRoute(new URL("http://localhost/processing/attempts"))).toBeNull();
    expect(readProcessingRoute(new URL("http://localhost/processing/runs/x"))).toBeNull();
    expect(readProcessingRoute(new URL("http://localhost/videos"))).toBeNull();
  });
});

describe("people and entities pages", () => {
  test("profiles keep the corpus and scope and parse back", () => {
    const current = new URL("http://localhost/people?corpus=c1&scope=all&name=ada");
    const url = identityUrl(current, "people", "id-1");
    expect(url.pathname).toBe("/people/id-1");
    expect(readUrlState(url)).toMatchObject({
      corpusId: "c1",
      identityScope: "all",
      identityQuery: "ada",
    });
    expect(readIdentityRoute(url)).toEqual({ identityId: "id-1" });
    expect(readIdentityRoute(new URL("http://localhost/entities"))).toEqual({ identityId: null });
    expect(readIdentityRoute(new URL("http://localhost/entities/a/b"))).toBeNull();
    expect(readIdentityRoute(new URL("http://localhost/videos/a"))).toBeNull();
  });
});
