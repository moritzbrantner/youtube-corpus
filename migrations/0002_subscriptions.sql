CREATE TABLE IF NOT EXISTS corpus_subscriptions (
  id uuid PRIMARY KEY,
  source_kind text NOT NULL CHECK (source_kind IN ('channel', 'playlist')),
  source_url text NOT NULL UNIQUE,
  name text,
  enabled boolean NOT NULL DEFAULT true,
  work_dir text NOT NULL,
  caption jsonb NOT NULL,
  asr_enabled boolean NOT NULL DEFAULT true,
  transcriber_command text,
  transcriber_args text[] NOT NULL DEFAULT '{}',
  max_items bigint,
  title_contains text,
  title_excludes text[] NOT NULL DEFAULT '{}',
  duration_min double precision,
  duration_max double precision,
  last_checked_at timestamptz,
  last_ingested_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS corpus_subscriptions_enabled_idx
  ON corpus_subscriptions(enabled);

CREATE TABLE IF NOT EXISTS corpus_subscription_items (
  id uuid PRIMARY KEY,
  subscription_id uuid NOT NULL REFERENCES corpus_subscriptions(id) ON DELETE CASCADE,
  source_url text NOT NULL,
  youtube_id text,
  title text,
  duration_seconds double precision,
  upload_date text,
  status text NOT NULL DEFAULT 'discovered',
  first_seen_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  ingested_at timestamptz,
  UNIQUE (subscription_id, source_url)
);

CREATE INDEX IF NOT EXISTS corpus_subscription_items_youtube_id_idx
  ON corpus_subscription_items(subscription_id, youtube_id);
