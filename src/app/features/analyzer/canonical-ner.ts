// Consumer adapter for NLP Stack's canonical browser NER
// (`@moritzbrantner/text-linguistics-wasm/browser-ner`). Inference, the model and its
// revision pin all live upstream; this file only loads the module from a pinned NLP Stack
// commit on explicit user action and anchors its entities to transcript segments.

/** Pushed, reviewed nlp-stack commit whose browser-ner module this app consumes. */
export const NLP_STACK_REVISION = "e1063c5b5918f7c18bf1725a461a6193d587b8ec";

export const BROWSER_NER_MODULE_BASE_URL = `https://cdn.jsdelivr.net/gh/moritzbrantner/nlp-stack@${NLP_STACK_REVISION}/packages/text-linguistics-wasm/`;

/** Languages the pinned module declares; the module itself stays the source of truth. */
export const CANONICAL_NER_LANGUAGES = ["en", "de", "es"] as const;

export interface CanonicalNerModel {
  id: string;
  revision: string;
  baseModel: string;
  dtype: string;
  labels: string[];
  supportedLanguages: string[];
}

export interface CanonicalNerProducer {
  package: string;
  module: string;
  contractVersion: number;
}

export interface CanonicalNerEntity {
  segmentId: string | number;
  label: string;
  text: string;
  score: number;
  start: number;
  end: number;
  byteStart: number;
  byteEnd: number;
}

export interface CanonicalNerResult {
  status: "completed" | "unsupported-language";
  language: string | null;
  producer: CanonicalNerProducer;
  model: CanonicalNerModel;
  entities: CanonicalNerEntity[];
  segments: Array<{ id: string | number; entityCount: number; truncated: boolean }>;
}

export interface CanonicalNerProgress {
  stage: "model";
  detail: Record<string, unknown>;
}

export interface CanonicalNerRequestOptions {
  signal?: AbortSignal;
  onProgress?: (progress: CanonicalNerProgress) => void;
}

export interface CanonicalNer {
  analyze(
    request: { language: string | null; segments: Array<{ id: string; text: string }> },
    options?: CanonicalNerRequestOptions,
  ): Promise<CanonicalNerResult>;
  dispose(): void;
}

interface BrowserNerModule {
  createBrowserNer(options: { createWorker: () => Worker }): CanonicalNer;
}

export interface TranscriptSegmentLike {
  segmentId: string;
  startSeconds: number | null;
  endSeconds: number | null;
  text: string;
}

export interface AnchoredEntity extends CanonicalNerEntity {
  segmentId: string;
  segmentText: string;
  startSeconds: number | null;
  endSeconds: number | null;
}

/**
 * Imports the pinned NLP Stack module and creates its adapter. Nothing is fetched until this
 * is called, so an ordinary Pages load stays offline and model-free.
 */
export async function loadCanonicalNer(
  baseUrl: string = BROWSER_NER_MODULE_BASE_URL,
): Promise<CanonicalNer> {
  const module = (await import(/* @vite-ignore */ `${baseUrl}browser-ner.js`)) as BrowserNerModule;
  return module.createBrowserNer({
    createWorker: () => crossOriginModuleWorker(`${baseUrl}browser-ner-worker.js`),
  });
}

// Browsers refuse cross-origin worker scripts, so a same-origin blob module imports the
// pinned upstream worker (whose relative imports then resolve against the CDN URL).
function crossOriginModuleWorker(url: string) {
  const blobUrl = URL.createObjectURL(
    new Blob([`import ${JSON.stringify(url)};`], { type: "text/javascript" }),
  );
  try {
    return new Worker(blobUrl, { type: "module" });
  } finally {
    URL.revokeObjectURL(blobUrl);
  }
}

/** Primary language subtag the NER module understands (`de-DE` → `de`), or null. */
export function primaryLanguage(language: string | null | undefined) {
  const primary = language?.trim().toLowerCase().split(/[-_]/, 1)[0];
  return primary ? primary : null;
}

export function nerSegments(segments: TranscriptSegmentLike[]) {
  return segments
    .filter((segment) => segment.text.trim())
    .map((segment) => ({ id: segment.segmentId, text: segment.text }));
}

/** Attaches each entity to its source segment's text and time range. */
export function anchorEntities(
  result: CanonicalNerResult,
  segments: TranscriptSegmentLike[],
): AnchoredEntity[] {
  const byId = new Map(segments.map((segment) => [segment.segmentId, segment]));
  return result.entities.flatMap((entity) => {
    const segment = byId.get(String(entity.segmentId));
    if (!segment) return [];
    return [
      {
        ...entity,
        segmentId: segment.segmentId,
        segmentText: segment.text,
        startSeconds: segment.startSeconds,
        endSeconds: segment.endSeconds,
      },
    ];
  });
}

/** Sums Transformers.js file progress events into one downloaded/total byte pair. */
export function modelDownloadBytes(files: Map<string, { loaded: number; total: number }>) {
  let loaded = 0;
  let total = 0;
  for (const file of files.values()) {
    loaded += file.loaded;
    total += file.total;
  }
  return { loaded, total };
}
