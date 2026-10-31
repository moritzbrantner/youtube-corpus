import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  Badge,
  Button,
  ErrorState,
  Input,
  LoadingState,
  NativeSelect,
  StateView,
  StateViewDescription,
  StateViewTitle,
} from "@moritzbrantner/ui";
import {
  Surface,
  SurfaceContent,
  SurfaceDescription,
  SurfaceHeader,
  SurfaceTitle,
} from "@moritzbrantner/ui/shell";

import {
  correctIdentity,
  getIdentityProfile,
  listIdentities,
  resolveIdentities,
  type IdentityCorrection,
  type IdentityLocalUnit,
  type IdentitySummary,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { nameStatusLabel } from "./PersonNamesReview";
import { ProfileEvidence } from "./ProfileEvidence";
import {
  corpusPageUrl,
  identityUrl,
  navigateInApp,
  type CorpusUrlState,
  type IdentityPage,
} from "./url-state";

/** Entity types with a canonical identity on the Entities page; dates and amounts get none. */
const ENTITY_TYPES = ["organization", "location", "product", "law", "work", "event", "misc"];

const identityKeys = {
  all: ["identities"] as const,
  list: (page: IdentityPage, corpusId: string | null, type: string, q: string) =>
    ["identities", "list", page, corpusId, type, q] as const,
  profile: (id: string, corpusId: string | null) =>
    ["identities", "profile", id, corpusId] as const,
};

type Props = {
  page: IdentityPage;
  corpusId: string;
  corpusName: string;
  identityId: string | null;
  state: CorpusUrlState;
  onUpdate: (patch: Partial<CorpusUrlState>, history?: "push" | "replace") => void;
};

/** `/people`, `/entities` and their profiles (#104, Atlas 5.2-5.3). */
export function IdentityPages(props: Props) {
  return props.identityId ? (
    <IdentityProfilePage {...props} identityId={props.identityId} />
  ) : (
    <IdentityList {...props} />
  );
}

function ScopeSelect({
  corpusName,
  state,
  onUpdate,
}: Pick<Props, "corpusName" | "state" | "onUpdate">) {
  return (
    <label className="grid gap-1 text-sm">
      Scope
      <NativeSelect
        aria-label="Scope"
        value={state.identityScope}
        onChange={(event) => onUpdate({ identityScope: event.target.value === "all" ? "all" : "" })}
      >
        <option value="">{corpusName}</option>
        <option value="all">All corpora</option>
      </NativeSelect>
    </label>
  );
}

function IdentityList({ page, corpusId, corpusName, state, onUpdate }: Props) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState(state.identityQuery);
  const scopedCorpus = state.identityScope === "all" ? null : corpusId;
  const type = page === "entities" ? state.identityType : "";
  const listQuery = useQuery({
    queryKey: identityKeys.list(page, scopedCorpus, type, state.identityQuery),
    queryFn: () =>
      listIdentities({
        kind: page === "people" ? "person" : "entity",
        entityType: type || undefined,
        corpusId: scopedCorpus,
        q: state.identityQuery,
      }),
  });
  const refresh = useMutation({
    mutationFn: resolveIdentities,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: identityKeys.all }),
  });
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>{page === "people" ? "People" : "Entities"}</SurfaceTitle>
        <SurfaceDescription>
          {page === "people"
            ? "Mentioned people and people inferred from face and voice tracks. "
            : "Organizations, places, products, laws, works, events and other named entities. "}
          Each profile groups corpus-local evidence. Groups join automatically only on shared
          evidence; the same name alone is a suggestion for review, never a link.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            onUpdate({ identityQuery: draft.trim() });
          }}
        >
          <ScopeSelect corpusName={corpusName} state={state} onUpdate={onUpdate} />
          {page === "entities" ? (
            <label className="grid gap-1 text-sm">
              Type
              <NativeSelect
                aria-label="Entity type"
                value={state.identityType}
                onChange={(event) => onUpdate({ identityType: event.target.value })}
              >
                <option value="">All types</option>
                {ENTITY_TYPES.map((entityType) => (
                  <option key={entityType} value={entityType}>
                    {entityType}
                  </option>
                ))}
              </NativeSelect>
            </label>
          ) : null}
          <label className="grid min-w-56 flex-1 gap-1 text-sm">
            Name
            <Input
              aria-label="Name"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="Filter by name"
            />
          </label>
          <Button type="submit">Filter</Button>
          <Button
            type="button"
            variant="outline"
            disabled={refresh.isPending}
            onClick={() => refresh.mutate()}
          >
            {refresh.isPending ? "Refreshing..." : "Refresh identities"}
          </Button>
        </form>
        {refresh.error ? (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(refresh.error)}
          </p>
        ) : null}
        {listQuery.isPending ? <LoadingState label="Loading identities" /> : null}
        {listQuery.error ? (
          <ErrorState>
            <StateViewTitle>Identities unavailable</StateViewTitle>
            <StateViewDescription>{errorMessage(listQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {listQuery.data?.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No identities</StateViewTitle>
            <StateViewDescription>
              Extract entities or match faces first; identities are refreshed after each run.
            </StateViewDescription>
          </StateView>
        ) : null}
        {listQuery.data && listQuery.data.length > 0 ? (
          <ul className="grid gap-2" aria-label={page === "people" ? "People" : "Entities"}>
            {listQuery.data.map((identity) => (
              <IdentityRow key={identity.id} page={page} identity={identity} />
            ))}
          </ul>
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

function nameSourceBadge(identity: IdentitySummary) {
  switch (identity.nameSource) {
    case "researcher":
      return <Badge variant="secondary">Researcher-confirmed name</Badge>;
    case "inferred_name":
      return <Badge variant="outline">Inferred name</Badge>;
    case "unnamed":
      return <Badge variant="outline">Inferred, unnamed</Badge>;
    default:
      return null;
  }
}

function EvidenceBadges({ identity }: { identity: IdentitySummary }) {
  return (
    <>
      {identity.spokenMentions > 0 ? (
        <Badge variant="secondary">Spoken ×{identity.spokenMentions}</Badge>
      ) : null}
      {identity.visibleTextMentions > 0 ? (
        <Badge variant="secondary">On-screen text ×{identity.visibleTextMentions}</Badge>
      ) : null}
      {identity.shownTracks > 0 ? (
        <Badge variant="secondary">Shown ×{identity.shownTracks}</Badge>
      ) : null}
      {identity.heardTracks > 0 ? (
        <Badge variant="secondary">Heard ×{identity.heardTracks}</Badge>
      ) : null}
      <span className="text-sm text-muted-foreground">
        {identity.videoCount} {identity.videoCount === 1 ? "video" : "videos"}
      </span>
    </>
  );
}

function IdentityLink({ page, identity }: { page: IdentityPage; identity: IdentitySummary }) {
  return (
    <a
      className="font-medium underline-offset-4 hover:underline"
      href={identityUrl(new URL(window.location.href), page, identity.id).href}
      onClick={navigateInApp}
    >
      {identity.displayName}
    </a>
  );
}

function IdentityRow({ page, identity }: { page: IdentityPage; identity: IdentitySummary }) {
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2">
      <IdentityLink page={page} identity={identity} />
      {page === "entities" ? <Badge variant="outline">{identity.entityType}</Badge> : null}
      {nameSourceBadge(identity)}
      <EvidenceBadges identity={identity} />
      <span className="ml-auto text-sm text-muted-foreground">
        {identity.corpora.map((corpus) => corpus.name).join(", ")}
      </span>
    </li>
  );
}

const basisLabels: Record<IdentityLocalUnit["basis"], string> = {
  local: "Own evidence",
  shared_evidence: "Shared evidence",
  name_evidence: "Name evidence",
  researcher: "Researcher decision",
};

function IdentityProfilePage({
  page,
  corpusId,
  corpusName,
  identityId,
  state,
  onUpdate,
}: Props & { identityId: string }) {
  const queryClient = useQueryClient();
  const scopedCorpus = state.identityScope === "all" ? null : corpusId;
  const [splitKeys, setSplitKeys] = React.useState<string[]>([]);
  const profileQuery = useQuery({
    queryKey: identityKeys.profile(identityId, scopedCorpus),
    queryFn: () => getIdentityProfile(identityId, scopedCorpus),
  });
  const correction = useMutation({
    mutationFn: (input: IdentityCorrection) => correctIdentity(identityId, input),
    onSuccess: async () => {
      setSplitKeys([]);
      await queryClient.invalidateQueries({ queryKey: identityKeys.all });
      await queryClient.invalidateQueries({ queryKey: ["corpora"] });
    },
  });
  const profile = profileQuery.data;

  // A merged identity redirects to the identity it was merged into.
  React.useEffect(() => {
    if (profile && profile.identity.id !== identityId) {
      const url = identityUrl(new URL(window.location.href), page, profile.identity.id);
      window.history.replaceState(null, "", url);
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }, [identityId, page, profile]);

  const backHref = corpusPageUrl(new URL(window.location.href), page).href;
  if (profileQuery.isPending) {
    return <LoadingState label="Loading profile" />;
  }
  if (profileQuery.error || !profile) {
    return (
      <ErrorState>
        <StateViewTitle>Profile unavailable</StateViewTitle>
        <StateViewDescription>{errorMessage(profileQuery.error)}</StateViewDescription>
        <Button asChild variant="outline">
          <a href={backHref} onClick={navigateInApp}>
            Back to {page === "people" ? "People" : "Entities"}
          </a>
        </Button>
      </ErrorState>
    );
  }
  const { identity } = profile;
  return (
    <div className="grid gap-4">
      <Surface>
        <SurfaceHeader>
          <div className="flex flex-wrap items-center gap-2">
            <a className="text-sm underline" href={backHref} onClick={navigateInApp}>
              {page === "people" ? "People" : "Entities"}
            </a>
          </div>
          <SurfaceTitle>{identity.displayName}</SurfaceTitle>
          <SurfaceDescription>
            A global profile over {identity.localUnits} corpus-local{" "}
            {identity.localUnits === 1 ? "group" : "groups"} in{" "}
            {identity.corpora.map((corpus) => corpus.name).join(", ")}. Local evidence, names and
            provenance stay with their corpus.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline">{identity.entityType}</Badge>
            {nameSourceBadge(identity)}
            {identity.hasConfirmedLinks ? (
              <Badge variant="secondary">Researcher-confirmed links</Badge>
            ) : (
              <Badge variant="outline">Automatic links</Badge>
            )}
            <EvidenceBadges identity={identity} />
          </div>
          <div className="flex flex-wrap items-end gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={correction.isPending || identity.hasConfirmedLinks}
              onClick={() => correction.mutate({ action: "confirm" })}
            >
              Confirm links
            </Button>
            <Button
              type="button"
              variant="outline"
              disabled={
                correction.isPending ||
                splitKeys.length === 0 ||
                splitKeys.length >= profile.units.length
              }
              onClick={() => correction.mutate({ action: "split", unitKeys: splitKeys })}
            >
              Split selected ({splitKeys.length})
            </Button>
          </div>
          {correction.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(correction.error)}
            </p>
          ) : null}
          <section className="grid gap-2" aria-label="Local groups">
            <h3 className="text-sm font-semibold">Local groups</h3>
            <ul className="grid gap-2">
              {profile.units.map((unit) => (
                <li
                  key={unit.unitKey}
                  className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2 text-sm"
                >
                  <input
                    type="checkbox"
                    aria-label={`Select ${unit.label} in ${unit.corpusName} for split`}
                    checked={splitKeys.includes(unit.unitKey)}
                    disabled={profile.units.length < 2}
                    onChange={(event) =>
                      setSplitKeys((keys) =>
                        event.target.checked
                          ? [...keys, unit.unitKey]
                          : keys.filter((key) => key !== unit.unitKey),
                      )
                    }
                  />
                  <span className="font-medium">{unit.label || "Unnamed person"}</span>
                  <Badge variant="outline">
                    {unit.unitKind === "inferred_person" ? "Face/voice person" : "Mentions"}
                  </Badge>
                  {unit.nameStatus ? (
                    <Badge variant="outline">{nameStatusLabel(unit.nameStatus)}</Badge>
                  ) : null}
                  <Badge variant={unit.basis === "researcher" ? "secondary" : "outline"}>
                    {basisLabels[unit.basis]}
                  </Badge>
                  {unit.status === "confirmed" ? (
                    <Badge variant="secondary">Confirmed</Badge>
                  ) : null}
                  <span className="ml-auto text-muted-foreground">{unit.corpusName}</span>
                </li>
              ))}
            </ul>
          </section>
          {profile.suggestions.length > 0 ? (
            <section className="grid gap-2" aria-label="Possible matches">
              <h3 className="text-sm font-semibold">Possible matches</h3>
              <p className="text-sm text-muted-foreground">
                These share a name, which is not evidence that they are the same referent.
              </p>
              <ul className="grid gap-2">
                {profile.suggestions.map(({ identity: other }) => (
                  <li
                    key={other.id}
                    className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2"
                  >
                    <IdentityLink page={page} identity={other} />
                    <EvidenceBadges identity={other} />
                    <span className="text-sm text-muted-foreground">
                      {other.corpora.map((corpus) => corpus.name).join(", ")}
                    </span>
                    <span className="ml-auto flex gap-2">
                      <Button
                        size="sm"
                        disabled={correction.isPending}
                        onClick={() =>
                          correction.mutate({ action: "merge", otherIdentityId: other.id })
                        }
                      >
                        Same referent: merge
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={correction.isPending}
                        onClick={() =>
                          correction.mutate({ action: "reject", otherIdentityId: other.id })
                        }
                      >
                        Different: keep apart
                      </Button>
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
          {profile.keptApart.length > 0 ? (
            <section className="grid gap-2" aria-label="Kept apart">
              <h3 className="text-sm font-semibold">Kept apart</h3>
              <ul className="grid gap-2">
                {profile.keptApart.map((other) => (
                  <li
                    key={other.id}
                    className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2"
                  >
                    <IdentityLink page={page} identity={other} />
                    <span className="text-sm text-muted-foreground">
                      {other.corpora.map((corpus) => corpus.name).join(", ")}
                    </span>
                    <Button
                      className="ml-auto"
                      size="sm"
                      variant="outline"
                      disabled={correction.isPending}
                      onClick={() =>
                        correction.mutate({ action: "unreject", otherIdentityId: other.id })
                      }
                    >
                      Undo keep apart
                    </Button>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
        </SurfaceContent>
      </Surface>
      <ProfileEvidence
        profile={profile}
        corpusId={corpusId}
        corpusName={corpusName}
        state={state}
        onUpdate={onUpdate}
      />
    </div>
  );
}
