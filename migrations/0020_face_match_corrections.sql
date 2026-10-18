-- Explicit research corrections survive unchanged detector/matcher reruns.
CREATE TABLE face_match_decisions (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    track_id uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
    person_id uuid REFERENCES people(id) ON DELETE SET NULL,
    decision text NOT NULL CHECK (decision IN ('unlink', 'split', 'link', 'confirm')),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corpus_id, track_id)
);

CREATE TABLE face_match_corrections (
    id uuid PRIMARY KEY,
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    decision text NOT NULL CHECK (decision IN ('unlink', 'split', 'link', 'confirm')),
    track_ids uuid[] NOT NULL,
    person_id uuid REFERENCES people(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE face_match_configurations (
    config_hash text PRIMARY KEY,
    configuration jsonb NOT NULL
);

CREATE TABLE face_match_evidence (
    cluster_id uuid NOT NULL,
    track_id uuid NOT NULL,
    config_hash text NOT NULL REFERENCES face_match_configurations(config_hash),
    PRIMARY KEY (cluster_id, track_id),
    FOREIGN KEY (cluster_id, track_id) REFERENCES face_cluster_members(cluster_id, track_id) ON DELETE CASCADE
);

CREATE TABLE face_match_rejections (
    corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
    track_a uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
    track_b uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corpus_id, track_a, track_b),
    CHECK (track_a < track_b)
);
