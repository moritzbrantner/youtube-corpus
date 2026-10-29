//! One corpus video's own page (#102): catalog metadata plus every retained transcript stream
//! and OCR track, each kept as its own evidence with its source provenance. Moments are
//! resolved by stable evidence id through `/api/transcript-context`, not stored here.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::corpora::{list_corpus_videos, CorpusVideoFilter, CorpusVideoSummary};
use crate::search::{attach_ocr_evidence, SearchResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInspection {
    pub video: CorpusVideoSummary,
    pub description: Option<String>,
    /// Retained transcript streams, the preferred research stream first.
    pub transcript_streams: Vec<TranscriptStreamEvidence>,
    /// Whether on-screen text was never attempted, attempted without tracks, or indexed.
    pub ocr_status: OcrStatus,
    /// OCR tracks in time order, each carrying its producer/run/observation provenance.
    pub visual_text: Vec<SearchResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrStatus {
    Unprocessed,
    AttemptedWithoutTracks,
    Indexed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStreamEvidence {
    pub stream_id: Uuid,
    pub source_kind: String,
    pub language: Option<String>,
    pub status: String,
    pub is_preferred: bool,
    pub processor: String,
    pub processor_version: String,
    pub retrieved_at: chrono::DateTime<chrono::Utc>,
    pub processing_revision: i64,
    pub segments: Vec<TranscriptSegmentEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegmentEvidence {
    pub segment_id: Uuid,
    pub segment_index: i64,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
}

/// The video's page data, or `None` when the video is not part of the corpus.
pub async fn inspect_video(
    pool: &PgPool,
    corpus_id: Uuid,
    video_id: Uuid,
) -> anyhow::Result<Option<VideoInspection>> {
    let filter = CorpusVideoFilter {
        video_id: Some(video_id),
        ..CorpusVideoFilter::default()
    };
    let Some(video) = list_corpus_videos(pool, corpus_id, 1, &filter)
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };

    let description: Option<String> =
        sqlx::query_scalar("SELECT description FROM videos WHERE id = $1")
            .bind(video_id)
            .fetch_one(pool)
            .await?;

    let stream_rows = sqlx::query(
        "SELECT st.id, st.source_kind, st.language, st.status, st.processor, st.processor_version,
                st.retrieved_at, st.processing_revision, (p.stream_id IS NOT NULL) AS is_preferred
         FROM transcript_streams st
         LEFT JOIN preferred_transcript_streams p ON p.stream_id = st.id
         WHERE st.video_id = $1
         ORDER BY (p.stream_id IS NOT NULL) DESC, st.source_kind, st.language NULLS LAST",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let segment_rows = sqlx::query(
        "SELECT id, stream_id, segment_index, start_seconds, end_seconds, text
         FROM transcript_segments
         WHERE video_id = $1
         ORDER BY stream_id, segment_index",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut transcript_streams = stream_rows
        .into_iter()
        .map(|row| {
            Ok(TranscriptStreamEvidence {
                stream_id: row.try_get("id")?,
                source_kind: row.try_get("source_kind")?,
                language: row.try_get("language")?,
                status: row.try_get("status")?,
                processor: row.try_get("processor")?,
                processor_version: row.try_get("processor_version")?,
                is_preferred: row.try_get("is_preferred")?,
                retrieved_at: row.try_get("retrieved_at")?,
                processing_revision: row.try_get("processing_revision")?,
                segments: Vec::new(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    for row in segment_rows {
        let stream_id: Uuid = row.try_get("stream_id")?;
        if let Some(stream) = transcript_streams
            .iter_mut()
            .find(|stream| stream.stream_id == stream_id)
        {
            stream.segments.push(TranscriptSegmentEvidence {
                segment_id: row.try_get("id")?,
                segment_index: row.try_get("segment_index")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                text: row.try_get("text")?,
            });
        }
    }

    let track_rows = sqlx::query(
        "SELECT t.id, t.run_id, t.language, t.start_seconds, t.end_seconds, t.text
         FROM visual_text_tracks t
         WHERE t.video_id = $1
         ORDER BY t.start_seconds NULLS LAST, t.track_key, t.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let mut visual_text = track_rows
        .into_iter()
        .map(|row| {
            Ok(SearchResult {
                segment_id: row.try_get("id")?,
                video_id,
                stream_id: row.try_get("run_id")?,
                source_kind: "visual_ocr".to_string(),
                language: row.try_get("language")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                text: row.try_get("text")?,
                source_url: video.source_url.clone(),
                title: video.title.clone(),
                score: 0.0,
                fts_score: 0.0,
                semantic_score: 0.0,
                ocr_evidence: None,
                translation: None,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    attach_ocr_evidence(pool, &mut visual_text).await?;

    let ocr_status = if !visual_text.is_empty() {
        OcrStatus::Indexed
    } else if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM media_processing_runs
                        WHERE video_id = $1 AND modality = 'ocr')",
    )
    .bind(video_id)
    .fetch_one(pool)
    .await?
    {
        OcrStatus::AttemptedWithoutTracks
    } else {
        OcrStatus::Unprocessed
    };

    Ok(Some(VideoInspection {
        video,
        description,
        transcript_streams,
        ocr_status,
        visual_text,
    }))
}
