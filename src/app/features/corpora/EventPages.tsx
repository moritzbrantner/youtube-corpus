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
  addEventClaim,
  correctEvent,
  createEvent,
  getEventProfile,
  listEvents,
  listIdentities,
  loadTranscriptContext,
  type ClaimInput,
  type ClaimPredicate,
  type EventClaim,
  type EventCorrection,
  type EventDate,
  type EventSummary,
} from "./api";
import { errorMessage } from "./CorpusLabels";
import { formatInterval } from "./evidence";
import {
  corpusPageUrl,
  eventUrl,
  identityUrl,
  navigateInApp,
  readMomentParam,
  videoUrl,
  type CorpusUrlState,
  type MomentLocator,
} from "./url-state";

const eventKeys = {
  all: ["events"] as const,
  list: (corpusId: string | null, q: string) => ["events", "list", corpusId, q] as const,
  profile: (id: string, corpusId: string | null) => ["events", "profile", id, corpusId] as const,
};

type Props = {
  corpusId: string;
  corpusName: string;
  eventId: string | null;
  state: CorpusUrlState;
  onUpdate: (patch: Partial<CorpusUrlState>, history?: "push" | "replace") => void;
};

/** `/events` and `/events/:id` (#106, Atlas 7.2-7.3). */
export function EventPages(props: Props) {
  const moment = readMomentParam(new URL(window.location.href));
  return props.eventId ? (
    <EventProfilePage {...props} eventId={props.eventId} moment={moment} />
  ) : (
    <EventList {...props} moment={moment} />
  );
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function formatYear(year: number) {
  return year < 0 ? `${-year} BC` : `${year}`;
}

function formatPoint(year: number, month?: number | null, day?: number | null) {
  return [day ?? null, month ? MONTHS[month - 1] : null, formatYear(year)]
    .filter((part) => part !== null)
    .join(" ");
}

/** `14 Oct 1066`, `c. 1067`, `44 BC – 43 BC`; a missing date stays visibly unknown. */
export function formatEventDate(date: EventDate | null) {
  if (!date) return "Date unknown";
  const start = formatPoint(date.startYear, date.startMonth, date.startDay);
  const text =
    date.endYear != null
      ? `${start} – ${formatPoint(date.endYear, date.endMonth, date.endDay)}`
      : start;
  return date.approximate ? `c. ${text}` : text;
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

function withoutMoment(url: URL) {
  url.searchParams.delete("moment");
  return url;
}

/** The video moment a claim will cite, as recorded in the source. */
function MomentEvidence({ moment }: { moment: MomentLocator }) {
  const context = useQuery({
    queryKey: ["events", "moment", moment.evidenceId],
    queryFn: () => loadTranscriptContext(moment.evidenceId),
  });
  const match = context.data?.match;
  return (
    <div className="grid gap-1 rounded-md border border-border bg-muted px-3 py-2 text-sm">
      <span className="text-xs text-muted-foreground">
        Evidence · {moment.modality === "visible" ? "On-screen text" : "Spoken"}
        {match ? ` · ${formatInterval(match.startSeconds, match.endSeconds)}` : ""}
      </span>
      {context.isPending ? <span>Loading moment…</span> : null}
      {context.error ? (
        <span className="text-destructive">{errorMessage(context.error)}</span>
      ) : null}
      {match ? <blockquote>{match.text}</blockquote> : null}
    </div>
  );
}

type ClaimDraft = Omit<ClaimInput, "videoId" | "modality" | "evidenceId">;

function numberOrNull(value: string) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Predicate, typed date range or identity, polarity, certainty and qualification of one claim. */
function ClaimFields({ onChange }: { onChange: (draft: ClaimDraft | null) => void }) {
  const [predicate, setPredicate] = React.useState<ClaimPredicate>("date");
  const [fields, setFields] = React.useState({
    startYear: "",
    startMonth: "",
    startDay: "",
    endYear: "",
    endMonth: "",
    endDay: "",
  });
  const [approximate, setApproximate] = React.useState(false);
  const [identityQuery, setIdentityQuery] = React.useState("");
  const [identityId, setIdentityId] = React.useState("");
  const [polarity, setPolarity] = React.useState<"affirms" | "denies">("affirms");
  const [certainty, setCertainty] = React.useState<"stated" | "hedged" | "inferred">("stated");
  const [qualification, setQualification] = React.useState("");
  const identities = useQuery({
    queryKey: ["identities", "event-picker", predicate, identityQuery],
    queryFn: () =>
      listIdentities({
        kind: predicate === "place" ? "entity" : undefined,
        entityType: predicate === "place" ? "location" : undefined,
        q: identityQuery,
      }),
    enabled: predicate !== "date",
  });

  React.useEffect(() => {
    const startYear = numberOrNull(fields.startYear);
    const date: EventDate | null =
      startYear === null
        ? null
        : {
            startYear,
            startMonth: numberOrNull(fields.startMonth),
            startDay: numberOrNull(fields.startDay),
            endYear: numberOrNull(fields.endYear),
            endMonth: numberOrNull(fields.endMonth),
            endDay: numberOrNull(fields.endDay),
            approximate,
          };
    const base = { polarity, certainty, qualification };
    if (predicate === "date") {
      onChange({ ...base, predicate, date });
    } else {
      onChange(identityId ? { ...base, predicate, identityId } : null);
    }
  }, [predicate, fields, approximate, identityId, polarity, certainty, qualification, onChange]);

  const field = (key: keyof typeof fields, label: string, width = "w-24") => (
    <label className={`grid gap-1 text-sm ${width}`}>
      {label}
      <Input
        aria-label={label}
        inputMode="numeric"
        value={fields[key]}
        onChange={(event) => setFields((current) => ({ ...current, [key]: event.target.value }))}
      />
    </label>
  );

  return (
    <div className="grid gap-3">
      <div className="flex flex-wrap items-end gap-2">
        <label className="grid gap-1 text-sm">
          Claim about
          <NativeSelect
            aria-label="Claim about"
            value={predicate}
            onChange={(event) => setPredicate(event.target.value as ClaimPredicate)}
          >
            <option value="date">Date</option>
            <option value="participant">Participant</option>
            <option value="place">Place</option>
          </NativeSelect>
        </label>
        <label className="grid gap-1 text-sm">
          Stance
          <NativeSelect
            aria-label="Stance"
            value={polarity}
            onChange={(event) => setPolarity(event.target.value as "affirms" | "denies")}
          >
            <option value="affirms">Affirms</option>
            <option value="denies">Denies</option>
          </NativeSelect>
        </label>
        <label className="grid gap-1 text-sm">
          Certainty
          <NativeSelect
            aria-label="Certainty"
            value={certainty}
            onChange={(event) =>
              setCertainty(event.target.value as "stated" | "hedged" | "inferred")
            }
          >
            <option value="stated">Stated</option>
            <option value="hedged">Hedged</option>
            <option value="inferred">Inferred</option>
          </NativeSelect>
        </label>
        <label className="grid min-w-56 flex-1 gap-1 text-sm">
          Qualification
          <Input
            aria-label="Qualification"
            value={qualification}
            placeholder="e.g. according to the narrator"
            onChange={(event) => setQualification(event.target.value)}
          />
        </label>
      </div>
      {predicate === "date" ? (
        <div className="flex flex-wrap items-end gap-2">
          {field("startYear", "Year (negative = BC)", "w-40")}
          {field("startMonth", "Month")}
          {field("startDay", "Day")}
          <span className="pb-2 text-sm text-muted-foreground">to</span>
          {field("endYear", "End year", "w-28")}
          {field("endMonth", "End month")}
          {field("endDay", "End day")}
          <label className="flex items-center gap-2 pb-2 text-sm">
            <input
              type="checkbox"
              checked={approximate}
              onChange={(event) => setApproximate(event.target.checked)}
            />
            Approximate
          </label>
          <span className="pb-2 text-xs text-muted-foreground">
            Leave the year empty when the source gives no date.
          </span>
        </div>
      ) : (
        <div className="flex flex-wrap items-end gap-2">
          <label className="grid min-w-56 gap-1 text-sm">
            Find {predicate === "place" ? "place" : "person or entity"}
            <Input
              aria-label="Find identity"
              value={identityQuery}
              onChange={(event) => setIdentityQuery(event.target.value)}
              placeholder="Name"
            />
          </label>
          <label className="grid min-w-56 gap-1 text-sm">
            {predicate === "place" ? "Place" : "Participant"}
            <NativeSelect
              aria-label="Identity"
              value={identityId}
              onChange={(event) => setIdentityId(event.target.value)}
            >
              <option value="">Choose…</option>
              {(identities.data ?? []).map((identity) => (
                <option key={identity.id} value={identity.id}>
                  {identity.displayName} ({identity.entityType})
                </option>
              ))}
            </NativeSelect>
          </label>
        </div>
      )}
    </div>
  );
}

function claimFromMoment(moment: MomentLocator, draft: ClaimDraft): ClaimInput {
  return {
    ...draft,
    videoId: moment.videoId,
    modality: moment.modality,
    evidenceId: moment.evidenceId,
  };
}

function EventBadges({ event }: { event: EventSummary }) {
  return (
    <>
      {event.disputed ? <Badge variant="destructive">Disputed</Badge> : null}
      {event.preferredDate ? (
        <Badge variant="secondary">Preferred: {formatEventDate(event.preferredDate)}</Badge>
      ) : null}
      <span className="text-sm text-muted-foreground">
        {event.dateReadings} date {event.dateReadings === 1 ? "reading" : "readings"} ·{" "}
        {event.claimCount} {event.claimCount === 1 ? "claim" : "claims"} · {event.videoCount}{" "}
        {event.videoCount === 1 ? "video" : "videos"}
      </span>
    </>
  );
}

function yearSpan(event: EventSummary) {
  if (event.earliestYear == null) return "Date unknown";
  const earliest = formatYear(event.earliestYear);
  return event.latestYear != null && event.latestYear !== event.earliestYear
    ? `${earliest} – ${formatYear(event.latestYear)}`
    : earliest;
}

function EventList({
  corpusId,
  corpusName,
  state,
  onUpdate,
  moment,
}: Props & {
  moment: MomentLocator | null;
}) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = React.useState(state.identityQuery);
  const [label, setLabel] = React.useState("");
  const [claim, setClaim] = React.useState<ClaimDraft | null>(null);
  const scopedCorpus = state.identityScope === "all" ? null : corpusId;
  const listQuery = useQuery({
    queryKey: eventKeys.list(scopedCorpus, state.identityQuery),
    queryFn: () => listEvents({ corpusId: scopedCorpus, q: state.identityQuery }),
  });
  const create = useMutation({
    mutationFn: () => createEvent(label, claimFromMoment(moment!, claim!)),
    onSuccess: async ({ id }) => {
      await queryClient.invalidateQueries({ queryKey: eventKeys.all });
      const url = withoutMoment(eventUrl(new URL(window.location.href), id));
      window.history.pushState(null, "", url);
      window.dispatchEvent(new PopStateEvent("popstate"));
    },
  });
  return (
    <Surface>
      <SurfaceHeader>
        <SurfaceTitle>Events</SurfaceTitle>
        <SurfaceDescription>
          Historical events as groups of separate claims from video evidence. An event is not a
          consensus fact: each claim keeps its own date, source moment and review status.
        </SurfaceDescription>
      </SurfaceHeader>
      <SurfaceContent className="grid gap-4">
        {moment ? (
          <section
            className="grid gap-3 rounded-md border border-border p-3"
            aria-label="Record a claim"
          >
            <h3 className="text-sm font-semibold">Record a claim from this moment</h3>
            <MomentEvidence moment={moment} />
            <p className="text-sm text-muted-foreground">
              Create a new event from it, or open an existing event below and add it there.
            </p>
            <ClaimFields onChange={setClaim} />
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                create.mutate();
              }}
            >
              <label className="grid min-w-56 flex-1 gap-1 text-sm">
                New event
                <Input
                  aria-label="New event label"
                  value={label}
                  onChange={(event) => setLabel(event.target.value)}
                  placeholder="e.g. Battle of Hastings"
                />
              </label>
              <Button type="submit" disabled={!label.trim() || !claim || create.isPending}>
                Create event
              </Button>
            </form>
            {create.error ? (
              <p role="alert" className="text-sm text-destructive">
                {errorMessage(create.error)}
              </p>
            ) : null}
          </section>
        ) : null}
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            onUpdate({ identityQuery: draft.trim() });
          }}
        >
          <ScopeSelect corpusName={corpusName} state={state} onUpdate={onUpdate} />
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
        </form>
        {listQuery.isPending ? <LoadingState label="Loading events" /> : null}
        {listQuery.error ? (
          <ErrorState>
            <StateViewTitle>Events unavailable</StateViewTitle>
            <StateViewDescription>{errorMessage(listQuery.error)}</StateViewDescription>
          </ErrorState>
        ) : null}
        {listQuery.data?.length === 0 ? (
          <StateView variant="empty">
            <StateViewTitle>No events</StateViewTitle>
            <StateViewDescription>
              Open a video moment and choose “Record event claim” to start one.
            </StateViewDescription>
          </StateView>
        ) : null}
        {listQuery.data && listQuery.data.length > 0 ? (
          <ul className="grid gap-2" aria-label="Events">
            {listQuery.data.map((event) => (
              <li
                key={event.id}
                className="flex flex-wrap items-center gap-2 rounded-md border border-border px-3 py-2"
              >
                <span className="w-32 text-sm tabular-nums text-muted-foreground">
                  {yearSpan(event)}
                </span>
                <a
                  className="font-medium underline-offset-4 hover:underline"
                  href={eventUrl(new URL(window.location.href), event.id).href}
                  onClick={navigateInApp}
                >
                  {event.label}
                </a>
                <EventBadges event={event} />
                <span className="ml-auto text-sm text-muted-foreground">
                  {moment ? "Add the claim here · " : ""}
                  {event.corpora.map((corpus) => corpus.name).join(", ")}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
      </SurfaceContent>
    </Surface>
  );
}

const reviewLabels: Record<EventClaim["reviewStatus"], string> = {
  unverified: "Unverified",
  confirmed: "Confirmed",
  rejected: "Rejected",
};

function ClaimRow({
  claim,
  selected,
  onSelect,
  busy,
  onCorrect,
}: {
  claim: EventClaim;
  selected: boolean;
  onSelect: (selected: boolean) => void;
  busy: boolean;
  onCorrect: (correction: EventCorrection) => void;
}) {
  const current = new URL(window.location.href);
  const subject =
    claim.predicate === "date" ? (
      <span className="font-medium">{formatEventDate(claim.date)}</span>
    ) : claim.identity ? (
      <a
        className="font-medium underline-offset-4 hover:underline"
        href={
          identityUrl(
            current,
            claim.identity.kind === "person" ? "people" : "entities",
            claim.identity.id,
          ).href
        }
        onClick={navigateInApp}
      >
        {claim.identity.displayName}
      </a>
    ) : (
      <span className="font-medium">Identity without current evidence</span>
    );
  return (
    <li
      className={`grid gap-2 rounded-md border px-3 py-2 text-sm ${
        claim.reviewStatus === "rejected" ? "border-dashed opacity-70" : "border-border"
      }`}
      aria-label={`Claim ${claim.id}`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="checkbox"
          aria-label="Select claim for split"
          checked={selected}
          onChange={(event) => onSelect(event.target.checked)}
        />
        {claim.polarity === "denies" ? <Badge variant="destructive">Denies</Badge> : null}
        {subject}
        {claim.preferred ? <Badge variant="secondary">Preferred reading</Badge> : null}
        <Badge variant={claim.reviewStatus === "confirmed" ? "secondary" : "outline"}>
          {reviewLabels[claim.reviewStatus]}
        </Badge>
        {claim.certainty !== "stated" ? <Badge variant="outline">{claim.certainty}</Badge> : null}
        {claim.qualification ? (
          <span className="text-muted-foreground">“{claim.qualification}”</span>
        ) : null}
        <span className="ml-auto flex flex-wrap gap-1">
          {claim.reviewStatus !== "confirmed" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() =>
                onCorrect({ action: "review", claimIds: [claim.id], reviewStatus: "confirmed" })
              }
            >
              Confirm
            </Button>
          ) : null}
          {claim.reviewStatus !== "rejected" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() =>
                onCorrect({ action: "review", claimIds: [claim.id], reviewStatus: "rejected" })
              }
            >
              Reject
            </Button>
          ) : null}
          {claim.predicate === "date" && claim.date && !claim.preferred ? (
            <Button
              size="sm"
              disabled={busy}
              onClick={() => onCorrect({ action: "prefer", claimIds: [claim.id] })}
            >
              Prefer this reading
            </Button>
          ) : null}
        </span>
      </div>
      <a
        className="grid gap-1 rounded-md bg-muted px-3 py-2 hover:underline"
        href={
          withoutMoment(
            videoUrl(current, claim.videoId, {
              modality: claim.modality,
              evidenceId: claim.evidenceId,
            }),
          ).href
        }
        onClick={navigateInApp}
      >
        <span className="text-xs text-muted-foreground">
          {claim.videoTitle ?? "Video"} ·{" "}
          {claim.modality === "visible" ? "On-screen text" : "Spoken"} ·{" "}
          {formatInterval(claim.startSeconds, claim.endSeconds)}
        </span>
        <span>{claim.evidenceText ?? "The original evidence was reprocessed or removed."}</span>
      </a>
    </li>
  );
}

