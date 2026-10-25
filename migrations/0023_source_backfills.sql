-- Initial history backfill when a channel/playlist is added (Atlas 1.10, 1.12, 1.14, 1.16; #67).
--
-- A backfill records the requested historical scope, the discovery snapshot of the source in its
-- displayed order, and which snapshot entries were selected. Selected videos are queued on their
-- `corpus_subscription_items` row (origin `history`); newly published videos found by source
-- checks are queued there with origin `new`. One corpus-owned dispatcher interleaves both.

ALTER TABLE corpus_subscription_items
  ADD COLUMN IF NOT EXISTS origin text NOT NULL DEFAULT 'new'
    CHECK (origin IN ('new', 'history')),
  ADD COLUMN IF NOT EXISTS queued_at timestamptz,
  ADD COLUMN IF NOT EXISTS last_error text;

CREATE INDEX IF NOT EXISTS corpus_subscription_items_queue_idx
  ON corpus_subscription_items(origin, queued_at)
  WHERE status = 'queued';

CREATE TABLE IF NOT EXISTS corpus_source_backfills (
  id uuid PRIMARY KEY,
  subscription_id uuid NOT NULL REFERENCES corpus_subscriptions(id) ON DELETE CASCADE,
  corpus_id uuid REFERENCES corpora(id) ON DELETE SET NULL,
  scope text NOT NULL CHECK (scope IN ('none', 'recent', 'limited', 'full')),
  recent_days integer CHECK (recent_days IS NULL OR recent_days > 0),
  limit_count bigint CHECK (limit_count IS NULL OR limit_count > 0),
  state text NOT NULL CHECK (
    state IN ('discovering', 'running', 'paused', 'canceled', 'awaiting_review',
              'completed', 'partial', 'failed')
  ),
  discovered_count bigint,
  reported_count bigint,
  discovery_message text,
  discovered_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (scope <> 'recent' OR recent_days IS NOT NULL),
  CHECK (scope <> 'limited' OR limit_count IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS corpus_source_backfills_subscription_idx
  ON corpus_source_backfills(subscription_id, created_at DESC);

CREATE INDEX IF NOT EXISTS corpus_source_backfills_state_idx
  ON corpus_source_backfills(state);

-- One row per entry of the discovery snapshot, in the source's displayed order (`position`
-- starts at 1). `selected` records the scope decision; `reason` explains an unselected entry.
-- `outcome` freezes the result of entries a cancel stopped; otherwise progress follows the
-- shared subscription item status, so the same video is never processed twice.
CREATE TABLE IF NOT EXISTS corpus_backfill_items (
  backfill_id uuid NOT NULL REFERENCES corpus_source_backfills(id) ON DELETE CASCADE,
  position integer NOT NULL CHECK (position > 0),
  subscription_id uuid NOT NULL,
  source_url text NOT NULL,
  youtube_id text,
  title text,
  upload_date text,
  selected boolean NOT NULL,
  reason text,
  outcome text,
  PRIMARY KEY (backfill_id, position),
  UNIQUE (backfill_id, source_url),
  FOREIGN KEY (subscription_id, source_url)
    REFERENCES corpus_subscription_items(subscription_id, source_url) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS corpus_backfill_items_item_idx
  ON corpus_backfill_items(subscription_id, source_url);
