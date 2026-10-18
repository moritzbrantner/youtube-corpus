-- Latest attempt of each detector stage per video (Atlas 2.1; #60). Work is shared across corpora
-- (2.2), so attempts are per video. Completed output itself stays in the evidence tables.
CREATE TABLE IF NOT EXISTS detector_stage_attempts (
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  detector text NOT NULL,
  state text NOT NULL CHECK (state IN ('running', 'completed', 'failed')),
  message text,
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  PRIMARY KEY (video_id, detector)
);
