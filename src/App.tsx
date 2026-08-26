import { Button } from "@moritzbrantner/ui";
import * as React from "react";

import LegacyApp from "./LegacyApp";
import { CorpusWorkspace } from "./app/features/corpora/CorpusWorkspace";

type AppRoute = "corpora" | "legacy";

function routeFromLocation(): AppRoute {
  if (typeof window === "undefined") {
    return "legacy";
  }
  return window.location.hash === "#corpora" ? "corpora" : "legacy";
}

export default function App() {
  const [route, setRoute] = React.useState<AppRoute>(routeFromLocation);

  React.useEffect(() => {
    const syncRoute = () => setRoute(routeFromLocation());
    window.addEventListener("hashchange", syncRoute);
    return () => window.removeEventListener("hashchange", syncRoute);
  }, []);

  if (route === "corpora") {
    return <CorpusWorkspace />;
  }

  return (
    <>
      <div className="fixed bottom-4 right-4 z-50">
        <Button asChild variant="outline" size="sm">
          <a href="#corpora">Research corpora</a>
        </Button>
      </div>
      <LegacyApp />
    </>
  );
}
