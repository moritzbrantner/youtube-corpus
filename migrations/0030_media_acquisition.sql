-- Plan-driven source media acquisition (#126) and its stream provenance (#120).
--
-- One JSON record of the media file behind `local_video_path` when the corpus downloaded it:
-- what the plan needed (audio, or video up to a height), the yt-dlp format selector and the
-- format ids it resolved to (including the audio track's language), and the stored file's
-- byte size and SHA-256. NULL for local input files and for media retained before #126.
ALTER TABLE videos ADD COLUMN media_acquisition jsonb;