function EventProfilePage({
  corpusId,
  corpusName,
  eventId,
  state,
  onUpdate,
  moment,
}: Props & { eventId: string; moment: MomentLocator | null }) {
  const queryClient = useQueryClient();
  const scopedCorpus = state.identityScope === "all" ? null : corpusId;
  const [selected, setSelected] = React.useState<string[]>([]);
  const [splitLabel, setSplitLabel] = React.useState("");
  const [mergeId, setMergeId] = React.useState("");
  const [claim, setClaim] = React.useState<ClaimDraft | null>(null);
  const profileQuery = useQuery({
    queryKey: eventKeys.profile(eventId, scopedCorpus),
    queryFn: () => getEventProfile(eventId, scopedCorpus),
  });
  const others = useQuery({
    queryKey: eventKeys.list(null, ""),
    queryFn: () => listEvents({}),
  });
  const refresh = () => queryClient.invalidateQueries({ queryKey: eventKeys.all });
  const correction = useMutation({
    mutationFn: (input: EventCorrection) => correctEvent(eventId, input),
    onSuccess: async () => {
      setSelected([]);
      setSplitLabel("");
      setMergeId("");
      await refresh();
    },
  });
  const add = useMutation({
    mutationFn: () => addEventClaim(eventId, claimFromMoment(moment!, claim!)),
    onSuccess: async () => {
      window.history.replaceState(null, "", withoutMoment(new URL(window.location.href)));
      window.dispatchEvent(new PopStateEvent("popstate"));
      await refresh();
    },
  });
  const profile = profileQuery.data;

  // A merged event redirects to the event it was merged into.
  React.useEffect(() => {
    if (profile && profile.event.id !== eventId) {
      window.history.replaceState(
        null,
        "",
        eventUrl(new URL(window.location.href), profile.event.id),
      );
      window.dispatchEvent(new PopStateEvent("popstate"));
    }
  }, [eventId, profile]);

  const backHref = corpusPageUrl(new URL(window.location.href), "events").href;
  if (profileQuery.isPending) {
    return <LoadingState label="Loading event" />;
  }
  if (profileQuery.error || !profile) {
    return (
      <ErrorState>
        <StateViewTitle>Event unavailable</StateViewTitle>
        <StateViewDescription>{errorMessage(profileQuery.error)}</StateViewDescription>
        <Button asChild variant="outline">
          <a href={backHref} onClick={navigateInApp}>
            Back to Events
          </a>
        </Button>
      </ErrorState>
    );
  }
  const { event } = profile;
  const dates = profile.claims.filter((c) => c.predicate === "date");
  const participants = profile.claims.filter((c) => c.predicate === "participant");
  const places = profile.claims.filter((c) => c.predicate === "place");
  const busy = correction.isPending;
  const row = (c: EventClaim) => (
    <ClaimRow
      key={c.id}
      claim={c}
      busy={busy}
      selected={selected.includes(c.id)}
      onSelect={(on) =>
        setSelected((ids) => (on ? [...ids, c.id] : ids.filter((id) => id !== c.id)))
      }
      onCorrect={(input) => correction.mutate(input)}
    />
  );
  return (
    <div className="grid gap-4">
      <Surface>
        <SurfaceHeader>
          <a className="text-sm underline" href={backHref} onClick={navigateInApp}>
            Events
          </a>
          <SurfaceTitle>{event.label}</SurfaceTitle>
          <SurfaceDescription>
            Claims from {event.corpora.map((corpus) => corpus.name).join(", ") || "no corpus"}.
            Contradictory claims stay side by side; a preferred reading keeps the alternatives.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-4">
          <div className="flex flex-wrap items-center gap-2">
            <EventBadges event={event} />
          </div>
          <div className="flex flex-wrap items-end gap-2">
            <ScopeSelect corpusName={corpusName} state={state} onUpdate={onUpdate} />
            {event.preferredDate ? (
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => correction.mutate({ action: "unprefer" })}
              >
                Clear preferred reading
              </Button>
            ) : null}
          </div>
          {profile.claimsOutsideScope > 0 ? (
            <p className="text-sm text-muted-foreground">
              {profile.claimsOutsideScope} more{" "}
              {profile.claimsOutsideScope === 1 ? "claim comes" : "claims come"} from videos outside{" "}
              {corpusName}. Switch the scope to All corpora to compare them.
            </p>
          ) : null}
          {correction.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(correction.error)}
            </p>
          ) : null}
          <section className="grid gap-2" aria-label="Date claims">
            <h3 className="text-sm font-semibold">Date claims</h3>
            {dates.length === 0 ? (
              <p className="text-sm text-muted-foreground">No date claims in this scope.</p>
            ) : (
              <ul className="grid gap-2">{dates.map(row)}</ul>
            )}
          </section>
          <section className="grid gap-2" aria-label="Participant claims">
            <h3 className="text-sm font-semibold">Participants</h3>
            {participants.length === 0 ? (
              <p className="text-sm text-muted-foreground">No participant claims in this scope.</p>
            ) : (
              <ul className="grid gap-2">{participants.map(row)}</ul>
            )}
          </section>
          <section className="grid gap-2" aria-label="Place claims">
            <h3 className="text-sm font-semibold">Places</h3>
            {places.length === 0 ? (
              <p className="text-sm text-muted-foreground">No place claims in this scope.</p>
            ) : (
              <ul className="grid gap-2">{places.map(row)}</ul>
            )}
          </section>
        </SurfaceContent>
      </Surface>
      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Grouping</SurfaceTitle>
          <SurfaceDescription>
            Merging and splitting move claims between events; claims and their video evidence are
            never deleted.
          </SurfaceDescription>
        </SurfaceHeader>
        <SurfaceContent className="grid gap-3">
          <form
            className="flex flex-wrap items-end gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              correction.mutate({
                action: "split",
                claimIds: selected,
                label: splitLabel.trim() || undefined,
              });
            }}
          >
            <label className="grid min-w-56 flex-1 gap-1 text-sm">
              New event for the selected claims
              <Input
                aria-label="Split event label"
                value={splitLabel}
                placeholder={`${event.label} (split)`}
                onChange={(e) => setSplitLabel(e.target.value)}
              />
            </label>
            <Button
              type="submit"
              variant="outline"
              disabled={busy || selected.length === 0 || selected.length >= profile.claims.length}
            >
              Split selected ({selected.length})
            </Button>
          </form>
          <form
            className="flex flex-wrap items-end gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              correction.mutate({ action: "merge", otherEventId: mergeId });
            }}
          >
            <label className="grid min-w-56 flex-1 gap-1 text-sm">
              Same event as
              <NativeSelect
                aria-label="Merge event"
                value={mergeId}
                onChange={(e) => setMergeId(e.target.value)}
              >
                <option value="">Choose an event…</option>
                {(others.data ?? [])
                  .filter((other) => other.id !== event.id)
                  .map((other) => (
                    <option key={other.id} value={other.id}>
                      {other.label} ({yearSpan(other)})
                    </option>
                  ))}
              </NativeSelect>
            </label>
            <Button type="submit" variant="outline" disabled={busy || !mergeId}>
              Merge into this event
            </Button>
          </form>
        </SurfaceContent>
      </Surface>
      <Surface>
        <SurfaceHeader>
          <SurfaceTitle>Add a claim</SurfaceTitle>
          <SurfaceDescription>
            {moment
              ? "Record what this moment says about the event."
              : "Open a video moment and choose “Record event claim”, then pick this event."}
          </SurfaceDescription>
        </SurfaceHeader>
        {moment ? (
          <SurfaceContent className="grid gap-3">
            <MomentEvidence moment={moment} />
            <ClaimFields onChange={setClaim} />
            <div>
              <Button disabled={!claim || add.isPending} onClick={() => add.mutate()}>
                Add claim
              </Button>
            </div>
            {add.error ? (
              <p role="alert" className="text-sm text-destructive">
                {errorMessage(add.error)}
              </p>
            ) : null}
          </SurfaceContent>
        ) : null}
      </Surface>
    </div>
  );
}
