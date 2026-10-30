-- Global canonical identities over corpus-local evidence (Atlas 5.2, 5.3; issue #104).
--
-- A *local unit* is the existing corpus-local evidence group: a named-entity mention group
-- (corpus, entity_type, entity_key) or an inferred corpus-local person (`people`). Units and their
-- mentions, tracks, clusters, names and corrections stay where they are; this layer only records
-- which canonical identity each unit currently belongs to. Same names or similar embeddings never
-- link units automatically: automatic links require shared source evidence, and researcher
-- decisions (confirmed links and kept-apart pairs) are never overwritten by re-resolution.
CREATE TABLE canonical_identities (
    id uuid PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('person', 'entity')),
    entity_type text NOT NULL,
    -- A merged identity keeps its id as a redirect to the identity it was merged into.
    merged_into uuid REFERENCES canonical_identities(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (merged_into IS NULL OR merged_into <> id)
);

CREATE INDEX canonical_identities_kind_idx ON canonical_identities(kind, entity_type)
    WHERE merged_into IS NULL;

CREATE TABLE canonical_identity_links (
    -- `entity:<corpus>:<type>:<key>` or `person:<people.id>`; stable across reruns.
    unit_key text PRIMARY KEY,
    unit_kind text NOT NULL CHECK (unit_kind IN ('entity_group', 'inferred_person')),
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    person_id uuid REFERENCES people(id) ON DELETE CASCADE,
    entity_type text,
    entity_key text,
    identity_id uuid NOT NULL REFERENCES canonical_identities(id) ON DELETE CASCADE,
    -- `auto` links are recomputed by resolution; `confirmed` links are researcher decisions.
    status text NOT NULL CHECK (status IN ('auto', 'confirmed')),
    -- Why the unit sits in its identity: `local` (its own identity), `shared_evidence`,
    -- `name_evidence` (an evidence-grounded #65 name) or `researcher`.
    basis text NOT NULL CHECK (basis IN ('local', 'shared_evidence', 'name_evidence', 'researcher')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (unit_kind = 'inferred_person' AND person_id IS NOT NULL)
        OR (unit_kind = 'entity_group' AND entity_type IS NOT NULL AND entity_key IS NOT NULL)
    )
);

CREATE INDEX canonical_identity_links_identity_idx ON canonical_identity_links(identity_id);
CREATE INDEX canonical_identity_links_key_idx ON canonical_identity_links(entity_type, entity_key);

-- Researcher decisions that two local units are different referents. Kept by unit key, so they
-- survive re-extraction, re-matching and re-resolution until the researcher clears them.
CREATE TABLE canonical_identity_rejections (
    unit_a text NOT NULL,
    unit_b text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (unit_a, unit_b),
    CHECK (unit_a < unit_b)
);

CREATE TABLE canonical_identity_corrections (
    id uuid PRIMARY KEY,
    action text NOT NULL CHECK (action IN ('confirm', 'merge', 'split', 'reject', 'unreject')),
    identity_id uuid NOT NULL,
    other_identity_id uuid,
    unit_keys text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Corpus membership in one place: direct video memberships plus videos of the corpus's tracked
-- sources. `corpora::corpus_video_ids` reads this view, and identity queries join it.
CREATE VIEW corpus_videos AS
SELECT vm.corpus_id, vm.video_id
FROM corpus_video_memberships vm
UNION
SELECT sm.corpus_id, v.id AS video_id
FROM corpus_source_memberships sm
JOIN corpus_subscription_items i ON i.subscription_id = sm.subscription_id
JOIN videos v
  ON v.source_url = i.source_url
  OR (v.youtube_id IS NOT NULL AND i.youtube_id = v.youtube_id);
