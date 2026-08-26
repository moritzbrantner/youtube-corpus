use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;

use crate::captions::TranscriptStream;
use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use crate::ingest::{
    ingest_corpus, transcript_segment_contract, transcript_segment_metadata, vector_literal,
    IngestReport, IngestRequest, TranscriptSegmentContractInput,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReprocessStage {
    Metadata,
    Captions,
    Asr,
    Segments,
    Embeddings,
    All,
}

impl ReprocessStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Captions => "captions",
            Self::Asr => "asr",
            Self::Segments => "segments",
            Self::Embeddings => "embeddings",
            Self::All => "all",
        }
    }

    fn includes_metadata(self) -> bool {
        matches!(
            self,
            Self::Metadata | Self::Captions | Self::Asr | Self::All
        )
    }

    fn includes_captions(self) -> bool {
        matches!(self, Self::Captions | Self::All)
    }

    fn includes_asr(self) -> bool {
        matches!(self, Self::Asr | Self::All)
    }

    fn includes_embeddings(self) -> bool {
        matches!(self, Self::Embeddings | Self::All)
    }
}

#[derive(Debug, Clone)]
pub struct ReprocessRequest {
    pub database_url: String,
    pub video_id: Uuid,
    pub stage: ReprocessStage,
    pub work_dir: PathBuf,
    pub caption_languages: Vec<String>,
    pub yt_dlp: YtDlpConfig,
    pub transcriber_command: Option<PathBuf>,
    pub transcriber_args: Vec<String>,
    pub transcriber_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReprocessReport {
    pub video_id: Uuid,
    pub stage: ReprocessStage,
    pub ingest: Option<IngestReport>,
    pub segments_resegmented: u64,
    pub segments_reembedded: u64,
    pub metadata_processing_revision: u64,
    pub stream_processing_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
enum StreamScope {
    All,
    Captions,
    Asr,
}

pub async fn reprocess_video(request: ReprocessRequest) -> anyhow::Result<ReprocessReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    let row = sqlx::query(
        "SELECT source_url, local_video_path
         FROM videos
         WHERE id = $1",
    )
    .bind(request.video_id)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("video not found"))?;

    let source_url: String = row.try_get("source_url")?;
    let local_video_path: Option<String> = row.try_get("local_video_path")?;
    let mut ingest = None;

    if request.stage.includes_metadata() {
        let source = source_for_reprocessing(&source_url, local_video_path.as_deref());
        let processing_config = serde_json::json!({
            "stage": request.stage.as_str(),
            "captions": {
                "enabled": request.stage.includes_captions(),
                "languages": &request.caption_languages,
            },
            "asr": {
                "enabled": request.stage.includes_asr(),
                "command": &request.transcriber_command,
                "args": &request.transcriber_args,
                "timeoutSeconds": request.transcriber_timeout_seconds,
            },
            "ytDlp": &request.yt_dlp,
        });
        let report = ingest_corpus(IngestRequest {
            run_id: Some(Uuid::new_v4()),
            database_url: request.database_url.clone(),
            source,
            work_dir: request.work_dir.clone(),
            caption: CaptionConfig {
                enabled: request.stage.includes_captions(),
                include_auto_captions: request.stage.includes_captions(),
                languages: request.caption_languages.clone(),
            },
            yt_dlp: request.yt_dlp.clone(),
            asr_enabled: request.stage.includes_asr(),
            transcriber_command: request.transcriber_command.clone(),
            transcriber_args: request.transcriber_args.clone(),
            transcriber_timeout_seconds: request.transcriber_timeout_seconds,
            max_items: None,
            title_contains: None,
            title_excludes: Vec::new(),
            duration_min: None,
            duration_max: None,
            migrate: false,
        })
        .await?;

        stamp_processing_provenance(&pool, request.video_id, request.stage, &processing_config)
            .await?;
        ingest = Some(report);
    }

    let segments_resegmented = if request.stage == ReprocessStage::Segments {
        let count = resegment_video(&pool, request.video_id, &source_url).await?;
        let config = serde_json::json!({
            "stage": "segments",
            "source": "stored-caption-files",
            "parser": "moenarch-text-transcripts",
            "embedding": {
                "provider": "hashed-text-embedder",
                "dimensions": 128,
            },
        });
        stamp_stream_provenance(&pool, request.video_id, StreamScope::Captions, &config).await?;
        count
    } else {
        0
    };

    let segments_reembedded = if request.stage.includes_embeddings() {
        let count = reembed_video(&pool, request.video_id).await?;
        let config = serde_json::json!({
            "stage": "embeddings",
            "embedder": "hashed-text-embedder",
            "dimensions": 128,
        });
        stamp_stream_provenance(&pool, request.video_id, StreamScope::All, &config).await?;
        count
    } else {
        0
    };

