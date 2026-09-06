import type { VideoAnalysisReport, VideoAnalysisSegment, VideoAnalysisStream } from "../../api";
import type { ParsedYouTubeUrl } from "./youtube-url";

export type AudioBrowserAnalysisStage = "capture" | "metadata" | "transcribing" | "analyzing";

type ProgressHandler = (stage: AudioBrowserAnalysisStage, message: string) => void;
type JsonRecord = Record<string, unknown>;

interface OEmbedMetadata {
  title?: string;
  author_name?: string;
  author_url?: string;
  thumbnail_url?: string;
}

interface AudioAnalysisProgress {
  stage?: string;
  message?: string;
}

interface AudioAnalysisSegment {
  index: number;
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
  language: string | null;
  attributes?: Record<string, string>;
}

interface AudioAnalysisResult {
  text: string;
  language: string | null;
  segments: AudioAnalysisSegment[];
  source: string;
  attributes: Record<string, string>;
}

interface AudioAnalysisModule {
  browserTranscriptionCapabilities?: () => JsonRecord;
  supportsBrowserTranscription?: () => Promise<boolean>;
  transcribeAudioBlob?: (
    source: Blob,
    options?: {
      source?: string;
      onProgress?: (progress: AudioAnalysisProgress) => void;
    },
  ) => Promise<AudioAnalysisResult>;
}

interface NlpWasmModule {
  default?: () => Promise<unknown> | unknown;
  runOperation?: (request: unknown) => unknown;
}

interface CapturedAudio {
  blob: Blob;
  durationSeconds: number;
  mimeType: string;
}

let audioAnalysisRuntimePromise: Promise<AudioAnalysisModule> | null = null;
let nlpRuntimePromise: Promise<NlpWasmModule> | null = null;

