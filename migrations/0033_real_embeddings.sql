-- Real multilingual embeddings and language-aware full-text search (#192).
--
-- Embeddings: the hashed bag-of-words vectors (`vector(128)`) are replaced by the pinned
-- nlp-stack `intfloat/multilingual-e5-base` preset (768 dimensions, mean pooling). The old
-- vectors carry no meaning in the new space and are dropped, not converted; `youtube-corpus
-- reembed` fills the new columns in resumable batches. Every vector references the
-- `embedding_models` row (model id, revision, pooling) that produced it, and a vector without
-- a model (or a model without a vector) is rejected.
--
-- Full-text search: each row's tsvector uses the text search configuration of its language
-- (german/english/spanish); `simple` is only the tokenizer for languages without a configured
-- stemmer, or rows with no language. The configuration is stored beside the vector so queries
-- are parsed with the same configuration as the row they match.

CREATE TABLE embedding_models (
    id serial PRIMARY KEY,
    model_id text NOT NULL CHECK (btrim(model_id) <> ''),
    revision text NOT NULL CHECK (btrim(revision) <> ''),
    pooling text NOT NULL CHECK (btrim(pooling) <> ''),
    dimensions integer NOT NULL CHECK (dimensions > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (model_id, revision, pooling)
);

-- Text search configuration for a stored language tag (ISO 639-1/-2 code, optionally with a
-- region or variant such as `de-DE` or `en-orig`, or the English language name).
CREATE FUNCTION corpus_text_search_config(language text) RETURNS regconfig
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT CASE lower(split_part(split_part(btrim(coalesce(language, '')), '-', 1), '_', 1))
    WHEN 'de' THEN 'pg_catalog.german'::regconfig
    WHEN 'deu' THEN 'pg_catalog.german'::regconfig
    WHEN 'ger' THEN 'pg_catalog.german'::regconfig
    WHEN 'german' THEN 'pg_catalog.german'::regconfig
    WHEN 'en' THEN 'pg_catalog.english'::regconfig
    WHEN 'eng' THEN 'pg_catalog.english'::regconfig
    WHEN 'english' THEN 'pg_catalog.english'::regconfig
    WHEN 'es' THEN 'pg_catalog.spanish'::regconfig
    WHEN 'spa' THEN 'pg_catalog.spanish'::regconfig
    WHEN 'spanish' THEN 'pg_catalog.spanish'::regconfig
    ELSE 'pg_catalog.simple'::regconfig
  END
$$;

DROP VIEW IF EXISTS corpus_search_segments;

-- Transcript segments.
DROP INDEX IF EXISTS transcript_segments_embedding_idx;
DROP INDEX IF EXISTS transcript_segments_fts_idx;
ALTER TABLE transcript_segments
    DROP COLUMN embedding,
    DROP COLUMN search_vector;
ALTER TABLE transcript_segments
    ADD COLUMN search_config regconfig GENERATED ALWAYS AS (corpus_text_search_config(language)) STORED,
    ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        to_tsvector(corpus_text_search_config(language), text)
    ) STORED,
    ADD COLUMN embedding vector(768),
    ADD COLUMN embedding_model integer REFERENCES embedding_models(id),
    ADD CONSTRAINT transcript_segments_embedding_model_check
        CHECK ((embedding IS NULL) = (embedding_model IS NULL));
CREATE INDEX transcript_segments_fts_idx ON transcript_segments USING gin(search_vector);
CREATE INDEX transcript_segments_embedding_idx
    ON transcript_segments USING hnsw (embedding vector_cosine_ops);
CREATE INDEX transcript_segments_pending_embedding_idx
    ON transcript_segments(id) WHERE embedding_model IS NULL;

-- OCR text tracks.
DROP INDEX IF EXISTS visual_text_tracks_search_idx;
ALTER TABLE visual_text_tracks
    DROP COLUMN embedding,
    DROP COLUMN search_vector;
ALTER TABLE visual_text_tracks
    ADD COLUMN search_config regconfig GENERATED ALWAYS AS (corpus_text_search_config(language)) STORED,
    ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        to_tsvector(corpus_text_search_config(language), coalesce(text, ''))
    ) STORED,
    ADD COLUMN embedding vector(768),
    ADD COLUMN embedding_model integer REFERENCES embedding_models(id),
    ADD CONSTRAINT visual_text_tracks_embedding_model_check
        CHECK ((embedding IS NULL) = (embedding_model IS NULL));
CREATE INDEX visual_text_tracks_search_idx ON visual_text_tracks USING gin(search_vector);
CREATE INDEX visual_text_tracks_embedding_idx
    ON visual_text_tracks USING hnsw (embedding vector_cosine_ops);

-- Translated spans: the language is the translation's target language.
DROP INDEX IF EXISTS translated_segments_search_idx;
ALTER TABLE translated_segments
    DROP COLUMN embedding,
    DROP COLUMN search_vector;
ALTER TABLE translated_segments
    ADD COLUMN search_config regconfig GENERATED ALWAYS AS (corpus_text_search_config(target_language)) STORED,
    ADD COLUMN search_vector tsvector GENERATED ALWAYS AS (
        to_tsvector(corpus_text_search_config(target_language), text)
    ) STORED,
    ADD COLUMN embedding vector(768),
    ADD COLUMN embedding_model integer REFERENCES embedding_models(id),
    ADD CONSTRAINT translated_segments_embedding_model_check
        CHECK ((embedding IS NULL) = (embedding_model IS NULL));
CREATE INDEX translated_segments_search_idx ON translated_segments USING gin(search_vector);
CREATE INDEX translated_segments_embedding_idx
    ON translated_segments USING hnsw (embedding vector_cosine_ops);

-- Same rows as 0022, plus each row's text search configuration and embedding model.
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
  s.search_config,
  s.search_vector,
  s.embedding,
  s.embedding_model,
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
  t.search_config,
  t.search_vector,
  t.embedding,
  t.embedding_model,
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
  tr.search_config,
  tr.search_vector,
  tr.embedding,
  tr.embedding_model,
  true AS translated
FROM translated_segments tr
LEFT JOIN transcript_segments seg ON seg.id = tr.transcript_segment_id
LEFT JOIN transcript_streams st ON st.id = seg.stream_id
LEFT JOIN visual_text_tracks track ON track.id = tr.visual_text_track_id
-- A translation of text that has since changed is stale and not searchable until refreshed.
WHERE encode(sha256(convert_to(coalesce(seg.text, track.text), 'UTF8')), 'hex') = tr.source_text_hash;
