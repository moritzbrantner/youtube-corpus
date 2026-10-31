import { expect, test, type Page } from "@playwright/test";

const IDENTITY = "11111111-1111-4111-8111-111111111111";
const corpora = [
  { id: "c-history", name: "History", isDefault: true },
  { id: "c-science", name: "Science", isDefault: false },
].map((corpus) => ({
  ...corpus,
  slug: corpus.id,
  description: null,
  videoCount: 3,
  sourceCount: 1,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
}));

function mention(id: string, modality: "spoken" | "visible_text", start: number) {
  return {
    id,
    modality,
    surfaceText: "Ada Lovelace",
    startSeconds: start,
    endSeconds: start + 4,
    context: "Ada Lovelace",
    byteStart: 0,
    byteEnd: 12,
    confidence: 0.9,
    transcriptSegmentId: modality === "spoken" ? `seg-${id}` : null,
    visualTextTrackId: modality === "visible_text" ? `ocr-${id}` : null,
    visualTextRole: null,
    runId: "run",
    producer: "nlp-stack",
    producerVersion: "1",
    model: "ner",
    configHash: "h",
  };
}

function mentionVideo(
  videoId: string,
  date: string | null,
  occurrences: ReturnType<typeof mention>[],
) {
  return {
    videoId,
    title: `Video ${videoId}`,
    sourceUrl: `https://www.youtube.com/watch?v=${videoId}`,
    publicationDate: date,
    spokenMentions: occurrences.filter((o) => o.modality === "spoken").length,
    visibleTextMentions: occurrences.filter((o) => o.modality === "visible_text").length,
    occurrences,
  };
}

const track = (kind: "shown" | "heard", trackId: string, start: number) => ({
  kind,
  trackId,
  clusterId: "cluster",
  startSeconds: start,
  endSeconds: start + 10,
  linkConfidence: 0.8,
  linkSource: "face_match",
  clusterSimilarity: 0.7,
  processor: "visual-analysis",
  processorVersion: "1",
  model: "arcface",
  modelVersion: "1",
});

const evidence = [
  {
    unitKey: "mentions:c-history",
    corpusId: "c-history",
    mentions: {
      entityType: "person",
      entityKey: "ada lovelace",
      videos: [
        mentionVideo("v2020", "2020-03-01", [mention("m1", "spoken", 12)]),
        mentionVideo("v2022a", "2022-06-15", [
          mention("m2", "spoken", 30),
          mention("m3", "visible_text", 40),
        ]),
        mentionVideo("v2022b", "2022-06-16", [mention("m4", "spoken", 5)]),
        mentionVideo("vundated", null, [mention("m5", "spoken", 7)]),
      ],
    },
    tracks: null,
  },
  {
    unitKey: "person:c-science",
    corpusId: "c-science",
    mentions: null,
    tracks: {
      person: { id: "p1", label: "Person 1", name: "Ada Lovelace", nameStatus: "confirmed" },
      videos: [
        {
          videoId: "v2024",
          title: "Video v2024",
          sourceUrl: "https://www.youtube.com/watch?v=v2024",
          publicationDate: "2024-01-10",
          shownTracks: 1,
          heardTracks: 1,
          occurrences: [track("shown", "t1", 60), track("heard", "t2", 62)],
        },
      ],
    },
  },
];

const units = [
  {
    unitKey: "mentions:c-history",
    unitKind: "entity_group",
    corpusId: "c-history",
    corpusName: "History",
  },
  {
    unitKey: "person:c-science",
    unitKind: "inferred_person",
    corpusId: "c-science",
    corpusName: "Science",
  },
].map((unit) => ({
  ...unit,
  label: "Ada Lovelace",
  entityType: "person",
  entityKey: null,
  personId: null,
  nameStatus: null,
  status: "auto",
  basis: "shared_evidence",
  inScope: true,
}));

function profile(corpusId: string | null) {
  return {
    requestedId: IDENTITY,
    identity: {
      id: IDENTITY,
      kind: "person",
      entityType: "person",
      displayName: "Ada Lovelace",
      nameSource: "mentions",
      corpora: corpora.map(({ id, name }) => ({ id, name })),
      localUnits: 2,
      hasConfirmedLinks: false,
      spokenMentions: 4,
      visibleTextMentions: 1,
      shownTracks: 1,
      heardTracks: 1,
      videoCount: 5,
    },
    units,
    evidence: evidence.filter((unit) => !corpusId || unit.corpusId === corpusId),
    suggestions: [],
    keptApart: [],
  };
}

async function mockApi(page: Page) {
  await page.route("**/api/**", (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/corpora") return route.fulfill({ json: corpora });
    if (url.pathname === `/api/identities/${IDENTITY}`) {
      return route.fulfill({ json: profile(url.searchParams.get("corpusId")) });
    }
    return route.fulfill({
      status: 404,
      json: { error: { message: `not mocked: ${url.pathname}` } },
    });
  });
}

const profileUrl = (query: string) => `/people/${IDENTITY}?corpus=c-history&${query}`;

async function centers(page: Page) {
  const markers = page.getByTestId("timeline-marker");
  const result: Record<string, { x: number; y: number }> = {};
  for (const marker of await markers.all()) {
    const box = (await marker.boundingBox())!;
    result[(await marker.getAttribute("data-video-id"))!] = {
      x: box.x + box.width / 2,
      y: box.y + box.height / 2,
    };
  }
  return result;
}

