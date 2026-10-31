import type { UseQueryResult } from "@tanstack/react-query";
import { Copy as CopyIcon, ExternalLink, FileText } from "lucide-react";

import { Badge, Button, CopyButton, MetricStrip } from "@moritzbrantner/ui";
import {
  PageContent,
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import type { DownloadedFile } from "../../api";
import {
  formatDate,
  formatDuration,
  segmentsMetric,
  sourceLabel,
  subscriptionsMetric,
  videosMetric,
  videoTitle,
} from "./format";
import { ConnectionStatusButton, SearchState } from "./SearchState";
import type { CorpusConnection } from "./use-corpus-connection";

type DownloadedFilesQuery = UseQueryResult<DownloadedFile[]>;

function DownloadedFilesList({
  connection,
  downloadedFiles,
  onOpenSettings,
}: {
  connection: CorpusConnection;
  downloadedFiles: DownloadedFilesQuery;
  onOpenSettings: () => void;
}) {
  const { corpusStatus, connectionReady, statusMessage } = connection;
  if (!connectionReady && !corpusStatus.isLoading) {
    return (
      <SearchState
        variant="error"
        title="Database unavailable"
        description={statusMessage ?? "Corpus database is not ready."}
        actions={<ConnectionStatusButton onClick={onOpenSettings} />}
      />
    );
  }

  if (corpusStatus.isLoading || downloadedFiles.isPending) {
    return <SearchState variant="loading" title="Loading downloaded files..." />;
  }

  if (downloadedFiles.error) {
    return (
      <SearchState
        variant="error"
        title="Downloaded files unavailable"
        description={String(downloadedFiles.error)}
      />
    );
  }

  const files = downloadedFiles.data ?? [];
  if (files.length === 0) {
    return (
      <SearchState
        title="No downloaded files"
        description="Ingest videos or subscriptions to create local media or caption files."
      />
    );
  }

  return (
    <div className="grid gap-3">
      {files.map((file) => {
        const transcriptKinds = file.transcriptSourceKinds.map(sourceLabel);
        return (
          <article
            key={file.id}
            className="grid gap-3 rounded-md border border-border bg-background px-4 py-4"
          >
            <div className="flex min-w-0 flex-wrap items-start gap-3">
              <FileText
                className="mt-0.5 size-4 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <div className="min-w-0 flex-1">
                <h2 className="truncate text-sm font-semibold">{videoTitle(file)}</h2>
                <p className="mt-1 truncate text-xs text-muted-foreground">{file.sourceUrl}</p>
              </div>
              <Button asChild variant="ghost" size="sm">
                <a href={file.sourceUrl} target="_blank" rel="noreferrer">
                  <ExternalLink className="size-4" aria-hidden="true" />
                  Source
                </a>
              </Button>
            </div>

            <div className="flex min-w-0 flex-wrap items-center gap-2">
              {file.mediaDownloaded ? <Badge variant="default">Media</Badge> : null}
              {file.captionFilesDownloaded ? <Badge variant="secondary">Captions</Badge> : null}
              {file.parsed ? <Badge variant="outline">Parsed</Badge> : null}
              {transcriptKinds.map((kind) => (
                <Badge key={kind} variant="outline">
                  {kind}
                </Badge>
              ))}
            </div>

            <div className="grid gap-2 text-sm text-muted-foreground md:grid-cols-2">
              <div className="min-w-0">
                <span className="font-medium text-foreground">Local file</span>
                <p className="mt-1 truncate">{file.localVideoPath ?? "No media file recorded"}</p>
              </div>
              <div className="min-w-0">
                <span className="font-medium text-foreground">Transcript</span>
                <p className="mt-1">
                  {file.transcriptStreams.toLocaleString()} streams /{" "}
                  {file.transcriptSegments.toLocaleString()} segments
                </p>
              </div>
              <div>
                <span className="font-medium text-foreground">Duration</span>
                <p className="mt-1">{formatDuration(file.durationSeconds)}</p>
              </div>
              <div>
                <span className="font-medium text-foreground">Updated</span>
                <p className="mt-1">{formatDate(file.updatedAt)}</p>
              </div>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              {file.localVideoPath ? (
                <CopyButton
                  value={file.localVideoPath}
                  variant="outline"
                  size="sm"
                  idleLabel={
                    <>
                      <CopyIcon className="size-4" aria-hidden="true" />
                      Copy path
                    </>
                  }
                  copiedLabel="Copied"
                />
              ) : null}
              {file.subscriptionNames.map((name) => (
                <Badge key={name} variant="outline">
                  {name}
                </Badge>
              ))}
            </div>
          </article>
        );
      })}
    </div>
  );
}

export function DownloadedFilesPage({
  connection,
  downloadedFiles,
  onOpenSettings,
}: {
  connection: CorpusConnection;
  downloadedFiles: DownloadedFilesQuery;
  onOpenSettings: () => void;
}) {
  const { stats, connectionReady } = connection;
  return (
    <PageContent className="grid gap-4">
      <MetricStrip
        items={[
          {
            id: "downloaded",
            label: "Downloaded",
            value: downloadedFiles.data ? downloadedFiles.data.length.toLocaleString() : "-",
            delta: "Media or captions",
          },
          videosMetric(stats, connectionReady),
          segmentsMetric(stats, connectionReady),
          subscriptionsMetric(stats),
        ]}
      />

      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Files</SurfaceTitle>
          <SurfaceDescription>
            Videos with local media paths or downloaded transcript source files.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="mt-5">
          <DownloadedFilesList
            connection={connection}
            downloadedFiles={downloadedFiles}
            onOpenSettings={onOpenSettings}
          />
        </SurfaceContent>
      </Surface>
    </PageContent>
  );
}
