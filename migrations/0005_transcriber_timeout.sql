ALTER TABLE corpus_subscriptions
  ADD COLUMN IF NOT EXISTS transcriber_timeout_seconds bigint;
