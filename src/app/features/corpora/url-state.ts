import * as React from "react";

import type { SearchMode, SourceKind } from "../../api";

export type CorpusUrlState = {
  corpusId: string | null;
  /** Tracked source open on the Channels & Sources page. */
  sourceId: string | null;
  /** YouTube channel id open on the Channels & Sources page. */
  channelId: string | null;
  query: string;
  segmentId: string | null;
  mode: SearchMode;
  modality: "all" | "spoken" | "visible";
  sourceKind: "all" | SourceKind;
  language: string;
  channelQuery: string;
  titleQuery: string;
  categoryQuery: string;
  tagQuery: string;
  uploadDateFrom: string;
  uploadDateTo: string;
  /** Video selected on the Processing page. */
  processingVideoId: string | null;
  /** Processing run list filters: stage (detector id), attempt state and history scope. */
  stage: string;
  attemptState: string;
  /** `all` lists superseded attempts too; empty lists the newest attempt per video stage. */
  attempts: "" | "all";
  /** People and Entities pages: `all` spans every corpus; empty scopes to the current corpus. */
  identityScope: "" | "all";
  /** Entities page type filter (organization, location, ...); empty lists every type. */
  identityType: string;
  /** People and Entities name filter. */
  identityQuery: string;
  /**
   * Profile view: empty shows the evidence (or claim) list, `timeline` the publication timeline
   * and `historical` the claimed historical dates (#107). The two timelines never share an axis.
   */
  profileView: "" | "timeline" | "historical";
  /** Profile evidence type filter shared by both views; empty keeps every type. */
  evidenceKind: "" | EvidenceKind;
  /** Profile publication-date range (`YYYY-MM-DD`, inclusive), shared by both views. */
  publishedFrom: string;
  publishedTo: string;
};

/** Profile evidence types: transcript and on-screen text mentions, face and voice tracks. */
export const evidenceKinds = ["spoken", "ocr", "shown", "heard"] as const;
export type EvidenceKind = (typeof evidenceKinds)[number];

export type CorpusSearchFilters = Pick<
  CorpusUrlState,
  | "mode"
  | "modality"
  | "sourceKind"
  | "language"
  | "channelQuery"
  | "titleQuery"
  | "categoryQuery"
  | "tagQuery"
  | "uploadDateFrom"
  | "uploadDateTo"
>;

type CorpusUrlPatch = Partial<CorpusUrlState>;

const keys: Array<[keyof CorpusUrlState, string]> = [
  ["corpusId", "corpus"],
  ["sourceId", "source"],
  ["channelId", "channelId"],
  ["query", "q"],
  ["segmentId", "segment"],
  ["mode", "mode"],
  ["modality", "modality"],
  ["sourceKind", "sourceKind"],
  ["language", "language"],
  ["channelQuery", "channel"],
  ["titleQuery", "title"],
  ["categoryQuery", "category"],
  ["tagQuery", "tag"],
  ["uploadDateFrom", "from"],
  ["uploadDateTo", "to"],
  ["processingVideoId", "video"],
  ["stage", "stage"],
  ["attemptState", "state"],
  ["attempts", "attempts"],
  ["identityScope", "scope"],
  ["identityType", "type"],
  ["identityQuery", "name"],
  ["profileView", "view"],
  ["evidenceKind", "evidence"],
  ["publishedFrom", "since"],
  ["publishedTo", "until"],
];

function selected<T extends string>(value: string | null, options: readonly T[], fallback: T): T {
  return options.find((option) => option === value) ?? fallback;
}

export function readUrlState(
  url = typeof window === "undefined" ? null : new URL(window.location.href),
): CorpusUrlState {
  if (!url) {
    return {
      corpusId: null,
      sourceId: null,
      channelId: null,
      query: "",
      segmentId: null,
      mode: "hybrid",
      modality: "all",
      sourceKind: "all",
      language: "",
      channelQuery: "",
      titleQuery: "",
      categoryQuery: "",
      tagQuery: "",
      uploadDateFrom: "",
      uploadDateTo: "",
      processingVideoId: null,
      stage: "",
      attemptState: "",
      attempts: "",
      identityScope: "",
      identityType: "",
      identityQuery: "",
      profileView: "",
      evidenceKind: "",
      publishedFrom: "",
      publishedTo: "",
    };
  }
  const params = url.searchParams;
  return {
    corpusId: params.get("corpus"),
    sourceId: params.get("source"),
    channelId: params.get("channelId"),
    query: params.get("q") ?? "",
    segmentId: params.get("segment"),
    mode: selected(params.get("mode"), ["hybrid", "fts", "semantic"], "hybrid"),
    modality: selected(params.get("modality"), ["all", "spoken", "visible"], "all"),
    sourceKind: selected(
      params.get("sourceKind"),
      ["all", "caption_manual", "caption_auto", "asr", "visual_ocr"],
      "all",
    ),
    language: params.get("language") ?? "",
    channelQuery: params.get("channel") ?? "",
    titleQuery: params.get("title") ?? "",
    categoryQuery: params.get("category") ?? "",
    tagQuery: params.get("tag") ?? "",
    uploadDateFrom: params.get("from") ?? "",
    uploadDateTo: params.get("to") ?? "",
    processingVideoId: params.get("video"),
    stage: params.get("stage") ?? "",
    attemptState: params.get("state") ?? "",
    attempts: params.get("attempts") === "all" ? "all" : "",
    identityScope: params.get("scope") === "all" ? "all" : "",
    identityType: params.get("type") ?? "",
    identityQuery: params.get("name") ?? "",
    profileView: selected(params.get("view"), ["", "timeline", "historical"], ""),
    evidenceKind: selected(params.get("evidence"), ["", ...evidenceKinds], ""),
    publishedFrom: params.get("since") ?? "",
    publishedTo: params.get("until") ?? "",
  };
}

