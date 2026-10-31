import { expect, test, type Page } from "@playwright/test";

// Route-mocked API responses in the shape the Rust backend returns (tests/event_extraction_db.rs
// covers the backend itself). Not production analysis.
const IDENTITY = "22222222-2222-4222-8222-222222222222";
const PRODUCER = "youtube-corpus/historical-date-mentions";
const corpora = [{ id: "c-history", name: "History", isDefault: true }].map((corpus) => ({
  ...corpus,
  slug: corpus.id,
  description: null,
  videoCount: 3,
  sourceCount: 1,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
}));

type Claim = Record<string, unknown> & { id: string; reviewStatus: string };

function dateClaim(
  id: string,
  video: string,
  modality: "spoken" | "visible",
  evidenceText: string,
  date: Record<string, unknown> | null,
  options: Partial<Claim> & { sourceText?: string; publicationDate?: string } = {},
): Claim {
  const { sourceText, publicationDate, ...rest } = options;
  const byteStart = sourceText
    ? new TextEncoder().encode(evidenceText.split(sourceText)[0]).length
    : 0;
  return {
    id,
    videoId: video,
    videoTitle: `Lecture ${video}`,
    modality,
    evidenceId: `${modality === "spoken" ? "seg" : "ocr"}-${id}`,
    evidenceText,
    startSeconds: 72,
    endSeconds: 78,
    predicate: "date",
    date: date && {
      startMonth: null,
      startDay: null,
      endYear: null,
      endMonth: null,
      endDay: null,
      approximate: false,
      ...date,
    },
    identityId: null,
    identity: null,
    polarity: "affirms",
    certainty: "stated",
    qualification: "",
    reviewStatus: "unverified",
    preferred: false,
    sourceUrl: `https://www.youtube.com/watch?v=${video}`,
    publicationDate: publicationDate ?? "2024-03-01",
    extraction: sourceText
      ? {
          runId: "run-1",
          producer: PRODUCER,
          producerVersion: "1",
          sourceText,
          sourceByteStart: byteStart,
          sourceByteEnd: byteStart + new TextEncoder().encode(sourceText).length,
          confidence: 0.8,
          extractedAt: "2026-10-31T10:00:00Z",
        }
      : null,
    ...rest,
  };
}

function summary(id: string, label: string, claims: Claim[]) {
  const live = claims.filter((c) => c.reviewStatus !== "rejected");
  const years = live.map((c) => (c.date as { startYear: number }).startYear);
  return {
    id,
    label,
    claimCount: claims.length,
    videoCount: new Set(claims.map((c) => c.videoId)).size,
    dateReadings: new Set(live.map((c) => JSON.stringify(c.date))).size,
    disputed: new Set(live.map((c) => JSON.stringify(c.date))).size > 1,
    earliestYear: Math.min(...years),
    latestYear: Math.max(...years),
    preferredDate: null,
    corpora: [{ id: "c-history", name: "History" }],
  };
}

function fixtures() {
  const hastings = [
    dateClaim(
      "h1",
      "vA",
      "spoken",
      "William won at Hastings on 14 October 1066.",
      {
        startYear: 1066,
        startMonth: 10,
        startDay: 14,
      },
      { reviewStatus: "confirmed", publicationDate: "2021-05-02" },
    ),
    dateClaim(
      "h2",
      "vB",
      "spoken",
      "some chroniclers say the battle of hastings probably happened around 1067",
      { startYear: 1067, approximate: true },
      { sourceText: "1067", certainty: "hedged", publicationDate: "2024-03-01" },
    ),
    dateClaim(
      "h3",
      "vB",
      "visible",
      "The Battle of Hastings, 1070",
      { startYear: 1070 },
      {
        sourceText: "1070",
        reviewStatus: "rejected",
      },
    ),
  ];
  const nicaea = [
    dateClaim(
      "n1",
      "vC",
      "visible",
      "The Council of Nicaea, 325 AD",
      { startYear: 325 },
      { sourceText: "325 AD" },
    ),
  ];
  return {
    events: [
      { event: summary("e-nicaea", "Council of Nicaea", nicaea), claims: nicaea },
      { event: summary("e-hastings", "Battle of Hastings", hastings), claims: hastings },
    ],
  };
}

