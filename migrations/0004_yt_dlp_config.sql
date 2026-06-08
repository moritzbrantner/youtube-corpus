ALTER TABLE corpus_subscriptions
  ADD COLUMN IF NOT EXISTS yt_dlp jsonb NOT NULL DEFAULT '{"args":[]}';