    let metadata_processing_revision = sqlx::query_scalar::<_, i64>(
        "SELECT metadata_processing_revision FROM videos WHERE id = $1",
    )
    .bind(request.video_id)
    .fetch_one(&pool)
    .await?
    .max(0) as u64;
    let stream_processing_revision = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(processing_revision) FROM transcript_streams WHERE video_id = $1",
    )
    .bind(request.video_id)
    .fetch_one(&pool)
    .await?
    .map(|value| value.max(0) as u64);

    Ok(ReprocessReport {
        video_id: request.video_id,
        stage: request.stage,
        ingest,
        segments_resegmented,
        segments_reembedded,
        metadata_processing_revision,
        stream_processing_revision,
    })
}

fn source_for_reprocessing(source_url: &str, local_video_path: Option<&str>) -> CorpusSource {
    if source_url.starts_with("https://") || source_url.starts_with("http://") {
        CorpusSource::YoutubeUrl {
            url: source_url.to_string(),
        }
    } else {
        CorpusSource::LocalFile {
            path: local_video_path
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(source_url)),
        }
    }
}

async fn stamp_processing_provenance(
    pool: &sqlx::PgPool,
    video_id: Uuid,
    stage: ReprocessStage,
    config: &serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE videos
         SET metadata_processor = 'youtube-corpus',
             metadata_processor_version = $2,
             metadata_processing_config = $3,
             metadata_retrieved_at = now()
         WHERE id = $1",
    )
    .bind(video_id)
    .bind(env!("CARGO_PKG_VERSION"))
    .bind(config)
    .execute(pool)
    .await?;

    match stage {
        ReprocessStage::Captions => {
            stamp_stream_provenance(pool, video_id, StreamScope::Captions, config).await?;
        }
        ReprocessStage::Asr => {
            stamp_stream_provenance(pool, video_id, StreamScope::Asr, config).await?;
        }
        ReprocessStage::All => {
            stamp_stream_provenance(pool, video_id, StreamScope::All, config).await?;
        }
        ReprocessStage::Metadata | ReprocessStage::Segments | ReprocessStage::Embeddings => {}
    }
    Ok(())
}

async fn stamp_stream_provenance(
    pool: &sqlx::PgPool,
    video_id: Uuid,
    scope: StreamScope,
    config: &serde_json::Value,
) -> anyhow::Result<()> {
    let sql = match scope {
        StreamScope::All => {
            "UPDATE transcript_streams
             SET processor = 'youtube-corpus',
                 processor_version = $2,
                 processing_config = $3,
                 retrieved_at = now()
             WHERE video_id = $1"
        }
        StreamScope::Captions => {
            "UPDATE transcript_streams
             SET processor = 'youtube-corpus',
                 processor_version = $2,
                 processing_config = $3,
                 retrieved_at = now()
             WHERE video_id = $1
               AND source_kind IN ('caption_manual', 'caption_auto')"
        }
        StreamScope::Asr => {
            "UPDATE transcript_streams
             SET processor = 'youtube-corpus',
                 processor_version = $2,
                 processing_config = $3,
                 retrieved_at = now()
             WHERE video_id = $1 AND source_kind = 'asr'"
        }
    };

    sqlx::query(sql)
        .bind(video_id)
        .bind(env!("CARGO_PKG_VERSION"))
        .bind(config)
        .execute(pool)
        .await?;
    Ok(())
}

