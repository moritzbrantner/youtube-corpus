import { expect, test, type Page } from "@playwright/test";

import { BROWSER_NER_MODULE_BASE_URL } from "../../src/app/features/analyzer/canonical-ner";
import {
  entityRow,
  EXPECTED,
  expectEntities,
  fixture,
  MODEL_HOSTS,
  openAnalyzer,
  runCanonicalNer,
} from "./canonical-ner-helpers";

// Pages journey for the opt-in canonical NER panel (#79) against a stub producer. NLP Stack's
// `browser-ner.js` is intercepted, so this covers opt-in, run, timestamps, provenance, cancel, a
// failed load and stale results without downloading anything. The real pinned module and model
// run in canonical-ner-model.spec.ts (opt-in).

// --- Stubbed producer -----------------------------------------------------------------------

const STUB_MODULE = `
const stub = (globalThis.__nerStub ??= { hold: false, runs: [], disposed: 0 });
const KNOWN = ${JSON.stringify(
  Object.values(EXPECTED)
    .flat()
    .map(({ text, label }) => [text, label]),
)};
const encoder = new TextEncoder();
export function createBrowserNer() {
  return {
    async analyze({ language, segments }, { signal, onProgress } = {}) {
      stub.runs.push(segments.map((segment) => segment.text));
      onProgress?.({ stage: "model", detail: { file: "onnx/model_quantized.onnx", loaded: 67500000, total: 135000000 } });
      if (stub.hold) {
        await new Promise((resolve, reject) => {
          stub.release = resolve;
          const abort = () => reject(new DOMException("Browser NER request was canceled.", "AbortError"));
          if (signal?.aborted) abort();
          signal?.addEventListener("abort", abort, { once: true });
        });
      }
      const entities = [];
      for (const segment of segments) {
        for (const [text, label] of KNOWN) {
          const start = segment.text.indexOf(text);
          if (start < 0) continue;
          const byteStart = encoder.encode(segment.text.slice(0, start)).length;
          entities.push({ segmentId: segment.id, label, text, score: 0.97, start, end: start + text.length,
            byteStart, byteEnd: byteStart + encoder.encode(text).length });
        }
      }
      return {
        status: "completed",
        language,
        producer: { package: "@moritzbrantner/text-linguistics-wasm", module: "browser-ner", contractVersion: 1 },
        model: { id: "stub/ner", revision: "0123456789abcdef", baseModel: "stub", dtype: "q8",
          labels: ["PER", "ORG", "LOC"], supportedLanguages: ["en", "de", "es"] },
        entities,
        segments: segments.map((segment) => ({ id: segment.id,
          entityCount: entities.filter((entity) => entity.segmentId === segment.id).length, truncated: false })),
      };
    },
    dispose() { stub.disposed += 1; },
  };
}
`;

async function stubProducer(page: Page) {
  await page.route(`${BROWSER_NER_MODULE_BASE_URL}browser-ner.js`, (route) =>
    route.fulfill({
      contentType: "text/javascript",
      headers: { "access-control-allow-origin": "*" },
      body: STUB_MODULE,
    }),
  );
}

test.describe("canonical NER panel (stubbed producer)", () => {
  test("a cold Pages load and lexical analysis make no model requests", async ({ page }) => {
    const modelRequests: string[] = [];
    page.on("request", (request) => {
      if (MODEL_HOSTS.test(request.url())) modelRequests.push(request.url());
    });
    await openAnalyzer(page, fixture("en"));
    await expect(page.getByRole("button", { name: "Run canonical NER" })).toBeVisible();
    expect(modelRequests).toEqual([]);
  });

  for (const language of ["en", "de", "es"] as const) {
    test(`opt in and run on the ${language} fixture`, async ({ page }) => {
      await stubProducer(page);
      await openAnalyzer(page, fixture(language));
      await runCanonicalNer(page, language);
      await expect(page.getByText(/entities in 4 segments/)).toBeVisible();
      await expectEntities(page, language);
      const timestamp = entityRow(page, EXPECTED[language][0].text).locator("a.timestamp");
      await expect(timestamp).toHaveAttribute("href", /&t=\d+s$/);
      await expect(
        page.getByText(
          /^Producer @moritzbrantner\/text-linguistics-wasm\/browser-ner \(contract v1\) at nlp-stack [0-9a-f]{12} · model stub\/ner@0123456789ab/,
        ),
      ).toBeVisible();
    });
  }

  test("cancel drops the run and a retry completes", async ({ page }) => {
    await stubProducer(page);
    await openAnalyzer(page, fixture("en"));
    await page.evaluate(
      () => ((globalThis as any).__nerStub = { hold: true, runs: [], disposed: 0 }),
    );
    await runCanonicalNer(page, "en");
    await expect(page.getByText("Loading model… 67.5 MB of 135.0 MB")).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByText("Canceled. No partial result is shown.")).toBeVisible();
    await expect(page.locator(".canonical-ner-list")).toHaveCount(0);

    await page.evaluate(() => ((globalThis as any).__nerStub.hold = false));
    await page.getByRole("button", { name: "Run again" }).click();
    await expectEntities(page, "en");
  });

  test("a failed (offline) module load reports an error and can be retried", async ({ page }) => {
    await page.route(`${BROWSER_NER_MODULE_BASE_URL}**`, (route) =>
      route.abort("internetdisconnected"),
    );
    await openAnalyzer(page, fixture("de"));
    await runCanonicalNer(page, "de");
    await expect(page.getByRole("alert")).toContainText("Canonical NER is unavailable");

    await page.unrouteAll();
    await stubProducer(page);
    await page.getByRole("button", { name: "Run again" }).click();
    await expectEntities(page, "de");
  });

  test("changing the transcript drops a pending run and its results", async ({ page }) => {
    await stubProducer(page);
    await openAnalyzer(page, fixture("en"));
    await page.evaluate(
      () => ((globalThis as any).__nerStub = { hold: true, runs: [], disposed: 0 }),
    );
    await runCanonicalNer(page, "en");
    await expect(page.getByRole("button", { name: "Cancel" })).toBeVisible();

    await page.getByLabel("Transcript evidence").fill(fixture("es"));
    await expect(page.getByRole("heading", { name: "Canonical named entities" })).toHaveCount(0);
    await page.evaluate(() => (globalThis as any).__nerStub.release?.());
    await page.getByRole("button", { name: "Analyze transcript" }).click();
    await expect(page.getByRole("button", { name: "Run canonical NER" })).toBeVisible();
    await expect(page.locator(".canonical-ner-list")).toHaveCount(0);

    await page.evaluate(() => ((globalThis as any).__nerStub.hold = false));
    await runCanonicalNer(page, "es");
    await expectEntities(page, "es");
    await expect(entityRow(page, "Julius Caesar")).toHaveCount(0);
  });

  test("re-analyzing the same transcript resets a finished panel", async ({ page }) => {
    await stubProducer(page);
    await openAnalyzer(page, fixture("en"));
    await runCanonicalNer(page, "en");
    await expectEntities(page, "en");
    await page.getByRole("button", { name: "Analyze transcript" }).click();
    await expect(page.getByRole("button", { name: "Run canonical NER" })).toBeVisible();
    await expect(page.locator(".canonical-ner-list")).toHaveCount(0);
  });
});
