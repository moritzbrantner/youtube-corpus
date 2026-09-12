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
  transcriptText: string;
  track: BrowserCaptionTrack;
  player: BrowserPlayerEvidence;
  client: string;
  endpoint: string;
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

// The release endpoint is also exposed by current YouTube.js and is useful for
// browser callers because a string POST can stay a CORS-simple request. Keep
// the normal YouTube endpoint as fallback rather than depending on the sandbox
// endpoint as a single authority.
const PLAYER_ENDPOINTS: PlayerEndpoint[] = [
  {
    id: "release",
    url: "https://release-youtubei.sandbox.googleapis.com/youtubei/v1/player",
  },
  {
    id: "youtube-simple",
    url: "https://www.youtube.com/youtubei/v1/player?prettyPrint=false",
  },
  {
    id: "youtube-json",
    url: "https://www.youtube.com/youtubei/v1/player?prettyPrint=false",
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

  for (const endpoint of PLAYER_ENDPOINTS) {
    for (const client of INNERTUBE_CLIENTS) {
      const player = await fetchPlayerEvidence(parsed, endpoint, client, attempts, fetcher);
      if (!player) continue;

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
        transcriptText,
        track: selected,
        player,
        client: client.id,
        endpoint: endpoint.id,
      };
    }
  }

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
  if (attempts.some((attempt) => attempt.outcome === "blocked")) {
    return "YouTube blocked the direct browser caption path for this origin or video. Paste/import the transcript, or use the local yt-dlp mode for the stronger fallback.";
  }
  if (attempts.some((attempt) => attempt.outcome === "empty")) {
    return "YouTube exposed caption tracks but returned empty timed-text data. This commonly indicates proof-of-origin enforcement. Paste/import the transcript, or use local yt-dlp.";
  }
  if (attempts.length > 0 && attempts.every((attempt) => attempt.outcome === "no-captions")) {
    return "No public caption track was exposed for this video. Paste/import a transcript or use local ASR.";
  }
  const last = attempts.at(-1);
  return last
    ? `Direct YouTube caption acquisition failed: ${last.detail}. Paste/import the transcript or use local yt-dlp.`
    : "Direct YouTube caption acquisition failed. Paste/import the transcript or use local yt-dlp.";
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
