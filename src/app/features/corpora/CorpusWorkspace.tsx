import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  Button,
  NativeSelect,
  StateView,
  ErrorState,
  LoadingState,
  StateViewDescription,
  StateViewTitle,
} from "@moritzbrantner/ui";
import {
  Navbar,
  PageActions,
  PageContent,
  PageHeader,
  PageShell,
  PageTitle,
} from "@moritzbrantner/ui/shell";

import { listCorpora } from "./api";
import { CorpusAnnotations } from "./CorpusAnnotations";
import { CorpusDetectors } from "./CorpusDetectors";
import { CorpusEntities } from "./CorpusEntities";
import { CorpusEvaluation } from "./CorpusEvaluation";
import { CorpusLabels } from "./CorpusLabels";
import { CorpusPicker } from "./CorpusPicker";
import { CorpusSearch } from "./CorpusSearch";
import { CorpusSources } from "./CorpusSources";
import { CorpusVideos } from "./CorpusVideos";
import { corpusKeys } from "./query-keys";
import { corpusPageUrl, useCorpusUrlState } from "./url-state";

export function CorpusWorkspace({ page }: { page: "corpora" | "search" }) {
  const { state, update } = useCorpusUrlState();
  const corporaQuery = useQuery({
    queryKey: corpusKeys.all,
    queryFn: listCorpora,
  });
  const selectedCorpus = corporaQuery.data?.find((corpus) => corpus.id === state.corpusId) ?? null;

  React.useEffect(() => {
    if (state.corpusId || !corporaQuery.data?.length || selectedCorpus) {
      return;
    }
    const fallback = corporaQuery.data.find((corpus) => corpus.isDefault) ?? corporaQuery.data[0];
    if (fallback) {
      update({ corpusId: fallback.id }, "replace");
    }
  }, [corporaQuery.data, selectedCorpus, state.corpusId, update]);

  return (
    <PageShell background="muted" maxWidth="wide">
      <Navbar
        brand={<span className="font-semibold">YouTube Corpus</span>}
        groups={[
          {
            id: "research",
            label: "Research",
            items: [
              {
                id: "corpora",
                label: "Corpora",
                href: corpusPageUrl(new URL(window.location.href), "corpora").href,
              },
              {
                id: "search",
                label: "Search",
                href: corpusPageUrl(new URL(window.location.href), "search").href,
              },
              { id: "analyzer", label: "Analyze video", href: "/" },
            ],
          },
        ]}
        activeItemId={page}
        defaultOpenGroupId="research"
      />
      <PageHeader>
        <div className="grid min-w-0 gap-2">
          <PageTitle>{page === "search" ? "Search" : "Corpora"}</PageTitle>
        </div>
        <PageActions>
          <label className="grid gap-1 text-sm">
            Current corpus
            <NativeSelect
              aria-label="Current corpus"
              value={selectedCorpus?.id ?? ""}
              onChange={(event) => update({ corpusId: event.target.value })}
            >
              <option value="" disabled>
                Select a corpus
              </option>
              {corporaQuery.data?.map((corpus) => (
                <option key={corpus.id} value={corpus.id}>
                  {corpus.name}
                </option>
              ))}
            </NativeSelect>
          </label>
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
          {state.corpusId && !selectedCorpus ? (
            <ErrorState>
              <StateViewTitle>Corpus unavailable</StateViewTitle>
              <StateViewDescription>
                The selected corpus no longer exists. Choose another corpus.
              </StateViewDescription>
            </ErrorState>
          ) : null}
          {page === "search" && !state.corpusId && corporaQuery.data.length === 0 ? (
            <StateView variant="empty">
              <StateViewTitle>No corpora yet</StateViewTitle>
              <Button asChild>
                <a href={corpusPageUrl(new URL(window.location.href), "corpora").href}>
                  Create a corpus
                </a>
              </Button>
            </StateView>
          ) : null}
          {page === "corpora" ? (
            <div className="grid gap-4 lg:grid-cols-[minmax(300px,380px)_minmax(0,1fr)]">
              <CorpusPicker
                corpora={corporaQuery.data}
                selectedCorpusId={selectedCorpus?.id ?? null}
                onSelect={(corpusId) => update({ corpusId })}
              />
              {selectedCorpus ? <CorpusSources corpusId={selectedCorpus.id} /> : null}
            </div>
          ) : null}

          {selectedCorpus ? (
            <>
              {page === "search" ? (
                <CorpusSearch
                  key={`${selectedCorpus.id}:${state.query}`}
                  corpusId={selectedCorpus.id}
                  submittedQuery={state.query}
                  selectedSegmentId={state.segmentId}
                  filters={{
                    mode: state.mode,
                    modality: state.modality,
                    sourceKind: state.sourceKind,
                    language: state.language,
                    channelQuery: state.channelQuery,
                    titleQuery: state.titleQuery,
                    categoryQuery: state.categoryQuery,
                    tagQuery: state.tagQuery,
                    uploadDateFrom: state.uploadDateFrom,
                    uploadDateTo: state.uploadDateTo,
                  }}
                  onUpdateFilters={(filters) => update(filters)}
                  onSubmitQuery={(query, filters) => update({ ...filters, query })}
                  onSelectSegment={(segmentId) => update({ segmentId })}
                />
              ) : null}
              {page === "corpora" ? (
                <>
                  <Button asChild>
                    <a href={corpusPageUrl(new URL(window.location.href), "search").href}>
                      Search corpus
                    </a>
                  </Button>
                  <CorpusDetectors
                    key={`detectors:${selectedCorpus.id}`}
                    corpusId={selectedCorpus.id}
                  />
                  <CorpusEntities key={selectedCorpus.id} corpusId={selectedCorpus.id} />
                  <div className="grid gap-4 xl:grid-cols-2">
                    <CorpusEvaluation corpusId={selectedCorpus.id} />
                    <CorpusAnnotations corpusId={selectedCorpus.id} />
                  </div>
                  <CorpusLabels corpusId={selectedCorpus.id} />
                  <CorpusVideos corpusId={selectedCorpus.id} />
                </>
              ) : null}
            </>
          ) : null}
        </PageContent>
      ) : null}
    </PageShell>
  );
}
