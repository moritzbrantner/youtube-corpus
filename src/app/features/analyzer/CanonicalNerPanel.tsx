import * as React from "react";

import type { VideoAnalysisReport } from "../../api";
import {
  anchorEntities,
  CANONICAL_NER_LANGUAGES,
  loadCanonicalNer,
  modelDownloadBytes,
  nerSegments,
  NLP_STACK_REVISION,
  primaryLanguage,
  type AnchoredEntity,
  type CanonicalNer,
  type CanonicalNerResult,
} from "./canonical-ner";

type NerState =
  | { phase: "idle" }
  | { phase: "running"; loaded: number; total: number; startedAt: number }
  | { phase: "done"; result: CanonicalNerResult; entities: AnchoredEntity[]; elapsedMs: number }
  | { phase: "canceled" }
  | { phase: "error"; message: string };

// One adapter per page, created on the first opt-in run. Its worker keeps the loaded model
// across transcripts: a reload from the browser cache costs 2–5 s, a warm run well under 1 s.
// Aborting a request (cancel, or the panel unmounting for a new transcript) terminates the
// worker inside the adapter, and the next request spawns a fresh one.
let sharedNer: Promise<CanonicalNer> | null = null;

function canonicalNer() {
  sharedNer ??= loadCanonicalNer().catch((error: unknown) => {
    sharedNer = null;
    throw error;
  });
  return sharedNer;
}

/**
 * Opt-in canonical NER from NLP Stack, shown next to (never merged with) the advisory
 * lexical preview. Mount it keyed by report so a new transcript drops stale results.
 */
export function CanonicalNerPanel({ report }: { report: VideoAnalysisReport }) {
  const streamLanguage = primaryLanguage(report.streams[0]?.language);
  const [language, setLanguage] = React.useState(streamLanguage ?? "");
  const [state, setState] = React.useState<NerState>({ phase: "idle" });
  const controller = React.useRef<AbortController | null>(null);

  React.useEffect(() => () => controller.current?.abort(), []);

  async function run() {
    controller.current?.abort();
    const request = new AbortController();
    controller.current = request;
    const files = new Map<string, { loaded: number; total: number }>();
    const startedAt = performance.now();
    setState({ phase: "running", loaded: 0, total: 0, startedAt });

    try {
      const ner = await canonicalNer();
      const result = await ner.analyze(
        { language: language || null, segments: nerSegments(report.segments) },
        {
          signal: request.signal,
          onProgress: ({ detail }) => {
            const file = typeof detail.file === "string" ? detail.file : null;
            const loaded = typeof detail.loaded === "number" ? detail.loaded : null;
            const total = typeof detail.total === "number" ? detail.total : null;
            if (!file || loaded === null || total === null) return;
            files.set(file, { loaded, total });
            if (controller.current !== request) return;
            setState({ phase: "running", ...modelDownloadBytes(files), startedAt });
          },
        },
      );
      if (controller.current !== request) return;
      setState({
        phase: "done",
        result,
        entities: anchorEntities(result, report.segments),
        elapsedMs: performance.now() - startedAt,
      });
    } catch (caught) {
      if (controller.current !== request) return;
      setState(
        request.signal.aborted
          ? { phase: "canceled" }
          : { phase: "error", message: caught instanceof Error ? caught.message : String(caught) },
      );
    }
  }

  function cancel() {
    controller.current?.abort();
  }

  const running = state.phase === "running";

  return (
    <section aria-labelledby="canonical-ner-heading">
      <div className="section-heading">
        <div>
          <span>NLP Stack</span>
          <h2 id="canonical-ner-heading">Canonical named entities</h2>
        </div>
        <span className="muted-label">Opt-in · model download on request</span>
      </div>
      <p className="browser-provenance">
        Runs NLP Stack&apos;s model-backed browser NER on this transcript. Nothing is downloaded
        until you start it: the first run fetches the pinned model and its runtime (about 140 MB)
        from Hugging Face and jsDelivr and caches them in this browser. The lexical analysis above
        stays advisory and is not merged with these results. No sentiment model is offered.
      </p>
      <div className="canonical-ner-controls">
        <label htmlFor="canonical-ner-language">Transcript language</label>
        <select
          id="canonical-ner-language"
          value={language}
          disabled={running}
          onChange={(event) => setLanguage(event.target.value)}
        >
          <option value="">Choose…</option>
          {CANONICAL_NER_LANGUAGES.map((code) => (
            <option key={code} value={code}>
              {languageName(code)}
            </option>
          ))}
          {streamLanguage && !isSupported(streamLanguage) ? (
            <option value={streamLanguage}>{streamLanguage} (unsupported)</option>
          ) : null}
        </select>
        {running ? (
          <button type="button" className="secondary-button" onClick={cancel}>
            Cancel
          </button>
        ) : (
          <button
            type="button"
            disabled={!language || report.segments.length === 0}
            onClick={() => void run()}
          >
            {state.phase === "done" || state.phase === "error" || state.phase === "canceled"
              ? "Run again"
              : "Run canonical NER"}
          </button>
        )}
      </div>
      <NerStatus state={state} />
      {state.phase === "done" && state.result.status === "completed" ? (
        <NerResults report={report} state={state} />
      ) : null}
    </section>
  );
}

