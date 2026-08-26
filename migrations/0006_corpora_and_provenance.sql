CREATE TABLE IF NOT EXISTS corpora (
  id uuid PRIMARY KEY,
  slug text NOT NULL UNIQUE,
  name text NOT NULL,
  description text,
  is_default boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS corpora_single_default_idx
  ON corpora ((is_default))
  WHERE is_default;

INSERT INTO corpora (id, slug, name, description, is_default)
VALUES (
  '00000000-0000-5000-8000-000000000001',
  'default',
  'Default corpus',
  'Automatically contains corpus data created before named corpora were introduced.',
  true
)
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS corpus_source_memberships (
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  subscription_id uuid NOT NULL REFERENCES corpus_subscriptions(id) ON DELETE CASCADE,
  added_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (corpus_id, subscription_id)
);

CREATE INDEX IF NOT EXISTS corpus_source_memberships_subscription_idx
  ON corpus_source_memberships(subscription_id);

CREATE TABLE IF NOT EXISTS corpus_video_memberships (
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  membership_kind text NOT NULL DEFAULT 'direct'
    CHECK (membership_kind IN ('direct', 'legacy')),
  added_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (corpus_id, video_id)
);

CREATE INDEX IF NOT EXISTS corpus_video_memberships_video_idx
  ON corpus_video_memberships(video_id);

INSERT INTO corpus_source_memberships (corpus_id, subscription_id)
SELECT '00000000-0000-5000-8000-000000000001', id
FROM corpus_subscriptions
ON CONFLICT DO NOTHING;

INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
SELECT '00000000-0000-5000-8000-000000000001', id, 'legacy'
FROM videos
ON CONFLICT DO NOTHING;

ALTER TABLE corpus_subscriptions
  ADD COLUMN IF NOT EXISTS last_check_status text,
  ADD COLUMN IF NOT EXISTS last_check_message text;

ALTER TABLE transcript_streams
  ADD COLUMN IF NOT EXISTS retrieved_at timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS processor text NOT NULL DEFAULT 'youtube-corpus',
  ADD COLUMN IF NOT EXISTS processor_version text NOT NULL DEFAULT '0.1.0',
  ADD COLUMN IF NOT EXISTS processing_config jsonb NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS processing_revision bigint NOT NULL DEFAULT 1;

ALTER TABLE transcript_streams
  ADD COLUMN IF NOT EXISTS content_checksum text
  GENERATED ALWAYS AS (md5(coalesce(full_text, ''))) STORED;

CREATE OR REPLACE FUNCTION youtube_corpus_bump_stream_revision()
RETURNS trigger AS $$
BEGIN
  IF ROW(NEW.full_text, NEW.source_path, NEW.status, NEW.processing_config)
     IS DISTINCT FROM
     ROW(OLD.full_text, OLD.source_path, OLD.status, OLD.processing_config) THEN
    NEW.processing_revision := OLD.processing_revision + 1;
    NEW.retrieved_at := now();
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS transcript_streams_revision_trigger ON transcript_streams;
CREATE TRIGGER transcript_streams_revision_trigger
BEFORE UPDATE OF full_text, source_path, status, processing_config
ON transcript_streams
FOR EACH ROW
EXECUTE FUNCTION youtube_corpus_bump_stream_revision();