function identityProfile() {
  return {
    requestedId: IDENTITY,
    identity: {
      id: IDENTITY,
      kind: "person",
      entityType: "person",
      displayName: "William",
      nameSource: "mentions",
      corpora: [{ id: "c-history", name: "History" }],
      localUnits: 1,
      hasConfirmedLinks: false,
      spokenMentions: 1,
      visibleTextMentions: 0,
      shownTracks: 0,
      heardTracks: 0,
      videoCount: 1,
    },
    units: [],
    evidence: [],
    suggestions: [],
    keptApart: [],
  };
}

async function mockApi(page: Page) {
  const state = fixtures();
  const posted: unknown[] = [];
  await page.route("**/api/**", (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    if (path === "/api/corpora") return route.fulfill({ json: corpora });
    if (path === "/api/events/timeline") {
      const eventId = url.searchParams.get("eventId");
      const identityId = url.searchParams.get("identityId");
      const events = eventId
        ? state.events.filter((entry) => entry.event.id === eventId)
        : identityId
          ? state.events.filter((entry) => entry.event.id === "e-hastings")
          : state.events;
      return route.fulfill({ json: { events, truncated: false } });
    }
    if (path === "/api/events") {
      return route.fulfill({ json: state.events.map((entry) => entry.event) });
    }
    if (path === "/api/corpora/c-history/events/extract") {
      return route.fulfill({
        json: {
          producer: PRODUCER,
          producerVersion: "1",
          videosProcessed: 3,
          videosWithoutText: 0,
          segmentsAnalyzed: 412,
          visualTextTracksAnalyzed: 18,
          claimsSuggested: 3,
          claimsKept: 0,
          reviewedSkipped: 1,
          staleRemoved: 0,
          lowConfidenceDropped: 5,
        },
      });
    }
    const correction = path.match(/^\/api\/events\/([^/]+)\/corrections$/);
    if (correction && request.method() === "POST") {
      const body = request.postDataJSON() as { claimIds: string[]; reviewStatus: string };
      posted.push(body);
      for (const entry of state.events) {
        for (const claim of entry.claims) {
          if (body.claimIds.includes(claim.id)) claim.reviewStatus = body.reviewStatus;
        }
      }
      return route.fulfill({ json: { eventId: correction[1], createdEventId: null } });
    }
    const profile = path.match(/^\/api\/events\/([^/]+)$/);
    if (profile) {
      const entry = state.events.find((e) => e.event.id === profile[1])!;
      return route.fulfill({
        json: {
          requestedId: entry.event.id,
          event: summary(entry.event.id, entry.event.label, entry.claims),
          claims: entry.claims,
          claimsOutsideScope: 0,
        },
      });
    }
    if (path === `/api/identities/${IDENTITY}`) return route.fulfill({ json: identityProfile() });
    return route.fulfill({ status: 404, json: { error: { message: `not mocked: ${path}` } } });
  });
  return posted;
}

async function barCenter(page: Page, claimId: string) {
  const box = (await page.locator(`[data-claim-id="${claimId}"]`).boundingBox())!;
  return { x: box.x + box.width / 2, y: box.y + box.height / 2, width: box.width };
}

