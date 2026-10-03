import initYoutubeBrowserWasm, {
  extractPlayerResponse,
  json3ToWebVtt,
  selectCaptionTrack,
} from "./wasm/youtube_browser_wasm";
import type { ParsedYouTubeUrl } from "./youtube-url";

export type BrowserCaptionTrack = {
  baseUrl: string;
  languageCode: string;
  name: string;
  sourceKind: "caption_manual" | "caption_auto";
  isTranslatable: boolean;
  vssId?: string | null;
};

export type BrowserPlayerEvidence = {
  playabilityStatus: string;
  playabilityReason?: string | null;
  title?: string | null;
  author?: string | null;
  channelId?: string | null;
  durationSeconds?: number | null;
  viewCount?: number | null;
  description?: string | null;
  thumbnailUrl?: string | null;
  captionTracks: BrowserCaptionTrack[];
};

export type BrowserCaptionAcquisition = {
  videoId: string;
  transcriptText: string;
  track: BrowserCaptionTrack;
  player: BrowserPlayerEvidence;
  client: string;
  endpoint: string;
  transport: "browser-direct" | "local-yt-dlp";
};

export type BrowserAcquisitionAttempt = {
  client: string;
  endpoint: string;
  stage: "player" | "captions";
  outcome: "blocked" | "http-error" | "no-captions" | "playability" | "empty" | "invalid";
  detail: string;
};

export class BrowserYouTubeAcquisitionError extends Error {
  readonly attempts: BrowserAcquisitionAttempt[];

  constructor(message: string, attempts: BrowserAcquisitionAttempt[]) {
    super(message);
    this.name = "BrowserYouTubeAcquisitionError";
    this.attempts = attempts;
  }
}

type InnertubeClient = {
  id: string;
  context: Record<string, unknown>;
};

type PlayerEndpoint = {
  id: string;
  url: string;
  contentType?: string;
};

type LocalCaptionBridgeResponse = {
  videoId?: unknown;
  transcriptText?: unknown;
  track?: unknown;
  player?: unknown;
};

type JsonRecord = Record<string, unknown>;

const INNERTUBE_PUBLIC_WEB_KEY = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const DEFAULT_LOCAL_YT_DLP_ORIGIN = "http://127.0.0.1:1420";

// The release endpoint is also exposed by current YouTube.js and is useful for
// browser callers because a string POST can stay a CORS-simple request. Current
// yt-dlp also supports no-key InnerTube requests by default, but the public web
// key is retained as a normal YouTube endpoint fallback for clients/rollouts
// that reject the unkeyed form.
const PLAYER_ENDPOINTS: PlayerEndpoint[] = [
  {
    id: "release",
    url: "https://release-youtubei.sandbox.googleapis.com/youtubei/v1/player",
  },
  {
    id: "youtube-keyed-simple",
    url: `https://www.youtube.com/youtubei/v1/player?key=${INNERTUBE_PUBLIC_WEB_KEY}&prettyPrint=false`,
  },
  {
    id: "youtube-simple",
    url: "https://www.youtube.com/youtubei/v1/player?prettyPrint=false",
  },
  {
    id: "youtube-keyed-json",
    url: `https://www.youtube.com/youtubei/v1/player?key=${INNERTUBE_PUBLIC_WEB_KEY}&prettyPrint=false`,
    contentType: "application/json",
  },
];

// Keep this bounded to clients that yt-dlp currently models as not requiring a
// subtitles PO token. Media/GVS policy is intentionally irrelevant here.
const INNERTUBE_CLIENTS: InnertubeClient[] = [
  {
    id: "ios",
    context: {
      clientName: "IOS",
      clientVersion: "21.26.4",
      deviceMake: "Apple",
      deviceModel: "iPhone16,2",
      osName: "iPhone",
      osVersion: "18.3.2.22D82",
      hl: "en",
      gl: "US",
      timeZone: "UTC",
      utcOffsetMinutes: 0,
    },
  },
  {
    id: "android",
    context: {
      clientName: "ANDROID",
      clientVersion: "21.26.364",
      androidSdkVersion: 30,
      osName: "Android",
      osVersion: "11",
      hl: "en",
      gl: "US",
      timeZone: "UTC",
      utcOffsetMinutes: 0,
    },
  },
  {
    id: "web_embedded",
    context: {
      clientName: "WEB_EMBEDDED_PLAYER",
      clientVersion: "2.20260708.00.00",
      hl: "en",
      gl: "US",
      timeZone: "UTC",
      utcOffsetMinutes: 0,
    },
  },
];

