import type {
  AddSourceKind,
  CorpusStatus,
  DownloadedFile,
  IngestReport,
  IngestRunStatus,
  SearchMode,
  SearchResult,
  SourceKind,
} from "../../api";

export const modeOptions: Array<{ value: SearchMode; label: string }> = [
  { value: "hybrid", label: "Hybrid" },
  { value: "fts", label: "FTS" },
  { value: "semantic", label: "Semantic" },
];

export const sourceOptions: Array<{ value: SourceKind | "all"; label: string }> = [
  { value: "all", label: "All sources" },
  { value: "caption_manual", label: "Manual captions" },
  { value: "caption_auto", label: "Auto captions" },
  { value: "asr", label: "ASR" },
];

export const addSourceKindOptions: Array<{ value: AddSourceKind; label: string }> = [
  { value: "video", label: "Video" },
  { value: "channel", label: "Channel" },
  { value: "playlist", label: "Playlist" },
];

export interface ResultGroup {
  videoId: string;
  title: string | null;
  sourceUrl: string;
  results: SearchResult[];
}

export function formatTimestamp(seconds: number | null) {
  if (seconds === null) {
    return null;
  }
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const rest = totalSeconds % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${rest.toString().padStart(2, "0")}`
    : `${minutes}:${rest.toString().padStart(2, "0")}`;
}

export function formatTimeRange(startSeconds: number | null, endSeconds: number | null) {
  const start = formatTimestamp(startSeconds);
  const end = formatTimestamp(endSeconds);
  if (start && end) {
    return `${start}-${end}`;
  }
  return start ?? null;
}

export function buildTimestampUrl(sourceUrl: string, seconds: number | null) {
  if (seconds === null) {
    return sourceUrl;
  }
  const separator = sourceUrl.includes("?") ? "&" : "?";
  return `${sourceUrl}${separator}t=${Math.max(0, Math.floor(seconds))}`;
}

export function sourceLabel(sourceKind: string) {
  return sourceOptions.find((option) => option.value === sourceKind)?.label ?? sourceKind;
}

export function scoreLabel(score: number) {
  return Number.isFinite(score) ? score.toFixed(3) : "0.000";
}

export function metricValue(value: number | undefined) {
  return value === undefined ? "-" : value.toLocaleString();
}

export function optionalNumber(value: string) {
  const trimmed = value.trim();
  if (trimmed === "") {
    return null;
  }
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

export function optionalInteger(value: string) {
  const parsed = optionalNumber(value);
  return parsed === null ? null : Math.floor(parsed);
}

export function splitList(value: string) {
  return value
    .split(/[\n,]/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function splitLines(value: string) {
  return value
    .split(/\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

export function addSourceKindLabel(value: AddSourceKind) {
  return addSourceKindOptions.find((option) => option.value === value)?.label ?? value;
}

export function isIngestReport(value: unknown): value is IngestReport {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<IngestReport>;
  return (
    typeof candidate.workflow === "string" &&
    typeof candidate.runId === "string" &&
    typeof candidate.videosSeen === "number" &&
    typeof candidate.videosIndexed === "number" &&
    typeof candidate.segmentsIndexed === "number" &&
    Array.isArray(candidate.items)
  );
}

export function ingestReportFromRun(run: IngestRunStatus | null) {
  if (run?.job?.ingest) {
    return run.job.ingest;
  }
  return run && isIngestReport(run.report) ? run.report : null;
}

export function ingestRunMessage(run: IngestRunStatus | null) {
  if (run?.job?.failure?.message) {
    return run.job.failure.message;
  }
  const message = run?.report.message;
  return typeof message === "string" ? message : null;
}

export function compactFilterLabel(label: string, value: string) {
  const trimmed = value.trim();
  return trimmed === "" ? null : `${label}: ${trimmed}`;
}

export function compactRangeFilterLabel(label: string, minValue: string, maxValue: string) {
  const min = minValue.trim();
  const max = maxValue.trim();
  if (min && max) {
    return `${label}: ${min}-${max}`;
  }
  if (min) {
    return `${label}: >= ${min}`;
  }
  if (max) {
    return `${label}: <= ${max}`;
  }
  return null;
}

export function statusBadgeVariant(status: string) {
  if (status === "indexed" || status === "completed" || status === "succeeded") {
    return "default" as const;
  }
  if (status === "failed") {
    return "destructive" as const;
  }
  return "secondary" as const;
}

export function formatDuration(seconds: number | null) {
  if (seconds === null) {
    return "Unknown duration";
  }
  return formatTimestamp(seconds) ?? "Unknown duration";
}

export function formatDate(value: string | null) {
  if (!value) {
    return "Unknown date";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function videoTitle(file: DownloadedFile) {
  return file.title ?? file.youtubeId ?? file.sourceUrl;
}

export function groupResultsByVideo(results: SearchResult[]) {
  const groups = new Map<string, ResultGroup>();
  for (const result of results) {
    const group = groups.get(result.videoId);
    if (group) {
      group.results.push(result);
    } else {
      groups.set(result.videoId, {
        videoId: result.videoId,
        title: result.title,
        sourceUrl: result.sourceUrl,
        results: [result],
      });
    }
  }
  return Array.from(groups.values());
}

type CorpusStats = CorpusStatus["stats"];

export function videosMetric(stats: CorpusStats, connectionReady: boolean | undefined) {
  return {
    id: "videos",
    label: "Videos",
    value: metricValue(stats?.videos),
    delta: connectionReady ? "Indexed" : "Unavailable",
  };
}

export function segmentsMetric(stats: CorpusStats, connectionReady: boolean | undefined) {
  return {
    id: "segments",
    label: "Segments",
    value: metricValue(stats?.segments),
    delta: connectionReady ? "Searchable" : "Unavailable",
  };
}

export function subscriptionsMetric(stats: CorpusStats) {
  return {
    id: "subscriptions",
    label: "Subscriptions",
    value: metricValue(stats?.subscriptions),
    delta:
      stats === null ? "Unavailable" : `${stats.enabledSubscriptions.toLocaleString()} enabled`,
  };
}