test.beforeEach(async ({ page }) => mockApi(page));

test("timeline places dated videos by publication date and never plots unknown dates", async ({
  page,
}, testInfo) => {
  await page.goto(profileUrl("scope=all&view=timeline"));
  await expect(page.getByTestId("timeline-marker")).toHaveCount(4);
  await expect(page.getByTestId("evidence-count")).toHaveText(
    "7 occurrences in 5 videos; 1 without a known publication date.",
  );

  const at = await centers(page);
  expect(Object.keys(at).sort()).toEqual(["v2020", "v2022a", "v2022b", "v2024"]);
  expect(at.v2020.x).toBeLessThan(at.v2022a.x);
  expect(at.v2022b.x).toBeLessThan(at.v2024.x);
  // Linear in time: 2020-03-01 -> 2022-06-15 spans about as much as 2022-06-15 -> 2024-01-10.
  const left = at.v2022a.x - at.v2020.x;
  const right = at.v2024.x - at.v2022a.x;
  expect(left / right).toBeCloseTo(836 / 574, 1);
  // One day apart: stacked into separate lanes instead of overlapping.
  expect(Math.abs(at.v2022b.x - at.v2022a.x)).toBeLessThan(5);
  expect(Math.abs(at.v2022b.y - at.v2022a.y)).toBeGreaterThan(15);
  const track = (await page.getByTestId("timeline-track").boundingBox())!;
  for (const point of Object.values(at)) {
    expect(point.x).toBeGreaterThan(track.x);
    expect(point.x).toBeLessThan(track.x + track.width);
  }

  const table = page.getByRole("table", { name: "Publication timeline table" });
  await expect(table.getByRole("row")).toHaveCount(6);
  await expect(table.getByRole("row").last()).toContainText("Unknown");
  await expect(page.locator('[data-video-id="v2024"]')).toHaveAttribute(
    "aria-label",
    /inferred identity/,
  );

  await page.locator('[data-video-id="v2022a"]').click();
  await expect(page.getByTestId("timeline-selection")).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("publication-timeline.png"), fullPage: true });
});

test("selecting a timeline event opens its exact source moment", async ({ page }) => {
  await page.goto(profileUrl("scope=all&view=timeline"));
  await page.locator('[data-video-id="v2020"]').click();
  await expect(page.locator('[data-video-id="v2020"]')).toHaveAttribute("aria-pressed", "true");
  const selection = page.getByTestId("timeline-selection");
  await expect(selection).toContainText("Video v2020");
  await expect(selection).toContainText("Published 2020-03-01");
  const open = selection.getByRole("link", { name: "Open moment 0:12–0:16" });
  await expect(open).toHaveAttribute("href", /\/videos\/v2020\/moments\/spoken\/seg-m1\?/);

  // Inferred tracks have no in-app moment and say so; they open the source at the timestamp.
  await page
    .getByRole("row", { name: /Video v2024/ })
    .getByRole("button", { name: "Select" })
    .click();
  await expect(selection).toContainText("Inferred identity");
  await expect(selection.getByRole("link", { name: "Source 1:00–1:10" })).toHaveAttribute(
    "href",
    "https://www.youtube.com/watch?v=v2024&t=60",
  );

  await selection.getByRole("link", { name: "Video v2024" }).click();
  await expect(page).toHaveURL(/\/videos\/v2024\?.*corpus=c-science/);
});

test("list and timeline share corpus, evidence type and zoom range in the URL", async ({
  page,
}) => {
  await page.goto(profileUrl("scope=all&view=timeline"));
  await page.getByLabel("Evidence type", { exact: true }).selectOption("shown");
  await expect(page).toHaveURL(/evidence=shown/);
  await expect(page.getByTestId("timeline-marker")).toHaveCount(1);
  await expect(page.getByTestId("evidence-count")).toHaveText("1 occurrence in 1 video.");

  await page.getByRole("button", { name: "Evidence list" }).click();
  await expect(page).not.toHaveURL(/view=timeline/);
  await expect(page.getByTestId("evidence-count")).toHaveText("1 occurrence in 1 video.");
  await expect(page.getByRole("link", { name: "Video v2024" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Video v2020" })).toHaveCount(0);

  await page.getByLabel("Evidence type", { exact: true }).selectOption("");
  await page.getByLabel("Corpus", { exact: true }).selectOption("c-history");
  await expect(page).not.toHaveURL(/scope=all/);
  await expect(page.getByTestId("evidence-count")).toHaveText(
    "5 occurrences in 4 videos; 1 without a known publication date.",
  );

  await page.getByRole("button", { name: "Publication timeline" }).click();
  await page.locator('[data-video-id="v2022a"]').click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page).toHaveURL(/since=\d{4}-\d{2}-\d{2}&until=\d{4}-\d{2}-\d{2}/);
  // Zoomed around the selected video: the 2020 video leaves the range, unknown dates stay out.
  await expect(page.getByTestId("timeline-marker")).toHaveCount(2);
  await expect(page.getByTestId("evidence-count")).toContainText("3 occurrences in 2 videos.");
  await page.getByRole("button", { name: "Evidence list" }).click();
  await expect(page.getByRole("link", { name: "Video v2020" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Video v2022a" })).toBeVisible();

  await page.getByRole("button", { name: "Publication timeline" }).click();
  await page.getByRole("button", { name: "Full range" }).click();
  await expect(page.getByTestId("timeline-marker")).toHaveCount(3);
});
