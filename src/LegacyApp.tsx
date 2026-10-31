import { useQuery } from "@tanstack/react-query";
import { Settings } from "lucide-react";
import * as React from "react";

import { Badge, Button, type ThemeMode, ThemeModeSwitch } from "@moritzbrantner/ui";
import {
  Navbar,
  NavbarActions,
  PageActions,
  PageDescription,
  PageHeader,
  PageShell,
  PageTitle,
} from "@moritzbrantner/ui/shell";
import { getDownloadedFiles } from "./app/api";
import { AddSourcePage } from "./app/features/legacy/AddSourcePage";
import { ConnectionSheet } from "./app/features/legacy/ConnectionSheet";
import { DownloadedFilesPage } from "./app/features/legacy/DownloadedFilesPage";
import { SearchPage } from "./app/features/legacy/SearchPage";
import { TranscriptContextDrawer } from "./app/features/legacy/TranscriptContextDrawer";
import { useAddSourceForm } from "./app/features/legacy/use-add-source-form";
import { useCorpusConnection } from "./app/features/legacy/use-corpus-connection";
import { useTranscriptContext } from "./app/features/legacy/use-transcript-context";
import { useTranscriptSearch } from "./app/features/legacy/use-transcript-search";

const themeStorageKey = "youtube-corpus-theme-mode";
type PageId = "add" | "search" | "files";

const navigationGroups = [
  {
    id: "corpus",
    label: "Corpus",
    items: [
      { id: "add", label: "Add sources", href: "#add" },
      { id: "search", label: "Search", href: "#search" },
      { id: "files", label: "Downloaded files", href: "#files" },
    ],
  },
];

function getInitialThemeMode(): ThemeMode {
  if (typeof window === "undefined") {
    return "light";
  }

  const storedMode = window.localStorage.getItem(themeStorageKey);
  if (storedMode === "light" || storedMode === "dark") {
    return storedMode;
  }

  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyThemeMode(mode: ThemeMode) {
  document.documentElement.classList.toggle("dark", mode === "dark");
  document.documentElement.style.colorScheme = mode;
}

function getInitialPage(): PageId {
  if (typeof window === "undefined") {
    return "search";
  }
  if (window.location.hash === "#add") {
    return "add";
  }
  return window.location.hash === "#files" ? "files" : "search";
}

export default function App() {
  const [activePage, setActivePage] = React.useState<PageId>(getInitialPage);
  const [themeMode, setThemeMode] = React.useState<ThemeMode>(getInitialThemeMode);
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const connection = useCorpusConnection();
  const { connectionReady, connectionBadge } = connection;
  const form = useAddSourceForm(connectionReady);
  const search = useTranscriptSearch();
  const context = useTranscriptContext();
  const openSettings = React.useCallback(() => setSettingsOpen(true), []);

  React.useLayoutEffect(() => {
    applyThemeMode(themeMode);
    window.localStorage.setItem(themeStorageKey, themeMode);
  }, [themeMode]);

  React.useEffect(() => {
    const syncPage = () => setActivePage(getInitialPage());
    syncPage();
    window.addEventListener("hashchange", syncPage);
    return () => window.removeEventListener("hashchange", syncPage);
  }, []);

  const downloadedFiles = useQuery({
    queryKey: ["downloaded-files"],
    queryFn: () =>
      getDownloadedFiles({
        downloadedOnly: true,
        limit: 100,
      }),
    enabled: activePage === "files" && connectionReady,
    refetchOnWindowFocus: false,
  });
  const themeModeSwitchProps = {
    mode: themeMode,
    onModeChange: setThemeMode,
  };

  return (
    <PageShell background="muted" maxWidth="wide">
      <Navbar
        brand={<span className="font-semibold">YouTube Corpus</span>}
        groups={navigationGroups}
        activeItemId={activePage}
        defaultOpenGroupId={null}
        actionSlot={
          <NavbarActions
            notificationMenu={{
              unreadCount: connectionReady ? 0 : 1,
              items: connectionReady
                ? [{ id: "database", title: "Corpus database ready", meta: "Ready" }]
                : [{ id: "database", title: "Connection needs attention", unread: true }],
            }}
            accountMenu={{
              user: { name: "Local Corpus", email: "local web UI", initials: "YC" },
              items: [{ id: "postgres", label: "Postgres corpus" }],
            }}
            themeModeSwitch={themeModeSwitchProps}
          />
        }
      />

      <PageHeader>
        <div className="grid min-w-0 gap-2">
          <PageTitle>
            {activePage === "add"
              ? "Add sources"
              : activePage === "files"
                ? "Downloaded files"
                : "Transcript search"}
          </PageTitle>
          <PageDescription>
            {activePage === "add"
              ? "Download and parse videos, playlists, and channels into the local corpus."
              : activePage === "files"
                ? "Review local media and caption files recorded in the corpus."
                : "Query the local Postgres corpus with full-text, semantic, or hybrid retrieval."}
          </PageDescription>
        </div>
        <PageActions>
          <Badge variant={connectionBadge.variant}>{connectionBadge.label}</Badge>
          <Button type="button" variant="outline" onClick={() => setSettingsOpen(true)}>
            <Settings className="size-4" aria-hidden="true" />
            Settings
          </Button>
          <ThemeModeSwitch {...themeModeSwitchProps} />
        </PageActions>
      </PageHeader>

      <ConnectionSheet open={settingsOpen} onOpenChange={setSettingsOpen} connection={connection} />

      {activePage === "add" ? (
        <AddSourcePage form={form} connection={connection} />
      ) : activePage === "search" ? (
        <SearchPage
          search={search}
          connection={connection}
          onOpenSettings={openSettings}
          onOpenContext={context.openContext}
        />
      ) : (
        <DownloadedFilesPage
          connection={connection}
          downloadedFiles={downloadedFiles}
          onOpenSettings={openSettings}
        />
      )}

      <TranscriptContextDrawer context={context} />
    </PageShell>
  );
}
