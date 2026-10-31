-- Automatically extracted, unreviewed historical event claims (Atlas 7.1; issue #107).
--
-- An extraction run reads one video's transcript segments and visual text (OCR) tracks and
-- suggests claims. Suggestions are ordinary `historical_event_claims` rows with
-- `review_status = 'unverified'` plus the producer, run and the exact matched source text, so a
-- suggestion is never shown as an established fact. Reruns keep reviewed (confirmed or rejected)
-- claims untouched, keep unchanged suggestions in place, and drop suggestions the new run no
-- longer produces. Runs are append-only so reviewed claims keep their provenance.
CREATE TABLE historical_event_extraction_runs (
    id uuid PRIMARY KEY,
    video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    producer text NOT NULL,
    producer_version text NOT NULL,
    config text NOT NULL,
    transcript_stream_id uuid REFERENCES transcript_streams(id) ON DELETE SET NULL,
    segments_analyzed integer NOT NULL CHECK (segments_analyzed >= 0),
    visual_text_tracks_analyzed integer NOT NULL CHECK (visual_text_tracks_analyzed >= 0),
    -- Date expressions found below the confidence threshold and not suggested.
    low_confidence_dropped integer NOT NULL CHECK (low_confidence_dropped >= 0),
    claims_suggested integer NOT NULL CHECK (claims_suggested >= 0),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX historical_event_extraction_runs_video_idx
    ON historical_event_extraction_runs(video_id, created_at);

ALTER TABLE historical_event_claims
    ADD COLUMN extraction_run_id uuid REFERENCES historical_event_extraction_runs(id),
    -- The matched expression exactly as it appears in the evidence text, and its byte range.
    ADD COLUMN source_text text,
    ADD COLUMN source_byte_start integer CHECK (source_byte_start >= 0),
    ADD COLUMN source_byte_end integer,
    ADD COLUMN extraction_confidence real
        CHECK (extraction_confidence IS NULL OR extraction_confidence BETWEEN 0 AND 1),
    ADD CONSTRAINT historical_event_claims_extraction_check CHECK (
        extraction_run_id IS NULL
        OR (source_text IS NOT NULL AND source_byte_start IS NOT NULL
            AND source_byte_end > source_byte_start AND extraction_confidence IS NOT NULL)
    );

CREATE INDEX historical_event_claims_run_idx ON historical_event_claims(extraction_run_id);
