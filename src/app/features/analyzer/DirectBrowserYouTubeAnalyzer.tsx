import * as React from "react";

import type { VideoAnalysisReport } from "../../api";
import { applyBrowserCaptionAcquisition } from "./browser-acquisition-report";
import { buildBrowserVideoAnalysis } from "./browser-analysis";
import {
  acquireYouTubeCaptions,
  BrowserYouTubeAcquisitionError,
  type BrowserCaptionAcquisition,
} from "./youtube-browser-extractor";
import { parseYouTubeVideoUrl } from "./youtube-url";
import "./browser-analysis.css";

type Phase = "idle" | "fetching" | "analyzing" | "done" | "error";
type JsonRecord = Record<string, unknown>;

export function shouldUseDirectBrowserAnalyzer() {
  if (typeof window === "undefined") return false;
  const requestedMode = new URLSearchParams(window.location.search).get("mode");
  return requestedMode === "browser" || window.location.hostname.endsWith("github.io");
}

export function DirectBrowserYouTubeAnalyzer() {
  const [sourceUrl, setSourceUrl] = React.useState(initialVideoUrl);
  const [transcriptText, setTranscriptText] = React.useState("");
  const [transcriptFileName, setTranscriptFileName] = React.useState<string | null>(null);
  const [acquisition, setAcquisition] = React.useState<BrowserCaptionAcquisition | null>(null);
  const [phase, setPhase] = React.useState<Phase>("idle");
  const [statusMessage, setStatusMessage] = React.useState(
    "Paste a YouTube URL. Pages will try to fetch its public captions directly.",
  );
  const [error, setError] = React.useState<string | null>(null);
  const [report, setReport] = React.useState<VideoAnalysisReport | null>(null);

  const parsedUrl = React.useMemo(() => parseYouTubeVideoUrl(sourceUrl), [sourceUrl]);
  const lexical = asRecord(report?.lexicalAnalysis);
  const lexicalSummary = asRecord(lexical?.summary);
  const lexicalStats = asRecord(lexicalSummary?.stats);
  const keywords = asRecordArray(lexical?.keywords);
  const phrases = asRecordArray(lexical?.phraseKeywords);
  const summarySentences = asRecordArray(lexical?.extractiveSummary);

  async function analyze(event: React.FormEvent) {
    event.preventDefault();
    const parsed = parseYouTubeVideoUrl(sourceUrl);
    if (!parsed) {
      fail("Enter a valid YouTube video URL (watch, youtu.be, Shorts, live, or embed URL).");
      return;
    }

    setError(null);
    setReport(null);
    let text = transcriptText;
    let nextAcquisition = acquisition;

    try {
      if (!text.trim()) {
        setPhase("fetching");
        setStatusMessage("Trying browser-safe YouTube clients and public caption tracks…");
        nextAcquisition = await acquireYouTubeCaptions(parsed);
        text = nextAcquisition.transcriptText;
        setTranscriptText(text);
        setTranscriptFileName(null);
        setAcquisition(nextAcquisition);
      }

      setPhase("analyzing");
      setStatusMessage("Parsing timestamps and running deterministic browser analysis…");
      let nextReport = buildBrowserVideoAnalysis(parsed, text);
      if (nextAcquisition) {
        nextReport = applyBrowserCaptionAcquisition(nextReport, nextAcquisition);
      }
      persistWorkbenchUrl(parsed.canonicalUrl);
      setReport(nextReport);
      setPhase("done");
      setStatusMessage(
        nextAcquisition
          ? `Analysis complete · ${nextAcquisition.track.name} · ${nextAcquisition.client}`
          : "Analysis complete from the transcript supplied in this browser.",
      );
    } catch (caught) {
      fail(acquisitionErrorMessage(caught));
    }
  }

  async function fetchCaptions() {
    const parsed = parseYouTubeVideoUrl(sourceUrl);
    if (!parsed) {
      fail("Enter a valid YouTube video URL before fetching captions.");
      return;
    }

    setError(null);
    setReport(null);
    setPhase("fetching");
    setStatusMessage("Trying browser-safe YouTube clients and public caption tracks…");
    try {
      const next = await acquireYouTubeCaptions(parsed);
      setAcquisition(next);
      setTranscriptText(next.transcriptText);
      setTranscriptFileName(null);
      setPhase("idle");
      setStatusMessage(
        `Fetched ${next.track.name} (${next.track.languageCode}) via ${next.client}. Ready to analyze.`,
      );
    } catch (caught) {
      fail(acquisitionErrorMessage(caught));
    }
  }

  async function loadTranscriptFile(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      setTranscriptText(text);
      setTranscriptFileName(file.name);
      setAcquisition(null);
      setReport(null);
      setError(null);
      setPhase("idle");
      setStatusMessage("Local transcript loaded. It will be analyzed without any network request.");
    } catch (caught) {
      fail(`Could not read transcript file: ${errorMessage(caught)}`);
    } finally {
      event.currentTarget.value = "";
    }
  }

  function fail(message: string) {
    setPhase("error");
    setError(message);
    setStatusMessage("Analysis stopped.");
  }

  const preview = report?.video.youtubeId
    ? parseYouTubeVideoUrl(`https://www.youtube.com/watch?v=${report.video.youtubeId}`)
    : parsedUrl;
  const busy = phase === "fetching" || phase === "analyzing";

  return (
    <main className="analyzer-shell">
      <header className="analyzer-header">
        <div>
          <a className="analyzer-kicker" href="https://github.com/moritzbrantner/youtube-corpus">
            youtube-corpus
          </a>
          <h1>Analyze YouTube captions directly on GitHub Pages</h1>
          <p>
            Paste a public YouTube URL. The page first tries YouTube's browser-facing player API and
            signed caption tracks, then parses the result with the Rust/WASM extraction core. No
            youtube-corpus server or Postgres instance is required.
          </p>
        </div>
        <nav className="analyzer-nav" aria-label="YouTube Corpus views">
          <a href="https://github.com/moritzbrantner/youtube-corpus">Local yt-dlp mode</a>
        </nav>
      </header>

      <section className="runtime-strip" aria-label="Runtime status">
        <span
          className={`runtime-dot ${phase === "error" ? "runtime-dot-unavailable" : "runtime-dot-ready"}`}
          aria-hidden="true"
        />
        <strong>Standalone Pages + Rust/WASM</strong>
        <span>{statusMessage}</span>
      </section>

      <section className="analyzer-input-panel">
        <form onSubmit={(event) => void analyze(event)} className="analyzer-form">
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
                setAcquisition(null);
                setReport(null);
                setPhase("idle");
                setError(null);
              }}
            />
            <button type="submit" disabled={busy}>
              {phase === "fetching"
                ? "Fetching captions…"
                : phase === "analyzing"
                  ? "Analyzing…"
                  : transcriptText.trim()
                    ? "Analyze transcript"
                    : "Fetch + analyze"}
            </button>
          </div>
          {sourceUrl && !parsedUrl ? (
            <p className="field-error">This is not a recognized YouTube video URL.</p>
          ) : null}

          <div className="browser-transcript-editor">
            <div className="browser-transcript-heading">
              <label htmlFor="transcript-input">Transcript evidence</label>
              <span>{formatNumber(transcriptText.length)} characters</span>
            </div>
            <textarea
              id="transcript-input"
              value={transcriptText}
              onChange={(event) => {
                setTranscriptText(event.target.value);
                setTranscriptFileName(null);
                setAcquisition(null);
                setReport(null);
                setPhase("idle");
              }}
              placeholder="Normally this fills automatically. You can still paste plain text, WebVTT, or SRT when YouTube blocks direct browser acquisition."
              spellCheck={false}
            />
            <div className="browser-transcript-actions">
              <div className="browser-action-group">
                <button
                  type="button"
                  className="secondary-button browser-fetch-button"
                  disabled={!parsedUrl || busy}
                  onClick={() => void fetchCaptions()}
                >
                  Fetch from YouTube
                </button>
                <label className="browser-file-button">
                  Load transcript file
                  <input
                    type="file"
                    accept=".vtt,.srt,.txt,text/plain,text/vtt,application/x-subrip"
                    onChange={(event) => void loadTranscriptFile(event)}
                  />
                </label>
              </div>
              <span>
                {acquisition
                  ? `${acquisition.track.name} · ${acquisition.track.sourceKind === "caption_auto" ? "automatic" : "manual"} · ${acquisition.client}`
                  : transcriptFileName ?? "Automatic acquisition is attempted before manual fallback."}
              </span>
            </div>
          </div>

          <p className="browser-analysis-note">
            Direct acquisition is best-effort and fail-closed. Some videos still require YouTube
            proof-of-origin, authentication, or a same-origin context. In those cases this page does
            not route your signed caption URL through a third-party proxy; paste/import a transcript
            or use the local yt-dlp pipeline instead.
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
            <h2>{report?.video.title ?? report?.video.youtubeId ?? "Browser YouTube analysis"}</h2>
            <p>{statusMessage}</p>
            <ol className="pipeline-list">
              <PipelineStep label="YouTube URL" active={Boolean(parsedUrl)} />
              <PipelineStep label="Caption evidence" active={Boolean(transcriptText.trim())} />
              <PipelineStep label="Rust/WASM extraction" active={Boolean(acquisition)} />
              <PipelineStep label="Browser lexical analysis" active={Boolean(report?.coverage.lexical)} />
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

      {report ? <BrowserReport report={report} acquisition={acquisition} /> : null}

      <footer className="analyzer-footer">
        <span>Static GitHub Pages · direct YouTube captions when allowed · Rust/WASM parsing</span>
        <a href="https://github.com/moritzbrantner/youtube-corpus">View source</a>
      </footer>
    </main>
  );
}

