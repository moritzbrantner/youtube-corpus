-- Historical events and their separate source-backed claims (Atlas 7.2, 7.3; issue #106).
--
-- An event is only a grouping of claims that researchers think refer to the same occurrence; it
-- is not a consensus fact. Each claim is one assertion (subject = the event, a predicate, an
-- object or literal value) anchored to one original video moment: a transcript segment
-- (`spoken`) or an OCR text track (`visible`). Claims keep their own date reading, certainty,
-- qualification and review status, aligned with the historical-source-atlas v2 assertion model.
-- Grouping corrections move claims between events; they never delete claims or source evidence.
CREATE TABLE historical_events (
    id uuid PRIMARY KEY,
    label text NOT NULL CHECK (length(trim(label)) > 0),
    -- A merged event keeps its id as a redirect to the event it was merged into.
    merged_into uuid REFERENCES historical_events(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (merged_into IS NULL OR merged_into <> id)
);

CREATE TABLE historical_event_claims (
    id uuid PRIMARY KEY,
    event_id uuid NOT NULL REFERENCES historical_events(id),
    video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    modality text NOT NULL CHECK (modality IN ('spoken', 'visible')),
    -- A transcript segment (`spoken`) or visual text track (`visible`) of `video_id`. Not a
    -- foreign key: reprocessing may replace evidence, and the claim must then say so, not vanish.
    evidence_id uuid NOT NULL,
    -- `date`: when the event happened; `participant`: who/what took part; `place`: where.
    predicate text NOT NULL CHECK (predicate IN ('date', 'participant', 'place')),
    -- Typed date range; years are historical (negative = BC, no year 0). A null start is a
    -- date the source leaves unknown; a null end is a single point (year, month or day).
    date_start_year integer CHECK (date_start_year <> 0),
    date_start_month smallint CHECK (date_start_month BETWEEN 1 AND 12),
    date_start_day smallint CHECK (date_start_day BETWEEN 1 AND 31),
    date_end_year integer CHECK (date_end_year <> 0),
    date_end_month smallint CHECK (date_end_month BETWEEN 1 AND 12),
    date_end_day smallint CHECK (date_end_day BETWEEN 1 AND 31),
    date_approximate boolean NOT NULL DEFAULT false,
    -- Participant or place: a global canonical identity (#104), never a per-corpus copy.
    identity_id uuid REFERENCES canonical_identities(id),
    -- `affirms` or `denies` the predicate value (e.g. "X did not take part").
    polarity text NOT NULL DEFAULT 'affirms' CHECK (polarity IN ('affirms', 'denies')),
    certainty text NOT NULL DEFAULT 'stated' CHECK (certainty IN ('stated', 'hedged', 'inferred')),
    qualification text NOT NULL DEFAULT '',
    review_status text NOT NULL DEFAULT 'unverified'
        CHECK (review_status IN ('unverified', 'confirmed', 'rejected')),
    -- The reviewed preferred date reading of its event; alternatives stay as they are.
    preferred boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (predicate <> 'date' OR identity_id IS NULL),
    CHECK (predicate = 'date' OR (identity_id IS NOT NULL AND date_start_year IS NULL
                                  AND date_end_year IS NULL)),
    CHECK (date_start_month IS NOT NULL OR date_start_day IS NULL),
    CHECK (date_end_month IS NOT NULL OR date_end_day IS NULL),
    CHECK (date_start_year IS NOT NULL OR date_end_year IS NULL),
    CHECK (NOT preferred OR (predicate = 'date' AND review_status = 'confirmed'))
);

CREATE INDEX historical_event_claims_event_idx ON historical_event_claims(event_id);
CREATE INDEX historical_event_claims_video_idx ON historical_event_claims(video_id);
CREATE INDEX historical_event_claims_identity_idx ON historical_event_claims(identity_id);
CREATE UNIQUE INDEX historical_event_claims_preferred_idx
    ON historical_event_claims(event_id) WHERE preferred;

CREATE TABLE historical_event_corrections (
    id uuid PRIMARY KEY,
    action text NOT NULL CHECK (action IN ('review', 'prefer', 'unprefer', 'rename', 'merge', 'split')),
    event_id uuid NOT NULL,
    other_event_id uuid,
    claim_ids uuid[] NOT NULL DEFAULT '{}',
    detail text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now()
);
