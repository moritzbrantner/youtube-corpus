-- Source-grounded entity occurrences (Atlas 4.1, 4.2). Mentions are extracted from transcript
-- segments and visible-text (OCR) tracks by the canonical nlp-stack entity producer; each one
-- keeps its video, interval, modality, source record and producer run. Entities are grouped by
-- (type, normalized key) at query time and never promoted to person identities.
CREATE TABLE IF NOT EXISTS entity_extraction_runs (
  id uuid PRIMARY KEY,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  producer text NOT NULL,
  producer_version text NOT NULL,
  model text NOT NULL,
  config_hash text NOT NULL,
  transcript_stream_id uuid REFERENCES transcript_streams(id) ON DELETE SET NULL,
  segments_analyzed integer NOT NULL DEFAULT 0 CHECK (segments_analyzed >= 0),
  visual_text_tracks_analyzed integer NOT NULL DEFAULT 0 CHECK (visual_text_tracks_analyzed >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (video_id, producer, producer_version, model, config_hash)
);

CREATE TABLE IF NOT EXISTS entity_mentions (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL REFERENCES entity_extraction_runs(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  modality text NOT NULL CHECK (modality IN ('spoken', 'visible_text')),
  entity_type text NOT NULL CHECK (entity_type IN (
    'person', 'organization', 'location', 'product', 'date', 'amount', 'law', 'work', 'event', 'misc'
  )),
  surface_text text NOT NULL CHECK (btrim(surface_text) <> ''),
  entity_key text NOT NULL CHECK (btrim(entity_key) <> ''),
  start_seconds double precision CHECK (start_seconds IS NULL OR start_seconds >= 0),
  end_seconds double precision CHECK (end_seconds IS NULL OR end_seconds >= 0),
  transcript_segment_id uuid REFERENCES transcript_segments(id) ON DELETE CASCADE,
  visual_text_track_id uuid REFERENCES visual_text_tracks(id) ON DELETE CASCADE,
  byte_start integer NOT NULL CHECK (byte_start >= 0),
  byte_end integer NOT NULL CHECK (byte_end > byte_start),
  confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (
    (modality = 'spoken' AND transcript_segment_id IS NOT NULL AND visual_text_track_id IS NULL)
    OR (modality = 'visible_text' AND visual_text_track_id IS NOT NULL AND transcript_segment_id IS NULL)
  )
);

CREATE INDEX IF NOT EXISTS entity_mentions_key_idx ON entity_mentions(entity_type, entity_key);
CREATE INDEX IF NOT EXISTS entity_mentions_video_idx ON entity_mentions(video_id);
CREATE INDEX IF NOT EXISTS entity_mentions_run_idx ON entity_mentions(run_id);