let wasmInitialization: Promise<unknown> | null = null;

export async function acquireYouTubeCaptions(
  parsed: ParsedYouTubeUrl,
  preferredLanguages = browserLanguages(),
  fetcher: typeof fetch = fetch,
): Promise<BrowserCaptionAcquisition> {
  await ensureWasm();
  const attempts: BrowserAcquisitionAttempt[] = [];
  const blockedOrigins = new Set<string>();

  for (const endpoint of PLAYER_ENDPOINTS) {
    const origin = new URL(endpoint.url).origin;
    if (blockedOrigins.has(origin)) continue;

    for (const client of INNERTUBE_CLIENTS) {
      const player = await fetchPlayerEvidence(parsed, endpoint, client, attempts, fetcher);
      if (!player) {
        const lastAttempt = attempts.at(-1);
        if (
          lastAttempt?.endpoint === endpoint.id &&
          lastAttempt.stage === "player" &&
          lastAttempt.outcome === "blocked"
        ) {
          // A browser CORS/network denial applies before YouTube sees the
          // InnerTube client payload. Retrying the same origin with more client
          // profiles cannot fix that boundary, so move on immediately.
          blockedOrigins.add(origin);
          break;
        }
        continue;
      }

      if (player.playabilityStatus !== "OK") {
        attempts.push({
          client: client.id,
          endpoint: endpoint.id,
          stage: "player",
          outcome: "playability",
          detail: player.playabilityReason ?? player.playabilityStatus,
        });
        continue;
      }
      if (player.captionTracks.length === 0) {
        attempts.push({
          client: client.id,
          endpoint: endpoint.id,
          stage: "player",
          outcome: "no-captions",
          detail: "player response contains no caption tracks",
        });
        continue;
      }

      const selected = selectCaptionTrack(player.captionTracks, preferredLanguages) as
        | BrowserCaptionTrack
        | null
        | undefined;
      if (!selected) {
        attempts.push({
          client: client.id,
          endpoint: endpoint.id,
          stage: "player",
          outcome: "no-captions",
          detail: "no usable caption track matched",
        });
        continue;
      }

      const transcriptText = await fetchCaptionTrack(selected, endpoint, client, attempts, fetcher);
      if (!transcriptText) continue;

      return {
        videoId: parsed.videoId,
        transcriptText,
        track: selected,
        player,
        client: client.id,
        endpoint: endpoint.id,
        transport: "browser-direct",
      };
    }
  }

  const localFallback = await fetchLocalYtDlpCaptions(
    parsed,
    preferredLanguages,
    attempts,
    fetcher,
  );
  if (localFallback) return localFallback;

  throw new BrowserYouTubeAcquisitionError(acquisitionFailureMessage(attempts), attempts);
}

async function fetchPlayerEvidence(
  parsed: ParsedYouTubeUrl,
  endpoint: PlayerEndpoint,
  client: InnertubeClient,
  attempts: BrowserAcquisitionAttempt[],
  fetcher: typeof fetch,
): Promise<BrowserPlayerEvidence | null> {
  try {
    const headers = endpoint.contentType ? { "Content-Type": endpoint.contentType } : undefined;
    const response = await fetcher(endpoint.url, {
      method: "POST",
      mode: "cors",
      credentials: "omit",
      headers,
      body: JSON.stringify(playerRequest(parsed, client)),
    });
    if (!response.ok) {
      attempts.push({
        client: client.id,
        endpoint: endpoint.id,
        stage: "player",
        outcome: "http-error",
        detail: `HTTP ${response.status}`,
      });
      return null;
    }
    const rawPlayer = await response.json();
    try {
      return extractPlayerResponse(rawPlayer) as BrowserPlayerEvidence;
    } catch (error) {
      attempts.push({
        client: client.id,
        endpoint: endpoint.id,
        stage: "player",
        outcome: "invalid",
        detail: errorMessage(error),
      });
      return null;
    }
  } catch (error) {
    attempts.push({
      client: client.id,
      endpoint: endpoint.id,
      stage: "player",
      outcome: "blocked",
      detail: networkError(error),
    });
    return null;
  }
}

