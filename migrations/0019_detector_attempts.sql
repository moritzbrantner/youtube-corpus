-- Attempt history is observability, not a second work queue. PostgreSQL transaction locks
-- serialize attempts and expose whether an owner is still alive after a request is cancelled.
CREATE TABLE detector_attempts (
  id uuid PRIMARY KEY,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  detector text NOT NULL,
  identity jsonb NOT NULL,
  state text NOT NULL CHECK (state IN ('running', 'completed', 'skipped', 'failed', 'unavailable')),
  message text,
  backend_pid integer NOT NULL,
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);
CREATE INDEX detector_attempts_video_stage_idx ON detector_attempts(video_id, detector, started_at DESC);
