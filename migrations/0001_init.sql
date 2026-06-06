CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS videos (
  id uuid PRIMARY KEY,
  youtube_id text,
  source_url text NOT NULL,
  title text,
  local_video_path text,
  duration_seconds double precision,
  upload_date text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS transcript_streams (
  id uuid PRIMARY KEY,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  source_kind text NOT NULL CHECK (source_kind IN ('caption_manual', 'caption_auto', 'asr')),
  language text,
  status text NOT NULL,
  source_path text,
  full_text text,
  message text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (video_id, source_kind, language)
);

CREATE TABLE IF NOT EXISTS transcript_segments (
  id uuid PRIMARY KEY,
  stream_id uuid NOT NULL REFERENCES transcript_streams(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  segment_index bigint NOT NULL,
  start_seconds double precision,
  end_seconds double precision,
  text text NOT NULL,
  language text,
  metadata jsonb NOT NULL DEFAULT '{}',
  search_vector tsvector GENERATED ALWAYS AS (
    to_tsvector('simple', text)
  ) STORED,
  embedding vector(128),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (stream_id, segment_index)
);

CREATE INDEX IF NOT EXISTS transcript_segments_fts_idx
  ON transcript_segments USING gin(search_vector);

CREATE INDEX IF NOT EXISTS transcript_segments_embedding_idx
  ON transcript_segments USING ivfflat (embedding vector_cosine_ops)
  WITH (lists = 100);

CREATE INDEX IF NOT EXISTS transcript_segments_video_idx
  ON transcript_segments(video_id);

CREATE TABLE IF NOT EXISTS ingest_runs (
  id uuid PRIMARY KEY,
  source_url text,
  status text NOT NULL,
  videos_seen bigint NOT NULL DEFAULT 0,
  videos_indexed bigint NOT NULL DEFAULT 0,
  segments_indexed bigint NOT NULL DEFAULT 0,
  report jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now()
);
