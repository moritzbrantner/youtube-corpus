-- Seed for the #64 real-browser acceptance journey (tests/acceptance/face-matching-browser.md).
-- Apply after `cargo run -- migrate` against the database the local server uses:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f tests/acceptance/face-matching-seed.sql
-- Idempotent. Creates corpus "Face matching demo" with completed faces-detector runs shaped like
-- the pinned YuNet/SFace producer output. Embeddings are synthetic 128-dimensional directions,
-- not biometric data:
--   person A (axis 0) in "Lecture one" 0:12, "Lecture two" 0:40 and "Interview" 1:35;
--   person B (cosine 0.5 to A) in "Panel" 0:05 and "Debate" 0:07;
--   "Q&A" 0:03 lies exactly between A and B (ambiguous);
--   "Short clip" 0:09 resembles A but has one weak detection (low confidence).
-- The video URLs are fixtures; their timestamp links must open the stated second.
BEGIN;

CREATE FUNCTION pg_temp.face_dir(angle double precision, jitter integer) RETURNS vector
LANGUAGE sql IMMUTABLE AS $$
  SELECT ('[' || string_agg(
    CASE WHEN i = 0 THEN cos(angle)::text
         WHEN i = 1 THEN sin(angle)::text
         WHEN i = jitter THEN '0.12'
         ELSE '0' END, ',' ORDER BY i) || ']')::vector
  FROM generate_series(0, 127) AS i
$$;

INSERT INTO corpora (id, slug, name, description)
VALUES ('6400a000-0000-4000-8000-000000000001', 'face-matching-demo', 'Face matching demo',
        'Acceptance fixture for cross-video face matching (#64)')
ON CONFLICT (id) DO NOTHING;

CREATE TEMP TABLE face_seed (n integer, title text, youtube text, angle double precision,
                             start_s double precision, observations integer, score double precision)
ON COMMIT DROP;
INSERT INTO face_seed VALUES
  (1, 'Lecture one', 'fmdemo00001', 0, 12, 3, 0.97),
  (2, 'Lecture two', 'fmdemo00002', 0, 40, 4, 0.95),
  (3, 'Interview',   'fmdemo00003', 0, 95, 3, 0.96),
  (4, 'Panel',       'fmdemo00004', acos(0.5), 5, 3, 0.96),
  (5, 'Debate',      'fmdemo00005', acos(0.5), 7, 3, 0.96),
  (6, 'Q&A',         'fmdemo00006', acos(0.5) / 2, 3, 3, 0.97),
  (7, 'Short clip',  'fmdemo00007', 0, 9, 1, 0.55);

INSERT INTO videos (id, youtube_id, source_url, title, duration_seconds)
SELECT ('6400a000-0000-4000-8000-0000000001' || lpad(n::text, 2, '0'))::uuid, youtube,
       'https://www.youtube.com/watch?v=' || youtube, title, 600
FROM face_seed
ON CONFLICT (id) DO NOTHING;

INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
SELECT '6400a000-0000-4000-8000-000000000001',
       ('6400a000-0000-4000-8000-0000000001' || lpad(n::text, 2, '0'))::uuid, 'direct'
FROM face_seed
ON CONFLICT DO NOTHING;

INSERT INTO media_processing_runs (id, video_id, modality, processor, processor_version, model,
                                   model_version, input_hash, config_hash, processing_config)
SELECT ('6400a000-0000-4000-8000-0000000002' || lpad(n::text, 2, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000000001' || lpad(n::text, 2, '0'))::uuid,
       'face', 'youtube-corpus/faces', 'acceptance-fixture', 'opencv-yunet+opencv-sface',
       'yunet:3cc26e7f1014a5ee5d74a42acee58bafc9d0a310;sface:3d7082438a6e4551e840c9b2bb60b71e8da4b524',
       'fixture-input-' || n, 'fixture-config', '{"complete": true, "sampleIntervalSeconds": 1}'
FROM face_seed
ON CONFLICT (id) DO NOTHING;

INSERT INTO face_tracks (id, run_id, video_id, track_key, start_seconds, end_seconds,
                         representative_embedding, embedding_dimensions, quality)
SELECT ('6400a000-0000-4000-8000-0000000003' || lpad(n::text, 2, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000000002' || lpad(n::text, 2, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000000001' || lpad(n::text, 2, '0'))::uuid,
       'track-' || n, start_s, start_s + observations, pg_temp.face_dir(angle, 10 + n), 128,
       '{"inferred": true, "verifiedIdentity": false}'
FROM face_seed
ON CONFLICT (id) DO NOTHING;

INSERT INTO face_observations (id, run_id, video_id, observation_key, start_seconds, frame_index,
                               bbox_x, bbox_y, bbox_width, bbox_height, detection_score,
                               embedding, embedding_dimensions)
SELECT ('6400a000-0000-4000-8000-0000004' || lpad(n::text, 2, '0') || lpad(k::text, 3, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000000002' || lpad(n::text, 2, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000000001' || lpad(n::text, 2, '0'))::uuid,
       'track-' || n || '-' || k, start_s + k, (start_s + k) * 25, 10, 10, 64, 64, score,
       pg_temp.face_dir(angle, 10 + n), 128
FROM face_seed, generate_series(0, observations - 1) AS k
ON CONFLICT (id) DO NOTHING;

INSERT INTO face_track_observations (track_id, observation_id)
SELECT ('6400a000-0000-4000-8000-0000000003' || lpad(n::text, 2, '0'))::uuid,
       ('6400a000-0000-4000-8000-0000004' || lpad(n::text, 2, '0') || lpad(k::text, 3, '0'))::uuid
FROM face_seed, generate_series(0, observations - 1) AS k
ON CONFLICT DO NOTHING;

COMMIT;