async fn resegment_video(
    pool: &sqlx::PgPool,
    video_id: Uuid,
    source_url: &str,
) -> anyhow::Result<u64> {
    let source_paths = sqlx::query_scalar::<_, String>(
        "SELECT source_path
         FROM transcript_streams
         WHERE video_id = $1
           AND source_kind IN ('caption_manual', 'caption_auto')
           AND source_path IS NOT NULL",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let directories = source_paths
        .into_iter()
        .filter_map(|path| PathBuf::from(path).parent().map(Path::to_path_buf))
        .collect::<HashSet<_>>();
    if directories.is_empty() {
        anyhow::bail!("no stored caption files are available for re-segmentation");
    }

    let mut streams = Vec::new();
    for directory in directories {
        streams.extend(crate::captions::parse_caption_files(&directory).await?);
    }
    if streams.is_empty() {
        anyhow::bail!("stored caption files did not produce transcript streams");
    }

    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let mut tx = pool.begin().await?;
    let mut indexed = 0_u64;
    for stream in streams {
        indexed += resegment_stream(&mut tx, video_id, source_url, &stream, &embedder).await?;
    }
    tx.commit().await?;
    Ok(indexed)
}

async fn resegment_stream(
    tx: &mut Transaction<'_, Postgres>,
    video_id: Uuid,
    source_url: &str,
    stream: &TranscriptStream,
    embedder: &HashedTextEmbedder,
) -> anyhow::Result<u64> {
    let language = stream.language.clone().or_else(|| {
        stream
            .segments
            .iter()
            .find_map(|segment| segment.language.clone())
    });
    let stream_label = format!(
        "{}:{}",
        stream.source_kind.as_str(),
        language.as_deref().unwrap_or("und")
    );
    let stream_id = crate::youtube::stable_child_id(video_id, &stream_label);
    let full_text = stream.text.clone().unwrap_or_else(|| {
        stream
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    });

    sqlx::query(
        "INSERT INTO transcript_streams
         (id, video_id, source_kind, language, status, source_path, full_text, message)
         VALUES ($1, $2, $3, $4, 'completed', $5, $6, $7)
         ON CONFLICT (id) DO UPDATE SET
           status = EXCLUDED.status,
           source_path = EXCLUDED.source_path,
           full_text = EXCLUDED.full_text,
           message = EXCLUDED.message",
    )
    .bind(stream_id)
    .bind(video_id)
    .bind(stream.source_kind.as_str())
    .bind(&language)
    .bind(
        stream
            .source_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
    )
    .bind(&full_text)
    .bind(&stream.message)
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM transcript_segments WHERE stream_id = $1")
        .bind(stream_id)
        .execute(&mut **tx)
        .await?;

    let mut indexed = 0_u64;
    for segment in &stream.segments {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        let embedding = embedder
            .embed_text(text)
            .ok()
            .map(|vector| vector_literal(vector.as_slice()));
        let segment_id = crate::youtube::stable_child_id(stream_id, &segment.index.to_string());
        let contract = transcript_segment_contract(TranscriptSegmentContractInput {
            stream_id,
            source_url,
            source_kind: stream.source_kind,
            segment_index: segment.index,
            text,
            language: segment.language.clone().or_else(|| language.clone()),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
        });
        let metadata = transcript_segment_metadata(
            source_url,
            stream.source_kind,
            stream_id,
            &contract,
            segment.start_seconds,
            segment.end_seconds,
        )?;
        sqlx::query(
            "INSERT INTO transcript_segments
             (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language, metadata, embedding)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::vector)",
        )
        .bind(segment_id)
        .bind(stream_id)
        .bind(video_id)
        .bind(segment.index as i64)
        .bind(segment.start_seconds)
        .bind(segment.end_seconds)
        .bind(text)
        .bind(segment.language.clone().or_else(|| language.clone()))
        .bind(metadata)
        .bind(embedding)
        .execute(&mut **tx)
        .await?;
        indexed += 1;
    }
    Ok(indexed)
}

async fn reembed_video(pool: &sqlx::PgPool, video_id: Uuid) -> anyhow::Result<u64> {
    let rows = sqlx::query(
        "SELECT id, text
         FROM transcript_segments
         WHERE video_id = $1
         ORDER BY stream_id, segment_index",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;
    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let mut updated = 0_u64;

    for row in rows {
        let segment_id: Uuid = row.try_get("id")?;
        let text: String = row.try_get("text")?;
        let vector = embedder.embed_text(&text)?;
        sqlx::query("UPDATE transcript_segments SET embedding = $2::vector WHERE id = $1")
            .bind(segment_id)
            .bind(vector_literal(vector.as_slice()))
            .execute(pool)
            .await?;
        updated += 1;
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{source_for_reprocessing, ReprocessStage};
    use crate::config::CorpusSource;

    #[test]
    fn youtube_source_remains_network_source() {
        let source = source_for_reprocessing("https://youtu.be/example", None);
        assert!(matches!(source, CorpusSource::YoutubeUrl { .. }));
    }

    #[test]
    fn metadata_stage_does_not_rebuild_transcripts() {
        assert!(ReprocessStage::Metadata.includes_metadata());
        assert!(!ReprocessStage::Metadata.includes_captions());
        assert!(!ReprocessStage::Metadata.includes_asr());
        assert!(!ReprocessStage::Metadata.includes_embeddings());
    }

    #[test]
    fn segments_stage_is_offline() {
        assert!(!ReprocessStage::Segments.includes_metadata());
        assert!(!ReprocessStage::Segments.includes_captions());
        assert!(!ReprocessStage::Segments.includes_asr());
        assert!(!ReprocessStage::Segments.includes_embeddings());
    }
}