function BrowserReport({
  report,
  acquisition,
}: {
  report: VideoAnalysisReport;
  acquisition: BrowserCaptionAcquisition | null;
}) {
  const lexical = asRecord(report.lexicalAnalysis);
  const summary = asRecord(lexical?.summary);
  const stats = asRecord(summary?.stats);
  const keywords = asRecordArray(lexical?.keywords);
  const phrases = asRecordArray(lexical?.phraseKeywords);
  const summarySentences = asRecordArray(lexical?.extractiveSummary);

  return (
    <div className="report-stack">
      <section>
        <div className="section-heading">
          <div>
            <span>Coverage</span>
            <h2>What Pages actually acquired</h2>
          </div>
          <a href={report.video.sourceUrl} target="_blank" rel="noreferrer">
            Open on YouTube
          </a>
        </div>
        <div className="coverage-grid">
          <CoverageCard label="URL" available detail="canonical video identity" />
          <CoverageCard
            label="Captions"
            available={report.coverage.transcript}
            detail={`${formatNumber(report.video.transcriptSegments)} timed segments`}
          />
          <CoverageCard
            label="Direct fetch"
            available={Boolean(acquisition)}
            detail={acquisition ? `${acquisition.client} · ${acquisition.track.languageCode}` : "manual fallback"}
          />
          <CoverageCard
            label="Metadata"
            available={report.coverage.metadata}
            detail={report.video.channel ?? "player metadata"}
          />
          <CoverageCard label="NLP" available={report.coverage.lexical} detail="browser-local analysis" />
          <CoverageCard label="Media" available={false} detail="not downloaded by Pages" />
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
          <Metric label="Duration" value={formatDuration(report.video.durationSeconds)} />
          <Metric label="Views" value={formatOptionalNumber(report.video.viewCount)} />
          <Metric label="Segments" value={formatNumber(report.segments.length)} />
          <Metric label="Words" value={valueOrDash(stats?.words)} />
          <Metric label="Unique terms" value={valueOrDash(summary?.uniqueTerms)} />
          <Metric label="Lexical diversity" value={formatRatio(summary?.lexicalDiversity)} />
        </div>
        {report.video.channel ? <p className="browser-provenance">Channel: {report.video.channel}</p> : null}
        {report.video.description ? (
          <p className="video-description">{report.video.description}</p>
        ) : null}
      </section>

      <section>
        <div className="section-heading">
          <div>
            <span>Language</span>
            <h2>Transcript analysis</h2>
          </div>
          <span className="muted-label">
            {report.streams[0]
              ? `${sourceKindLabel(report.streams[0].sourceKind)} · ${report.streams[0].language ?? "unknown language"}`
              : "No stream"}
          </span>
        </div>
        {summarySentences.length ? (
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
            <h2>Browser extraction boundary</h2>
          </div>
        </div>
        <p className="browser-provenance">
          {acquisition
            ? `The browser requested an InnerTube player response, selected ${acquisition.track.name}, fetched its signed timed-text URL directly from YouTube, and converted json3 to WebVTT in Rust/WASM. No youtube-corpus server, Postgres database, or third-party CORS relay was used.`
            : "The report uses transcript evidence supplied by you. No youtube-corpus server, Postgres database, or third-party relay was used."}
        </p>
        <details className="raw-details">
          <summary>Raw browser analysis metadata</summary>
          <pre>{JSON.stringify(report.video.metadata, null, 2)}</pre>
        </details>
      </section>
    </div>
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

function CoverageCard({ label, available, detail }: { label: string; available: boolean; detail: string }) {
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

function acquisitionErrorMessage(error: unknown) {
  if (error instanceof BrowserYouTubeAcquisitionError) {
    const evidence = error.attempts
      .map((attempt) => `${attempt.client}/${attempt.stage}: ${attempt.detail}`)
      .join(" · ");
    return evidence ? `${error.message} Attempts: ${evidence}` : error.message;
  }
  return errorMessage(error);
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
    case "fetching":
      return "Fetching";
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

function sourceKindLabel(sourceKind: string) {
  if (sourceKind === "caption_manual") return "Manual captions";
  if (sourceKind === "caption_auto") return "Automatic captions";
  if (sourceKind === "asr") return "ASR";
  return sourceKind;
}

function termLabel(item: JsonRecord) {
  const text = readString(item.text ?? item.term);
  const count = readNumber(item.count);
  return text ? (count ? `${text} · ${formatNumber(count)}` : text) : "";
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

function formatOptionalNumber(value: number | null) {
  return value === null ? "—" : formatNumber(value);
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
