import * as React from "react";

import {
  addSource,
  configureApiBaseUrl,
  getCorpusStatus,
  getIngestRun,
  getVideoAnalysis,
  type VideoAnalysisReport,
} from "../../api";
import { parseYouTubeVideoUrl } from "./youtube-url";

type Phase = "idle" | "connecting" | "ingesting" | "analyzing" | "done" | "error";
type BackendState = "checking" | "ready" | "unavailable";
type JsonRecord = Record<string, unknown>;

export function YouTubeAnalyzer() {
  const [sourceUrl, setSourceUrl] = React.useState(initialVideoUrl);
  const [backendUrl, setBackendUrl] = React.useState(initialBackendUrl);
  const [backendState, setBackendState] = React.useState<BackendState>("checking");
  const [backendMessage, setBackendMessage] = React.useState("Checking analysis backend…");
  const [useAsr, setUseAsr] = React.useState(false);
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

  React.useEffect(() => {
    void probeBackend(backendUrl);
  }, []);

  async function probeBackend(candidate: string) {
    setBackendState("checking");
    setBackendMessage("Checking analysis backend…");
    configureApiBaseUrl(candidate);
    try {
      const status = await getCorpusStatus();
      if (!status.configured) {
        throw new Error("The backend is running without DATABASE_URL.");
      }
      if (!status.reachable) {
        throw new Error(status.message ?? "The configured database is not reachable.");
      }
      if (!status.schemaReady) {
        throw new Error(status.message ?? "The corpus schema is not ready. Start the backend with --migrate.");
      }
      setBackendState("ready");
      setBackendMessage(
        status.stats
          ? `Ready · ${formatNumber(status.stats.videos)} videos · ${formatNumber(status.stats.segments)} transcript segments`
          : "Ready",
      );
      return true;
    } catch (caught) {
      setBackendState("unavailable");
      setBackendMessage(errorMessage(caught));
      return false;
    }
  }

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

    try {
      setPhase("connecting");
      setProgressMessage("Connecting to the corpus analysis backend…");
      const ready = await probeBackend(backendUrl);
      if (!ready) {
        throw new Error(
          isGitHubPages()
            ? "Start the local youtube-corpus backend, then retry. The static Pages site cannot run yt-dlp or Postgres itself."
            : "The youtube-corpus backend is not ready.",
        );
      }

      persistWorkbenchUrl(parsed.canonicalUrl, backendUrl);
      setPhase("ingesting");
      setProgressMessage("Fetching metadata and captions, normalizing the transcript, and indexing it…");
      const ingest = await addSource({
        sourceKind: "video",
        sourceUrl: parsed.canonicalUrl,
        captionLanguages: ["all"],
        captionsEnabled: true,
        autoCaptionsEnabled: true,
        asrEnabled: useAsr,
        ingestNow: true,
        async: true,
        migrate: false,
      });

      if (ingest.jobId) {
        await waitForIngest(ingest.jobId, (message) => setProgressMessage(message));
      } else if (ingest.ingest && ingest.ingest.items.some((item) => item.status === "failed")) {
        const failed = ingest.ingest.items.find((item) => item.status === "failed");
        throw new Error(failed?.message ?? "Video ingestion failed.");
      }

      setPhase("analyzing");
      setProgressMessage("Composing metadata, transcript, lexical, and multimodal evidence…");
      const nextReport = await getVideoAnalysis(parsed.canonicalUrl);
      setReport(nextReport);
      setPhase("done");
      setProgressMessage("Analysis complete.");
    } catch (caught) {
      setPhase("error");
      setError(errorMessage(caught));
      setProgressMessage("Analysis stopped.");
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
          <h1>Analyze a YouTube video from one URL</h1>
          <p>
            Paste a public YouTube URL. The corpus backend retrieves metadata and captions, preserves provenance,
            indexes the transcript, and runs deterministic NLP analysis. Existing multimodal evidence is surfaced when
            available.
          </p>
        </div>
        <nav className="analyzer-nav" aria-label="YouTube Corpus views">
          <a href="#corpora">Research corpora</a>
          <a href="#legacy">Advanced local UI</a>
        </nav>
      </header>

      <section className="runtime-strip" aria-label="Runtime status">
        <span className={`runtime-dot runtime-dot-${backendState}`} aria-hidden="true" />
        <strong>{isGitHubPages() ? "Static GitHub Pages + corpus backend" : "Corpus workbench"}</strong>
        <span>{backendMessage}</span>
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
            <button type="submit" disabled={phase === "connecting" || phase === "ingesting" || phase === "analyzing"}>
              {phase === "ingesting" || phase === "analyzing" ? "Analyzing…" : "Analyze video"}
            </button>
          </div>
          {sourceUrl && !parsedUrl ? <p className="field-error">This is not a recognized YouTube video URL.</p> : null}

          <div className="analysis-options">
            <label className="checkbox-row">
              <input type="checkbox" checked={useAsr} onChange={(event) => setUseAsr(event.target.checked)} />
              <span>
                <strong>ASR fallback</strong>
                <small>Download media and run Whisper when available. This is off by default because it is heavier.</small>
              </span>
            </label>
            <details>
              <summary>Backend connection</summary>
              <div className="backend-editor">
                <label htmlFor="backend-url">Analysis backend</label>
                <div className="backend-row">
                  <input
                    id="backend-url"
                    type="url"
                    placeholder="Same origin or http://127.0.0.1:1420"
                    value={backendUrl}
                    onChange={(event) => setBackendUrl(event.target.value)}
                  />
                  <button type="button" className="secondary-button" onClick={() => void probeBackend(backendUrl)}>
                    Check
                  </button>
                </div>
                <p>
                  On GitHub Pages the browser only parses the URL and renders the workbench. yt-dlp, transcript
                  persistence, and NLP run through the corpus backend. The default Pages backend is local loopback.
                </p>
              </div>
            </details>
          </div>
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
              <PipelineStep label="Metadata + captions" active={phaseReached(phase, "ingesting")} />
              <PipelineStep label="Transcript index" active={Boolean(report?.coverage.transcript)} />
              <PipelineStep label="Lexical analysis" active={Boolean(report?.coverage.lexical)} />
              <PipelineStep label="Report" active={phase === "done"} />
            </ol>
          </div>
        </section>
      ) : null}

      {error ? <div className="analysis-error" role="alert">{error}</div> : null}

      {report ? (
        <div className="report-stack">
          <section>
            <div className="section-heading">
              <div>
                <span>Coverage</span>
                <h2>What this run actually analyzed</h2>
              </div>
              <a href={report.video.sourceUrl} target="_blank" rel="noreferrer">Open on YouTube</a>
            </div>
            <div className="coverage-grid">
              <CoverageCard label="Metadata" available={report.coverage.metadata} detail="yt-dlp video metadata" />
              <CoverageCard
                label="Transcript"
                available={report.coverage.transcript}
                detail={`${formatNumber(report.video.transcriptSegments)} timed segments`}
              />
              <CoverageCard label="NLP" available={report.coverage.lexical} detail="text-lexical deterministic analysis" />
              <CoverageCard
                label="Media retained"
                available={report.coverage.mediaRetained}
                detail={report.coverage.mediaRetained ? "local media available" : "caption-first ingest"}
              />
              <CoverageCard label="Visual timeline" available={report.coverage.visualTimeline} detail="scene / face evidence" />
              <CoverageCard label="Audio evidence" available={report.coverage.audioFeatures} detail="voice observations" />
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
              <Metric label="Duration" value={report.video.durationString ?? formatDuration(report.video.durationSeconds)} />
              <Metric label="Views" value={formatOptionalNumber(report.video.viewCount)} />
              <Metric label="Likes" value={formatOptionalNumber(report.video.likeCount)} />
              <Metric label="Comments" value={formatOptionalNumber(report.video.commentCount)} />
              <Metric label="Transcript words" value={valueOrDash(lexicalStats?.words)} />
              <Metric label="Unique terms" value={valueOrDash(lexicalSummary?.unique_terms ?? lexicalSummary?.uniqueTerms)} />
              <Metric
                label="Lexical diversity"
                value={formatRatio(lexicalSummary?.lexical_diversity ?? lexicalSummary?.lexicalDiversity)}
              />
              <Metric
                label="Avg. sentence words"
                value={formatDecimal(readability?.average_sentence_words ?? readability?.averageSentenceWords)}
              />
            </div>
            <dl className="metadata-list">
              <MetadataRow label="Channel" value={report.video.channel ?? report.video.uploader} />
              <MetadataRow label="Uploaded" value={report.video.uploadDate} />
              <MetadataRow label="Availability" value={report.video.availability} />
              <MetadataRow label="Live status" value={report.video.liveStatus} />
              <MetadataRow label="Categories" value={report.video.categories.join(", ") || null} />
            </dl>
            {report.video.description ? <p className="video-description">{report.video.description}</p> : null}
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
                      <li key={`${readString(sentence.text) ?? "summary"}-${index}`}>{readString(sentence.text)}</li>
                    ))}
                  </ol>
                </div>
              ) : null}

              <div className="analysis-columns">
                <AnalysisList title="Keywords" items={keywords.map(termLabel).filter(Boolean)} />
                <AnalysisList title="Key phrases" items={phrases.map(phraseLabel).filter(Boolean)} />
                <AnalysisList title="Rule entities" items={entities.map(entityLabel).filter(Boolean)} />
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
                <h2>Timestamped evidence</h2>
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
              <p className="empty-state">
                No usable transcript was returned. Enable ASR fallback when the backend has Whisper installed, or retry a
                video with captions.
              </p>
            )}
          </section>

          <section>
            <div className="section-heading">
              <div>
                <span>Provenance</span>
                <h2>Transcript streams and raw metadata</h2>
              </div>
            </div>
            <div className="stream-list">
              {report.streams.map((stream) => (
                <div key={stream.streamId} className="stream-row">
                  <div>
                    <strong>{sourceKindLabel(stream.sourceKind)}</strong>
                    <span>{stream.language ?? "unknown language"}</span>
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
              <summary>Raw metadata JSON</summary>
              <pre>{JSON.stringify(report.video.metadata, null, 2)}</pre>
            </details>
            {report.lexicalAnalysis ? (
              <details className="raw-details">
                <summary>Raw lexical analysis JSON</summary>
                <pre>{JSON.stringify(report.lexicalAnalysis, null, 2)}</pre>
              </details>
            ) : null}
          </section>
        </div>
      ) : null}

      <footer className="analyzer-footer">
        <span>Static GitHub Pages workbench · corpus-owned ingestion and persistence · nlp-stack lexical analysis</span>
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
        <ul>{items.slice(0, 16).map((item) => <li key={item}>{item}</li>)}</ul>
      ) : (
        <p className="muted-label">No results.</p>
      )}
    </div>
  );
}