test("historical timeline separates readings by event and keeps suggestions unverified", async ({
  page,
}, testInfo) => {
  await mockApi(page);
  await page.goto("/events?corpus=c-history&view=historical");
  const bars = page.getByTestId("historical-bar");
  // The rejected OCR reading is hidden until asked for.
  await expect(bars).toHaveCount(3);
  await expect(page.getByTestId("historical-row")).toHaveCount(2);
  await expect(page.getByTestId("historical-count")).toContainText(
    "4 date claims on 2 events; 2 unverified",
  );

  const nicaea = await barCenter(page, "n1");
  const confirmed = await barCenter(page, "h1");
  const suggested = await barCenter(page, "h2");
  expect(nicaea.x).toBeLessThan(confirmed.x);
  expect(confirmed.x).toBeLessThan(suggested.x);
  // Disagreeing sources sit on the same event row.
  expect(Math.abs(confirmed.y - suggested.y)).toBeLessThan(20);
  await expect(page.locator('[data-claim-id="h2"]')).toHaveAttribute("data-review", "unverified");
  await expect(page.locator('[data-claim-id="h2"]')).toHaveAttribute(
    "aria-label",
    /c\. 1067: Battle of Hastings; Unverified, suggested automatically/,
  );
  await expect(page.getByTestId("historical-row").nth(1)).toContainText("Disputed");

  await page.getByLabel("Show rejected (1)").check();
  await expect(bars).toHaveCount(4);
  await page.getByLabel("Show rejected (1)").uncheck();

  await page.locator('[data-claim-id="h2"]').click();
  const selection = page.getByTestId("historical-selection");
  await expect(selection).toContainText("Not reviewed; not an established fact.");
  await expect(selection.locator("mark")).toHaveText("1067");
  await expect(selection).toContainText(`Suggested automatically · ${PRODUCER}@1`);
  await expect(selection).toContainText("video published 2024-03-01");
  await expect(selection.getByRole("link", { name: /Open moment/ })).toHaveAttribute(
    "href",
    /\/videos\/vB\/moments\/spoken\/seg-h2\?/,
  );
  await page.screenshot({ path: testInfo.outputPath("historical-timeline.png"), fullPage: true });

  // The source evidence opens from the timeline.
  await selection.getByRole("link", { name: /Open moment/ }).click();
  await expect(page).toHaveURL(/\/videos\/vB\/moments\/spoken\/seg-h2/);
});

test("suggesting claims reports what was read and what stays for review", async ({ page }) => {
  await mockApi(page);
  await page.goto("/events?corpus=c-history");
  await page.getByRole("button", { name: "Suggest dated claims" }).click();
  await expect(page.getByLabel("Suggest claims")).toContainText(
    "3 new unverified claims from 412 transcript segments and 18 on-screen texts in History; 0 kept, 1 already reviewed",
  );
});

test("an event profile reviews suggestions and switches between both timelines", async ({
  page,
}, testInfo) => {
  const posted = await mockApi(page);
  await page.goto("/events/e-hastings?corpus=c-history");
  const suggestion = page.getByLabel("Claim h2");
  await expect(suggestion).toContainText("Unverified");
  await expect(suggestion.getByTestId("claim-provenance")).toContainText(
    `Suggested automatically · ${PRODUCER}@1 · confidence 0.80`,
  );
  await expect(suggestion.locator("mark")).toHaveText("1067");
  await page.screenshot({ path: testInfo.outputPath("event-review.png"), fullPage: true });
  await suggestion.getByRole("button", { name: "Confirm" }).click();
  await expect(suggestion).toContainText("Confirmed");
  expect(posted).toEqual([{ action: "review", claimIds: ["h2"], reviewStatus: "confirmed" }]);

  await page.getByRole("button", { name: "Publication timeline" }).click();
  await expect(page).toHaveURL(/view=timeline/);
  // Two source videos at their publication dates, not the claimed 1066/1067.
  await expect(page.getByTestId("timeline-marker")).toHaveCount(2);
  await expect(page.locator('[data-video-id="vA"]')).toHaveAttribute("data-date", "2021-05-02");

  await page.getByRole("button", { name: "Historical timeline" }).click();
  await expect(page).toHaveURL(/view=historical/);
  await expect(page.getByTestId("historical-row")).toHaveCount(1);
  await expect(page.locator('[data-claim-id="h2"]')).toHaveAttribute("data-review", "confirmed");
});

test("an identity profile shows the events it takes part in on the historical axis", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto(`/people/${IDENTITY}?corpus=c-history&view=historical`);
  await expect(page.getByRole("button", { name: "Historical timeline" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("historical-row")).toHaveCount(1);
  await expect(page.getByTestId("historical-row")).toContainText("Battle of Hastings");
  await expect(page.getByLabel("Published from")).toBeDisabled();
});