export async function analyzeYouTubeAudioInBrowser(
  parsed: ParsedYouTubeUrl,
  onProgress: ProgressHandler,
): Promise<VideoAnalysisReport> {
  onProgress(
    "capture",
    "Choose the tab playing this video, enable tab audio, then stop sharing when the video is finished.",
  );

  const displayStreamPromise = requestDisplayAudio();
  const metadataPromise = fetchOEmbedMetadata(parsed).catch(() => null);
  const audioRuntimePromise = loadAudioAnalysisRuntime();
  const displayStream = await displayStreamPromise;

  const audioRuntime = await audioRuntimePromise;
  if (typeof audioRuntime.supportsBrowserTranscription !== "function") {
    stopStream(displayStream);
    throw new Error(
      "The bundled audio-analysis runtime does not expose browser transcription support.",
    );
  }
  if (!(await audioRuntime.supportsBrowserTranscription())) {
    stopStream(displayStream);
    throw new Error(
      "This browser does not provide WebGPU for audio-analysis transcription. No CPU or backend fallback is used.",
    );
  }

  onProgress(
    "capture",
    "Recording shared tab audio in memory. Play the video, then use the browser's Stop sharing control when it ends.",
  );
  const captured = await recordSharedAudio(displayStream);
  const metadata = await metadataPromise;

  if (typeof audioRuntime.transcribeAudioBlob !== "function") {
    throw new Error("The bundled audio-analysis runtime does not expose transcribeAudioBlob().");
  }

  onProgress("transcribing", "Preparing captured audio for local audio-analysis transcription…");
  const transcription = await audioRuntime.transcribeAudioBlob(captured.blob, {
    source: `youtube-tab-${parsed.videoId}`,
    onProgress: ({ message }) => {
      if (message) onProgress("transcribing", message);
    },
  });

  const transcriptText = transcription.text.trim();
  if (!transcriptText) {
    throw new Error(
      "audio-analysis completed but did not detect transcribable speech in the captured audio.",
    );
  }

  const streamId = `browser-asr-${parsed.videoId}`;
  const segments = toVideoSegments(streamId, transcription, captured.durationSeconds);

  onProgress("analyzing", "Running deterministic nlp-stack analysis locally in Rust/Wasm…");
  const lexicalAnalysis = await analyzeTranscriptWithWasm(parsed.videoId, transcriptText);
  const now = new Date().toISOString();
  const runtimeCapabilities = audioRuntime.browserTranscriptionCapabilities?.() ?? null;
  const stream: VideoAnalysisStream = {
    streamId,
    sourceKind: "asr",
    language: transcription.language,
    status: "ready",
    segmentCount: segments.length,
    message: "Captured and transcribed locally in the browser; raw audio was not persisted.",
  };

  return {
    video: {
      id: `browser-${parsed.videoId}`,
      youtubeId: parsed.videoId,
      sourceUrl: parsed.canonicalUrl,
      title: readString(metadata?.title),
      mediaDownloaded: false,
      localVideoPath: null,
      captionFilesDownloaded: false,
      parsed: true,
      transcriptStreams: 1,
      transcriptSegments: segments.length,
      transcriptSourceKinds: ["asr"],
      subscriptionNames: [],
      subscriptionSourceUrls: [],
      durationSeconds: null,
      uploadDate: null,
      description: null,
      channel: readString(metadata?.author_name),
      channelId: null,
      channelUrl: metadata?.author_url ?? null,
      uploader: readString(metadata?.author_name),
      uploaderId: null,
      uploaderUrl: metadata?.author_url ?? null,
      thumbnailUrl: metadata?.thumbnail_url ?? parsed.thumbnailUrl,
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
        metadataSource: metadata ? "youtube-oembed" : "url-only",
        transcriptSource: "user-shared-tab-audio",
        audioAnalysis: {
          ...transcription.attributes,
          capabilities: runtimeCapabilities,
        },
        capture: {
          durationSeconds: captured.durationSeconds,
          mimeType: captured.mimeType,
          bytes: captured.blob.size,
          retained: false,
        },
        oembed: metadata,
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

async function requestDisplayAudio() {
  if (!navigator.mediaDevices?.getDisplayMedia) {
    throw new Error("This browser does not support tab-audio sharing through getDisplayMedia().");
  }
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
  } catch (caught) {
    if (caught instanceof DOMException && caught.name === "NotAllowedError") {
      throw new Error("Tab-audio sharing was cancelled or denied.");
    }
    throw caught;
  }
  if (stream.getAudioTracks().length === 0) {
    stopStream(stream);
    throw new Error(
      "No audio track was shared. Choose the tab playing the video and enable the browser's Share tab audio option.",
    );
  }
  return stream;
}

async function recordSharedAudio(stream: MediaStream): Promise<CapturedAudio> {
  if (typeof MediaRecorder === "undefined") {
    stopStream(stream);
    throw new Error("This browser cannot record the shared audio stream with MediaRecorder.");
  }

  const audioTrack = stream.getAudioTracks()[0];
  if (!audioTrack) {
    stopStream(stream);
    throw new Error("The shared display stream contains no audio track.");
  }

  const audioStream = new MediaStream([audioTrack]);
  const mimeType = preferredRecorderMimeType();
  const recorder = mimeType
    ? new MediaRecorder(audioStream, { mimeType })
    : new MediaRecorder(audioStream);
  const chunks: BlobPart[] = [];
  const startedAt = performance.now();

  const stopped = new Promise<void>((resolve, reject) => {
    recorder.addEventListener("dataavailable", (event) => {
      if (event.data.size > 0) chunks.push(event.data);
    });
    recorder.addEventListener("error", () => reject(new Error("Browser audio recording failed.")));
    recorder.addEventListener("stop", () => resolve(), { once: true });
  });

  const stopRecorder = () => {
    if (recorder.state !== "inactive") recorder.stop();
  };
  for (const track of stream.getTracks()) {
    track.addEventListener("ended", stopRecorder, { once: true });
  }

  recorder.start(1000);
  try {
    await stopped;
  } finally {
    stopStream(stream);
  }

  const durationSeconds = Math.max(0, (performance.now() - startedAt) / 1000);
  const blob = new Blob(chunks, { type: recorder.mimeType || mimeType || "audio/webm" });
  if (blob.size === 0) {
    throw new Error("The shared tab audio recording was empty.");
  }
  return { blob, durationSeconds, mimeType: blob.type };
}

function preferredRecorderMimeType() {
  const candidates = ["audio/webm;codecs=opus", "audio/ogg;codecs=opus", "audio/webm"];
  return candidates.find((candidate) => MediaRecorder.isTypeSupported(candidate)) ?? "";
}

function stopStream(stream: MediaStream) {
  for (const track of stream.getTracks()) track.stop();
}

function toVideoSegments(
  streamId: string,
  transcription: AudioAnalysisResult,
  captureDurationSeconds: number,
): VideoAnalysisSegment[] {
  const sourceSegments =
    transcription.segments.length > 0
      ? transcription.segments
      : [
          {
            index: 0,
            startSeconds: 0,
            endSeconds: captureDurationSeconds,
            text: transcription.text,
            language: transcription.language,
          },
        ];

  return sourceSegments
    .filter((segment) => segment.text.trim().length > 0)
    .map((segment, index) => ({
      segmentId: `${streamId}-${index}`,
      streamId,
      segmentIndex: index,
      startSeconds: segment.startSeconds ?? 0,
      endSeconds: segment.endSeconds,
      text: segment.text.trim(),
      language: segment.language ?? transcription.language,
    }));
}

async function fetchOEmbedMetadata(parsed: ParsedYouTubeUrl): Promise<OEmbedMetadata> {
  const endpoint = new URL("https://www.youtube.com/oembed");
  endpoint.searchParams.set("url", parsed.canonicalUrl);
  endpoint.searchParams.set("format", "json");
  const response = await fetch(endpoint, { credentials: "omit" });
  if (!response.ok) throw new Error(`YouTube oEmbed returned ${response.status}.`);
  return (await response.json()) as OEmbedMetadata;
}

async function loadAudioAnalysisRuntime(): Promise<AudioAnalysisModule> {
  if (audioAnalysisRuntimePromise) return audioAnalysisRuntimePromise;
  audioAnalysisRuntimePromise = (async () => {
    const moduleUrl = new URL("audio-analysis/transcription.js", document.baseURI).href;
    try {
      return (await import(/* @vite-ignore */ moduleUrl)) as AudioAnalysisModule;
    } catch (caught) {
      throw new Error(
        `Unable to load the bundled audio-analysis browser runtime. ${errorMessage(caught)}`,
      );
    }
  })();
  return audioAnalysisRuntimePromise;
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

function normalizeLexicalAnalysis(lexical: JsonRecord): JsonRecord {
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
      throw new Error(`Unable to load the bundled nlp-stack Wasm runtime. ${errorMessage(caught)}`);
    }
    if (typeof runtime.default === "function") await runtime.default();
    return runtime;
  })();
  return nlpRuntimePromise;
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

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
