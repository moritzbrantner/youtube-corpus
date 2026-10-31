-- Spoken self-introductions as person-name evidence (Atlas 4.7, issue #135).
--
-- Name evidence is either an on-screen name tag aligned with one face track (`visible_text`,
-- #65) or a self-introduction in a transcript segment aligned with one voice track (`spoken`).
ALTER TABLE person_name_evidence
    DROP CONSTRAINT person_name_evidence_modality_check,
    ALTER COLUMN visual_text_track_id DROP NOT NULL,
    ALTER COLUMN face_track_id DROP NOT NULL,
    ADD COLUMN transcript_segment_id uuid REFERENCES transcript_segments(id) ON DELETE CASCADE,
    ADD COLUMN voice_track_id uuid REFERENCES voice_tracks(id) ON DELETE CASCADE,
    ADD CONSTRAINT person_name_evidence_modality_check CHECK (
        (modality = 'visible_text'
            AND visual_text_track_id IS NOT NULL AND face_track_id IS NOT NULL
            AND transcript_segment_id IS NULL AND voice_track_id IS NULL)
        OR (modality = 'spoken'
            AND transcript_segment_id IS NOT NULL AND voice_track_id IS NOT NULL
            AND visual_text_track_id IS NULL AND face_track_id IS NULL)
    );
