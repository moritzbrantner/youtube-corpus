import type { VideoAnalysisReport, VideoAnalysisSegment } from "../../api";
import type { ParsedYouTubeUrl } from "./youtube-url";

type BrowserTranscriptSegment = {
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
};

type RankedText = {
  text: string;
  count: number;
};

const STOP_WORDS = new Set([
  "a",
  "about",
  "an",
  "and",
  "are",
  "as",
  "at",
  "be",
  "because",
  "been",
  "but",
  "by",
  "das",
  "de",
  "der",
  "die",
  "ein",
  "eine",
  "el",
  "en",
  "es",
  "for",
  "from",
  "für",
  "he",
  "her",
  "his",
  "how",
  "i",
  "if",
  "im",
  "in",
  "into",
  "is",
  "it",
  "its",
  "la",
  "las",
  "los",
  "mit",
  "nicht",
  "of",
  "on",
  "or",
  "our",
  "que",
  "she",
  "so",
  "that",
  "the",
  "their",
  "them",
  "then",
  "there",
  "these",
  "they",
  "this",
  "to",
  "und",
  "von",
  "was",
  "we",
  "were",
  "what",
  "when",
  "where",
  "which",
  "who",
  "why",
  "will",
  "with",
  "you",
  "your",
  "zu",
]);

const POSITIVE_WORDS = new Set([
  "accurate",
  "better",
  "clear",
  "correct",
  "effective",
  "excellent",
  "good",
  "great",
  "helpful",
  "improve",
  "improved",
  "love",
  "positive",
  "reliable",
  "safe",
  "strong",
  "success",
  "successful",
  "useful",
  "works",
]);

const NEGATIVE_WORDS = new Set([
  "bad",
  "broken",
  "confusing",
  "danger",
  "difficult",
  "error",
  "fail",
  "failed",
  "failure",
  "hate",
  "incorrect",
  "negative",
  "problem",
  "risk",
  "slow",
  "unsafe",
  "weak",
  "worse",
  "wrong",
]);

export function buildBrowserVideoAnalysis(
  parsed: ParsedYouTubeUrl,
  transcriptInput: string,
): VideoAnalysisReport {
  const transcript = parseTranscript(transcriptInput);
  if (transcript.length === 0) {
    throw new Error("Paste a transcript or load a .vtt, .srt, or .txt file before analyzing.");
  }

  const streamId = `browser:${parsed.videoId}:transcript`;
  const segments = transcript.map<VideoAnalysisSegment>((segment, index) => ({
    segmentId: `browser:${parsed.videoId}:segment:${index}`,
    streamId,
    segmentIndex: index,
    startSeconds: segment.startSeconds,
    endSeconds: segment.endSeconds,
    text: segment.text,
    language: null,
  }));
  const text = segments.map((segment) => segment.text).join(" ");
  const lexicalAnalysis = analyzeText(text);
  const now = new Date().toISOString();

  return {
    video: {
      id: `browser:${parsed.videoId}`,
      youtubeId: parsed.videoId,
      sourceUrl: parsed.canonicalUrl,
      title: null,
      mediaDownloaded: false,
      localVideoPath: null,
      captionFilesDownloaded: true,
      parsed: true,
      transcriptStreams: 1,
      transcriptSegments: segments.length,
      transcriptSourceKinds: ["caption_manual"],
      subscriptionNames: [],
      subscriptionSourceUrls: [],
      durationSeconds: inferDuration(transcript),
      uploadDate: null,
      description: null,
      channel: null,
      channelId: null,
      channelUrl: null,
      uploader: null,
      uploaderId: null,
      uploaderUrl: null,
      thumbnailUrl: parsed.thumbnailUrl,
      durationString: null,
      timestamp: null,
      releaseTimestamp: null,
      viewCount: null,
      likeCount: null,
      commentCount: null,
      liveStatus: null,
      availability: null,
      ageLimit: null,
      categories: [],
      tags: [],
      metadata: {
        browserAnalysis: {
          schemaVersion: 1,
          engine: "youtube-corpus-browser-lexical",
          persistence: "none",
          transcriptSource: "user-provided",
        },
      },
      createdAt: now,
      updatedAt: now,
    },
    streams: [
      {
        streamId,
        sourceKind: "caption_manual",
        language: null,
        status: "ready",
        segmentCount: segments.length,
        message: "Transcript imported and analyzed entirely in the browser.",
      },
    ],
    primaryStreamId: streamId,
    segments,
    lexicalAnalysis,
    coverage: {
      metadata: false,
      transcript: true,
      lexical: true,
      mediaRetained: false,
      visualTimeline: false,
      audioFeatures: false,
    },
  };
}

