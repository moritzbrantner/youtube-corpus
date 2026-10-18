-- Detector profiles (Atlas 2.3, 2.6, 2.8, 2.10). One row is an explicit setting of one detector
-- for one scope of one corpus: the corpus default, a tracked source of that corpus, or a video of
-- that corpus (video overrides never leak into other corpora). A missing source/video row means
-- "inherit"; a missing corpus row means "off".
CREATE TABLE IF NOT EXISTS detector_settings (
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  scope_kind text NOT NULL CHECK (scope_kind IN ('corpus', 'source', 'video')),
  scope_id uuid NOT NULL,
  detector text NOT NULL CHECK (btrim(detector) <> ''),
  enabled boolean NOT NULL,
  target_languages text[] NOT NULL DEFAULT '{}',
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (corpus_id, scope_kind, scope_id, detector)
);