async function fetchCaptionTrack(
  selected: BrowserCaptionTrack,
  endpoint: PlayerEndpoint,
  client: InnertubeClient,
  attempts: BrowserAcquisitionAttempt[],
  fetcher: typeof fetch,
): Promise<string | null> {
  try {
    const captionUrl = captionJson3Url(selected.baseUrl);
    const response = await fetcher(captionUrl, {
      method: "GET",
      mode: "cors",
      credentials: "omit",
    });
    if (!response.ok) {
      attempts.push({
        client: client.id,
        endpoint: endpoint.id,
        stage: "captions",
        outcome: "http-error",
        detail: `${selected.languageCode}: HTTP ${response.status}`,
      });
      return null;
    }
    const payload = await response.text();
    if (!payload.trim()) {
      attempts.push({
        client: client.id,
        endpoint: endpoint.id,
        stage: "captions",
        outcome: "empty",
        detail: `${selected.languageCode}: YouTube returned an empty timed-text body`,
      });
      return null;
    }
    try {
      return json3ToWebVtt(payload);
    } catch (error) {
      attempts.push({
        client: client.id,
        endpoint: endpoint.id,
        stage: "captions",
        outcome: "invalid",
        detail: `${selected.languageCode}: ${errorMessage(error)}`,
      });
      return null;
    }
  } catch (error) {
    attempts.push({
      client: client.id,
      endpoint: endpoint.id,
      stage: "captions",
      outcome: "blocked",
      detail: `${selected.languageCode}: ${networkError(error)}`,
    });
    return null;
  }
}

async function fetchLocalYtDlpCaptions(
  parsed: ParsedYouTubeUrl,
  preferredLanguages: string[],
  attempts: BrowserAcquisitionAttempt[],
  fetcher: typeof fetch,
): Promise<BrowserCaptionAcquisition | null> {
  const origin = localYtDlpBridgeOrigin();
  if (!origin) return null;

  try {
    const response = await fetcher(`${origin}/api/youtube-captions`, {
      method: "POST",
      mode: "cors",
      credentials: "omit",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        sourceUrl: parsed.canonicalUrl,
        preferredLanguages,
      }),
    });
    if (!response.ok) {
      const detail = await localBridgeError(response);
      attempts.push({
        client: "local",
        endpoint: "yt-dlp-bridge",
        stage: "captions",
        outcome: "http-error",
        detail: `HTTP ${response.status}${detail ? `: ${detail}` : ""}`,
      });
      return null;
    }

    const payload = (await response.json()) as LocalCaptionBridgeResponse;
    const acquisition = parseLocalBridgeResponse(parsed, payload);
    if (!acquisition) {
      attempts.push({
        client: "local",
        endpoint: "yt-dlp-bridge",
        stage: "captions",
        outcome: "invalid",
        detail: "local yt-dlp bridge returned an invalid caption response",
      });
      return null;
    }
    return acquisition;
  } catch (error) {
    attempts.push({
      client: "local",
      endpoint: "yt-dlp-bridge",
      stage: "captions",
      outcome: "blocked",
      detail: networkError(error),
    });
    return null;
  }
}

function parseLocalBridgeResponse(
  parsed: ParsedYouTubeUrl,
  payload: LocalCaptionBridgeResponse,
): BrowserCaptionAcquisition | null {
  const transcriptText = typeof payload.transcriptText === "string" ? payload.transcriptText : null;
  const trackValue = asRecord(payload.track);
  const playerValue = asRecord(payload.player);
  if (!transcriptText?.trim() || !trackValue || !playerValue) return null;

  const returnedVideoId = optionalString(payload.videoId);
  if (returnedVideoId && returnedVideoId !== parsed.videoId) return null;

  const sourceKind = optionalString(trackValue.sourceKind);
  if (sourceKind !== "caption_manual" && sourceKind !== "caption_auto") return null;
  const languageCode = optionalString(trackValue.languageCode);
  const name = optionalString(trackValue.name);
  if (!languageCode || !name) return null;

  const track: BrowserCaptionTrack = {
    baseUrl: `local://yt-dlp/${parsed.videoId}`,
    languageCode,
    name,
    sourceKind,
    isTranslatable: false,
    vssId: null,
  };
  const player: BrowserPlayerEvidence = {
    playabilityStatus: optionalString(playerValue.playabilityStatus) ?? "OK",
    playabilityReason: optionalString(playerValue.playabilityReason),
    title: optionalString(playerValue.title),
    author: optionalString(playerValue.author),
    channelId: optionalString(playerValue.channelId),
    durationSeconds: optionalNumber(playerValue.durationSeconds),
    viewCount: optionalNumber(playerValue.viewCount),
    description: optionalString(playerValue.description),
    thumbnailUrl: optionalString(playerValue.thumbnailUrl),
    captionTracks: [track],
  };

  return {
    videoId: parsed.videoId,
    transcriptText,
    track,
    player,
    client: "local",
    endpoint: "yt-dlp-bridge",
    transport: "local-yt-dlp",
  };
}

