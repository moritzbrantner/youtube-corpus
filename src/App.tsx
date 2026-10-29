import * as React from "react";
import { ErrorState, StateViewTitle, StateViewDescription, Button } from "@moritzbrantner/ui";

import LegacyApp from "./LegacyApp";
import {
  DirectBrowserYouTubeAnalyzer,
  shouldUseDirectBrowserAnalyzer,
} from "./app/features/analyzer/DirectBrowserYouTubeAnalyzer";
import { YouTubeAnalyzer } from "./app/features/analyzer/YouTubeAnalyzer";
import { CorpusWorkspace } from "./app/features/corpora/CorpusWorkspace";

type AppRoute = "analyzer" | "corpora" | "search" | "sources" | "legacy";

function routeFromLocation(): AppRoute {
  if (typeof window === "undefined") {
    return "analyzer";
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
    if (route === "corpora" || route === "search" || route === "sources") {
      return (
        <ErrorState>
          <StateViewTitle>Local corpus app required</StateViewTitle>
          <StateViewDescription>
            Corpora, Search and Channels & Sources need the local app and its database.
          </StateViewDescription>
          <Button asChild>
            <a href="./?mode=browser">Open browser analyzer</a>
          </Button>
        </ErrorState>
      );
    }
    return <DirectBrowserYouTubeAnalyzer />;
  }

  if (route === "corpora" || route === "search" || route === "sources") {
    return <CorpusWorkspace page={route} />;
  }

  if (route === "legacy") {
    return <LegacyApp />;
  }

  return <YouTubeAnalyzer />;
}