export function useCorpusUrlState() {
  const [state, setState] = React.useState<CorpusUrlState>(readUrlState);

  React.useEffect(() => {
    const syncFromLocation = () => setState(readUrlState());
    window.addEventListener("popstate", syncFromLocation);
    window.addEventListener("hashchange", syncFromLocation);
    return () => {
      window.removeEventListener("popstate", syncFromLocation);
      window.removeEventListener("hashchange", syncFromLocation);
    };
  }, []);

  const update = React.useCallback(
    (patch: CorpusUrlPatch, history: "push" | "replace" = "push") => {
      const url = patchCorpusUrl(new URL(window.location.href), patch);
      if (url.href === window.location.href) {
        return;
      }
      window.history[history === "push" ? "pushState" : "replaceState"](null, "", url);
      window.dispatchEvent(new PopStateEvent("popstate"));
    },
    [],
  );

  return { state, update };
}

export function patchCorpusUrl(current: URL, patch: CorpusUrlPatch): URL {
  const url = new URL(current);
  for (const [field, param] of keys) {
    const value = patch[field];
    if (value !== undefined) {
      setOrDelete(url, param, value);
    }
  }
  // A new query, filter or corpus invalidates the selected moment; opening a source or
  // channel keeps it so returning to Search restores the original result.
  if (Object.keys(patch).some((field) => !directoryFields.has(field))) {
    url.searchParams.delete("segment");
  }
  // Sources and channel selections belong to one corpus.
  if (patch.corpusId !== undefined && patch.corpusId !== current.searchParams.get("corpus")) {
    if (patch.sourceId === undefined) url.searchParams.delete("source");
    if (patch.channelId === undefined) url.searchParams.delete("channelId");
    if (patch.processingVideoId === undefined) url.searchParams.delete("video");
  }
  return url;
}

/** Selections that leave the selected search moment intact. */
const directoryFields = new Set([
  "segmentId",
  "sourceId",
  "channelId",
  "processingVideoId",
  "stage",
  "attemptState",
  "attempts",
  "identityScope",
  "identityType",
  "identityQuery",
  "profileView",
  "evidenceKind",
  "publishedFrom",
  "publishedTo",
]);

export type CorpusPage =
  | "corpora"
  | "search"
  | "sources"
  | "videos"
  | "processing"
  | "people"
  | "entities"
  | "events";

/** Parses `/events` and `/events/:eventId`. */
export function readEventRoute(url: URL): { eventId: string | null } | null {
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts[0] !== "events") {
    return null;
  }
  if (parts.length === 1) {
    return { eventId: null };
  }
  return parts.length === 2 ? { eventId: parts[1] } : null;
}

/** An event's profile; keeps the corpus, scope and a pending claim moment. */
export function eventUrl(current: URL, eventId: string): URL {
  const url = corpusPageUrl(current, "events");
  url.pathname = `/events/${encodeURIComponent(eventId)}`;
  return url;
}

/**
 * The Events directory holding one video moment as the evidence for a claim to record, either on
 * a new event or on an existing one (`moment=<videoId>/<modality>/<evidenceId>`).
 */
export function eventsForMomentUrl(current: URL, moment: MomentLocator): URL {
  const url = corpusPageUrl(current, "events");
  url.searchParams.set("moment", `${moment.videoId}/${moment.modality}/${moment.evidenceId}`);
  return url;
}

export function readMomentParam(url: URL): MomentLocator | null {
  const [videoId, modality, evidenceId, ...rest] = (url.searchParams.get("moment") ?? "").split(
    "/",
  );
  if (!videoId || !evidenceId || rest.length > 0) return null;
  if (modality !== "spoken" && modality !== "visible") return null;
  return { videoId, modality, evidenceId };
}

export type IdentityPage = "people" | "entities";

/** Parses `/people`, `/entities` and their `/:identityId` profiles. */
export function readIdentityRoute(url: URL): { identityId: string | null } | null {
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts[0] !== "people" && parts[0] !== "entities") {
    return null;
  }
  if (parts.length === 1) {
    return { identityId: null };
  }
  return parts.length === 2 ? { identityId: parts[1] } : null;
}

/** A canonical identity's profile; keeps the corpus, scope and list filters for the way back. */
export function identityUrl(current: URL, page: IdentityPage, identityId: string): URL {
  const url = corpusPageUrl(current, page);
  url.pathname = `/${page}/${encodeURIComponent(identityId)}`;
  return url;
}

