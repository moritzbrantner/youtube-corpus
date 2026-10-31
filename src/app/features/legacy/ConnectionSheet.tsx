import { Database } from "lucide-react";

import {
  Alert,
  AlertDescription,
  Badge,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@moritzbrantner/ui";

import type { CorpusConnection } from "./use-corpus-connection";

export function ConnectionSheet({
  open,
  onOpenChange,
  connection,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connection: CorpusConnection;
}) {
  const { corpusStatus, connectionReady, statusMessage } = connection;
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full sm:max-w-md">
        <SheetHeader>
          <SheetTitle>Connection</SheetTitle>
          <SheetDescription>
            The local web server reads the Postgres connection from DATABASE_URL.
          </SheetDescription>
        </SheetHeader>
        <div className="mt-6 grid gap-5">
          <div className="grid gap-2">
            <span className="text-sm font-medium">Database URL</span>
            <div className="flex min-w-0 items-center gap-2 rounded-md border border-border bg-background px-3 py-2">
              <Database className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 truncate text-sm">
                {corpusStatus.data?.databaseUrl ?? "Not configured"}
              </span>
            </div>
          </div>

          <div className="grid gap-3 rounded-md border border-border p-4 text-sm">
            <div className="flex items-center justify-between gap-3">
              <span className="text-muted-foreground">Configured</span>
              <Badge variant={corpusStatus.data?.configured ? "default" : "secondary"}>
                {corpusStatus.data?.configured ? "Yes" : "No"}
              </Badge>
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-muted-foreground">Reachable</span>
              <Badge variant={corpusStatus.data?.reachable ? "default" : "secondary"}>
                {corpusStatus.data?.reachable ? "Yes" : "No"}
              </Badge>
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-muted-foreground">Schema ready</span>
              <Badge variant={corpusStatus.data?.schemaReady ? "default" : "secondary"}>
                {corpusStatus.data?.schemaReady ? "Yes" : "No"}
              </Badge>
            </div>
          </div>

          {statusMessage ? (
            <Alert variant={connectionReady ? "default" : "destructive"}>
              <AlertDescription>{statusMessage}</AlertDescription>
            </Alert>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}
