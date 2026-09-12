import * as React from "react";

import type { VideoAnalysisReport } from "../../api";
import { buildBrowserVideoAnalysis } from "./browser-analysis";
import { parseYouTubeVideoUrl } from "./youtube-url";
import "./browser-analysis.css";

type Phase = "idle" | "analyzing" | "done" | "error";
type JsonRecord = Record<string, unknown>;

export function shouldUseBrowserAnalyzer() {
  if (typeof window === "undefined") return false;
  const requestedMode = new URLSearchParams(window.location.search).get("mode");
  return requestedMode === "browser" || window.location.hostname.endsWith("github.io");
}

export function BrowserYouTubeAnalyzer() {
  const [sourceUrl, setSourceUrl] = React.useState(initialVideoUrl);
  const [transcriptText, setTranscriptText] = React.useState("");
  const [transcriptFileName, setTranscriptFileName] = React.useState<string | null>(null);
  const [phase, setPhase] = React.useState<Phase>("idle");
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

  function analyze(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setReport(null);

    const parsed = parseYouTubeVideoUrl(sourceUrl);
    if (!parsed) {
      setPhase("error");
      setError("Enter a valid YouTube video URL (watch, youtu.be, Shorts, live, or embed URL).");
      return;
    }

    try {
      setPhase("analyzing");
      const nextReport = buildBrowserVideoAnalysis(parsed, transcriptText);
      persistWorkbenchUrl(parsed.canonicalUrl);
      setReport(nextReport);
      setPhase("done");
    } catch (caught) {
      setPhase("error");
      setError(errorMessage(caught));
    }
  }

  async function loadTranscriptFile(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      setTranscriptText(text);
      setTranscriptFileName(file.name);
      setReport(null);
      setError(null);
      setPhase("idle");
    } catch (caught) {
      setError(`Could not read transcript file: ${errorMessage(caught)}`);
      setPhase("error");
    } finally {
      event.currentTarget.value = "";
    }
  }

  const preview = report?.video.youtubeId
    ? parseYouTubeVideoUrl(`https://www.youtube.com/watch?v=${report.video.youtubeId}`)
    : parsedUrl;

  return (
    <main className="analyzer-shell">
      <header className="analyzer-header">
        <div>
          <a className="analyzer-kicker" href="https://github.com/moritzbrantner/youtube-corpus">
            youtube-corpus
          </a>
          <h1>Analyze a YouTube transcript in the browser</h1>
          <p>
            Paste a YouTube URL and its transcript, or load a WebVTT, SRT, or text file. GitHub
            Pages parses and analyzes the transcript locally without a server, Postgres, or stored
            media.
          </p>
        </div>
        <nav className="analyzer-nav" aria-label="YouTube Corpus views">
          <a href="https://github.com/moritzbrantner/youtube-corpus">Source and local ingest</a>
        </nav>
      </header>

      <section className="runtime-strip" aria-label="Runtime status">
        <span className="runtime-dot runtime-dot-ready" aria-hidden="true" />
        <strong>Standalone GitHub Pages</strong>
        <span>Browser analysis ready · no backend or database required</span>
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
              onChange={(event) => {
                setSourceUrl(event.target.value);
                setReport(null);
                setPhase("idle");
              }}
            />
            <button type="submit" disabled={phase === "analyzing"}>
              {phase === "analyzing" ? "Analyzing…" : "Analyze transcript"}
            </button>
          </div>
          {sourceUrl && !parsedUrl ? (
            <p className="field-error">This is not a recognized YouTube video URL.</p>
          ) : null}

          <div className="browser-transcript-editor">
            <div className="browser-transcript-heading">
              <label htmlFor="transcript-input">Transcript or captions</label>
              <span>{formatNumber(transcriptText.length)} characters</span>
            </div>
            <textarea
              id="transcript-input"
              value={transcriptText}
              onChange={(event) => {
                setTranscriptText(event.target.value);
                setTranscriptFileName(null);
                setReport(null);
                setPhase("idle");
              }}
              placeholder={
                "Paste plain transcript text, WebVTT, or SRT here. Timed caption files preserve timestamps."
              }
              spellCheck={false}
            />
            <div className="browser-transcript-actions">
              <label className="browser-file-button">
                Load transcript file
                <input
                  type="file"
                  accept=".vtt,.srt,.txt,text/plain,text/vtt,application/x-subrip"
                  onChange={(event) => void loadTranscriptFile(event)}
                />
              </label>
              <span>
                {transcriptFileName ??
                  "Nothing is uploaded; the selected file is read directly by your browser."}
              </span>
            </div>
          </div>

          <p className="browser-analysis-note">
            Pure GitHub Pages cannot reliably download public YouTube caption tracks itself. The
            local Rust mode remains the authoritative path for yt-dlp ingestion, automatic captions,
            ASR, Postgres search, and multimodal evidence.
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
            <h2>{report?.video.youtubeId ?? "Browser transcript analysis"}</h2>
            <p>
              {phase === "done"
                ? "Analysis complete in this browser."
                : "Load transcript evidence, then run deterministic browser analysis."}
            </p>
            <ol className="pipeline-list">
              <PipelineStep label="URL + privacy-enhanced preview" active={Boolean(preview)} />
              <PipelineStep label="Transcript imported" active={Boolean(transcriptText.trim())} />
              <PipelineStep
                label="Timestamp parsing"
                active={Boolean(report?.coverage.transcript)}
              />
              <PipelineStep
                label="Browser lexical analysis"
                active={Boolean(report?.coverage.lexical)}
              />
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
                <h2>What the static site analyzed</h2>
              </div>
              <a href={report.video.sourceUrl} target="_blank" rel="noreferrer">
                Open on YouTube
              </a>
            </div>
            <div className="coverage-grid">
              <CoverageCard label="URL" available detail="canonical YouTube video identity" />
              <CoverageCard
                label="Transcript"
                available={report.coverage.transcript}
                detail={`${formatNumber(report.video.transcriptSegments)} imported segments`}
              />
              <CoverageCard
                label="Lexical"
                available={report.coverage.lexical}
                detail="browser-local deterministic analysis"
              />
              <CoverageCard label="Metadata" available={false} detail="not fetched by Pages" />
              <CoverageCard label="Visual" available={false} detail="requires local ingest" />
              <CoverageCard label="Audio" available={false} detail="requires local ingest" />
            </div>
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Transcript</span>
                <h2>{report.video.youtubeId}</h2>
              </div>
            </div>
            <div className="metric-grid">
              <Metric label="Segments" value={formatNumber(report.segments.length)} />
              <Metric label="Words" value={valueOrDash(lexicalStats?.words)} />
              <Metric label="Unique terms" value={valueOrDash(lexicalSummary?.uniqueTerms)} />
              <Metric
                label="Lexical diversity"
                value={formatRatio(lexicalSummary?.lexicalDiversity)}
              />
              <Metric
                label="Avg. sentence words"
                value={formatDecimal(readability?.averageSentenceWords)}
              />
              <Metric label="Sentiment" value={sentimentLabel(sentiment)} />
            </div>
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Language</span>
                <h2>Browser lexical analysis</h2>
              </div>
              <span className="muted-label">Advisory static-site engine</span>
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
              <AnalysisList title="Key phrases" items={phrases.map(termLabel).filter(Boolean)} />
              <AnalysisList
                title="Rule entities"
                items={entities.map(entityLabel).filter(Boolean)}
              />
              <div className="analysis-card">
                <h3>Sentiment</h3>
                <p className="analysis-emphasis">{sentimentLabel(sentiment)}</p>
                <p className="muted-label">Small deterministic lexicon signal, not model output.</p>
              </div>
            </div>
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Evidence</span>
                <h2>Timestamped transcript</h2>
              </div>
              <span className="muted-label">{formatNumber(report.segments.length)} segments</span>
            </div>
            <div className="transcript-list">
              {report.segments.map((segment) => (
                <article key={segment.segmentId} className="transcript-segment">
                  {segment.startSeconds !== null ? (
                    <a
                      className="timestamp"
                      href={`${report.video.sourceUrl}&t=${Math.floor(segment.startSeconds)}s`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {formatTimestamp(segment.startSeconds)}
                    </a>
                  ) : (
                    <span className="timestamp">—</span>
                  )}
                  <p>{segment.text}</p>
                </article>
              ))}
            </div>
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Provenance</span>
                <h2>Portable browser evidence</h2>
              </div>
            </div>
            <p className="browser-provenance">
              This report used only the URL and transcript text supplied to this page. It did not
              contact a youtube-corpus backend, write to Postgres, retain media, or claim the
              nlp-stack lexical implementation.
            </p>
            <details className="raw-details">
              <summary>Raw browser lexical analysis JSON</summary>
              <pre>{JSON.stringify(report.lexicalAnalysis, null, 2)}</pre>
            </details>
          </section>
        </div>
      ) : null}

      <footer className="analyzer-footer">
        <span>
          Static GitHub Pages analyzer · browser-local transcript processing · no persistence
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

function phaseLabel(phase: Phase) {
  switch (phase) {
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

function termLabel(item: JsonRecord) {
  const text = readString(item.text ?? item.term);
  const count = readNumber(item.count);
  return text ? (count ? `${text} · ${formatNumber(count)}` : text) : "";
}

function entityLabel(item: JsonRecord) {
  const text = readString(item.text ?? item.value);
  const kind = readString(item.kind ?? item.label ?? item.entityType);
  if (!text) return "";
  return kind ? `${text} · ${kind}` : text;
}

function sentimentLabel(sentiment: JsonRecord | null) {
  if (!sentiment) return "No signal";
  const label = readString(sentiment.label);
  const score = readNumber(sentiment.score);
  if (label && score !== null) return `${label} · ${score.toFixed(2)}`;
  return label ?? "No signal";
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

function formatTimestamp(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
