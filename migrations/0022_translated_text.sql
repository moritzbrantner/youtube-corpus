-- Opt-in translated text derived from original spoken transcript segments and on-screen OCR
-- tracks (Atlas 3.9, 3.11, 3.13; issue #69). Translations are derived records beside the
-- original evidence, never replacements: each row keeps its source span, modality, source and
-- target language and the translation provider/model/configuration that produced it.
--
-- One current translation exists per source span and target language, so overlapping requests
-- for the same span/target share one row. A changed source text or translation configuration
-- replaces only that row.
CREATE TABLE translated_segments (
    id uuid PRIMARY KEY,
    video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    modality text NOT NULL CHECK (modality IN ('spoken', 'visible_text')),
    -- Not foreign keys: re-ingesting a transcript replaces its rows under the same stable IDs,
    -- and an unchanged span must keep reusing its translation. Search and status only expose
    -- translations whose source span still exists; the translation stage removes orphans.
    transcript_segment_id uuid,
    visual_text_track_id uuid,
    -- Hex SHA-256 of the exact source text that was translated.
    source_text_hash text NOT NULL,
    source_language text NOT NULL,
    target_language text NOT NULL CHECK (target_language <> source_language),
    start_seconds double precision CHECK (start_seconds IS NULL OR start_seconds >= 0),
    end_seconds double precision CHECK (
        end_seconds IS NULL OR (end_seconds >= 0 AND (start_seconds IS NULL OR end_seconds >= start_seconds))
    ),
    text text NOT NULL,
    provider text NOT NULL,
    producer_revision text NOT NULL,
    plan jsonb NOT NULL,
    config jsonb NOT NULL,
    config_hash text NOT NULL,
    search_vector tsvector GENERATED ALWAYS AS (to_tsvector('simple', text)) STORED,
    embedding vector(128),
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (
        (modality = 'spoken' AND transcript_segment_id IS NOT NULL AND visual_text_track_id IS NULL)
        OR (modality = 'visible_text' AND visual_text_track_id IS NOT NULL AND transcript_segment_id IS NULL)
    )
);

CREATE UNIQUE INDEX translated_segments_source_target_idx
    ON translated_segments ((coalesce(transcript_segment_id, visual_text_track_id)), target_language);
CREATE INDEX translated_segments_video_idx
    ON translated_segments(video_id, modality, target_language);
CREATE INDEX translated_segments_search_idx
    ON translated_segments USING gin(search_vector);

-- Latest outcome per video, modality and target language. Status is per target, so a failed or
-- unsupported target never reads as translated because another target or modality succeeded.
CREATE TABLE translation_runs (
    video_id uuid NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
    modality text NOT NULL CHECK (modality IN ('spoken', 'visible_text')),
    target_language text NOT NULL,
    state text NOT NULL CHECK (state IN ('completed', 'reused', 'failed', 'unavailable', 'skipped')),
    message text,
    translated integer NOT NULL DEFAULT 0 CHECK (translated >= 0),
    reused integer NOT NULL DEFAULT 0 CHECK (reused >= 0),
    untranslatable integer NOT NULL DEFAULT 0 CHECK (untranslatable >= 0),
    finished_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (video_id, modality, target_language)
);

-- Unified search over original and translated evidence. A translated row keeps the original
-- evidence's source kind and stream, so source and modality filters describe where the text
-- came from; its language is the translation's target language.
DROP VIEW IF EXISTS corpus_search_segments;

CREATE VIEW corpus_search_segments AS
SELECT
  s.id,
  s.video_id,
  s.stream_id,
  st.source_kind,
  s.language,
  s.start_seconds,
  s.end_seconds,
  s.text,
  s.search_vector,
  s.embedding,
  false AS translated
FROM transcript_segments s
JOIN transcript_streams st ON st.id = s.stream_id
UNION ALL
SELECT
  t.id,
  t.video_id,
  t.run_id AS stream_id,
  'visual_ocr'::text AS source_kind,
  t.language,
  t.start_seconds,
  t.end_seconds,
  t.text,
  t.search_vector,
  t.embedding,
  false AS translated
FROM visual_text_tracks t
UNION ALL
SELECT
  tr.id,
  tr.video_id,
  coalesce(seg.stream_id, track.run_id) AS stream_id,
  coalesce(st.source_kind, 'visual_ocr') AS source_kind,
  tr.target_language AS language,
  tr.start_seconds,
  tr.end_seconds,
  tr.text,
  tr.search_vector,
  tr.embedding,
  true AS translated
FROM translated_segments tr
LEFT JOIN transcript_segments seg ON seg.id = tr.transcript_segment_id
LEFT JOIN transcript_streams st ON st.id = seg.stream_id
LEFT JOIN visual_text_tracks track ON track.id = tr.visual_text_track_id
-- A translation of text that has since changed is stale and not searchable until refreshed.
WHERE encode(sha256(convert_to(coalesce(seg.text, track.text), 'UTF8')), 'hex') = tr.source_text_hash;
