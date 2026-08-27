import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  ErrorState,
  LoadingState,
  StateViewDescription,
  StateViewTitle,
} from "@moritzbrantner/ui";
import {
  Navbar,
  PageActions,
  PageContent,
  PageDescription,
  PageHeader,
  PageShell,
  PageTitle,
} from "@moritzbrantner/ui/shell";

import { listCorpora } from "./api";
import { CorpusEvaluation } from "./CorpusEvaluation";
import { CorpusPicker } from "./CorpusPicker";
import { CorpusSearch } from "./CorpusSearch";
import { CorpusSources } from "./CorpusSources";
import { CorpusVideos } from "./CorpusVideos";
import { corpusKeys } from "./query-keys";
import { useCorpusUrlState } from "./url-state";

const navigationGroups = [
  {
    id: "research",
    label: "Research",
    items: [
      { id: "corpora", label: "Corpora", href: "#corpora" },
      { id: "search", label: "Global search", href: "#search" },
      { id: "add", label: "Global ingest", href: "#add" },
    ],
  },
];

export function CorpusWorkspace() {
  const { state, update } = useCorpusUrlState();
  const corporaQuery = useQuery({
    queryKey: corpusKeys.all,
    queryFn: listCorpora,
  });
  const selectedCorpus = corporaQuery.data?.find((corpus) => corpus.id === state.corpusId) ?? null;

  React.useEffect(() => {
    if (!corporaQuery.data?.length || selectedCorpus) {
      return;
    }
    const fallback = corporaQuery.data.find((corpus) => corpus.isDefault) ?? corporaQuery.data[0];
    if (fallback) {
      update({ corpusId: fallback.id });
    }
  }, [corporaQuery.data, selectedCorpus, update]);

  return (
    <PageShell background="muted" maxWidth="wide">
      <Navbar
        brand={<span className="font-semibold">YouTube Corpus</span>}
        groups={navigationGroups}
        activeItemId="corpora"
        defaultOpenGroupId="research"
      />
      <PageHeader>
        <div className="grid min-w-0 gap-2">
          <PageTitle>{selectedCorpus?.name ?? "Research corpora"}</PageTitle>
          <PageDescription>
            Organize sources, select trustworthy transcript streams, evaluate retrieval quality,
            annotate findings, and reprocess corpus material reproducibly.
          </PageDescription>
        </div>
        <PageActions>
          {selectedCorpus ? <Badge variant="outline">{selectedCorpus.slug}</Badge> : null}
          {selectedCorpus?.isDefault ? <Badge variant="secondary">Default corpus</Badge> : null}
        </PageActions>
      </PageHeader>

      {corporaQuery.isPending ? (
        <PageContent>
          <LoadingState label="Loading research corpora" />
        </PageContent>
      ) : null}
      {corporaQuery.error ? (
        <PageContent>
          <ErrorState>
            <StateViewTitle>Corpora unavailable</StateViewTitle>
            <StateViewDescription>
              {String(corporaQuery.error)} Run the latest database migrations before opening this
              workspace.
            </StateViewDescription>
          </ErrorState>
        </PageContent>
      ) : null}
      {corporaQuery.data ? (
        <PageContent className="grid gap-4">
          <div className="grid gap-4 lg:grid-cols-[minmax(300px,380px)_minmax(0,1fr)]">
            <CorpusPicker
              corpora={corporaQuery.data}
              selectedCorpusId={selectedCorpus?.id ?? null}
              onSelect={(corpusId) => update({ corpusId })}
            />
            {selectedCorpus ? <CorpusSources corpusId={selectedCorpus.id} /> : null}
          </div>

          {selectedCorpus ? (
            <>
              <CorpusSearch
                key={`${selectedCorpus.id}:${state.query}`}
                corpusId={selectedCorpus.id}
                submittedQuery={state.query}
                selectedSegmentId={state.segmentId}
                onSubmitQuery={(query) => update({ query })}
                onSelectSegment={(segmentId) => update({ segmentId })}
              />
              <CorpusEvaluation corpusId={selectedCorpus.id} />
              <CorpusVideos corpusId={selectedCorpus.id} />
            </>
          ) : null}
        </PageContent>
      ) : null}
    </PageShell>
  );
}
