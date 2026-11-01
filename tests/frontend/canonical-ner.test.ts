import { describe, expect, test } from "bun:test";

import capabilities from "../../public/analysis-capabilities.json";
import {
  anchorEntities,
  modelDownloadBytes,
  nerSegments,
  NLP_STACK_REVISION,
  primaryLanguage,
  type CanonicalNerResult,
} from "../../src/app/features/analyzer/canonical-ner";

const segments = [
  { segmentId: "s0", startSeconds: 1, endSeconds: 3, text: "María visitó Madrid." },
  { segmentId: "s1", startSeconds: 4, endSeconds: 6, text: "   " },
  { segmentId: "s2", startSeconds: 7, endSeconds: 9, text: "Angela Merkel sprach in Berlin." },
];

describe("canonical NER adapter", () => {
  test("sends only non-empty segments with their ids", () => {
    expect(nerSegments(segments)).toEqual([
      { id: "s0", text: "María visitó Madrid." },
      { id: "s2", text: "Angela Merkel sprach in Berlin." },
    ]);
  });

  test("anchors entities to their segment text and time range", () => {
    const result = {
      status: "completed",
      language: "es",
      entities: [
        {
          segmentId: "s0",
          label: "LOC",
          text: "Madrid",
          score: 0.99,
          start: 13,
          end: 19,
          byteStart: 14,
          byteEnd: 20,
        },
        {
          segmentId: "missing",
          label: "PER",
          text: "X",
          score: 0.5,
          start: 0,
          end: 1,
          byteStart: 0,
          byteEnd: 1,
        },
      ],
    } as unknown as CanonicalNerResult;

    const [entity, ...rest] = anchorEntities(result, segments);
    expect(rest).toEqual([]);
    expect(entity?.segmentText.slice(entity.start, entity.end)).toBe("Madrid");
    expect(entity?.startSeconds).toBe(1);
    expect(entity?.endSeconds).toBe(3);
  });

  test("normalizes caption language codes to primary subtags", () => {
    expect(primaryLanguage("de-DE")).toBe("de");
    expect(primaryLanguage("es_419")).toBe("es");
    expect(primaryLanguage("")).toBeNull();
    expect(primaryLanguage(null)).toBeNull();
  });

  test("sums per-file model download progress", () => {
    const files = new Map([
      ["model.onnx", { loaded: 50, total: 100 }],
      ["tokenizer.json", { loaded: 10, total: 10 }],
    ]);
    expect(modelDownloadBytes(files)).toEqual({ loaded: 60, total: 110 });
  });

  test("capability manifest pins the same NLP Stack revision", () => {
    expect(capabilities.browser.canonicalAnalysis.ner.nlpStackRevision).toBe(NLP_STACK_REVISION);
    expect(capabilities.browser.canonicalAnalysis.ner.loadsOnPageLoad).toBe(false);
  });
});
