import type {
  SourceKind,
  VideoAnalysisReport,
  VideoAnalysisSegment,
  VideoAnalysisStream,
} from "../../api";
import type { ParsedYouTubeUrl } from "./youtube-url";

export type BrowserAnalysisStage = "metadata" | "captions" | "analyzing";

type ProgressHandler = (stage: BrowserAnalysisStage, message: string) => void;
type JsonRecord = Record<string, unknown>;

interface OEmbedMetadata {
  title?: string;
  author_name?: string;
  author_url?: string;
  thumbnail_url?: string;
  provider_name?: string;
}

interface CaptionTrack {
  languageCode: string;
  kind: string | null;
  name: string | null;
  baseUrl: string | null;
  isTranslatable: boolean;
}

interface CaptionSegment {
  startSeconds: number;
  endSeconds: number | null;
  text: string;
}

interface PlayerEvidence {
  tracks: CaptionTrack[];
  durationSeconds: number | null;
  videoData: JsonRecord;
}

interface YouTubePlayer {
  destroy(): void;
  getDuration(): number;
  getOption(module: string, option: string): unknown;
  getOptions(module?: string): string[];
  getVideoData(): unknown;
  mute(): void;
  pauseVideo(): void;
  playVideo(): void;
}

interface YouTubePlayerEvent {
  target: YouTubePlayer;
  data?: number;
}

interface YouTubePlayerOptions {
  videoId: string;
  width?: number;
  height?: number;
  playerVars?: Record<string, string | number>;
  events?: {
    onReady?: (event: YouTubePlayerEvent) => void;
    onApiChange?: (event: YouTubePlayerEvent) => void;
    onError?: (event: YouTubePlayerEvent) => void;
  };
}

interface YouTubeApi {
  Player: new (element: HTMLElement, options: YouTubePlayerOptions) => YouTubePlayer;
}

interface BrowserWindow extends Window {
  YT?: YouTubeApi;
  onYouTubeIframeAPIReady?: () => void;
}

interface NlpWasmModule {
  default?: () => Promise<unknown> | unknown;
  runOperation?: (request: unknown) => unknown;
}

let iframeApiPromise: Promise<YouTubeApi> | null = null;
let nlpRuntimePromise: Promise<NlpWasmModule> | null = null;

export async function analyzeYouTubeInBrowser(
  parsed: ParsedYouTubeUrl,
  onProgress: ProgressHandler,
): Promise<VideoAnalysisReport> {
  onProgress("metadata", "Reading public YouTube metadata directly in the browser…");
  const metadata = await fetchOEmbedMetadata(parsed).catch(() => null);

  onProgress("captions", "Discovering an available caption track through the YouTube player…");
  const playerEvidence = await inspectPlayer(parsed.videoId);
  const preferredTrack = selectCaptionTrack(playerEvidence.tracks, browserLanguages());

  onProgress("captions", "Loading and normalizing captions in memory…");
  const captionResult = await loadCaptions(parsed.videoId, preferredTrack, browserLanguages());
  if (captionResult.segments.length === 0) {
    throw new Error(
      "YouTube did not expose a browser-readable caption track for this video. The static analyzer does not use a proxy or backend ASR.",
    );
  }

  const streamId = `browser-caption-${parsed.videoId}-${captionResult.track.languageCode || "und"}`;
  const segments = captionResult.segments.map<VideoAnalysisSegment>((segment, index) => ({
    segmentId: `${streamId}-${index}`,
    streamId,
    segmentIndex: index,
    startSeconds: segment.startSeconds,
    endSeconds: segment.endSeconds,
    text: segment.text,
    language: captionResult.track.languageCode || null,
  }));
  const transcriptText = segments.map((segment) => segment.text).join(" ");

  onProgress("analyzing", "Running deterministic nlp-stack analysis locally in Rust/Wasm…");
  const lexicalAnalysis = await analyzeTranscriptWithWasm(parsed.videoId, transcriptText);
  const now = new Date().toISOString();
  const videoData = playerEvidence.videoData;
  const sourceKind: SourceKind = captionResult.track.kind === "asr" ? "caption_auto" : "caption_manual";
  const stream: VideoAnalysisStream = {
    streamId,
    sourceKind,
    language: captionResult.track.languageCode || null,
    status: "ready",
    segmentCount: segments.length,
    message: "Loaded directly by the browser; not persisted.",
  };
  const title = firstString(metadata?.title, videoData.title);
  const channel = firstString(metadata?.author_name, videoData.author);
  const thumbnailUrl = metadata?.thumbnail_url ?? parsed.thumbnailUrl;

  return {
    video: {
      id: `browser-${parsed.videoId}`,
      youtubeId: parsed.videoId,
      sourceUrl: parsed.canonicalUrl,
      title,
      mediaDownloaded: false,
      localVideoPath: null,
      captionFilesDownloaded: false,
      parsed: true,
      transcriptStreams: 1,
      transcriptSegments: segments.length,
      transcriptSourceKinds: [sourceKind],
      subscriptionNames: [],
      subscriptionSourceUrls: [],
      durationSeconds: playerEvidence.durationSeconds,
      uploadDate: null,
      description: null,
      channel,
      channelId: null,
      channelUrl: metadata?.author_url ?? null,
      uploader: channel,
      uploaderId: null,
      uploaderUrl: metadata?.author_url ?? null,
      thumbnailUrl,
      durationString: null,
      timestamp: null,
      releaseTimestamp: null,
      viewCount: null,
      likeCount: null,
      commentCount: null,
      liveStatus: null,
      availability: "public browser access",
      ageLimit: null,
      categories: [],
      tags: [],
      metadata: {
        runtime: "browser",
        persistence: "none",
        metadataSource: metadata ? "youtube-oembed+iframe-player" : "iframe-player",
        oembed: metadata,
        captionTrack: publicCaptionTrack(captionResult.track),
      },
      createdAt: now,
      updatedAt: now,
    },
    streams: [stream],
    primaryStreamId: streamId,
    segments,
    lexicalAnalysis,
    coverage: {
      metadata: true,
      transcript: segments.length > 0,
      lexical: true,
      mediaRetained: false,
      visualTimeline: false,
      audioFeatures: false,
    },
  };
}

