-- Seed for the #59 real-browser acceptance journey (tests/acceptance/user-labels-browser.md).
-- Apply after `cargo run -- migrate` against the database the local server uses:
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f tests/acceptance/user-labels-seed.sql
-- Idempotent. Creates corpus "Labels demo" with a monitored channel and playlist that both
-- contain "Shared lecture", a channel-only video, and a directly added video. The source URLs
-- are fixtures, not real YouTube sources: do not press "Check now" on them.
BEGIN;

INSERT INTO corpora (id, slug, name, description)
VALUES ('5900a000-0000-4000-8000-000000000001', 'labels-demo', 'Labels demo',
        'Acceptance fixture for user-defined labels (#59)')
ON CONFLICT (id) DO NOTHING;

INSERT INTO corpus_subscriptions (id, source_kind, source_url, name, work_dir, caption)
VALUES
  ('5900a000-0000-4000-8000-0000000000c1', 'channel',
   'https://www.youtube.com/@labels-demo-fixture', 'Lecture channel',
   'use-case-output/acceptance', '{}'::jsonb),
  ('5900a000-0000-4000-8000-0000000000c2', 'playlist',
   'https://www.youtube.com/playlist?list=PLlabelsdemofixture', 'Seminar playlist',
   'use-case-output/acceptance', '{}'::jsonb)
ON CONFLICT (id) DO NOTHING;

INSERT INTO corpus_source_memberships (corpus_id, subscription_id)
VALUES ('5900a000-0000-4000-8000-000000000001', '5900a000-0000-4000-8000-0000000000c1'),
       ('5900a000-0000-4000-8000-000000000001', '5900a000-0000-4000-8000-0000000000c2')
ON CONFLICT DO NOTHING;

INSERT INTO videos (id, youtube_id, source_url, title, channel, upload_date, duration_seconds,
                    categories, tags, metadata)
VALUES
  ('5900a000-0000-4000-8000-0000000000d1', 'lblShared01',
   'https://www.youtube.com/watch?v=lblShared01', 'Shared lecture', 'Example Lectures',
   '20260101', 120, ARRAY['Education'], ARRAY['history', 'lecture'],
   '{"categories":["Education"],"tags":["history","lecture"],"like_count":7}'),
  ('5900a000-0000-4000-8000-0000000000d2', 'lblChannel1',
   'https://www.youtube.com/watch?v=lblChannel1', 'Channel-only lecture', 'Example Lectures',
   '20260102', 95, ARRAY['Education'], ARRAY['lecture'],
   '{"categories":["Education"],"tags":["lecture"]}'),
  ('5900a000-0000-4000-8000-0000000000d3', 'lblDirect01',
   'https://www.youtube.com/watch?v=lblDirect01', 'Directly added talk', 'Guest speaker',
   '20260103', 300, ARRAY['People & Blogs'], ARRAY['talk'],
   '{"categories":["People & Blogs"],"tags":["talk"]}')
ON CONFLICT (id) DO NOTHING;

INSERT INTO corpus_subscription_items (id, subscription_id, source_url, youtube_id, title, status)
VALUES
  ('5900a000-0000-4000-8000-0000000000e1', '5900a000-0000-4000-8000-0000000000c1',
   'https://www.youtube.com/watch?v=lblShared01', 'lblShared01', 'Shared lecture', 'indexed'),
  ('5900a000-0000-4000-8000-0000000000e2', '5900a000-0000-4000-8000-0000000000c2',
   'https://www.youtube.com/watch?v=lblShared01', 'lblShared01', 'Shared lecture', 'indexed'),
  ('5900a000-0000-4000-8000-0000000000e3', '5900a000-0000-4000-8000-0000000000c1',
   'https://www.youtube.com/watch?v=lblChannel1', 'lblChannel1', 'Channel-only lecture', 'indexed')
ON CONFLICT DO NOTHING;

INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
VALUES ('5900a000-0000-4000-8000-000000000001', '5900a000-0000-4000-8000-0000000000d3', 'direct')
ON CONFLICT DO NOTHING;

INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
VALUES ('5900a000-0000-4000-8000-0000000000f1', '5900a000-0000-4000-8000-0000000000d1',
        'caption_manual', 'en', 'completed', 'grace perfects nature')
ON CONFLICT DO NOTHING;

INSERT INTO transcript_segments
  (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
VALUES ('5900a000-0000-4000-8000-0000000000f2', '5900a000-0000-4000-8000-0000000000d1',
        '5900a000-0000-4000-8000-0000000000f1', 0, 1, 4, 'grace perfects nature', 'en')
ON CONFLICT DO NOTHING;

COMMIT;
