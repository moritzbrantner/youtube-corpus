ALTER TABLE media_processing_runs
  DROP CONSTRAINT IF EXISTS media_processing_runs_modality_check;

ALTER TABLE media_processing_runs
  ADD CONSTRAINT media_processing_runs_modality_check
  CHECK (modality IN ('face', 'voice', 'scene', 'ocr'));

CREATE TABLE IF NOT EXISTS video_scenes (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  scene_index bigint NOT NULL CHECK (scene_index >= 0),
  start_frame bigint NOT NULL CHECK (start_frame >= 0),
  end_frame bigint NOT NULL CHECK (end_frame >= start_frame),
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, scene_index)
);

CREATE INDEX IF NOT EXISTS video_scenes_video_time_idx
  ON video_scenes(video_id, start_seconds, end_seconds);

CREATE TABLE IF NOT EXISTS visual_text_observations (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  observation_key text NOT NULL,
  text text NOT NULL CHECK (length(trim(text)) > 0),
  language text,
  frame_index bigint CHECK (frame_index IS NULL OR frame_index >= 0),
  timestamp_seconds double precision CHECK (timestamp_seconds IS NULL OR timestamp_seconds >= 0),
  scene_index bigint CHECK (scene_index IS NULL OR scene_index >= 0),
  bbox_x bigint CHECK (bbox_x IS NULL OR bbox_x >= 0),
  bbox_y bigint CHECK (bbox_y IS NULL OR bbox_y >= 0),
  bbox_width bigint CHECK (bbox_width IS NULL OR bbox_width > 0),
  bbox_height bigint CHECK (bbox_height IS NULL OR bbox_height > 0),
  confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
  attributes jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, observation_key)
);

CREATE INDEX IF NOT EXISTS visual_text_observations_video_time_idx
  ON visual_text_observations(video_id, timestamp_seconds);

CREATE TABLE IF NOT EXISTS visual_text_tracks (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  track_key text NOT NULL,
  text text NOT NULL CHECK (length(trim(text)) > 0),
  role text NOT NULL CHECK (role IN ('subtitle', 'end_credit', 'presentation_slide', 'scene_text')),
  language text,
  sample_count integer NOT NULL CHECK (sample_count > 0),
  start_frame bigint CHECK (start_frame IS NULL OR start_frame >= 0),
  end_frame bigint CHECK (
    end_frame IS NULL OR (end_frame >= 0 AND (start_frame IS NULL OR end_frame >= start_frame))
  ),
  start_seconds double precision CHECK (start_seconds IS NULL OR start_seconds >= 0),
  end_seconds double precision CHECK (
    end_seconds IS NULL OR (end_seconds >= 0 AND (start_seconds IS NULL OR end_seconds >= start_seconds))
  ),
  bbox_x bigint CHECK (bbox_x IS NULL OR bbox_x >= 0),
  bbox_y bigint CHECK (bbox_y IS NULL OR bbox_y >= 0),
  bbox_width bigint CHECK (bbox_width IS NULL OR bbox_width > 0),
  bbox_height bigint CHECK (bbox_height IS NULL OR bbox_height > 0),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  search_vector tsvector GENERATED ALWAYS AS (to_tsvector('simple', coalesce(text, ''))) STORED,
  embedding vector,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, track_key)
);

CREATE INDEX IF NOT EXISTS visual_text_tracks_video_time_idx
  ON visual_text_tracks(video_id, start_seconds, end_seconds);

CREATE INDEX IF NOT EXISTS visual_text_tracks_search_idx
  ON visual_text_tracks USING gin(search_vector);

CREATE TABLE IF NOT EXISTS visual_text_track_observations (
  track_id uuid NOT NULL REFERENCES visual_text_tracks(id) ON DELETE CASCADE,
  observation_id uuid NOT NULL REFERENCES visual_text_observations(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, observation_id)
);

CREATE INDEX IF NOT EXISTS visual_text_track_observations_observation_idx
  ON visual_text_track_observations(observation_id);

CREATE TABLE IF NOT EXISTS visual_text_track_scenes (
  track_id uuid NOT NULL REFERENCES visual_text_tracks(id) ON DELETE CASCADE,
  scene_id uuid NOT NULL REFERENCES video_scenes(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, scene_id)
);

CREATE INDEX IF NOT EXISTS visual_text_track_scenes_scene_idx
  ON visual_text_track_scenes(scene_id);

DROP VIEW IF EXISTS corpus_search_segments;

CREATE VIEW corpus_search_segments AS
SELECT
  s.id,
  s.video_id,
  s.stream_id,
  st.source_kind,
  s.language,
  s.start_seconds,
  s.end_seconds,
  s.text,
  s.search_vector,
  s.embedding
FROM transcript_segments s
JOIN transcript_streams st ON st.id = s.stream_id
UNION ALL
SELECT
  t.id,
  t.video_id,
  t.run_id AS stream_id,
  'visual_ocr'::text AS source_kind,
  t.language,
  t.start_seconds,
  t.end_seconds,
  t.text,
  t.search_vector,
  t.embedding
FROM visual_text_tracks t;
