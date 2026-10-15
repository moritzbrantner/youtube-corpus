// Independent acceptance for #59 (user-defined labels), authored before the implementation.
//
// Seam: the corpus workspace's typed client in src/app/features/corpora/api.ts, which the
// CorpusSources/CorpusVideos UI uses. These tests pin the client to the HTTP contract that
// tests/user_labels_db.rs verifies against Postgres, so the UI cannot silently talk to a
// different route. Client function names below are the acceptance author's routine choice:
//
//   listCorpusLabels(corpusId)                         GET    /api/corpora/{c}/labels
//   createCorpusLabel(corpusId, { name })              POST   /api/corpora/{c}/labels
//   renameCorpusLabel(corpusId, labelId, name)         PATCH  /api/corpora/{c}/labels/{l}
//   deleteCorpusLabel(corpusId, labelId)               DELETE /api/corpora/{c}/labels/{l}
//   assignSourceLabel(corpusId, sourceId, labelId)     PUT    /api/corpora/{c}/sources/{s}/labels/{l}
//   removeSourceLabel(corpusId, sourceId, labelId)     DELETE /api/corpora/{c}/sources/{s}/labels/{l}
//   assignVideoLabel(corpusId, videoId, labelId)       PUT    /api/corpora/{c}/videos/{v}/labels/{l}
//   removeVideoLabel(corpusId, videoId, labelId)       DELETE /api/corpora/{c}/videos/{v}/labels/{l}
//   listCorpusVideos(corpusId, limit, { labelId })     GET    /api/corpora/{c}/videos?limit=..&labelId=..
//
// The module is imported as a namespace so that this file loads on main and fails per test
// until the client exists.
import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import * as corpora from "../../src/app/features/corpora/api";

type Call = { url: string; method: string; body: unknown };

const corpusId = "11111111-1111-4111-8111-111111111111";
const sourceId = "22222222-2222-4222-8222-222222222222";
const videoId = "33333333-3333-4333-8333-333333333333";
const labelId = "44444444-4444-4444-8444-444444444444";

let calls: Call[] = [];
let nextResponse: () => Response;
const originalFetch = globalThis.fetch;

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function client(name: string): (...args: unknown[]) => Promise<unknown> {
  const fn = (corpora as Record<string, unknown>)[name];
  if (typeof fn !== "function") {
    throw new Error(`src/app/features/corpora/api.ts must export ${name}()`);
  }
  return fn as (...args: unknown[]) => Promise<unknown>;
}

beforeEach(() => {
  calls = [];
  nextResponse = () => json({ id: labelId, name: "Theology" });
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ url: String(input), method: (init?.method ?? "GET").toUpperCase(), body });
    return nextResponse();
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("corpus label client (#59)", () => {
  test("lists, creates, renames and deletes corpus labels", async () => {
    nextResponse = () => json([{ id: labelId, name: "Theology" }]);
    expect(await client("listCorpusLabels")(corpusId)).toEqual([{ id: labelId, name: "Theology" }]);

    nextResponse = () => json({ id: labelId, name: "Theology" }, 201);
    await client("createCorpusLabel")(corpusId, { name: "Theology" });
    nextResponse = () => json({ id: labelId, name: "Church history" });
    await client("renameCorpusLabel")(corpusId, labelId, "Church history");
    nextResponse = () => new Response(null, { status: 204 });
    await client("deleteCorpusLabel")(corpusId, labelId);

    expect(calls).toEqual([
      { url: `/api/corpora/${corpusId}/labels`, method: "GET", body: undefined },
      { url: `/api/corpora/${corpusId}/labels`, method: "POST", body: { name: "Theology" } },
      {
        url: `/api/corpora/${corpusId}/labels/${labelId}`,
        method: "PATCH",
        body: { name: "Church history" },
      },
      { url: `/api/corpora/${corpusId}/labels/${labelId}`, method: "DELETE", body: undefined },
    ]);
  });

  test("assigns and removes labels on sources and videos", async () => {
    nextResponse = () => new Response(null, { status: 204 });
    await client("assignSourceLabel")(corpusId, sourceId, labelId);
    await client("removeSourceLabel")(corpusId, sourceId, labelId);
    await client("assignVideoLabel")(corpusId, videoId, labelId);
    await client("removeVideoLabel")(corpusId, videoId, labelId);

    expect(calls.map(({ method, url }) => `${method} ${url}`)).toEqual([
      `PUT /api/corpora/${corpusId}/sources/${sourceId}/labels/${labelId}`,
      `DELETE /api/corpora/${corpusId}/sources/${sourceId}/labels/${labelId}`,
      `PUT /api/corpora/${corpusId}/videos/${videoId}/labels/${labelId}`,
      `DELETE /api/corpora/${corpusId}/videos/${videoId}/labels/${labelId}`,
    ]);
  });

  test("filters the corpus catalog by label", async () => {
    nextResponse = () => json([]);
    await client("listCorpusVideos")(corpusId, 100, { labelId });
    await client("listCorpusVideos")(corpusId, 100);

    const [filtered, unfiltered] = calls.map((call) => new URL(call.url, "http://localhost"));
    expect(filtered.pathname).toBe(`/api/corpora/${corpusId}/videos`);
    expect(filtered.searchParams.get("labelId")).toBe(labelId);
    expect(filtered.searchParams.get("limit")).toBe("100");
    expect(unfiltered.searchParams.has("labelId")).toBe(false);
  });

  test("surfaces the server's message for a rejected duplicate label", async () => {
    nextResponse = () =>
      json({ error: { message: "A label named Theology already exists." } }, 409);
    await expect(client("createCorpusLabel")(corpusId, { name: "Theology" })).rejects.toThrow(
      "A label named Theology already exists.",
    );
  });
});
