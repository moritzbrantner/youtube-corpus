import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import { type SearchMode, type SearchReport, type SourceKind, searchTranscripts } from "../../api";
import {
  compactFilterLabel,
  compactRangeFilterLabel,
  groupResultsByVideo,
  optionalInteger,
  optionalNumber,
  sourceLabel,
} from "./format";

const defaultQuery = "first uploaded youtube video";

export function useTranscriptSearch() {
  const [query, setQuery] = React.useState(defaultQuery);
  const [mode, setMode] = React.useState<SearchMode>("hybrid");
  const [topK, setTopK] = React.useState(5);
  const [sourceKind, setSourceKind] = React.useState<SourceKind | "all">("all");
  const [filterPanelOpen, setFilterPanelOpen] = React.useState(false);
  const [languageFilter, setLanguageFilter] = React.useState("");
  const [transcriptStartMin, setTranscriptStartMin] = React.useState("");
  const [transcriptStartMax, setTranscriptStartMax] = React.useState("");
  const [uploadDateFrom, setUploadDateFrom] = React.useState("");
  const [uploadDateTo, setUploadDateTo] = React.useState("");
  const [searchDurationMin, setSearchDurationMin] = React.useState("");
  const [searchDurationMax, setSearchDurationMax] = React.useState("");
  const [channelQuery, setChannelQuery] = React.useState("");
  const [titleQuery, setTitleQuery] = React.useState("");
  const [categoryQuery, setCategoryQuery] = React.useState("");
  const [tagQuery, setTagQuery] = React.useState("");
  const [metadataQuery, setMetadataQuery] = React.useState("");
  const [viewCountMin, setViewCountMin] = React.useState("");
  const [viewCountMax, setViewCountMax] = React.useState("");
  const [lastSearchSourceKind, setLastSearchSourceKind] = React.useState<SourceKind | "all">("all");
  const [lastSearchFilterLabels, setLastSearchFilterLabels] = React.useState<string[]>([]);
  const [lastReport, setLastReport] = React.useState<SearchReport | null>(null);

  const searchMutation = useMutation({
    mutationFn: searchTranscripts,
    onSuccess: (report) => {
      setLastReport(report);
    },
  });

  const report = searchMutation.data ?? lastReport;
  const groupedResults = React.useMemo(
    () => (report ? groupResultsByVideo(report.results) : []),
    [report],
  );
  const resultCount = report?.results.length ?? 0;
  const topScore = report?.results[0]?.score ?? null;
  const activeSearchFilterLabels = buildSearchFilterLabels(sourceKind);
  const activeSearchFilterCount = activeSearchFilterLabels.length;

  function runSearch(nextSourceKind: SourceKind | "all" = sourceKind) {
    const trimmedQuery = query.trim();
    if (!trimmedQuery) {
      return;
    }
    setLastSearchSourceKind(nextSourceKind);
    setLastSearchFilterLabels(buildSearchFilterLabels(nextSourceKind));
    searchMutation.mutate({
      query: trimmedQuery,
      mode,
      topK,
      sourceKind: nextSourceKind === "all" ? null : nextSourceKind,
      language: languageFilter.trim() || null,
      transcriptStartMin: optionalNumber(transcriptStartMin),
      transcriptStartMax: optionalNumber(transcriptStartMax),
      uploadDateFrom: uploadDateFrom.trim() || null,
      uploadDateTo: uploadDateTo.trim() || null,
      durationMin: optionalNumber(searchDurationMin),
      durationMax: optionalNumber(searchDurationMax),
      channelQuery: channelQuery.trim() || null,
      titleQuery: titleQuery.trim() || null,
      categoryQuery: categoryQuery.trim() || null,
      tagQuery: tagQuery.trim() || null,
      metadataQuery: metadataQuery.trim() || null,
      viewCountMin: optionalInteger(viewCountMin),
      viewCountMax: optionalInteger(viewCountMax),
    });
  }

  function submitSearch(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    runSearch();
  }

  function searchAllSources() {
    setSourceKind("all");
    runSearch("all");
  }

  function clearSearchFilters() {
    setSourceKind("all");
    setLanguageFilter("");
    setTranscriptStartMin("");
    setTranscriptStartMax("");
    setUploadDateFrom("");
    setUploadDateTo("");
    setSearchDurationMin("");
    setSearchDurationMax("");
    setChannelQuery("");
    setTitleQuery("");
    setCategoryQuery("");
    setTagQuery("");
    setMetadataQuery("");
    setViewCountMin("");
    setViewCountMax("");
  }

  function buildSearchFilterLabels(nextSourceKind: SourceKind | "all") {
    return [
      nextSourceKind === "all" ? null : `Source: ${sourceLabel(nextSourceKind)}`,
      compactFilterLabel("Language", languageFilter),
      compactRangeFilterLabel("Transcript seconds", transcriptStartMin, transcriptStartMax),
      compactRangeFilterLabel("Uploaded", uploadDateFrom, uploadDateTo),
      compactRangeFilterLabel("Duration seconds", searchDurationMin, searchDurationMax),
      compactFilterLabel("Channel/uploader", channelQuery),
      compactFilterLabel("Title", titleQuery),
      compactFilterLabel("Category", categoryQuery),
      compactFilterLabel("Tag", tagQuery),
      compactFilterLabel("Metadata", metadataQuery),
      compactRangeFilterLabel("Views", viewCountMin, viewCountMax),
    ].filter((label): label is string => label !== null);
  }

  return {
    query,
    setQuery,
    mode,
    setMode,
    topK,
    setTopK,
    sourceKind,
    setSourceKind,
    filterPanelOpen,
    setFilterPanelOpen,
    languageFilter,
    setLanguageFilter,
    transcriptStartMin,
    setTranscriptStartMin,
    transcriptStartMax,
    setTranscriptStartMax,
    uploadDateFrom,
    setUploadDateFrom,
    uploadDateTo,
    setUploadDateTo,
    searchDurationMin,
    setSearchDurationMin,
    searchDurationMax,
    setSearchDurationMax,
    channelQuery,
    setChannelQuery,
    titleQuery,
    setTitleQuery,
    categoryQuery,
    setCategoryQuery,
    tagQuery,
    setTagQuery,
    metadataQuery,
    setMetadataQuery,
    viewCountMin,
    setViewCountMin,
    viewCountMax,
    setViewCountMax,
    lastSearchSourceKind,
    lastSearchFilterLabels,
    searchMutation,
    report,
    groupedResults,
    resultCount,
    topScore,
    activeSearchFilterCount,
    submitSearch,
    searchAllSources,
    clearSearchFilters,
  };
}

export type TranscriptSearch = ReturnType<typeof useTranscriptSearch>;
