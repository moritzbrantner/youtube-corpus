-- Per-source automatic check schedule (Atlas 1.18, 1.20; #68).
--
-- Every monitored source has its own check interval. `next_check_at` is when the server's
-- scheduler checks the source next; after any completed check it becomes the check time plus
-- the interval, so a server that was stopped for several intervals checks an overdue source
-- once and then resumes its own cadence. `check_started_at` marks the one running check of a
-- source so scheduled, manual and restart catch-up checks coalesce instead of duplicating work.
ALTER TABLE corpus_subscriptions
  ADD COLUMN IF NOT EXISTS check_interval_seconds bigint NOT NULL DEFAULT 21600
    CHECK (check_interval_seconds >= 60),
  ADD COLUMN IF NOT EXISTS next_check_at timestamptz,
  ADD COLUMN IF NOT EXISTS check_started_at timestamptz;

UPDATE corpus_subscriptions
SET next_check_at = coalesce(last_checked_at, created_at)
                    + check_interval_seconds * interval '1 second'
WHERE next_check_at IS NULL;

CREATE INDEX IF NOT EXISTS corpus_subscriptions_next_check_idx
  ON corpus_subscriptions(next_check_at)
  WHERE enabled;