async function localBridgeError(response: Response) {
  try {
    const payload = (await response.json()) as unknown;
    const body = asRecord(payload);
    const error = asRecord(body?.error);
    return optionalString(error?.message) ?? "";
  } catch {
    return "";
  }
}

function localYtDlpBridgeOrigin() {
  if (typeof window === "undefined") return null;
  const configured = new URLSearchParams(window.location.search).get("backend");
  const candidate = configured || DEFAULT_LOCAL_YT_DLP_ORIGIN;
  try {
    const url = new URL(candidate);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (!isLoopbackHost(url.hostname)) return null;
    return url.origin;
  } catch {
    return null;
  }
}

function isLoopbackHost(hostname: string) {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return host === "localhost" || host === "127.0.0.1" || host === "::1";
}

export function captionJson3Url(baseUrl: string) {
  const url = new URL(baseUrl, "https://www.youtube.com");
  if (url.protocol !== "https:" || !isYoutubeHost(url.hostname)) {
    throw new Error("YouTube returned an unexpected caption host.");
  }
  url.searchParams.set("fmt", "json3");
  return url.toString();
}

export function browserLanguages() {
  if (typeof navigator === "undefined") return ["en"];
  const languages = [...navigator.languages, navigator.language, "en"]
    .filter(Boolean)
    .map((language) => language.toLowerCase());
  return [...new Set(languages)];
}

function playerRequest(parsed: ParsedYouTubeUrl, client: InnertubeClient) {
  const context: Record<string, unknown> = { client: client.context };
  if (client.id === "web_embedded") {
    context.thirdParty = { embedUrl: window.location.href };
  }
  return {
    context,
    videoId: parsed.videoId,
    contentCheckOk: true,
    racyCheckOk: true,
  };
}

function captionHost(hostname: string) {
  return hostname.toLowerCase().replace(/^www\./, "");
}

function isYoutubeHost(hostname: string) {
  const host = captionHost(hostname);
  return host === "youtube.com" || host.endsWith(".youtube.com");
}

function ensureWasm() {
  wasmInitialization ??= initYoutubeBrowserWasm();
  return wasmInitialization;
}

function acquisitionFailureMessage(attempts: BrowserAcquisitionAttempt[]) {
  const localAttempt = attempts.find((attempt) => attempt.endpoint === "yt-dlp-bridge");
  if (localAttempt?.outcome === "blocked") {
    return "YouTube blocked direct browser captions and the local yt-dlp bridge was not reachable. Start youtube-corpus locally, then retry; Postgres is not required for this fallback.";
  }
  if (localAttempt) {
    return "Direct browser acquisition failed and local yt-dlp could not produce captions. Check the local yt-dlp cookies/PO-token setup, or paste/import a transcript.";
  }
  if (attempts.some((attempt) => attempt.outcome === "blocked")) {
    return "YouTube blocked the direct browser caption path for this origin or video. Start the local youtube-corpus yt-dlp fallback or paste/import the transcript.";
  }
  if (attempts.some((attempt) => attempt.outcome === "empty")) {
    return "YouTube exposed caption tracks but returned empty timed-text data. This commonly indicates proof-of-origin enforcement. Use local yt-dlp or paste/import the transcript.";
  }
  if (attempts.length > 0 && attempts.every((attempt) => attempt.outcome === "no-captions")) {
    return "No public caption track was exposed for this video. Paste/import a transcript or use local ASR.";
  }
  const last = attempts.at(-1);
  return last
    ? `YouTube caption acquisition failed: ${last.detail}. Use local yt-dlp or paste/import the transcript.`
    : "YouTube caption acquisition failed. Use local yt-dlp or paste/import the transcript.";
}

function asRecord(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function optionalString(value: unknown) {
  return typeof value === "string" && value.trim() ? value : null;
}

function optionalNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function networkError(error: unknown) {
  const message = errorMessage(error);
  return message === "Failed to fetch" || message === "Load failed"
    ? `${message} (likely CORS or network policy)`
    : message;
}
