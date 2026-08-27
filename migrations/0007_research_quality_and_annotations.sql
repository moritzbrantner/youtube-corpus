CREATE TABLE IF NOT EXISTS transcript_stream_quality (
  stream_id uuid PRIMARY KEY REFERENCES transcript_streams(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  score double precision NOT NULL CHECK (score >= 0 AND score <= 1),
  source_priority integer NOT NULL,
  coverage_ratio double precision NOT NULL CHECK (coverage_ratio >= 0 AND coverage_ratio <= 1),
  timed_segment_ratio double precision NOT NULL CHECK (timed_segment_ratio >= 0 AND timed_segment_ratio <= 1),
  text_characters bigint NOT NULL DEFAULT 0,
  segment_count bigint NOT NULL DEFAULT 0,
  reasons jsonb NOT NULL DEFAULT '{}'::jsonb,
  evaluator text NOT NULL DEFAULT 'youtube-corpus',
  evaluator_version text NOT NULL DEFAULT '0.1.0',
  evaluation_config jsonb NOT NULL DEFAULT '{}'::jsonb,
  evaluated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS transcript_stream_quality_video_idx
  ON transcript_stream_quality(video_id, score DESC);

CREATE TABLE IF NOT EXISTS preferred_transcript_streams (
  video_id uuid PRIMARY KEY REFERENCES videos(id) ON DELETE CASCADE,
  stream_id uuid NOT NULL UNIQUE REFERENCES transcript_streams(id) ON DELETE CASCADE,
  score double precision NOT NULL CHECK (score >= 0 AND score <= 1),
  selector text NOT NULL DEFAULT 'youtube-corpus',
  selector_version text NOT NULL DEFAULT '0.1.0',
  selected_at timestamptz NOT NULL DEFAULT now()
);

WITH stream_stats AS (
  SELECT
    st.id AS stream_id,
    st.video_id,
    st.source_kind,
    count(ts.id)::bigint AS segment_count,
    coalesce(sum(length(ts.text)), 0)::bigint AS text_characters,
    coalesce(
      avg(CASE WHEN ts.start_seconds IS NOT NULL THEN 1.0 ELSE 0.0 END),
      0.0
    )::float8 AS timed_segment_ratio,
    CASE
      WHEN v.duration_seconds IS NOT NULL AND v.duration_seconds > 0 THEN
        least(coalesce(max(ts.end_seconds), 0.0) / v.duration_seconds, 1.0)::float8
      ELSE 0.0::float8
    END AS coverage_ratio
  FROM transcript_streams st
  JOIN videos v ON v.id = st.video_id
  LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
  GROUP BY st.id, st.video_id, st.source_kind, v.duration_seconds
), scored AS (
  SELECT
    stream_id,
    video_id,
    CASE source_kind
      WHEN 'caption_manual' THEN 3
      WHEN 'caption_auto' THEN 2
      ELSE 1
    END AS source_priority,
    coverage_ratio,
    timed_segment_ratio,
    text_characters,
    segment_count,
    least(
      1.0,
      CASE source_kind
        WHEN 'caption_manual' THEN 0.45
        WHEN 'caption_auto' THEN 0.30
        ELSE 0.25
      END
      + CASE WHEN text_characters > 0 THEN 0.15 ELSE 0.0 END
      + 0.15 * least(text_characters::float8 / 20000.0, 1.0)
      + 0.15 * timed_segment_ratio
      + 0.10 * coverage_ratio
    )::float8 AS score
  FROM stream_stats
)
INSERT INTO transcript_stream_quality (
  stream_id,
  video_id,
  score,
  source_priority,
  coverage_ratio,
  timed_segment_ratio,
  text_characters,
  segment_count,
  reasons,
  evaluation_config
)
SELECT
  stream_id,
  video_id,
  score,
  source_priority,
  coverage_ratio,
  timed_segment_ratio,
  text_characters,
  segment_count,
  jsonb_build_object(
    'sourcePriority', source_priority,
    'coverageRatio', coverage_ratio,
    'timedSegmentRatio', timed_segment_ratio,
    'textCharacters', text_characters,
    'segmentCount', segment_count
  ),
  jsonb_build_object('algorithm', 'source-coverage-v1')
FROM scored
ON CONFLICT (stream_id) DO NOTHING;

INSERT INTO preferred_transcript_streams (video_id, stream_id, score)
SELECT DISTINCT ON (q.video_id)
  q.video_id,
  q.stream_id,
  q.score
FROM transcript_stream_quality q
ORDER BY q.video_id, q.score DESC, q.source_priority DESC, q.text_characters DESC, q.stream_id
ON CONFLICT (video_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS research_annotations (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  stream_id uuid REFERENCES transcript_streams(id) ON DELETE SET NULL,
  segment_id uuid REFERENCES transcript_segments(id) ON DELETE SET NULL,
  kind text NOT NULL CHECK (kind ~ '^[a-z][a-z0-9_-]{0,63}$'),
  start_seconds double precision,
  end_seconds double precision,
  label text,
  text text,
  payload jsonb NOT NULL DEFAULT '{}'::jsonb,
  source_kind text NOT NULL DEFAULT 'user' CHECK (source_kind IN ('user', 'processor')),
  processor text,
  processor_version text,
  processing_config jsonb NOT NULL DEFAULT '{}'::jsonb,
  revision bigint NOT NULL DEFAULT 1,
  content_checksum text GENERATED ALWAYS AS (
    md5(
      coalesce(kind, '') || '|' ||
      coalesce(label, '') || '|' ||
      coalesce(text, '') || '|' ||
      coalesce(payload::text, '{}') || '|' ||
      coalesce(start_seconds::text, '') || '|' ||
      coalesce(end_seconds::text, '')
    )
  ) STORED,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (start_seconds IS NULL OR start_seconds >= 0),
  CHECK (end_seconds IS NULL OR end_seconds >= 0),
  CHECK (start_seconds IS NULL OR end_seconds IS NULL OR end_seconds >= start_seconds)
);

CREATE INDEX IF NOT EXISTS research_annotations_corpus_idx
  ON research_annotations(corpus_id, created_at DESC);
CREATE INDEX IF NOT EXISTS research_annotations_video_idx
  ON research_annotations(video_id, start_seconds);
CREATE INDEX IF NOT EXISTS research_annotations_segment_idx
  ON research_annotations(segment_id) WHERE segment_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS research_annotations_kind_idx
  ON research_annotations(corpus_id, kind);

CREATE OR REPLACE FUNCTION youtube_corpus_bump_annotation_revision()
RETURNS trigger AS $$
BEGIN
  IF ROW(NEW.kind, NEW.start_seconds, NEW.end_seconds, NEW.label, NEW.text, NEW.payload,
         NEW.source_kind, NEW.processor, NEW.processor_version, NEW.processing_config)
     IS DISTINCT FROM
     ROW(OLD.kind, OLD.start_seconds, OLD.end_seconds, OLD.label, OLD.text, OLD.payload,
         OLD.source_kind, OLD.processor, OLD.processor_version, OLD.processing_config) THEN
    NEW.revision := OLD.revision + 1;
    NEW.updated_at := now();
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS research_annotations_revision_trigger ON research_annotations;
CREATE TRIGGER research_annotations_revision_trigger
BEFORE UPDATE ON research_annotations
FOR EACH ROW
EXECUTE FUNCTION youtube_corpus_bump_annotation_revision();

CREATE TABLE IF NOT EXISTS retrieval_evaluation_cases (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  query text NOT NULL,
  mode text NOT NULL CHECK (mode IN ('fts', 'semantic', 'hybrid')),
  top_k bigint NOT NULL DEFAULT 10 CHECK (top_k > 0 AND top_k <= 100),
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (corpus_id, query, mode, top_k)
);

CREATE TABLE IF NOT EXISTS retrieval_evaluation_targets (
  id uuid PRIMARY KEY,
  case_id uuid NOT NULL REFERENCES retrieval_evaluation_cases(id) ON DELETE CASCADE,
  video_id uuid REFERENCES videos(id) ON DELETE CASCADE,
  segment_id uuid REFERENCES transcript_segments(id) ON DELETE SET NULL,
  source_url text,
  start_seconds double precision,
  end_seconds double precision,
  relevance integer NOT NULL DEFAULT 1 CHECK (relevance >= 1 AND relevance <= 3),
  notes text,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (video_id IS NOT NULL OR segment_id IS NOT NULL OR source_url IS NOT NULL),
  CHECK (start_seconds IS NULL OR start_seconds >= 0),
  CHECK (end_seconds IS NULL OR end_seconds >= 0),
  CHECK (start_seconds IS NULL OR end_seconds IS NULL OR end_seconds >= start_seconds)
);

CREATE INDEX IF NOT EXISTS retrieval_evaluation_cases_corpus_idx
  ON retrieval_evaluation_cases(corpus_id, created_at);
CREATE INDEX IF NOT EXISTS retrieval_evaluation_targets_case_idx
  ON retrieval_evaluation_targets(case_id);

CREATE TABLE IF NOT EXISTS retrieval_evaluation_runs (
  id uuid PRIMARY KEY,
  corpus_id uuid NOT NULL REFERENCES corpora(id) ON DELETE CASCADE,
  cases_count bigint NOT NULL,
  recall_at_k double precision NOT NULL,
  mean_reciprocal_rank double precision NOT NULL,
  ndcg_at_k double precision NOT NULL,
  mean_latency_ms double precision NOT NULL,
  report jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS retrieval_evaluation_runs_corpus_idx
  ON retrieval_evaluation_runs(corpus_id, created_at DESC);
