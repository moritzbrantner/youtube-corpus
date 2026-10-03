import * as React from "react";

import LegacyApp from "./LegacyApp";
import {
  DirectBrowserYouTubeAnalyzer,
  shouldUseDirectBrowserAnalyzer,
} from "./app/features/analyzer/DirectBrowserYouTubeAnalyzer";
import { YouTubeAnalyzer } from "./app/features/analyzer/YouTubeAnalyzer";
import { CorpusWorkspace } from "./app/features/corpora/CorpusWorkspace";

type AppRoute = "analyzer" | "corpora" | "legacy";

function routeFromLocation(): AppRoute {
  if (typeof window === "undefined") {
    return "analyzer";
  }
  if (window.location.hash === "#corpora") {
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
    return () => window.removeEventListener("hashchange", syncRoute);
  }, []);

  if (shouldUseDirectBrowserAnalyzer()) {
    return <DirectBrowserYouTubeAnalyzer />;
  }

  if (route === "corpora") {
    return <CorpusWorkspace />;
  }

  if (route === "legacy") {
    return <LegacyApp />;
  }

  return <YouTubeAnalyzer />;
}
