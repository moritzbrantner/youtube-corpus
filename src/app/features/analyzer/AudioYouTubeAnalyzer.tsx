import * as React from "react";

import type { VideoAnalysisReport } from "../../api";
import {
  analyzeYouTubeAudioInBrowser,
  type AudioBrowserAnalysisStage,
} from "./audio-browser-analysis";
import { parseYouTubeVideoUrl } from "./youtube-url";

type Phase =
  | "idle"
  | "capture"
  | "metadata"
  | "transcribing"
  | "analyzing"
  | "done"
  | "error";
type JsonRecord = Record<string, unknown>;

export function AudioYouTubeAnalyzer() {
  const [sourceUrl, setSourceUrl] = React.useState(initialVideoUrl);
  const [phase, setPhase] = React.useState<Phase>("idle");
  const [progressMessage, setProgressMessage] = React.useState("Paste a YouTube URL to begin.");
  const [error, setError] = React.useState<string | null>(null);
  const [report, setReport] = React.useState<VideoAnalysisReport | null>(null);

  const parsedUrl = React.useMemo(() => parseYouTubeVideoUrl(sourceUrl), [sourceUrl]);
  const lexical = asRecord(report?.lexicalAnalysis);
  const lexicalSummary = asRecord(lexical?.summary);
  const lexicalStats = asRecord(lexicalSummary?.stats);
  const readability = asRecord(lexical?.readability);
  const sentiment = asRecord(lexical?.sentiment);
  const keywords = asRecordArray(lexical?.keywords);
  const phrases = asRecordArray(lexical?.phraseKeywords);
  const summarySentences = asRecordArray(lexical?.extractiveSummary);
  const entities = asRecordArray(lexical?.ruleEntities);

  async function analyze(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setReport(null);

    const parsed = parseYouTubeVideoUrl(sourceUrl);
    if (!parsed) {
      setPhase("error");
      setError("Enter a valid YouTube video URL (watch, youtu.be, Shorts, live, or embed URL).");
      return;
    }

    persistWorkbenchUrl(parsed.canonicalUrl);
    try {
      setPhase("capture");
      setProgressMessage(
        "Choose the tab playing this video and enable Share tab audio. Stop sharing when the video is finished.",
      );
      const nextReport = await analyzeYouTubeAudioInBrowser(parsed, (stage, message) => {
        setPhase(phaseForStage(stage));
        setProgressMessage(message);
      });
      setReport(nextReport);
      setPhase("done");
      setProgressMessage(
        "Browser analysis complete. Audio was transcribed locally and nothing was sent to a corpus backend.",
      );
    } catch (caught) {
      setPhase("error");
      setError(errorMessage(caught));
      setProgressMessage("Analysis stopped.");
    }
  }

  const preview = report?.video.youtubeId
    ? parseYouTubeVideoUrl(`https://www.youtube.com/watch?v=${report.video.youtubeId}`)
    : parsedUrl;
  const busy = ["capture", "metadata", "transcribing", "analyzing"].includes(phase);

  return (
    <main className="analyzer-shell">
      <header className="analyzer-header">
        <div>
          <a className="analyzer-kicker" href="https://github.com/moritzbrantner/youtube-corpus">
            youtube-corpus
          </a>
          <h1>Analyze a YouTube video from one URL</h1>
          <p>
            Paste a public YouTube URL. GitHub Pages reads public metadata, captures audio from the
            tab you explicitly share, transcribes it locally with the audio-analysis WebGPU stack,
            and runs deterministic nlp-stack analysis in Rust/Wasm. No Postgres or corpus backend is
            required.
          </p>
        </div>
        <nav className="analyzer-nav" aria-label="YouTube Corpus views">
          <a href="#corpora">Research corpora</a>
          <a href="#legacy">Advanced local UI</a>
        </nav>
      </header>

      <section className="runtime-strip" aria-label="Runtime status">
        <span className="runtime-dot runtime-dot-ready" aria-hidden="true" />
        <strong>Browser-only GitHub Pages</strong>
        <span>
          Direct YouTube metadata · shared tab audio · audio-analysis WebGPU ASR · local nlp-stack
          Rust/Wasm · no corpus backend
        </span>
      </section>

      <section className="analyzer-input-panel">
        <form onSubmit={analyze} className="analyzer-form">
          <label htmlFor="youtube-url">YouTube URL</label>
          <div className="analyzer-input-row">
            <input
              id="youtube-url"
              type="url"
              inputMode="url"
              autoComplete="url"
              placeholder="https://www.youtube.com/watch?v=…"
              value={sourceUrl}
              onChange={(event) => setSourceUrl(event.target.value)}
            />
            <button type="submit" disabled={busy}>
              {busy ? "Analyzing…" : "Share audio & analyze"}
            </button>
          </div>
          {sourceUrl && !parsedUrl ? (
            <p className="field-error">This is not a recognized YouTube video URL.</p>
          ) : null}
          <p className="muted-label">
            The browser must grant a user-initiated tab-share. Choose the tab that is playing the
            video, enable tab audio, then stop sharing when playback is finished. Raw audio stays in
            memory and is discarded after transcription.
          </p>
        </form>
      </section>

      {preview ? (
        <section className="preview-grid">
          <div className="video-frame">
            <iframe
              src={preview.embedUrl}
              title="YouTube video preview"
              allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
              allowFullScreen
            />
          </div>
          <div className="progress-panel">
            <span className={`phase-badge phase-${phase}`}>{phaseLabel(phase)}</span>
            <h2>{report?.video.title ?? "Video analysis"}</h2>
            <p>{progressMessage}</p>
            <ol className="pipeline-list">
              <PipelineStep label="URL + preview" active={Boolean(preview)} />
              <PipelineStep label="Shared video audio" active={phaseReached(phase, "capture")} />
              <PipelineStep
                label="audio-analysis transcription"
                active={phaseReached(phase, "transcribing")}
              />
              <PipelineStep
                label="In-memory transcript"
                active={Boolean(report?.coverage.transcript)}
              />
              <PipelineStep label="Rust/Wasm NLP" active={Boolean(report?.coverage.lexical)} />
              <PipelineStep label="Report" active={phase === "done"} />
            </ol>
          </div>
        </section>
      ) : null}

      {error ? (
        <div className="analysis-error" role="alert">
          {error}
        </div>
      ) : null}

      {report ? (
        <div className="report-stack">
          <section>
            <div className="section-heading">
              <div>
                <span>Coverage</span>
                <h2>What this browser run actually analyzed</h2>
              </div>
              <a href={report.video.sourceUrl} target="_blank" rel="noreferrer">
                Open on YouTube
              </a>
            </div>
            <div className="coverage-grid">
              <CoverageCard
                label="Metadata"
                available={report.coverage.metadata}
                detail="YouTube oEmbed metadata"
              />
              <CoverageCard
                label="Transcript"
                available={report.coverage.transcript}
                detail={`${formatNumber(report.video.transcriptSegments)} audio-analysis ASR segments`}
              />
              <CoverageCard
                label="NLP"
                available={report.coverage.lexical}
                detail="nlp-stack Rust/Wasm analysis"
              />
              <CoverageCard
                label="Audio evidence"
                available={report.coverage.transcript}
                detail="user-shared tab audio transcribed locally"
              />
              <CoverageCard
                label="Media retained"
                available={report.coverage.mediaRetained}
                detail="captured audio discarded after transcription"
              />
              <CoverageCard
                label="Visual timeline"
                available={report.coverage.visualTimeline}
                detail="not produced in this audio-first browser run"
              />
            </div>
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Overview</span>
                <h2>{report.video.title ?? report.video.youtubeId ?? "YouTube video"}</h2>
              </div>
            </div>
            <div className="metric-grid">
              <Metric label="Transcript segments" value={formatNumber(report.segments.length)} />
              <Metric label="Transcript words" value={valueOrDash(lexicalStats?.words)} />
              <Metric
                label="Unique terms"
                value={valueOrDash(lexicalSummary?.unique_terms ?? lexicalSummary?.uniqueTerms)}
              />
              <Metric
                label="Lexical diversity"
                value={formatRatio(
                  lexicalSummary?.lexical_diversity ?? lexicalSummary?.lexicalDiversity,
                )}
              />
              <Metric
                label="Avg. sentence words"
                value={formatDecimal(
                  readability?.average_sentence_words ?? readability?.averageSentenceWords,
                )}
              />
            </div>
            <dl className="metadata-list">
              <MetadataRow label="Channel" value={report.video.channel ?? report.video.uploader} />
              <MetadataRow label="Availability" value={report.video.availability} />
              <MetadataRow label="Transcript source" value="audio-analysis WebGPU ASR" />
            </dl>
          </section>

          {report.coverage.lexical ? (
            <section>
              <div className="section-heading">
                <div>
                  <span>Language</span>
                  <h2>Transcript analysis</h2>
                </div>
                <span className="muted-label">{primaryStreamLabel(report)}</span>
              </div>

              {summarySentences.length > 0 ? (
                <div className="summary-block">
                  <h3>Extractive summary</h3>
                  <ol>
                    {summarySentences.map((sentence, index) => (
                      <li key={`${readString(sentence.text) ?? "summary"}-${index}`}>
                        {readString(sentence.text)}
                      </li>
                    ))}
                  </ol>
                </div>
              ) : null}

              <div className="analysis-columns">
                <AnalysisList title="Keywords" items={keywords.map(termLabel).filter(Boolean)} />
                <AnalysisList
                  title="Key phrases"
                  items={phrases.map(phraseLabel).filter(Boolean)}
                />
                <AnalysisList
                  title="Rule entities"
                  items={entities.map(entityLabel).filter(Boolean)}
                />
                <div className="analysis-card">
                  <h3>Sentiment</h3>
                  <p className="analysis-emphasis">{sentimentLabel(sentiment)}</p>
                  <p className="muted-label">Deterministic lexicon signal, not a model judgment.</p>
                </div>
              </div>
            </section>
          ) : null}

          <section>
            <div className="section-heading">
              <div>
                <span>Transcript</span>
                <h2>Timestamped audio-analysis evidence</h2>
              </div>
              <span className="muted-label">{formatNumber(report.segments.length)} segments</span>
            </div>
            {report.segments.length > 0 ? (
              <div className="transcript-list">
                {report.segments.map((segment) => (
                  <article key={segment.segmentId} className="transcript-segment">
                    <a
                      className="timestamp"
                      href={`${report.video.sourceUrl}&t=${Math.floor(segment.startSeconds ?? 0)}s`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {formatTimestamp(segment.startSeconds)}
                    </a>
                    <p>{segment.text}</p>
                  </article>
                ))}
              </div>
            ) : (
              <p className="empty-state">No speech was detected in the shared audio.</p>
            )}
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Provenance</span>
                <h2>Browser transcript and analysis evidence</h2>
              </div>
            </div>
            <div className="stream-list">
              {report.streams.map((stream) => (
                <div key={stream.streamId} className="stream-row">
                  <div>
                    <strong>{sourceKindLabel(stream.sourceKind)}</strong>
                    <span>{stream.language ?? "language detected by model"}</span>
                  </div>
                  <div>
                    <span>{formatNumber(stream.segmentCount)} segments</span>
                    <span>{stream.status}</span>
                  </div>
                  {stream.message ? <p>{stream.message}</p> : null}
                </div>
              ))}
            </div>
            <details className="raw-details">
              <summary>Raw browser metadata JSON</summary>
              <pre>{JSON.stringify(report.video.metadata, null, 2)}</pre>
            </details>
            {report.lexicalAnalysis ? (
              <details className="raw-details">
                <summary>Raw nlp-stack analysis JSON</summary>
                <pre>{JSON.stringify(report.lexicalAnalysis, null, 2)}</pre>
              </details>
            ) : null}
          </section>
        </div>
      ) : null}

      <footer className="analyzer-footer">
        <span>
          Static GitHub Pages · browser-owned audio capture · audio-analysis WebGPU transcription ·
          nlp-stack Rust/Wasm analysis · no corpus backend
        </span>
        <a href="https://github.com/moritzbrantner/youtube-corpus">View source</a>
      </footer>
    </main>
  );
}

function PipelineStep({ label, active }: { label: string; active: boolean }) {
  return (
    <li className={active ? "pipeline-active" : ""}>
      <span aria-hidden="true">{active ? "✓" : "·"}</span>
      {label}
    </li>
  );
}

function CoverageCard({
  label,
  available,
  detail,
}: {
  label: string;
  available: boolean;
  detail: string;
}) {
  return (
    <article className={available ? "coverage-card coverage-yes" : "coverage-card"}>
      <span>{available ? "Available" : "Not produced"}</span>
      <h3>{label}</h3>
      <p>{detail}</p>
    </article>
  );
}

function Metric({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value ?? "—"}</strong>
    </div>
  );
}

