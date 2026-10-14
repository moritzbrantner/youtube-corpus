import * as React from "react";

import type { SearchMode, SourceKind } from "../../api";

export type CorpusUrlState = {
  corpusId: string | null;
  query: string;
  segmentId: string | null;
  mode: SearchMode;
  modality: "all" | "spoken" | "visible";
  sourceKind: "all" | SourceKind;
  language: string;
  channelQuery: string;
  titleQuery: string;
  categoryQuery: string;
  tagQuery: string;
  uploadDateFrom: string;
  uploadDateTo: string;
};

export type CorpusSearchFilters = Pick<
  CorpusUrlState,
  | "mode"
  | "modality"
  | "sourceKind"
  | "language"
  | "channelQuery"
  | "titleQuery"
  | "categoryQuery"
  | "tagQuery"
  | "uploadDateFrom"
  | "uploadDateTo"
>;

type CorpusUrlPatch = Partial<CorpusUrlState>;

const keys: Array<[keyof CorpusUrlState, string]> = [
  ["corpusId", "corpus"],
  ["query", "q"],
  ["segmentId", "segment"],
  ["mode", "mode"],
  ["modality", "modality"],
  ["sourceKind", "sourceKind"],
  ["language", "language"],
  ["channelQuery", "channel"],
  ["titleQuery", "title"],
  ["categoryQuery", "category"],
  ["tagQuery", "tag"],
  ["uploadDateFrom", "from"],
  ["uploadDateTo", "to"],
];

function selected<T extends string>(value: string | null, options: readonly T[], fallback: T): T {
  return options.find((option) => option === value) ?? fallback;
}

function readUrlState(): CorpusUrlState {
  if (typeof window === "undefined") {
    return {
      corpusId: null,
      query: "",
      segmentId: null,
      mode: "hybrid",
      modality: "all",
      sourceKind: "all",
      language: "",
      channelQuery: "",
      titleQuery: "",
      categoryQuery: "",
      tagQuery: "",
      uploadDateFrom: "",
      uploadDateTo: "",
    };
  }
  const params = new URL(window.location.href).searchParams;
  return {
    corpusId: params.get("corpus"),
    query: params.get("q") ?? "",
    segmentId: params.get("segment"),
    mode: selected(params.get("mode"), ["hybrid", "fts", "semantic"], "hybrid"),
    modality: selected(params.get("modality"), ["all", "spoken", "visible"], "all"),
    sourceKind: selected(
      params.get("sourceKind"),
      ["all", "caption_manual", "caption_auto", "asr", "visual_ocr"],
      "all",
    ),
    language: params.get("language") ?? "",
    channelQuery: params.get("channel") ?? "",
    titleQuery: params.get("title") ?? "",
    categoryQuery: params.get("category") ?? "",
    tagQuery: params.get("tag") ?? "",
    uploadDateFrom: params.get("from") ?? "",
    uploadDateTo: params.get("to") ?? "",
  };
}

export function useCorpusUrlState() {
  const [state, setState] = React.useState<CorpusUrlState>(readUrlState);

  React.useEffect(() => {
    const syncFromLocation = () => setState(readUrlState());
    window.addEventListener("popstate", syncFromLocation);
    window.addEventListener("hashchange", syncFromLocation);
    return () => {
      window.removeEventListener("popstate", syncFromLocation);
      window.removeEventListener("hashchange", syncFromLocation);
    };
  }, []);

  const update = React.useCallback((patch: CorpusUrlPatch) => {
    const url = new URL(window.location.href);
    for (const [field, param] of keys) {
      if (patch[field] !== undefined) {
        setOrDelete(url, param, patch[field] as string | null);
      }
    }
    if (Object.keys(patch).some((field) => field !== "segmentId")) {
      url.searchParams.delete("segment");
    }
    window.history.replaceState(null, "", url);
    setState(readUrlState());
  }, []);

  return { state, update };
}

function setOrDelete(url: URL, key: string, value: string | null) {
  if (value) {
    url.searchParams.set(key, value);
  } else {
    url.searchParams.delete(key);
  }
}
