import {
  Button,
  ErrorState,
  LoadingState,
  StateView,
  StateViewActions,
  StateViewDescription,
  StateViewTitle,
} from "@moritzbrantner/ui";
import { Settings } from "lucide-react";
import * as React from "react";

export function SearchState({
  variant = "empty",
  title,
  description,
  actions,
}: {
  variant?: "empty" | "loading" | "error";
  title: string;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  const content = (
    <>
      <StateViewTitle>{title}</StateViewTitle>
      {description ? <StateViewDescription>{description}</StateViewDescription> : null}
      {actions ? <StateViewActions>{actions}</StateViewActions> : null}
    </>
  );

  if (variant === "loading") {
    return <LoadingState size="lg" label={title} />;
  }
  if (variant === "error") {
    return <ErrorState size="lg">{content}</ErrorState>;
  }
  return (
    <StateView variant="empty" size="lg">
      {content}
    </StateView>
  );
}

export function ConnectionStatusButton({ onClick }: { onClick: () => void }) {
  return (
    <Button type="button" variant="outline" onClick={onClick}>
      <Settings className="size-4" aria-hidden="true" />
      Connection status
    </Button>
  );
}