export function parseTranscript(input: string): BrowserTranscriptSegment[] {
  const normalized = input.replace(/\r\n?/g, "\n").trim();
  if (!normalized) return [];

  const timed = parseTimedTranscript(normalized);
  if (timed.length > 0) return timed;

  return splitSentences(normalized)
    .map((text) => ({ startSeconds: null, endSeconds: null, text }))
    .filter((segment) => segment.text.length > 0);
}

function parseTimedTranscript(input: string) {
  const lines = input.split("\n");
  const segments: BrowserTranscriptSegment[] = [];

  for (let index = 0; index < lines.length; index += 1) {
    const current = lines[index]?.trim() ?? "";
    if (!current || current === "WEBVTT" || current.startsWith("NOTE")) continue;

    let timingLine = current;
    if (!timingLine.includes("-->") && /^\d+$/.test(timingLine)) {
      timingLine = lines[index + 1]?.trim() ?? "";
      if (timingLine.includes("-->")) index += 1;
    }
    if (!timingLine.includes("-->")) continue;

    const [startRaw, endRawWithSettings] = timingLine.split("-->", 2);
    const startSeconds = parseTimestamp(startRaw ?? "");
    const endSeconds = parseTimestamp((endRawWithSettings ?? "").trim().split(/\s+/, 1)[0] ?? "");
    if (startSeconds === null || endSeconds === null) continue;

    const textLines: string[] = [];
    for (index += 1; index < lines.length; index += 1) {
      const textLine = lines[index] ?? "";
      if (!textLine.trim()) break;
      textLines.push(textLine);
    }

    const text = cleanCueText(textLines.join(" "));
    if (text) {
      segments.push({ startSeconds, endSeconds, text });
    }
  }

  return segments;
}

function parseTimestamp(value: string) {
  const cleaned = value.trim().replace(",", ".");
  if (!cleaned) return null;
  const parts = cleaned.split(":").map(Number);
  if (parts.some((part) => !Number.isFinite(part))) return null;

  if (parts.length === 3) {
    return parts[0]! * 3600 + parts[1]! * 60 + parts[2]!;
  }
  if (parts.length === 2) {
    return parts[0]! * 60 + parts[1]!;
  }
  return null;
}

function cleanCueText(value: string) {
  return value
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/\s+/g, " ")
    .trim();
}

function analyzeText(text: string) {
  const words = tokenize(text);
  const normalizedWords = words.map((word) => word.toLowerCase());
  const sentences = splitSentences(text);
  const uniqueTerms = new Set(normalizedWords).size;
  const keywordCounts = countContentWords(normalizedWords);
  const keywords = rankCounts(keywordCounts, 20);
  const phraseKeywords = rankPhrases(sentences, 16);
  const extractiveSummary = summarize(sentences, keywordCounts, 4).map((sentence) => ({
    text: sentence,
  }));
  const ruleEntities = extractEntities(text).map((entity) => ({
    text: entity,
    kind: "proper-noun-like",
  }));
  const sentiment = analyzeSentiment(normalizedWords);

  return {
    engine: "youtube-corpus-browser-lexical",
    engineVersion: 1,
    summary: {
      stats: {
        words: words.length,
        sentences: sentences.length,
      },
      uniqueTerms,
      lexicalDiversity: words.length === 0 ? 0 : uniqueTerms / words.length,
    },
    readability: {
      averageSentenceWords: sentences.length === 0 ? 0 : words.length / sentences.length,
    },
    sentiment,
    keywords,
    phraseKeywords,
    extractiveSummary,
    ruleEntities,
  };
}