function MetadataRow({ label, value }: { label: string; value: string | null | undefined }) {
  if (!value) return null;
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function AnalysisList({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="analysis-card">
      <h3>{title}</h3>
      {items.length ? (
        <ul>
          {items.slice(0, 16).map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      ) : (
        <p className="muted-label">No results.</p>
      )}
    </div>
  );
}

function initialVideoUrl() {
  if (typeof window === "undefined") return "";
  return new URLSearchParams(window.location.search).get("url") ?? "";
}

function persistWorkbenchUrl(sourceUrl: string) {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  url.searchParams.set("url", sourceUrl);
  url.searchParams.delete("backend");
  window.history.replaceState(null, "", url);
}

function phaseForStage(stage: AudioBrowserAnalysisStage): Phase {
  if (stage === "capture") return "capture";
  if (stage === "metadata") return "metadata";
  if (stage === "transcribing") return "transcribing";
  return "analyzing";
}

function phaseLabel(phase: Phase) {
  switch (phase) {
    case "capture":
      return "Audio capture";
    case "metadata":
      return "Metadata";
    case "transcribing":
      return "Transcribing";
    case "analyzing":
      return "Analyzing";
    case "done":
      return "Complete";
    case "error":
      return "Needs attention";
    default:
      return "Ready";
  }
}

function phaseReached(phase: Phase, threshold: "capture" | "transcribing") {
  const order: Phase[] = ["idle", "capture", "metadata", "transcribing", "analyzing", "done"];
  const currentIndex = order.indexOf(phase);
  const thresholdIndex = order.indexOf(threshold);
  return currentIndex >= thresholdIndex && phase !== "error";
}

function primaryStreamLabel(report: VideoAnalysisReport) {
  const stream = report.streams.find((item) => item.streamId === report.primaryStreamId);
  return stream
    ? `${sourceKindLabel(stream.sourceKind)} · ${stream.language ?? "model-detected language"}`
    : "No transcript";
}

function sourceKindLabel(sourceKind: string) {
  if (sourceKind === "asr") return "audio-analysis ASR";
  return sourceKind;
}

function termLabel(item: JsonRecord) {
  const text = readString(item.text ?? item.term);
  const count = readNumber(item.count);
  return text ? (count ? `${text} · ${formatNumber(count)}` : text) : "";
}

function phraseLabel(item: JsonRecord) {
  const text = readString(item.text);
  if (text) return text;
  const terms = Array.isArray(item.terms)
    ? item.terms.filter((value): value is string => typeof value === "string")
    : [];
  return terms.join(" ");
}

function entityLabel(item: JsonRecord) {
  const text = readString(item.text ?? item.value);
  const kind = readString(item.kind ?? item.label ?? item.entity_type ?? item.entityType);
  if (!text) return "";
  return kind ? `${text} · ${kind}` : text;
}

function sentimentLabel(sentiment: JsonRecord | null) {
  if (!sentiment) return "No signal";
  const label = readString(sentiment.label ?? sentiment.sentiment);
  const score = readNumber(sentiment.score);
  if (label && score !== null) return `${label} · ${score.toFixed(2)}`;
  if (label) return label;
  if (score !== null) return score.toFixed(2);
  return "Computed";
}

function asRecord(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function asRecordArray(value: unknown): JsonRecord[] {
  return Array.isArray(value)
    ? value.map(asRecord).filter((item): item is JsonRecord => item !== null)
    : [];
}

function readString(value: unknown) {
  return typeof value === "string" ? value : null;
}

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function valueOrDash(value: unknown) {
  const number = readNumber(value);
  if (number !== null) return formatNumber(number);
  const text = readString(value);
  return text ?? "—";
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}

function formatDecimal(value: unknown) {
  const number = readNumber(value);
  return number === null ? "—" : number.toFixed(1);
}

function formatRatio(value: unknown) {
  const number = readNumber(value);
  return number === null ? "—" : number.toFixed(3);
}

function formatDuration(seconds: number | null) {
  if (seconds === null) return "—";
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function formatTimestamp(seconds: number | null) {
  return formatDuration(seconds);
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
