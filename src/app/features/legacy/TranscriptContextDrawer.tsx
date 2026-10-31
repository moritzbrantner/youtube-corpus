import { Clock, Copy as CopyIcon, ExternalLink } from "lucide-react";

import {
  Badge,
  Button,
  CopyButton,
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@moritzbrantner/ui";

import { buildTimestampUrl, formatTimeRange } from "./format";
import { SearchState } from "./SearchState";
import type { TranscriptContext } from "./use-transcript-context";

export function TranscriptContextDrawer({ context }: { context: TranscriptContext }) {
  const { contextOpen, setContextOpen, transcriptContext, contextMatch, contextDescription } =
    context;
  return (
    <Drawer open={contextOpen} onOpenChange={setContextOpen}>
      <DrawerContent className="mx-auto max-h-[85vh] w-full max-w-3xl">
        <DrawerHeader className="border-b border-border text-left">
          <div className="flex min-w-0 flex-wrap items-start gap-3">
            <div className="min-w-0 flex-1">
              <DrawerTitle className="truncate">
                {contextMatch?.title ?? "Transcript context"}
              </DrawerTitle>
              <DrawerDescription>{contextDescription}</DrawerDescription>
            </div>
            {contextMatch ? (
              <div className="flex flex-wrap items-center gap-2">
                <Button asChild variant="outline" size="sm">
                  <a
                    href={buildTimestampUrl(contextMatch.sourceUrl, contextMatch.startSeconds)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    <ExternalLink className="size-4" aria-hidden="true" />
                    Open source
                  </a>
                </Button>
                <CopyButton
                  value={contextMatch.text}
                  variant="outline"
                  size="sm"
                  idleLabel={
                    <>
                      <CopyIcon className="size-4" aria-hidden="true" />
                      Copy
                    </>
                  }
                  copiedLabel="Copied"
                />
              </div>
            ) : null}
          </div>
        </DrawerHeader>

        <div className="overflow-y-auto px-4 py-4">
          {transcriptContext.isPending && contextOpen ? (
            <SearchState variant="loading" title="Loading transcript context..." />
          ) : transcriptContext.error ? (
            <SearchState
              variant="error"
              title="Context unavailable"
              description={String(transcriptContext.error)}
            />
          ) : transcriptContext.data ? (
            <ol className="grid gap-2">
              {transcriptContext.data.segments.map((segment) => {
                const timeRange = formatTimeRange(segment.startSeconds, segment.endSeconds);
                return (
                  <li
                    key={segment.segmentId}
                    className={[
                      "grid gap-2 rounded-md border px-3 py-3 text-sm",
                      segment.isMatch
                        ? "border-primary/50 bg-primary/5"
                        : "border-border bg-background",
                    ].join(" ")}
                  >
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      {timeRange ? (
                        <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
                          <Clock className="size-3" aria-hidden="true" />
                          {timeRange}
                        </span>
                      ) : null}
                      {segment.isMatch ? <Badge variant="default">Match</Badge> : null}
                    </div>
                    <p className="leading-6 text-muted-foreground">{segment.text}</p>
                  </li>
                );
              })}
            </ol>
          ) : (
            <SearchState
              title="Choose a result"
              description="Open context from a transcript match."
            />
          )}
        </div>
      </DrawerContent>
    </Drawer>
  );
}
