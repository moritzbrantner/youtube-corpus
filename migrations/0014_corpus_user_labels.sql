-- User-defined labels for organizing tracked material inside a corpus (Atlas 1.3, 3.3).
-- Labels are corpus-owned and kept separate from YouTube-supplied categories/tags (Atlas 1.4);
-- removing a label or an assignment never touches videos, sources, metadata or evidence.
CREATE TABLE IF NOT EXISTS corpus_labels (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  name text NOT NULL CHECK (btrim(name) <> ''),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (corpus_id, name),
  UNIQUE (corpus_id, id)
);

CREATE TABLE IF NOT EXISTS corpus_source_labels (
  corpus_id uuid NOT NULL,
  label_id uuid NOT NULL,
  subscription_id uuid NOT NULL,
  assigned_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (label_id, subscription_id),
  FOREIGN KEY (corpus_id, label_id) REFERENCES corpus_labels(corpus_id, id) ON DELETE CASCADE,
  FOREIGN KEY (corpus_id, subscription_id)
    REFERENCES corpus_source_memberships(corpus_id, subscription_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS corpus_source_labels_source_idx
  ON corpus_source_labels(corpus_id, subscription_id);

CREATE TABLE IF NOT EXISTS corpus_video_labels (
  corpus_id uuid NOT NULL,
  label_id uuid NOT NULL,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  assigned_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (label_id, video_id),
  FOREIGN KEY (corpus_id, label_id) REFERENCES corpus_labels(corpus_id, id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS corpus_video_labels_video_idx
  ON corpus_video_labels(corpus_id, video_id);