function NerStatus({ state }: { state: NerState }) {
  switch (state.phase) {
    case "idle":
      return null;
    case "running":
      return (
        <p className="browser-provenance" role="status">
          {state.total > 0
            ? `Loading model… ${formatBytes(state.loaded)} of ${formatBytes(state.total)}`
            : "Loading NLP Stack module and model…"}
        </p>
      );
    case "canceled":
      return (
        <p className="browser-provenance" role="status">
          Canceled. No partial result is shown.
        </p>
      );
    case "error":
      return (
        <p className="analysis-error" role="alert">
          Canonical NER is unavailable: {state.message}
        </p>
      );
    case "done":
      return state.result.status === "unsupported-language" ? (
        <p className="browser-provenance" role="status">
          NLP Stack&apos;s browser NER does not support language &quot;
          {state.result.language ?? "unknown"}&quot; (it supports{" "}
          {state.result.model.supportedLanguages.join(", ")}). No model was loaded and no entities
          are claimed.
        </p>
      ) : null;
  }
}

function NerResults({
  report,
  state,
}: {
  report: VideoAnalysisReport;
  state: Extract<NerState, { phase: "done" }>;
}) {
  const { result, entities } = state;
  const truncated = result.segments.filter((segment) => segment.truncated).length;
  return (
    <>
      <p className="browser-provenance" role="status">
        {entities.length} entities in {result.segments.length} segments ·{" "}
        {(state.elapsedMs / 1000).toFixed(1)} s · language {result.language}
        {truncated ? ` · ${truncated} segments truncated by model input length` : ""}
      </p>
      {entities.length ? (
        <ul className="canonical-ner-list">
          {entities.map((entity) => (
            <li key={`${entity.segmentId}:${entity.start}:${entity.end}`}>
              <div className="canonical-ner-entity">
                <strong>{entity.text}</strong>
                <span className="canonical-ner-label">{entity.label}</span>
                <span className="muted-label">confidence {entity.score.toFixed(2)}</span>
                {entity.startSeconds !== null ? (
                  <a
                    className="timestamp"
                    href={`${report.video.sourceUrl}&t=${Math.floor(entity.startSeconds)}s`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {formatTimestamp(entity.startSeconds)}
                  </a>
                ) : null}
              </div>
              <p className="canonical-ner-passage">
                {entity.segmentText.slice(0, entity.start)}
                <mark>{entity.segmentText.slice(entity.start, entity.end)}</mark>
                {entity.segmentText.slice(entity.end)}
              </p>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted-label">The model found no entities in this transcript.</p>
      )}
      <p className="browser-provenance">
        Producer {result.producer.package}/{result.producer.module} (contract v
        {result.producer.contractVersion}) at nlp-stack {NLP_STACK_REVISION.slice(0, 12)} · model{" "}
        {result.model.id}@{result.model.revision.slice(0, 12)} ({result.model.dtype}, from{" "}
        {result.model.baseModel}) · spans are segment-local UTF-16 offsets.
      </p>
    </>
  );
}

function isSupported(language: string) {
  return (CANONICAL_NER_LANGUAGES as readonly string[]).includes(language);
}

function languageName(code: string) {
  return code === "en" ? "English" : code === "de" ? "German" : code === "es" ? "Spanish" : code;
}

function formatBytes(bytes: number) {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

function formatTimestamp(seconds: number) {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = String(total % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${remainder}`
    : `${minutes}:${remainder}`;
}
