import { useQuery } from "@tanstack/react-query";

import { getCorpusStatus } from "../../api";

export function useCorpusConnection() {
  const corpusStatus = useQuery({
    queryKey: ["corpus-status"],
    queryFn: () => getCorpusStatus(),
    refetchOnWindowFocus: false,
  });
  const stats = corpusStatus.data?.stats ?? null;
  const statusMessage =
    corpusStatus.data?.message ?? (corpusStatus.error ? String(corpusStatus.error) : null);
  const connectionReady = corpusStatus.data?.reachable && corpusStatus.data.schemaReady;
  const connectionBadge = corpusStatus.isLoading
    ? { label: "Checking", variant: "outline" as const }
    : connectionReady
      ? { label: "Ready", variant: "default" as const }
      : corpusStatus.data?.reachable
        ? { label: "Needs migration", variant: "secondary" as const }
        : { label: "Not connected", variant: "destructive" as const };

  return { corpusStatus, stats, statusMessage, connectionReady, connectionBadge };
}

export type CorpusConnection = ReturnType<typeof useCorpusConnection>;