async function fetchOEmbedMetadata(parsed: ParsedYouTubeUrl): Promise<OEmbedMetadata> {
  const endpoint = new URL("https://www.youtube.com/oembed");
  endpoint.searchParams.set("url", parsed.canonicalUrl);
  endpoint.searchParams.set("format", "json");
  const response = await fetch(endpoint, { credentials: "omit" });
  if (!response.ok) {
    throw new Error(`YouTube oEmbed returned ${response.status}.`);
  }
  return (await response.json()) as OEmbedMetadata;
}

async function inspectPlayer(videoId: string): Promise<PlayerEvidence> {
  const youtube = await loadYouTubeIframeApi();
  const host = document.createElement("div");
  host.setAttribute("aria-hidden", "true");
  Object.assign(host.style, {
    position: "fixed",
    left: "-10000px",
    top: "0",
    width: "2px",
    height: "2px",
    opacity: "0",
    pointerEvents: "none",
    overflow: "hidden",
  });
  document.body.append(host);

  return await new Promise<PlayerEvidence>((resolve, reject) => {
    let player: YouTubePlayer | null = null;
    let settled = false;
    let pollTimer: number | null = null;
    const deadline = Date.now() + 7000;

    const cleanup = () => {
      if (pollTimer !== null) window.clearTimeout(pollTimer);
      try {
        player?.pauseVideo();
      } catch {
        // The player may already have been torn down by YouTube.
      }
      try {
        player?.destroy();
      } catch {
        // Keep cleanup best-effort.
      }
      host.remove();
    };

    const finish = (tracks: CaptionTrack[]) => {
      if (settled) return;
      settled = true;
      const duration = safeNumber(() => player?.getDuration());
      const videoData = asRecord(safeValue(() => player?.getVideoData())) ?? {};
      cleanup();
      resolve({ tracks, durationSeconds: duration, videoData });
    };

    const fail = (message: string) => {
      if (settled) return;
      settled = true;
      cleanup();
      reject(new Error(message));
    };

    const poll = () => {
      if (settled || !player) return;
      const tracks = readPlayerCaptionTracks(player);
      if (tracks.length > 0) {
        finish(tracks);
        return;
      }
      if (Date.now() >= deadline) {
        finish([]);
        return;
      }
      pollTimer = window.setTimeout(poll, 200);
    };

    try {
      player = new youtube.Player(host, {
        videoId,
        width: 2,
        height: 2,
        playerVars: {
          autoplay: 0,
          controls: 0,
          playsinline: 1,
          cc_load_policy: 1,
          origin: window.location.origin,
        },
        events: {
          onReady: ({ target }) => {
            player = target;
            try {
              target.mute();
              target.playVideo();
            } catch {
              // Caption discovery can still succeed without autoplay.
            }
            poll();
          },
          onApiChange: ({ target }) => {
            player = target;
            poll();
          },
          onError: ({ data }) => {
            fail(`The YouTube player could not inspect this video (error ${data ?? "unknown"}).`);
          },
        },
      });
    } catch (caught) {
      fail(errorMessage(caught));
    }
  });
}