function tokenize(text: string) {
  return text.match(/[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*/gu) ?? [];
}

function splitSentences(text: string) {
  const collapsed = text.replace(/\s+/g, " ").trim();
  if (!collapsed) return [];
  const matches = collapsed.match(/[^.!?]+(?:[.!?]+|$)/g) ?? [collapsed];
  return matches.map((sentence) => sentence.trim()).filter(Boolean);
}

function countContentWords(words: string[]) {
  const counts = new Map<string, number>();
  for (const word of words) {
    if (!isContentWord(word)) continue;
    counts.set(word, (counts.get(word) ?? 0) + 1);
  }
  return counts;
}

function rankCounts(counts: Map<string, number>, limit: number): RankedText[] {
  return [...counts.entries()]
    .map(([text, count]) => ({ text, count }))
    .sort((left, right) => right.count - left.count || left.text.localeCompare(right.text))
    .slice(0, limit);
}

function rankPhrases(sentences: string[], limit: number): RankedText[] {
  const counts = new Map<string, number>();
  for (const sentence of sentences) {
    const words = tokenize(sentence).map((word) => word.toLowerCase());
    for (let index = 0; index + 1 < words.length; index += 1) {
      const first = words[index]!;
      const second = words[index + 1]!;
      if (!isContentWord(first) || !isContentWord(second)) continue;
      const phrase = `${first} ${second}`;
      counts.set(phrase, (counts.get(phrase) ?? 0) + 1);
    }
  }
  return rankCounts(counts, limit);
}

function summarize(sentences: string[], keywordCounts: Map<string, number>, limit: number) {
  const ranked = sentences
    .map((sentence, index) => {
      const words = tokenize(sentence).map((word) => word.toLowerCase());
      const score = words.reduce((total, word) => total + (keywordCounts.get(word) ?? 0), 0);
      return {
        sentence,
        index,
        score: words.length === 0 ? 0 : score / Math.sqrt(words.length),
      };
    })
    .filter((item) => item.score > 0)
    .sort((left, right) => right.score - left.score || left.index - right.index)
    .slice(0, limit)
    .sort((left, right) => left.index - right.index);

  return ranked.map((item) => item.sentence);
}

function extractEntities(text: string) {
  const candidates =
    text.match(/\b[\p{Lu}][\p{L}\p{M}'’-]+(?:\s+[\p{Lu}][\p{L}\p{M}'’-]+){0,2}\b/gu) ?? [];
  const counts = new Map<string, number>();
  for (const candidate of candidates) {
    const normalized = candidate.trim();
    if (STOP_WORDS.has(normalized.toLowerCase())) continue;
    counts.set(normalized, (counts.get(normalized) ?? 0) + 1);
  }

  return [...counts.entries()]
    .filter(([text, count]) => text.includes(" ") || count > 1)
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .slice(0, 16)
    .map(([text]) => text);
}

function analyzeSentiment(words: string[]) {
  let positiveTerms = 0;
  let negativeTerms = 0;
  for (const word of words) {
    if (POSITIVE_WORDS.has(word)) positiveTerms += 1;
    if (NEGATIVE_WORDS.has(word)) negativeTerms += 1;
  }
  const matched = positiveTerms + negativeTerms;
  const score = matched === 0 ? 0 : (positiveTerms - negativeTerms) / matched;
  const label = score > 0.15 ? "positive" : score < -0.15 ? "negative" : "neutral";
  return { label, score, positiveTerms, negativeTerms };
}

function isContentWord(word: string) {
  return word.length >= 3 && !STOP_WORDS.has(word) && !/^\d+$/.test(word);
}

function inferDuration(segments: BrowserTranscriptSegment[]) {
  const end = segments.reduce<number | null>((latest, segment) => {
    if (segment.endSeconds === null) return latest;
    return latest === null ? segment.endSeconds : Math.max(latest, segment.endSeconds);
  }, null);
  return end;
}
