import * as React from "react";

export type CorpusUrlState = {
  corpusId: string | null;
  query: string;
  segmentId: string | null;
};

type CorpusUrlPatch = Partial<CorpusUrlState>;

function readUrlState(): CorpusUrlState {
  if (typeof window === "undefined") {
    return { corpusId: null, query: "", segmentId: null };
  }
  const url = new URL(window.location.href);
  return {
    corpusId: url.searchParams.get("corpus"),
    query: url.searchParams.get("q") ?? "",
    segmentId: url.searchParams.get("segment"),
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
    if (patch.corpusId !== undefined) {
      setOrDelete(url, "corpus", patch.corpusId);
      if (patch.corpusId !== readUrlState().corpusId) {
        url.searchParams.delete("segment");
      }
    }
    if (patch.query !== undefined) {
      setOrDelete(url, "q", patch.query.trim() || null);
      url.searchParams.delete("segment");
    }
    if (patch.segmentId !== undefined) {
      setOrDelete(url, "segment", patch.segmentId);
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