function readPlayerCaptionTracks(player: YouTubePlayer): CaptionTrack[] {
  try {
    const modules = player.getOptions();
    if (!modules.includes("captions")) return [];
    const value = player.getOption("captions", "tracklist");
    return Array.isArray(value)
      ? value.map(normalizeCaptionTrack).filter((track): track is CaptionTrack => track !== null)
      : [];
  } catch {
    return [];
  }
}

function normalizeCaptionTrack(value: unknown): CaptionTrack | null {
  const track = asRecord(value);
  if (!track) return null;
  const languageCode = readString(track.languageCode ?? track.lang_code ?? track.lang);
  if (!languageCode) return null;
  return {
    languageCode,
    kind: readString(track.kind),
    name: captionName(track.name ?? track.displayName ?? track.languageName),
    baseUrl: readString(track.baseUrl ?? track.base_url),
    isTranslatable: track.isTranslatable === true,
  };
}

export function selectCaptionTrack(
  tracks: CaptionTrack[],
  preferredLanguages: string[],
): CaptionTrack | null {
  if (tracks.length === 0) return null;
  const preferred = preferredLanguages.map(normalizeLanguage).filter(Boolean);
  return [...tracks].sort((left, right) => scoreTrack(right, preferred) - scoreTrack(left, preferred))[0] ?? null;
}

function scoreTrack(track: CaptionTrack, preferred: string[]) {
  const language = normalizeLanguage(track.languageCode);
  const baseLanguage = language.split("-")[0] ?? language;
  let score = track.kind === "asr" ? 0 : 100;
  if (track.baseUrl) score += 5;
  const exactIndex = preferred.indexOf(language);
  if (exactIndex >= 0) score += 80 - Math.min(exactIndex, 20);
  const baseIndex = preferred.findIndex((value) => value.split("-")[0] === baseLanguage);
  if (baseIndex >= 0) score += 50 - Math.min(baseIndex, 20);
  if (baseLanguage === "en") score += 10;
  return score;
}

async function loadCaptions(
  videoId: string,
  preferredTrack: CaptionTrack | null,
  languages: string[],
): Promise<{ track: CaptionTrack; segments: CaptionSegment[] }> {
  const errors: string[] = [];

  if (preferredTrack) {
    for (const url of captionUrls(videoId, preferredTrack)) {
      try {
        const segments = await fetchCaptionDocument(url);
        if (segments.length > 0) return { track: preferredTrack, segments };
      } catch (caught) {
        errors.push(errorMessage(caught));
      }
    }
  }

  try {
    const tracks = await listTimedTextTracks(videoId);
    const selected = selectCaptionTrack(tracks, languages);
    if (selected) {
      for (const url of captionUrls(videoId, selected)) {
        try {
          const segments = await fetchCaptionDocument(url);
          if (segments.length > 0) return { track: selected, segments };
        } catch (caught) {
          errors.push(errorMessage(caught));
        }
      }
    }
  } catch (caught) {
    errors.push(errorMessage(caught));
  }

  const candidates = uniqueLanguages(languages);
  for (const languageCode of candidates) {
    const track: CaptionTrack = {
      languageCode,
      kind: null,
      name: null,
      baseUrl: null,
      isTranslatable: false,
    };
    try {
      const segments = await fetchCaptionDocument(directTimedTextUrl(videoId, track));
      if (segments.length > 0) return { track, segments };
    } catch (caught) {
      errors.push(errorMessage(caught));
    }
  }

  const reason = errors.find(Boolean);
  throw new Error(
    reason
      ? `No browser-readable captions were returned. ${reason}`
      : "No browser-readable captions were returned for this video.",
  );
}

function captionUrls(videoId: string, track: CaptionTrack) {
  const urls: string[] = [];
  if (track.baseUrl) {
    const signed = new URL(track.baseUrl);
    signed.searchParams.set("fmt", "json3");
    signed.searchParams.set("xorp", "true");
    urls.push(signed.toString());
  }
  urls.push(directTimedTextUrl(videoId, track));
  return [...new Set(urls)];
}

function directTimedTextUrl(videoId: string, track: CaptionTrack) {
  const url = new URL("https://www.youtube.com/api/timedtext");
  url.searchParams.set("v", videoId);
  url.searchParams.set("lang", track.languageCode);
  url.searchParams.set("fmt", "json3");
  url.searchParams.set("xorp", "true");
  if (track.kind) url.searchParams.set("kind", track.kind);
  if (track.name) url.searchParams.set("name", track.name);
  return url.toString();
}

