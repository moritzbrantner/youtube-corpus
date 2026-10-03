CREATE TABLE IF NOT EXISTS sponsorblock_snapshots (
  id uuid PRIMARY KEY,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  youtube_id text NOT NULL,
  hash_prefix text NOT NULL,
  categories jsonb NOT NULL,
  response_hash text NOT NULL,
  data_license text NOT NULL,
  attribution text NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (video_id, response_hash)
);

CREATE INDEX IF NOT EXISTS sponsorblock_snapshots_video_fetched_idx
  ON sponsorblock_snapshots(video_id, fetched_at DESC);

CREATE TABLE IF NOT EXISTS sponsorblock_segments (
  id uuid PRIMARY KEY,
  snapshot_id uuid NOT NULL REFERENCES sponsorblock_snapshots(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  segment_uuid text NOT NULL,
  category text NOT NULL,
  action_type text,
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
  video_duration double precision CHECK (video_duration IS NULL OR video_duration >= 0),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (snapshot_id, segment_uuid)
);

CREATE INDEX IF NOT EXISTS sponsorblock_segments_video_time_idx
  ON sponsorblock_segments(video_id, start_seconds, end_seconds);