/** Parses `/processing` and `/processing/attempts/:attemptId`. */
export function readProcessingRoute(url: URL): { attemptId: string | null } | null {
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts[0] !== "processing") {
    return null;
  }
  if (parts.length === 1) {
    return { attemptId: null };
  }
  return parts[1] === "attempts" && parts[2] && parts.length === 3 ? { attemptId: parts[2] } : null;
}

/** The Processing page, optionally with one video selected. */
export function processingUrl(current: URL, videoId?: string | null): URL {
  return corpusPageUrl(
    videoId === undefined ? current : patchCorpusUrl(current, { processingVideoId: videoId }),
    "processing",
  );
}

/** Deep link to one recorded detector attempt; keeps the Processing filters for the way back. */
export function attemptUrl(current: URL, attemptId: string): URL {
  const url = corpusPageUrl(current, "processing");
  url.pathname = `/processing/attempts/${encodeURIComponent(attemptId)}`;
  return url;
}

export function corpusPageUrl(current: URL, page: CorpusPage): URL {
  const url = new URL(current);
  url.pathname = `/${page}`;
  url.hash = "";
  if (page !== "events") {
    url.searchParams.delete("moment");
  }
  return url;
}

/** Speech (transcript segment) or on-screen text (OCR track) evidence. */
export type MomentModality = "spoken" | "visible";

/**
 * A source-grounded moment: the modality and stable id of one evidence record in one video. Its
 * time interval belongs to the evidence record, so the URL never carries a second copy of it.
 */
export type MomentLocator = {
  videoId: string;
  modality: MomentModality;
  evidenceId: string;
};

export type VideoRoute = { videoId: string; moment: MomentLocator | null };

/** Parses `/videos/:videoId` and `/videos/:videoId/moments/:modality/:evidenceId`. */
export function readVideoRoute(url: URL): VideoRoute | null {
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts[0] !== "videos" || !parts[1]) {
    return null;
  }
  const videoId = parts[1];
  if (parts.length === 2) {
    return { videoId, moment: null };
  }
  const [, , kind, modality, evidenceId, ...rest] = parts;
  if (
    kind !== "moments" ||
    (modality !== "spoken" && modality !== "visible") ||
    !evidenceId ||
    rest.length > 0
  ) {
    return null;
  }
  return { videoId, moment: { videoId, modality, evidenceId } };
}

/**
 * A video's page, or one of its moments. Keeps the search query, filters and selected search
 * result so returning to Search restores the originating results.
 */
export function videoUrl(
  current: URL,
  videoId: string,
  moment?: Omit<MomentLocator, "videoId">,
): URL {
  const url = corpusPageUrl(current, "videos");
  url.pathname = moment
    ? `/videos/${encodeURIComponent(videoId)}/moments/${moment.modality}/${encodeURIComponent(moment.evidenceId)}`
    : `/videos/${encodeURIComponent(videoId)}`;
  return url;
}

/** Opens a search result's moment and marks it selected for the way back to Search. */
export function searchMomentUrl(current: URL, locator: MomentLocator): URL {
  return videoUrl(patchCorpusUrl(current, { segmentId: locator.evidenceId }), locator.videoId, {
    modality: locator.modality,
    evidenceId: locator.evidenceId,
  });
}

export function momentModality(sourceKind: string): MomentModality {
  return sourceKind === "visual_ocr" ? "visible" : "spoken";
}

/** Channels & Sources page showing the directory, one tracked source or one channel. */
export function directoryUrl(
  current: URL,
  selection: { sourceId?: string; channelId?: string } = {},
): URL {
  return corpusPageUrl(
    patchCorpusUrl(current, {
      sourceId: selection.sourceId ?? null,
      channelId: selection.channelId ?? null,
    }),
    "sources",
  );
}

/** Same-document navigation for in-app links: pushes history and notifies URL-state hooks. */
export function navigateInApp(event: React.MouseEvent<HTMLAnchorElement>) {
  if (pushInAppUrl(event)) {
    window.scrollTo({ top: 0 });
  }
}

/** Like {@link navigateInApp}, but keeps the scroll position (moments within one page). */
export function navigateInPlace(event: React.MouseEvent<HTMLAnchorElement>) {
  pushInAppUrl(event);
}

function pushInAppUrl(event: React.MouseEvent<HTMLAnchorElement>): boolean {
  if (
    event.defaultPrevented ||
    event.button !== 0 ||
    event.metaKey ||
    event.ctrlKey ||
    event.shiftKey ||
    event.altKey
  ) {
    return false;
  }
  const target = new URL(event.currentTarget.href);
  if (target.origin !== window.location.origin) {
    return false;
  }
  event.preventDefault();
  window.history.pushState(null, "", target);
  window.dispatchEvent(new PopStateEvent("popstate"));
  return true;
}

function setOrDelete(url: URL, key: string, value: string | null) {
  if (value) {
    url.searchParams.set(key, value);
  } else {
    url.searchParams.delete(key);
  }
}