async function listTimedTextTracks(videoId: string): Promise<CaptionTrack[]> {
  const url = new URL("https://www.youtube.com/api/timedtext");
  url.searchParams.set("type", "list");
  url.searchParams.set("v", videoId);
  url.searchParams.set("xorp", "true");
  const response = await fetch(url, { credentials: "omit" });
  if (!response.ok) throw new Error(`YouTube caption list returned ${response.status}.`);
  const xml = await response.text();
  if (!xml.trim()) return [];
  const document = new DOMParser().parseFromString(xml, "application/xml");
  if (document.querySelector("parsererror")) return [];
  return Array.from(document.querySelectorAll("track")).map((element) => ({
    languageCode: element.getAttribute("lang_code") ?? element.getAttribute("lang") ?? "",
    kind: element.getAttribute("kind"),
    name: element.getAttribute("name"),
    baseUrl: null,
    isTranslatable: element.getAttribute("cantran") === "true",
  })).filter((track) => Boolean(track.languageCode));
}

async function fetchCaptionDocument(url: string): Promise<CaptionSegment[]> {
  const response = await fetch(url, { credentials: "omit" });
  if (!response.ok) throw new Error(`YouTube captions returned ${response.status}.`);
  const body = await response.text();
  if (!body.trim()) return [];
  try {
    return parseTimedTextJson3(JSON.parse(body));
  } catch {
    return parseTimedTextXml(body);
  }
}

export function parseTimedTextJson3(value: unknown): CaptionSegment[] {
  const root = asRecord(value);
  const events = Array.isArray(root?.events) ? root.events : [];
  const segments: CaptionSegment[] = [];
  for (const eventValue of events) {
    const event = asRecord(eventValue);
    if (!event) continue;
    const startMs = readNumber(event.tStartMs);
    if (startMs === null) continue;
    const pieces = Array.isArray(event.segs) ? event.segs : [];
    const text = cleanCaptionText(
      pieces.map((piece) => readString(asRecord(piece)?.utf8) ?? "").join(""),
    );
    if (!text) continue;
    const durationMs = readNumber(event.dDurationMs);
    segments.push({
      startSeconds: startMs / 1000,
      endSeconds: durationMs === null ? null : (startMs + durationMs) / 1000,
      text,
    });
  }
  return coalesceCaptionSegments(segments);
}

export function parseTimedTextXml(xml: string): CaptionSegment[] {
  const document = new DOMParser().parseFromString(xml, "application/xml");
  if (document.querySelector("parsererror")) return [];
  const segments = Array.from(document.querySelectorAll("text")).flatMap((element) => {
    const start = Number(element.getAttribute("start"));
    const duration = Number(element.getAttribute("dur"));
    const text = cleanCaptionText(element.textContent ?? "");
    if (!Number.isFinite(start) || !text) return [];
    return [{
      startSeconds: start,
      endSeconds: Number.isFinite(duration) ? start + duration : null,
      text,
    }];
  });
  return coalesceCaptionSegments(segments);
}

function coalesceCaptionSegments(segments: CaptionSegment[]) {
  const result: CaptionSegment[] = [];
  for (const segment of segments) {
    const previous = result[result.length - 1];
    if (
      previous &&
      previous.text === segment.text &&
      segment.startSeconds - previous.startSeconds < 1.5
    ) {
      previous.endSeconds = segment.endSeconds ?? previous.endSeconds;
      continue;
    }
    result.push({ ...segment });
  }
  return result;
}

async function analyzeTranscriptWithWasm(videoId: string, text: string): Promise<JsonRecord> {
  const runtime = await loadNlpRuntime();
  if (typeof runtime.runOperation !== "function") {
    throw new Error("The bundled nlp-stack Wasm runtime does not expose runOperation().");
  }
  const response = fromWasm(
    await Promise.resolve(
      runtime.runOperation({
        operation: "analysis.document",
        input: {
          id: `youtube-${videoId}`,
          text,
          profile: "deterministic",
          keywordLimit: 16,
          summarySentences: 6,
          ngramSizes: [2, 3],
          shingleSizes: [3, 5],
          linguistics: { mode: "heuristicBalanced" },
          embedding: { mode: "hashed", dimensions: 128, useIdf: false },
        },
      }),
    ),
  );
  const responseRecord = asRecord(response);
  const value = asRecord(responseRecord?.value) ?? responseRecord;
  const result = asRecord(value?.result) ?? value;
  const lexical = asRecord(result?.lexical);
  if (!lexical) throw new Error("nlp-stack returned no lexical analysis for the transcript.");
  return normalizeLexicalAnalysis(lexical);
}

