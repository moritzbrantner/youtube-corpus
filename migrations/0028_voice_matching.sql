-- Cross-video voice matching into corpus people and face-to-voice links (issue #134).
--
-- Voice matches reuse the #8 identity tables (`voice_clusters`, `voice_cluster_members`,
-- `person_voice_clusters`) exactly as face matching (#64) uses the face ones. Automatic
-- associations are unverified suggestions until a researcher confirms them; the tables below
-- keep those corrections across reruns, like `face_match_*`.
CREATE TABLE voice_match_decisions (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    track_id uuid NOT NULL REFERENCES voice_tracks(id) ON DELETE CASCADE,
    person_id uuid REFERENCES people(id) ON DELETE SET NULL,
    decision text NOT NULL CHECK (decision IN ('unlink', 'split', 'link', 'confirm')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corpus_id, track_id)
);

CREATE TABLE voice_match_configurations (
    config_hash text PRIMARY KEY,
    configuration jsonb NOT NULL
);

CREATE TABLE voice_match_evidence (
    cluster_id uuid NOT NULL,
    track_id uuid NOT NULL,
    config_hash text NOT NULL REFERENCES voice_match_configurations(config_hash),
    PRIMARY KEY (cluster_id, track_id),
    FOREIGN KEY (cluster_id, track_id) REFERENCES voice_cluster_members(cluster_id, track_id) ON DELETE CASCADE
);

CREATE TABLE voice_match_rejections (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    track_a uuid NOT NULL REFERENCES voice_tracks(id) ON DELETE CASCADE,
    track_b uuid NOT NULL REFERENCES voice_tracks(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corpus_id, track_a, track_b),
    CHECK (track_a < track_b)
);

-- Suggested links between a face-only and a voice-only corpus person, from the time their
-- face tracks and voice observations co-occur across videos. A suggestion is never applied on
-- its own: confirming one merges the voice person into the face person (logged in
-- `voice_match_corrections`); rejecting one keeps the pair apart across reruns.
CREATE TABLE person_face_voice_links (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    face_person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    voice_person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    status text NOT NULL CHECK (status IN ('unverified', 'rejected')),
    shared_videos integer NOT NULL CHECK (shared_videos >= 0),
    voice_seconds double precision NOT NULL CHECK (voice_seconds >= 0),
    overlap_seconds double precision NOT NULL CHECK (overlap_seconds >= 0),
    coverage double precision NOT NULL CHECK (coverage >= 0 AND coverage <= 1),
    -- Coverage lead over the voice's next-best face person (1 when there is none).
    margin double precision NOT NULL,
    videos jsonb NOT NULL DEFAULT '[]'::jsonb,
    config_hash text NOT NULL REFERENCES voice_match_configurations(config_hash),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (face_person_id, voice_person_id),
    CHECK (face_person_id <> voice_person_id)
);

CREATE INDEX person_face_voice_links_corpus_idx ON person_face_voice_links(corpus_id, status);

CREATE TABLE voice_match_corrections (
    id uuid PRIMARY KEY,
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    decision text NOT NULL CHECK (
        decision IN ('unlink', 'split', 'link', 'confirm', 'confirm_face_link', 'reject_face_link')
    ),
    track_ids uuid[] NOT NULL DEFAULT '{}',
    person_id uuid REFERENCES people(id) ON DELETE SET NULL,
    -- Face-link corrections: the face person and the (possibly merged-away) voice person.
    face_person_id uuid REFERENCES people(id) ON DELETE SET NULL,
    voice_person_id uuid,
    created_at timestamptz NOT NULL DEFAULT now()
);
