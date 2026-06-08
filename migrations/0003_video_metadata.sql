ALTER TABLE videos
  ADD COLUMN IF NOT EXISTS description text,
  ADD COLUMN IF NOT EXISTS channel text,
  ADD COLUMN IF NOT EXISTS channel_id text,
  ADD COLUMN IF NOT EXISTS channel_url text,
  ADD COLUMN IF NOT EXISTS uploader text,
  ADD COLUMN IF NOT EXISTS uploader_id text,
  ADD COLUMN IF NOT EXISTS uploader_url text,
  ADD COLUMN IF NOT EXISTS thumbnail_url text,
  ADD COLUMN IF NOT EXISTS duration_string text,
  ADD COLUMN IF NOT EXISTS timestamp bigint,
  ADD COLUMN IF NOT EXISTS release_timestamp bigint,
  ADD COLUMN IF NOT EXISTS view_count bigint,
  ADD COLUMN IF NOT EXISTS like_count bigint,
  ADD COLUMN IF NOT EXISTS comment_count bigint,
  ADD COLUMN IF NOT EXISTS live_status text,
  ADD COLUMN IF NOT EXISTS availability text,
  ADD COLUMN IF NOT EXISTS age_limit bigint,
  ADD COLUMN IF NOT EXISTS categories text[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS tags text[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS metadata jsonb NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS videos_channel_id_idx
  ON videos(channel_id);

CREATE INDEX IF NOT EXISTS videos_uploader_id_idx
  ON videos(uploader_id);

CREATE INDEX IF NOT EXISTS videos_upload_date_idx
  ON videos(upload_date);

CREATE INDEX IF NOT EXISTS videos_view_count_idx
  ON videos(view_count);

CREATE INDEX IF NOT EXISTS videos_categories_idx
  ON videos USING gin(categories);

CREATE INDEX IF NOT EXISTS videos_tags_idx
  ON videos USING gin(tags);

CREATE INDEX IF NOT EXISTS videos_metadata_idx
  ON videos USING gin(metadata);

ALTER TABLE corpus_subscription_items
  ADD COLUMN IF NOT EXISTS channel text,
  ADD COLUMN IF NOT EXISTS channel_id text,
  ADD COLUMN IF NOT EXISTS uploader text,
  ADD COLUMN IF NOT EXISTS uploader_id text,
  ADD COLUMN IF NOT EXISTS view_count bigint,
  ADD COLUMN IF NOT EXISTS categories text[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS tags text[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS metadata jsonb NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS corpus_subscription_items_channel_id_idx
  ON corpus_subscription_items(subscription_id, channel_id);

CREATE INDEX IF NOT EXISTS corpus_subscription_items_uploader_id_idx
  ON corpus_subscription_items(subscription_id, uploader_id);

CREATE INDEX IF NOT EXISTS corpus_subscription_items_categories_idx
  ON corpus_subscription_items USING gin(categories);

CREATE INDEX IF NOT EXISTS corpus_subscription_items_tags_idx
  ON corpus_subscription_items USING gin(tags);