export function normalizeLexicalAnalysis(lexical: JsonRecord): JsonRecord {
  const summary = asRecord(lexical.summary) ?? {};
  const stats = asRecord(summary.stats) ?? {};
  return {
    ...lexical,
    summary: {
      ...summary,
      stats: {
        ...stats,
        words: lexical.wordCount ?? stats.words,
      },
      uniqueTerms: lexical.uniqueTerms ?? summary.uniqueTerms,
      lexicalDiversity: lexical.lexicalDiversity ?? summary.lexicalDiversity,
    },
  };
}

async function loadNlpRuntime(): Promise<NlpWasmModule> {
  if (nlpRuntimePromise) return nlpRuntimePromise;
  nlpRuntimePromise = (async () => {
    const moduleUrl = new URL("wasm/moenarch_text_analysis_wasm.js", document.baseURI).href;
    let runtime: NlpWasmModule;
    try {
      runtime = (await import(/* @vite-ignore */ moduleUrl)) as NlpWasmModule;
    } catch (caught) {
      throw new Error(
        `Unable to load the bundled nlp-stack Wasm runtime. ${errorMessage(caught)}`,
      );
    }
    if (typeof runtime.default === "function") await runtime.default();
    return runtime;
  })();
  return nlpRuntimePromise;
}

function loadYouTubeIframeApi(): Promise<YouTubeApi> {
  const browser = window as BrowserWindow;
  if (browser.YT?.Player) return Promise.resolve(browser.YT);
  if (iframeApiPromise) return iframeApiPromise;

  iframeApiPromise = new Promise<YouTubeApi>((resolve, reject) => {
    const previousReady = browser.onYouTubeIframeAPIReady;
    const timeout = window.setTimeout(() => reject(new Error("YouTube iframe API timed out.")), 12000);
    browser.onYouTubeIframeAPIReady = () => {
      try {
        previousReady?.();
      } finally {
        window.clearTimeout(timeout);
        if (browser.YT?.Player) resolve(browser.YT);
        else reject(new Error("YouTube iframe API loaded without the Player constructor."));
      }
    };

    if (!document.querySelector('script[src="https://www.youtube.com/iframe_api"]')) {
      const script = document.createElement("script");
      script.src = "https://www.youtube.com/iframe_api";
      script.async = true;
      script.onerror = () => {
        window.clearTimeout(timeout);
        reject(new Error("Unable to load the YouTube iframe API."));
      };
      document.head.append(script);
    }
  });
  return iframeApiPromise;
}

function browserLanguages() {
  return uniqueLanguages([
    ...(navigator.languages ?? []),
    navigator.language,
    "en",
  ]);
}

function uniqueLanguages(values: string[]) {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const normalized = normalizeLanguage(value);
    const base = normalized.split("-")[0] ?? normalized;
    for (const candidate of [normalized, base]) {
      if (candidate && !seen.has(candidate)) {
        seen.add(candidate);
        result.push(candidate);
      }
    }
  }
  return result;
}

function normalizeLanguage(value: string) {
  return value.trim().replace(/_/g, "-").toLowerCase();
}

function publicCaptionTrack(track: CaptionTrack) {
  return {
    languageCode: track.languageCode,
    kind: track.kind,
    name: track.name,
    isTranslatable: track.isTranslatable,
  };
}

function captionName(value: unknown): string | null {
  if (typeof value === "string") return value;
  const record = asRecord(value);
  const simple = readString(record?.simpleText);
  if (simple) return simple;
  const runs = Array.isArray(record?.runs) ? record.runs : [];
  const text = runs.map((run) => readString(asRecord(run)?.text) ?? "").join("");
  return text || null;
}

function cleanCaptionText(value: string) {
  return value.replace(/[\u200b\u200e\u200f]/g, "").replace(/\s+/g, " ").trim();
}

function fromWasm(value: unknown): unknown {
  if (value instanceof Map) {
    return Object.fromEntries(
      Array.from(value.entries(), ([key, entry]) => [String(key), fromWasm(entry)]),
    );
  }
  if (Array.isArray(value)) return value.map(fromWasm);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as JsonRecord).map(([key, entry]) => [key, fromWasm(entry)]),
    );
  }
  return value;
}

function asRecord(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function readString(value: unknown) {
  return typeof value === "string" && value.trim() ? value : null;
}

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function firstString(...values: unknown[]) {
  for (const value of values) {
    const text = readString(value);
    if (text) return text;
  }
  return null;
}

function safeValue(read: () => unknown) {
  try {
    return read();
  } catch {
    return null;
  }
}

function safeNumber(read: () => unknown) {
  return readNumber(safeValue(read));
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
