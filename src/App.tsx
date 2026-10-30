import * as React from "react";
import { ErrorState, StateViewTitle, StateViewDescription, Button } from "@moritzbrantner/ui";

import LegacyApp from "./LegacyApp";
import {
  DirectBrowserYouTubeAnalyzer,
  shouldUseDirectBrowserAnalyzer,
} from "./app/features/analyzer/DirectBrowserYouTubeAnalyzer";
import { YouTubeAnalyzer } from "./app/features/analyzer/YouTubeAnalyzer";
import { CorpusWorkspace } from "./app/features/corpora/CorpusWorkspace";
import type { CorpusPage } from "./app/features/corpora/url-state";

type CorpusRoute = CorpusPage;
type AppRoute = "analyzer" | CorpusRoute | "legacy";

const corpusRoutes: readonly AppRoute[] = [
  "corpora",
  "search",
  "sources",
  "videos",
  "processing",
  "people",
  "entities",
];

function isCorpusRoute(route: AppRoute): route is CorpusRoute {
  return corpusRoutes.includes(route);
}

function routeFromLocation(): AppRoute {
  if (typeof window === "undefined") {
    return "analyzer";
  }
  for (const page of ["people", "entities"] as const) {
    const path = window.location.pathname;
    if (path === `/${page}` || path.startsWith(`/${page}/`)) {
      return page;
    }
  }
  if (
    window.location.pathname === "/processing" ||
    window.location.pathname.startsWith("/processing/")
  ) {
    return "processing";
  }
  if (window.location.pathname === "/videos" || window.location.pathname.startsWith("/videos/")) {
    return "videos";
  }
  if (window.location.pathname === "/sources") {
    return "sources";
  }
  if (window.location.pathname === "/search") {
    return "search";
  }
  if (window.location.pathname === "/corpora" || window.location.hash === "#corpora") {
    return "corpora";
  }
  if (window.location.hash === "#legacy") {
    return "legacy";
  }
  return "analyzer";
}

export default function App() {
  const [route, setRoute] = React.useState<AppRoute>(routeFromLocation);

  React.useEffect(() => {
    const syncRoute = () => setRoute(routeFromLocation());
    window.addEventListener("hashchange", syncRoute);
    window.addEventListener("popstate", syncRoute);
    return () => {
      window.removeEventListener("hashchange", syncRoute);
      window.removeEventListener("popstate", syncRoute);
    };
  }, []);

  if (shouldUseDirectBrowserAnalyzer()) {
    if (isCorpusRoute(route)) {
      return (
        <ErrorState>
          <StateViewTitle>Local corpus app required</StateViewTitle>
          <StateViewDescription>
            Corpora, Search, Videos, Channels & Sources and Processing need the local app and its
            database.
          </StateViewDescription>
          <Button asChild>
            <a href="./?mode=browser">Open browser analyzer</a>
          </Button>
        </ErrorState>
      );
    }
    return <DirectBrowserYouTubeAnalyzer />;
  }

  if (isCorpusRoute(route)) {
    return <CorpusWorkspace page={route} />;
  }

  if (route === "legacy") {
    return <LegacyApp />;
  }

  return <YouTubeAnalyzer />;
}
