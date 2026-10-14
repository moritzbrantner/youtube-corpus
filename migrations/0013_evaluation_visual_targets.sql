-- Evaluation targets for visible-text (OCR track) hits keep their own identity, so an
-- overlapping spoken transcript result can never satisfy a judged OCR target. The
-- transcript-only `segment_id` foreign key stays NULL for these targets.
ALTER TABLE retrieval_evaluation_targets
  ADD COLUMN IF NOT EXISTS visual_text_track_id uuid
    REFERENCES visual_text_tracks(id) ON DELETE CASCADE;

ALTER TABLE retrieval_evaluation_targets
  DROP CONSTRAINT IF EXISTS retrieval_evaluation_targets_check;

ALTER TABLE retrieval_evaluation_targets
  ADD CONSTRAINT retrieval_evaluation_targets_target_check CHECK (
    video_id IS NOT NULL OR segment_id IS NOT NULL OR source_url IS NOT NULL
    OR visual_text_track_id IS NOT NULL
  );
