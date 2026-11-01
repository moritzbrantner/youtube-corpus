import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import { chromium, expect, test, type BrowserContext, type Page } from "@playwright/test";

import {
  entityRow,
  expectEntities,
  fixture,
  MODEL_HOSTS,
  openAnalyzer,
  runCanonicalNer,
} from "./canonical-ner-helpers";

// The real pinned NLP Stack module and model in the Pages journey (#79). Opt-in:
//
//   CANONICAL_NER_MODEL=1 bun run test:e2e tests/e2e/canonical-ner-model.spec.ts
//
// The tests share a persistent Chromium profile (CANONICAL_NER_PROFILE, default
// ~/.cache/youtube-corpus/canonical-ner-profile), so the ~135 MB model lands in that profile's
// Cache Storage on the first run only, exactly as it would in a returning visitor's browser.
// CANONICAL_NER_MEASURE=1 also runs the footprint measurement from a fresh profile (a real cold
// download) and writes test-results/canonical-ner-metrics.json.

const PROFILE =
  process.env.CANONICAL_NER_PROFILE ??
  join(homedir(), ".cache/youtube-corpus/canonical-ner-profile");
const MODEL_TIMEOUT = 300_000;
const HUGGING_FACE = /^https:\/\/(huggingface\.co|[^/]*\.hf\.co)\//;

test.describe.configure({ mode: "serial" });
test.skip(!process.env.CANONICAL_NER_MODEL, "set CANONICAL_NER_MODEL=1 to run the real model");
test.skip(({ isMobile }) => isMobile, "desktop project only");
test.setTimeout(900_000);

async function withProfile(
  baseURL: string | undefined,
  body: (page: Page, context: BrowserContext) => Promise<void>,
) {
  mkdirSync(PROFILE, { recursive: true });
  const context = await chromium.launchPersistentContext(PROFILE, { baseURL });
  try {
    await body(context.pages()[0] ?? (await context.newPage()), context);
  } finally {
    await context.close();
  }
}

async function expectCompleted(page: Page) {
  await expect(page.getByText(/entities in \d+ segments/)).toBeVisible({ timeout: MODEL_TIMEOUT });
}

test("en, de and es fixtures yield anchored entities with provenance", async ({ baseURL }) => {
  await withProfile(baseURL, async (page) => {
    for (const language of ["en", "de", "es"] as const) {
      await openAnalyzer(page, fixture(language));
      await runCanonicalNer(page, language);
      await expectCompleted(page);
      await expectEntities(page, language);
      await expect(
        page.getByText(
          /at nlp-stack e1063c5b5918 · model Xenova\/distilbert-base-multilingual-cased-ner-hrl@c2a4dbf593c5 \(q8, from Davlan\//,
        ),
      ).toBeVisible();
    }
  });
});

test("cancel while the runtime loads, then retry", async ({ baseURL }) => {
  await withProfile(baseURL, async (page, context) => {
    // Hold the Transformers.js module so the cancel provably lands mid-load.
    let release!: () => void;
    const held = new Promise<void>((resolve) => (release = resolve));
    await context.route(/@huggingface\/transformers@/, async (route) => {
      await held;
      await route.continue().catch(() => {});
    });
    await openAnalyzer(page, fixture("en"));
    await runCanonicalNer(page, "en");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByText("Canceled. No partial result is shown.")).toBeVisible();
    release();
    await context.unrouteAll();

    await page.getByRole("button", { name: "Run again" }).click();
    await expectCompleted(page);
    await expectEntities(page, "en");
  });
});

test("a cold offline model load fails cleanly and a retry fails the same way", async ({
  browser,
  baseURL,
}) => {
  const cold = await browser.newContext({ baseURL });
  try {
    await cold.route(HUGGING_FACE, (route) => route.abort("internetdisconnected"));
    const page = await cold.newPage();
    await openAnalyzer(page, fixture("es"));
    await runCanonicalNer(page, "es");
    await expect(page.getByRole("alert")).toContainText("Canonical NER is unavailable", {
      timeout: MODEL_TIMEOUT,
    });
    await expect(page.locator(".canonical-ner-list")).toHaveCount(0);
    await runCanonicalNer(page, "es");
    await expect(page.getByRole("alert")).toContainText("Canonical NER is unavailable", {
      timeout: MODEL_TIMEOUT,
    });
  } finally {
    await cold.close();
  }
});

