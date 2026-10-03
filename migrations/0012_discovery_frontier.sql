CREATE TABLE IF NOT EXISTS discovery_targets (
  id uuid PRIMARY KEY,
  kind text NOT NULL CHECK (kind IN ('video', 'channel', 'playlist')),
  canonical_key text NOT NULL,
  target_url text NOT NULL,
  state text NOT NULL CHECK (state IN ('pending', 'claimed', 'completed', 'deferred', 'failed')),
  depth integer NOT NULL CHECK (depth >= 0),
  priority double precision NOT NULL CHECK (priority >= 0 AND priority <= 1),
  confidence double precision NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  relevance double precision NOT NULL CHECK (relevance >= 0 AND relevance <= 1),
  novelty double precision NOT NULL CHECK (novelty >= 0 AND novelty <= 1),
  claimed_by text,
  workflow_run_id text,
  attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  last_error text,
  discovered_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  claimed_at timestamptz,
  claim_expires_at timestamptz,
  completed_at timestamptz,
  UNIQUE (kind, canonical_key)
);

CREATE INDEX IF NOT EXISTS discovery_targets_frontier_idx
  ON discovery_targets(priority DESC, discovered_at ASC, id ASC)
  WHERE state = 'pending';

CREATE INDEX IF NOT EXISTS discovery_targets_state_idx
  ON discovery_targets(state, updated_at DESC);

CREATE TABLE IF NOT EXISTS discovery_evidence (
  id uuid PRIMARY KEY,
  target_id uuid NOT NULL REFERENCES discovery_targets(id) ON DELETE CASCADE,
  source_video_id uuid REFERENCES videos(id) ON DELETE SET NULL,
  parent_target_id uuid REFERENCES discovery_targets(id) ON DELETE SET NULL,
  method text NOT NULL CHECK (
    method IN (
      'seed',
      'source_expansion',
      'channel_metadata',
      'description_link',
      'transcript_link',
      'manual'
    )
  ),
  evidence jsonb NOT NULL DEFAULT '{}',
  confidence double precision NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
  relevance double precision NOT NULL CHECK (relevance >= 0 AND relevance <= 1),
  novelty double precision NOT NULL CHECK (novelty >= 0 AND novelty <= 1),
  depth integer NOT NULL CHECK (depth >= 0),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS discovery_evidence_target_idx
  ON discovery_evidence(target_id, created_at ASC);

CREATE INDEX IF NOT EXISTS discovery_evidence_source_video_idx
  ON discovery_evidence(source_video_id)
  WHERE source_video_id IS NOT NULL;
