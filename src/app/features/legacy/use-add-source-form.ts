import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { type AddSourceKind, addSource, getIngestRun, listIngestRuns } from "../../api";
import { optionalInteger, optionalNumber, splitList, splitLines } from "./format";

export function useAddSourceForm(connectionReady: boolean | undefined) {
  const queryClient = useQueryClient();
  const [addSourceKind, setAddSourceKind] = React.useState<AddSourceKind>("video");
  const [sourceUrl, setSourceUrl] = React.useState("");
  const [sourceName, setSourceName] = React.useState("");
  const [workDir, setWorkDir] = React.useState("use-case-output/youtube-corpus");
  const [captionLanguages, setCaptionLanguages] = React.useState("en");
  const [captionsEnabled, setCaptionsEnabled] = React.useState(true);
  const [autoCaptionsEnabled, setAutoCaptionsEnabled] = React.useState(true);
  const [ytDlpArgs, setYtDlpArgs] = React.useState("");
  const [ytDlpTimeoutSeconds, setYtDlpTimeoutSeconds] = React.useState("");
  const [asrEnabled, setAsrEnabled] = React.useState(false);
  const [transcriberCommand, setTranscriberCommand] = React.useState("");
  const [transcriberArgs, setTranscriberArgs] = React.useState("");
  const [transcriberTimeoutSeconds, setTranscriberTimeoutSeconds] = React.useState("");
  const [maxItems, setMaxItems] = React.useState("10");
  const [titleContains, setTitleContains] = React.useState("");
  const [titleExcludes, setTitleExcludes] = React.useState("");
  const [durationMin, setDurationMin] = React.useState("");
  const [durationMax, setDurationMax] = React.useState("");
  const [runMigrations, setRunMigrations] = React.useState(false);
  const [saveSubscription, setSaveSubscription] = React.useState(false);
  const [ingestNow, setIngestNow] = React.useState(true);
  const [asyncIngest, setAsyncIngest] = React.useState(true);

  React.useEffect(() => {
    if (addSourceKind === "video") {
      setSaveSubscription(false);
      setIngestNow(true);
      return;
    }
    setSaveSubscription(true);
    setMaxItems((value) => (value.trim() === "" ? "10" : value));
  }, [addSourceKind]);

  const addSourceMutation = useMutation({
    mutationFn: addSource,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["corpus-status"] }),
        queryClient.invalidateQueries({ queryKey: ["downloaded-files"] }),
      ]);
    },
  });

  const activeJobId = addSourceMutation.data?.jobId ?? null;
  const activeIngestRun = useQuery({
    queryKey: ["ingest-run", activeJobId],
    queryFn: () => getIngestRun(activeJobId!),
    enabled: activeJobId !== null,
    refetchInterval: (query) =>
      (query.state.data?.job?.status ?? query.state.data?.status) === "running" ? 2000 : false,
  });

  const recentIngestRuns = useQuery({
    queryKey: ["ingest-runs", 10],
    queryFn: () => listIngestRuns({ limit: 10 }),
    enabled: Boolean(connectionReady),
    refetchInterval: activeIngestRun.data?.status === "running" ? 4000 : false,
    refetchOnWindowFocus: false,
  });

  const addReport = addSourceMutation.data ?? null;
  const addIngestRun = activeIngestRun.data ?? addReport?.ingestRun ?? null;
  const completedJobRefreshRef = React.useRef<string | null>(null);
  const canSaveSubscription = addSourceKind !== "video";
  const canSubmitSource =
    sourceUrl.trim() !== "" &&
    !addSourceMutation.isPending &&
    (ingestNow || (canSaveSubscription && saveSubscription));

  React.useEffect(() => {
    const run = activeIngestRun.data;
    const jobStatus = run?.job?.status;
    if (!run || !jobStatus || jobStatus === "running") {
      return;
    }
    const refreshKey = `${run.id}:${jobStatus}`;
    if (completedJobRefreshRef.current === refreshKey) {
      return;
    }
    completedJobRefreshRef.current = refreshKey;
    void queryClient.invalidateQueries({ queryKey: ["ingest-runs"] });
    if (jobStatus === "succeeded") {
      void queryClient.invalidateQueries({ queryKey: ["corpus-status"] });
      void queryClient.invalidateQueries({ queryKey: ["downloaded-files"] });
    }
  }, [activeIngestRun.data, queryClient]);

  function submitSource(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canSubmitSource) {
      return;
    }

    const parsedMaxItems = optionalNumber(maxItems);
    addSourceMutation.mutate({
      sourceKind: addSourceKind,
      sourceUrl: sourceUrl.trim(),
      name: sourceName.trim() || null,
      workDir: workDir.trim() || undefined,
      captionLanguages: splitList(captionLanguages),
      captionsEnabled,
      autoCaptionsEnabled,
      ytDlpArgs: splitLines(ytDlpArgs),
      ytDlpTimeoutSeconds: optionalInteger(ytDlpTimeoutSeconds),
      asrEnabled,
      transcriberCommand: transcriberCommand.trim() || null,
      transcriberArgs: splitList(transcriberArgs),
      transcriberTimeoutSeconds: optionalInteger(transcriberTimeoutSeconds),
      maxItems: parsedMaxItems === null ? null : Math.max(1, Math.floor(parsedMaxItems)),
      titleContains: titleContains.trim() || null,
      titleExcludes: splitList(titleExcludes),
      durationMin: optionalNumber(durationMin),
      durationMax: optionalNumber(durationMax),
      migrate: runMigrations,
      subscribe: canSaveSubscription && saveSubscription,
      ingestNow,
      async: asyncIngest,
    });
  }

  return {
    addSourceKind,
    setAddSourceKind,
    sourceUrl,
    setSourceUrl,
    sourceName,
    setSourceName,
    workDir,
    setWorkDir,
    captionLanguages,
    setCaptionLanguages,
    captionsEnabled,
    setCaptionsEnabled,
    autoCaptionsEnabled,
    setAutoCaptionsEnabled,
    ytDlpArgs,
    setYtDlpArgs,
    ytDlpTimeoutSeconds,
    setYtDlpTimeoutSeconds,
    asrEnabled,
    setAsrEnabled,
    transcriberCommand,
    setTranscriberCommand,
    transcriberArgs,
    setTranscriberArgs,
    transcriberTimeoutSeconds,
    setTranscriberTimeoutSeconds,
    maxItems,
    setMaxItems,
    titleContains,
    setTitleContains,
    titleExcludes,
    setTitleExcludes,
    durationMin,
    setDurationMin,
    durationMax,
    setDurationMax,
    runMigrations,
    setRunMigrations,
    saveSubscription,
    setSaveSubscription,
    ingestNow,
    setIngestNow,
    asyncIngest,
    setAsyncIngest,
    addSourceMutation,
    recentIngestRuns,
    addReport,
    addIngestRun,
    canSubmitSource,
    submitSource,
  };
}

export type AddSourceForm = ReturnType<typeof useAddSourceForm>;
