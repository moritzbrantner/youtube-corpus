-- Raw display cues of a stream whose indexed segments are derived passages (#92).
--
-- Automatic captions roll, so their `transcript_segments` are de-rolled passages; the cues they
-- were derived from stay here unchanged as evidence. They are exactly what such a stream indexed
-- before passages existed, so `span --representation source-cues` still reproduces the
-- `span_sha256` of earlier citations. Rebuilt with the stream, never edited in place.
CREATE TABLE transcript_source_cues (
    stream_id uuid NOT NULL REFERENCES transcript_streams(id) ON DELETE CASCADE,
    cue_index bigint NOT NULL CHECK (cue_index >= 0),
    start_seconds double precision,
    end_seconds double precision,
    text text NOT NULL,
    PRIMARY KEY (stream_id, cue_index)
);
