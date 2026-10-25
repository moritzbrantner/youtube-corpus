-- Evidence-grounded names for inferred corpus-local people (Atlas 4.7, issue #65). A name is
-- stored beside the person, never in place of its face/voice observations or matching links,
-- and keeps the exact supporting spans plus producer/algorithm provenance.
CREATE TABLE person_name_attributions (
    person_id uuid PRIMARY KEY REFERENCES people(id) ON DELETE CASCADE,
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    name text NOT NULL CHECK (btrim(name) <> ''),
    name_key text NOT NULL CHECK (btrim(name_key) <> ''),
    status text NOT NULL CHECK (status IN ('inferred', 'confirmed', 'user_provided')),
    confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    algorithm text NOT NULL,
    algorithm_version text NOT NULL,
    configuration jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX person_name_attributions_corpus_idx ON person_name_attributions(corpus_id, name_key);

CREATE TABLE person_name_evidence (
    id uuid PRIMARY KEY,
    person_id uuid NOT NULL REFERENCES person_name_attributions(person_id) ON DELETE CASCADE,
    video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    modality text NOT NULL CHECK (modality IN ('visible_text')),
    entity_mention_id uuid NOT NULL REFERENCES entity_mentions(id) ON DELETE CASCADE,
    visual_text_track_id uuid NOT NULL REFERENCES visual_text_tracks(id) ON DELETE CASCADE,
    face_track_id uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
    start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
    end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
    surface_text text NOT NULL,
    source_text text NOT NULL,
    byte_start integer NOT NULL CHECK (byte_start >= 0),
    byte_end integer NOT NULL CHECK (byte_end > byte_start),
    confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    provenance jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX person_name_evidence_person_idx ON person_name_evidence(person_id);

-- A researcher-rejected name stays rejected for that person across reruns.
CREATE TABLE person_name_rejections (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    name_key text NOT NULL,
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (person_id, name_key)
);

CREATE TABLE person_name_corrections (
    id uuid PRIMARY KEY,
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    action text NOT NULL CHECK (action IN ('confirm', 'reject', 'replace')),
    previous_name text,
    name text,
    created_at timestamptz NOT NULL DEFAULT now()
);
