-- Per-source admission policy for newly discovered videos (Atlas 1.6, 1.8; #66). Review-first
-- items are registered with status 'awaiting_review' and only processed once approved, unless
-- another applicable source processes the same video automatically.
ALTER TABLE corpus_subscriptions
  ADD COLUMN IF NOT EXISTS processing_policy text NOT NULL DEFAULT 'automatic'
    CHECK (processing_policy IN ('automatic', 'review_first'));
