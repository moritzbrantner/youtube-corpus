import { describe, expect, test } from "bun:test";

import {
  buildBrowserVideoAnalysis,
  parseTranscript,
} from "../../src/app/features/analyzer/browser-analysis";
import { parseYouTubeVideoUrl } from "../../src/app/features/analyzer/youtube-url";

describe("browser transcript analysis", () => {
  test("parses timed SRT and WebVTT-style cues", () => {
    const segments = parseTranscript(`WEBVTT

00:00:01.000 --> 00:00:03.500
Hello world.

2
00:00:04,000 --> 00:00:06,250
Second cue.
`);

    expect(segments).toEqual([
      { startSeconds: 1, endSeconds: 3.5, text: "Hello world." },
      { startSeconds: 4, endSeconds: 6.25, text: "Second cue." },
    ]);
  });

  test("builds a deterministic local report without backend state", () => {
    const parsed = parseYouTubeVideoUrl("https://youtu.be/jNQXAC9IVRw");
    if (!parsed) throw new Error("expected valid YouTube URL");

    const report = buildBrowserVideoAnalysis(
      parsed,
      "Reliable tooling improves reliable systems. Reliable systems reduce failure risk.",
    );

    expect(report.video.youtubeId).toBe("jNQXAC9IVRw");
    expect(report.video.transcriptSegments).toBe(2);
    expect(report.coverage.transcript).toBe(true);
    expect(report.coverage.lexical).toBe(true);
    expect(report.coverage.metadata).toBe(false);
    expect(report.video.metadata).toMatchObject({
      browserAnalysis: {
        engine: "youtube-corpus-browser-lexical",
        persistence: "none",
      },
    });
    expect(report.lexicalAnalysis).toMatchObject({
      engine: "youtube-corpus-browser-lexical",
      summary: {
        stats: { words: 10, sentences: 2 },
      },
    });
  });
});
