CREATE TABLE IF NOT EXISTS media_processing_runs (
  id uuid PRIMARY KEY,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  modality text NOT NULL CHECK (modality IN ('face', 'voice')),
  processor text NOT NULL,
  processor_version text NOT NULL,
  model text NOT NULL,
  model_version text NOT NULL,
  input_hash text NOT NULL,
  config_hash text NOT NULL,
  processing_config jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (video_id, modality, processor, processor_version, model, model_version, input_hash, config_hash),
  UNIQUE (id, video_id)
);

CREATE INDEX IF NOT EXISTS media_processing_runs_video_idx
  ON media_processing_runs(video_id, modality);

CREATE TABLE IF NOT EXISTS face_observations (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  observation_key text NOT NULL,
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision CHECK (end_seconds IS NULL OR end_seconds >= start_seconds),
  frame_index bigint CHECK (frame_index IS NULL OR frame_index >= 0),
  bbox_x double precision NOT NULL CHECK (bbox_x >= 0),
  bbox_y double precision NOT NULL CHECK (bbox_y >= 0),
  bbox_width double precision NOT NULL CHECK (bbox_width > 0),
  bbox_height double precision NOT NULL CHECK (bbox_height > 0),
  detection_score double precision CHECK (
    detection_score IS NULL OR (detection_score >= 0 AND detection_score <= 1)
  ),
  embedding vector,
  embedding_dimensions integer CHECK (embedding_dimensions IS NULL OR embedding_dimensions > 0),
  quality jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, observation_key)
);

CREATE INDEX IF NOT EXISTS face_observations_video_time_idx
  ON face_observations(video_id, start_seconds);

CREATE TABLE IF NOT EXISTS face_tracks (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  track_key text NOT NULL,
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
  representative_embedding vector,
  embedding_dimensions integer CHECK (embedding_dimensions IS NULL OR embedding_dimensions > 0),
  quality jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, track_key)
);

CREATE INDEX IF NOT EXISTS face_tracks_video_time_idx
  ON face_tracks(video_id, start_seconds);

CREATE TABLE IF NOT EXISTS face_track_observations (
  track_id uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
  observation_id uuid NOT NULL REFERENCES face_observations(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, observation_id)
);

CREATE INDEX IF NOT EXISTS face_track_observations_observation_idx
  ON face_track_observations(observation_id);

CREATE TABLE IF NOT EXISTS voice_observations (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  observation_key text NOT NULL,
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
  confidence double precision CHECK (
    confidence IS NULL OR (confidence >= 0 AND confidence <= 1)
  ),
  transcript_segment_id uuid REFERENCES transcript_segments(id) ON DELETE SET NULL,
  embedding vector,
  embedding_dimensions integer CHECK (embedding_dimensions IS NULL OR embedding_dimensions > 0),
  quality jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, observation_key)
);

CREATE INDEX IF NOT EXISTS voice_observations_video_time_idx
  ON voice_observations(video_id, start_seconds);

CREATE INDEX IF NOT EXISTS voice_observations_segment_idx
  ON voice_observations(transcript_segment_id)
  WHERE transcript_segment_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS voice_tracks (
  id uuid PRIMARY KEY,
  run_id uuid NOT NULL,
  video_id uuid NOT NULL,
  track_key text NOT NULL,
  start_seconds double precision NOT NULL CHECK (start_seconds >= 0),
  end_seconds double precision NOT NULL CHECK (end_seconds >= start_seconds),
  representative_embedding vector,
  embedding_dimensions integer CHECK (embedding_dimensions IS NULL OR embedding_dimensions > 0),
  quality jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (run_id, video_id) REFERENCES media_processing_runs(id, video_id) ON DELETE CASCADE,
  UNIQUE (run_id, track_key)
);

CREATE INDEX IF NOT EXISTS voice_tracks_video_time_idx
  ON voice_tracks(video_id, start_seconds);

CREATE TABLE IF NOT EXISTS voice_track_observations (
  track_id uuid NOT NULL REFERENCES voice_tracks(id) ON DELETE CASCADE,
  observation_id uuid NOT NULL REFERENCES voice_observations(id) ON DELETE CASCADE,
  PRIMARY KEY (track_id, observation_id)
);

CREATE INDEX IF NOT EXISTS voice_track_observations_observation_idx
  ON voice_track_observations(observation_id);

CREATE TABLE IF NOT EXISTS face_clusters (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  label text,
  algorithm text NOT NULL,
  algorithm_version text NOT NULL,
  config_hash text NOT NULL,
  revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS face_clusters_corpus_idx
  ON face_clusters(corpus_id);

CREATE TABLE IF NOT EXISTS face_cluster_members (
  cluster_id uuid NOT NULL REFERENCES face_clusters(id) ON DELETE CASCADE,
  track_id uuid NOT NULL REFERENCES face_tracks(id) ON DELETE CASCADE,
  similarity double precision,
  assignment_algorithm text,
  assigned_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (cluster_id, track_id)
);

CREATE INDEX IF NOT EXISTS face_cluster_members_track_idx
  ON face_cluster_members(track_id);

CREATE TABLE IF NOT EXISTS voice_clusters (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  label text,
  algorithm text NOT NULL,
  algorithm_version text NOT NULL,
  config_hash text NOT NULL,
  revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS voice_clusters_corpus_idx
  ON voice_clusters(corpus_id);

CREATE TABLE IF NOT EXISTS voice_cluster_members (
  cluster_id uuid NOT NULL REFERENCES voice_clusters(id) ON DELETE CASCADE,
  track_id uuid NOT NULL REFERENCES voice_tracks(id) ON DELETE CASCADE,
  similarity double precision,
  assignment_algorithm text,
  assigned_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (cluster_id, track_id)
);

CREATE INDEX IF NOT EXISTS voice_cluster_members_track_idx
  ON voice_cluster_members(track_id);

CREATE TABLE IF NOT EXISTS people (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  label text,
  identity_confidence double precision CHECK (
    identity_confidence IS NULL OR (identity_confidence >= 0 AND identity_confidence <= 1)
  ),
  identity_source text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS people_corpus_idx
  ON people(corpus_id);

CREATE TABLE IF NOT EXISTS person_face_clusters (
  person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
  cluster_id uuid NOT NULL REFERENCES face_clusters(id) ON DELETE CASCADE,
  confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
  source text,
  linked_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (person_id, cluster_id)
);

CREATE INDEX IF NOT EXISTS person_face_clusters_cluster_idx
  ON person_face_clusters(cluster_id);

CREATE TABLE IF NOT EXISTS person_voice_clusters (
  person_id uuid NOT NULL REFERENCES people(id) ON DELETE CASCADE,
  cluster_id uuid NOT NULL REFERENCES voice_clusters(id) ON DELETE CASCADE,
  confidence double precision CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
  source text,
  linked_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (person_id, cluster_id)
);

CREATE INDEX IF NOT EXISTS person_voice_clusters_cluster_idx
  ON person_voice_clusters(cluster_id);
