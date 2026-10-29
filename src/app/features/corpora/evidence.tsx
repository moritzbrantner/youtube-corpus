import type { SearchResult } from "../../api";

/** Shared presentation of timestamped speech and on-screen text evidence. */
export function sourceLabel(kind: string) {
  switch (kind) {
    case "visual_ocr":
      return "On-screen OCR";
    case "caption_manual":
      return "Manual captions";
    case "caption_auto":
      return "Automatic captions";
    case "asr":
      return "Speech recognition";
    default:
      return kind;
  }
}

export function formatTimestamp(seconds: number | null) {
  if (seconds === null) {
    return "No timestamp";
  }
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${rest.toString().padStart(2, "0")}`
    : `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/** `start–end` of an evidence interval, or its start alone when the end is unknown. */
export function formatInterval(start: number | null, end: number | null) {
  return end === null || end === start
    ? formatTimestamp(start)
    : `${formatTimestamp(start)}–${formatTimestamp(end)}`;
}

export function timestampUrl(sourceUrl: string, seconds: number | null) {
  if (seconds === null) {
    return sourceUrl;
  }
  try {
    const url = new URL(sourceUrl);
    url.searchParams.set("t", String(Math.max(0, Math.floor(seconds))));
    return url.toString();
  } catch {
    return sourceUrl;
  }
}

export function OcrEvidenceDetails({
  evidence,
  open = false,
}: {
  evidence: SearchResult["ocrEvidence"];
  open?: boolean;
}) {
  if (!evidence) {
    return null;
  }
  return (
    <details className="text-xs text-muted-foreground" open={open}>
      <summary className="cursor-pointer">OCR observation and producer provenance</summary>
      <div className="mt-2 grid gap-2">
        <p>
          Track {evidence.trackKey} · {evidence.role.replaceAll("_", " ")} · {evidence.sampleCount}{" "}
          samples
        </p>
        <p>
          Producer {evidence.processor} {evidence.processorVersion}; model {evidence.model}{" "}
          {evidence.modelVersion}
        </p>
        <p className="break-all">
          Run {evidence.runId} · input {evidence.inputHash} · config {evidence.configHash}
        </p>
        <p className="break-all">Processing config: {JSON.stringify(evidence.processingConfig)}</p>
        {evidence.observations.map((observation) => (
          <p key={observation.id}>
            Observation {observation.id} · {formatTimestamp(observation.timestampSeconds)} · frame{" "}
            {observation.frameIndex ?? "unknown"} · scene {observation.sceneIndex ?? "unknown"}
          </p>
        ))}
        {evidence.scenes.map((scene) => (
          <p key={scene.id}>
            Scene {scene.sceneIndex} ({scene.id}), {formatTimestamp(scene.startSeconds)}–
            {formatTimestamp(scene.endSeconds)}
          </p>
        ))}
      </div>
    </details>
  );
}