async function waitForIngest(jobId: string, onProgress: (message: string) => void) {
  for (let attempt = 0; attempt < 900; attempt += 1) {
    const run = await getIngestRun(jobId);
    if (run.status === "completed" || run.job?.status === "succeeded") {
      return;
    }
    if (run.status === "failed" || run.job?.status === "failed" || run.job?.status === "cancelled") {
      throw new Error(run.job?.failure?.message ?? "Video ingestion failed.");
    }
    const progress = run.job?.progress;
    onProgress(
      progress?.message ??
        `Ingesting video · ${formatNumber(run.videosIndexed)} indexed · ${formatNumber(run.segmentsIndexed)} segments`,
    );
    await delay(1000);
  }
  throw new Error("The ingest is still running after 15 minutes. Check the backend ingest run for details.");
}

function initialBackendUrl() {
  if (typeof window === "undefined") return "";
  const configured = new URLSearchParams(window.location.search).get("backend");
  if (configured) return configured;
  return isGitHubPages() ? "http://127.0.0.1:1420" : "";
}

function initialVideoUrl() {
  if (typeof window === "undefined") return "";
  return new URLSearchParams(window.location.search).get("url") ?? "";
}

function persistWorkbenchUrl(sourceUrl: string, backendUrl: string) {
  if (typeof window === "undefined") return;
  const url = new URL(window.location.href);
  url.searchParams.set("url", sourceUrl);
  if (backendUrl) url.searchParams.set("backend", backendUrl);
  else url.searchParams.delete("backend");
  window.history.replaceState(null, "", url);
}

function isGitHubPages() {
  return typeof window !== "undefined" && window.location.hostname === "moritzbrantner.github.io";
}

function phaseLabel(phase: Phase) {
  switch (phase) {
    case "connecting":
      return "Connecting";
    case "ingesting":
      return "Ingesting";
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

function phaseReached(phase: Phase, threshold: "ingesting") {
  if (threshold === "ingesting") return ["ingesting", "analyzing", "done"].includes(phase);
  return false;
}

function primaryStreamLabel(report: VideoAnalysisReport) {
  const stream = report.streams.find((item) => item.streamId === report.primaryStreamId);
  return stream ? `${sourceKindLabel(stream.sourceKind)} · ${stream.language ?? "unknown language"}` : "No transcript";
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

function phraseLabel(item: JsonRecord) {
  const text = readString(item.text);
  if (text) return text;
  const terms = Array.isArray(item.terms) ? item.terms.filter((value): value is string => typeof value === "string") : [];
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
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as JsonRecord) : null;
}

function asRecordArray(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.map(asRecord).filter((item): item is JsonRecord => item !== null) : [];
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

function delay(milliseconds: number) {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}