// Transformers.js still probes huggingface.co (on the moving `main` revision) when the model is
// cached, and fails offline: https://github.com/moritzbrantner/nlp-stack/issues/179
test.fixme("a cached model works offline", async ({ baseURL }) => {
  await withProfile(baseURL, async (page, context) => {
    await context.route(HUGGING_FACE, (route) => route.abort("internetdisconnected"));
    await openAnalyzer(page, fixture("es"));
    await runCanonicalNer(page, "es");
    await expectCompleted(page);
    await expectEntities(page, "es");
  });
});

test("changing the transcript mid-run never shows stale entities", async ({ baseURL }) => {
  await withProfile(baseURL, async (page) => {
    await openAnalyzer(page, fixture("en"));
    await runCanonicalNer(page, "en");
    await page.getByLabel("Transcript evidence").fill(fixture("de"));
    await page.getByRole("button", { name: "Analyze transcript" }).click();
    // Give the abandoned English run time to finish if it were still alive.
    await page.waitForTimeout(10_000);
    await expect(page.locator(".canonical-ner-list")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Run canonical NER" })).toBeVisible();

    await runCanonicalNer(page, "de");
    await expectCompleted(page);
    await expectEntities(page, "de");
    await expect(entityRow(page, "Julius Caesar")).toHaveCount(0);
  });
});

// --- Footprint measurement ------------------------------------------------------------------

/** Summed resident set size of a process and all its descendants (Linux /proc). */
function treeRssBytes(rootPid: number) {
  const children = new Map<number, number[]>();
  const rss = new Map<number, number>();
  for (const entry of readdirSync("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      const stat = readFileSync(`/proc/${entry}/stat`, "utf8");
      const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      const pid = Number(entry);
      const ppid = Number(fields[1]);
      children.set(ppid, [...(children.get(ppid) ?? []), pid]);
      rss.set(pid, Number(fields[21]) * 4096);
    } catch {
      // Process exited while scanning.
    }
  }
  let total = 0;
  const stack = [rootPid];
  while (stack.length) {
    const pid = stack.pop()!;
    total += rss.get(pid) ?? 0;
    stack.push(...(children.get(pid) ?? []));
  }
  return total;
}

function longTranscript(cues: number) {
  const texts = ["en", "de", "es"].flatMap((language) =>
    fixture(language)
      .split("\n")
      .filter((line) => line && !line.startsWith("WEBVTT") && !line.includes("-->")),
  );
  const stamp = (seconds: number) =>
    `${String(Math.floor(seconds / 3600)).padStart(2, "0")}:${String(Math.floor((seconds % 3600) / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}.000`;
  const body = Array.from(
    { length: cues },
    (_, index) => `${stamp(index * 5)} --> ${stamp(index * 5 + 4)}\n${texts[index % texts.length]}`,
  );
  return `WEBVTT\n\n${body.join("\n\n")}\n`;
}

test("measure cold download, startup, memory and analysis time", async ({ baseURL }, testInfo) => {
  test.skip(!process.env.CANONICAL_NER_MEASURE, "set CANONICAL_NER_MEASURE=1 to measure");
  const profile = mkdtempSync(join(homedir(), ".cache/youtube-corpus/canonical-ner-measure-"));
  const context = await chromium.launchPersistentContext(profile, { baseURL });
  const browserPid = Number(
    readdirSync("/proc").find((entry) => {
      try {
        return (
          /^\d+$/.test(entry) &&
          readFileSync(`/proc/${entry}/cmdline`, "utf8").includes(`--user-data-dir=${profile}`) &&
          !readFileSync(`/proc/${entry}/cmdline`, "utf8").includes("--type=")
        );
      } catch {
        return false;
      }
    }),
  );
  let peak = 0;
  const sampler = setInterval(() => (peak = Math.max(peak, treeRssBytes(browserPid))), 50);
  const downloads = new Map<string, number>();
  context.on("requestfinished", async (request) => {
    if (!MODEL_HOSTS.test(request.url())) return;
    const sizes = await request.sizes().catch(() => null);
    // Signed CDN query strings are dropped; the body size is what crossed the network.
    if (sizes) downloads.set(request.url().split("?")[0], sizes.responseBodySize);
  });

  const page = context.pages()[0] ?? (await context.newPage());
  async function timedRun(language: string) {
    const started = Date.now();
    await runCanonicalNer(page, language);
    await expectCompleted(page);
    return Date.now() - started;
  }
  async function phase<T>(run: () => Promise<T>) {
    peak = treeRssBytes(browserPid);
    const before = peak;
    const value = await run();
    return { value, baselineMb: before / 1e6, peakMb: peak / 1e6 };
  }

  try {
    const transcript = longTranscript(240);
    const lexical = await phase(async () => {
      await openAnalyzer(page, fixture("en"));
      return page.evaluate(async (text) => {
        const { buildBrowserVideoAnalysis } =
          await import("/src/app/features/analyzer/browser-analysis.ts");
        const parsed = { videoId: "dQw4w9WgXcQ" } as never;
        buildBrowserVideoAnalysis(parsed, text);
        const started = performance.now();
        const report = buildBrowserVideoAnalysis(parsed, text);
        return {
          ms: performance.now() - started,
          ruleEntities: (report.lexicalAnalysis as { ruleEntities: { text: string }[] })
            .ruleEntities.length,
        };
      }, transcript);
    });
    const cold = await phase(() => timedRun("en"));
    const coldBytes = [...downloads.values()].reduce((sum, bytes) => sum + bytes, 0);
    const warm = await phase(() => timedRun("en"));
    // A different transcript in the same page reuses the loaded worker.
    await page.getByLabel("Transcript evidence").fill(fixture("es"));
    await page.getByRole("button", { name: "Analyze transcript" }).click();
    const nextTranscript = await phase(() => timedRun("es"));
    // Reload: the model comes from Cache Storage, so this is worker + runtime startup only.
    const cached = await phase(async () => {
      await openAnalyzer(page, fixture("de"));
      return timedRun("de");
    });
    // What each producer finds on the fixtures: canonical "LABEL text" vs lexical guesses.
    const comparison: Record<string, { canonical: string[]; lexical: string[] }> = {};
    for (const language of ["en", "de", "es"]) {
      await openAnalyzer(page, fixture(language));
      await timedRun(language);
      const canonical = await page
        .locator(".canonical-ner-entity")
        .evaluateAll((items) =>
          items.map(
            (item) =>
              `${item.querySelector(".canonical-ner-label")?.textContent} ${item.querySelector("strong")?.textContent}`,
          ),
        );
      const lexicalGuesses = await page.evaluate(async (text) => {
        const { buildBrowserVideoAnalysis } =
          await import("/src/app/features/analyzer/browser-analysis.ts");
        const report = buildBrowserVideoAnalysis({ videoId: "dQw4w9WgXcQ" } as never, text);
        return (report.lexicalAnalysis as { ruleEntities: { text: string }[] }).ruleEntities.map(
          (entity) => entity.text,
        );
      }, fixture(language));
      comparison[language] = { canonical, lexical: lexicalGuesses };
    }
    const long = await phase(async () => {
      await openAnalyzer(page, transcript);
      const firstMs = await timedRun("en");
      const rerunMs = await timedRun("en");
      const summary = await page.getByText(/entities in \d+ segments/).innerText();
      return { firstMs, rerunMs, entities: Number(summary.match(/^(\d+)/)?.[1]) };
    });

    const metrics = {
      machine: `${process.platform} ${process.arch}, Playwright Chromium (headless, Wasm CPU backend)`,
      coldDownload: {
        bytes: coldBytes,
        files: Object.fromEntries([...downloads].filter(([, bytes]) => bytes > 100_000)),
      },
      coldFirstRunMs: cold.value,
      warmRerunMs: warm.value,
      nextTranscriptRunMs: nextTranscript.value,
      cachedReloadFirstRunMs: cached.value,
      longTranscript: { cues: 240, ...long.value },
      comparison,
      lexicalBaseline: {
        cues: 240,
        ms: lexical.value.ms,
        ruleEntities: lexical.value.ruleEntities,
      },
      memoryMb: {
        baselineBeforeOptIn: lexical.peakMb,
        peakColdRun: cold.peakMb,
        peakLongTranscript: long.peakMb,
      },
    };
    console.log(JSON.stringify(metrics, null, 2));
    writeFileSync(
      testInfo.outputPath("canonical-ner-metrics.json"),
      JSON.stringify(metrics, null, 2),
    );
    mkdirSync("test-results", { recursive: true });
    writeFileSync("test-results/canonical-ner-metrics.json", JSON.stringify(metrics, null, 2));
  } finally {
    clearInterval(sampler);
    await context.close();
    rmSync(profile, { recursive: true, force: true });
  }
});
