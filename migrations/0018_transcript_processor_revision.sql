-- Historical release labels cannot establish which code produced an output.
-- Leave these rows unknown; only actual transcript writes record a revision.
ALTER TABLE transcript_streams ADD COLUMN processor_revision text;

CREATE OR REPLACE FUNCTION youtube_corpus_bump_stream_revision()
RETURNS trigger AS $$
BEGIN
  IF ROW(NEW.full_text, NEW.source_path, NEW.status, NEW.processing_config,
         NEW.processor, NEW.processor_version, NEW.processor_revision)
     IS DISTINCT FROM
     ROW(OLD.full_text, OLD.source_path, OLD.status, OLD.processing_config,
         OLD.processor, OLD.processor_version, OLD.processor_revision) THEN
    NEW.processing_revision := OLD.processing_revision + 1;
    NEW.retrieved_at := now();
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS transcript_streams_revision_trigger ON transcript_streams;
CREATE TRIGGER transcript_streams_revision_trigger
BEFORE UPDATE OF full_text, source_path, status, processing_config,
                 processor, processor_version, processor_revision
ON transcript_streams
FOR EACH ROW EXECUTE FUNCTION youtube_corpus_bump_stream_revision();
