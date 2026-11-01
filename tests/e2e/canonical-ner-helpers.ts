import { readFileSync } from "node:fs";
import { join } from "node:path";

import { expect, type Page } from "@playwright/test";

// Shared Pages-journey helpers for the canonical NER specs (#79).

const FIXTURES = join(import.meta.dirname, "../fixtures/canonical-ner");
export const VIDEO_URL = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
export const MODEL_HOSTS = /^https:\/\/(cdn\.jsdelivr\.net|huggingface\.co|[^/]*\.hf\.co)\//;

export type FixtureLanguage = "en" | "de" | "es";

/** Entities each fixture must yield, with the timestamp of their caption cue. */
export const EXPECTED: Record<
  FixtureLanguage,
  Array<{ text: string; label: string; at: string }>
> = {
  en: [
    { text: "Julius Caesar", label: "PER", at: "0:05" },
    { text: "London", label: "LOC", at: "1:12" },
  ],
  de: [
    { text: "Angela Merkel", label: "PER", at: "0:06" },
    { text: "Hamburg", label: "LOC", at: "1:30" },
  ],
  es: [
    { text: "Miguel de Cervantes", label: "PER", at: "0:05" },
    { text: "Málaga", label: "LOC", at: "2:20" },
  ],
};

export function fixture(language: string) {
  return readFileSync(join(FIXTURES, `${language}.vtt`), "utf8");
}

export async function openAnalyzer(page: Page, transcript: string) {
  // Keep the YouTube preview iframe offline; the panel never needs it.
  await page.route(/^https:\/\/(www\.)?youtube(-nocookie)?\.com\//, (route) => route.abort());
  await page.goto("/?mode=browser");
  await page.getByLabel("YouTube URL").fill(VIDEO_URL);
  await page.getByLabel("Transcript evidence").fill(transcript);
  await page.getByRole("button", { name: "Analyze transcript" }).click();
  await expect(page.getByRole("heading", { name: "Canonical named entities" })).toBeVisible();
}

export async function runCanonicalNer(page: Page, language: string) {
  await page.getByLabel("Transcript language").selectOption(language);
  await page.getByRole("button", { name: /^(Run canonical NER|Run again)$/ }).click();
}

export function entityRow(page: Page, text: string) {
  return page.locator(".canonical-ner-list li").filter({
    has: page.locator("strong", { hasText: new RegExp(`^${text}$`) }),
  });
}

export async function expectEntities(page: Page, language: FixtureLanguage, timeout = 10_000) {
  for (const { text, label, at } of EXPECTED[language]) {
    const row = entityRow(page, text).first();
    await expect(row).toBeVisible({ timeout });
    await expect(row.locator(".canonical-ner-label")).toHaveText(new RegExp(label));
    await expect(row.locator("a.timestamp")).toHaveText(at);
    await expect(row.locator("mark")).toHaveText(text);
  }
}
