use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ingest::{IngestReport, IngestRequest};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestProvenance {
    pub workflow: String,
    pub source_url: String,
    pub caption: crate::config::CaptionConfig,
    pub yt_dlp: crate::config::YtDlpConfig,
    pub asr_enabled: bool,
    pub transcriber_command: Option<String>,
    pub transcriber_args: Vec<String>,
    pub transcriber_timeout_seconds: Option<u64>,
    pub embedding: EmbeddingProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProvenance {
    pub provider: String,
    pub dimensions: u64,
}

impl IngestProvenance {
    pub fn from_request(request: &IngestRequest) -> Self {
        Self {
            workflow: "youtube_corpus_ingest".to_string(),
            source_url: request.source.source_url(),
            caption: request.caption.clone(),
            yt_dlp: request.yt_dlp.clone(),
            asr_enabled: request.asr_enabled,
            transcriber_command: request
                .transcriber_command
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            transcriber_args: request.transcriber_args.clone(),
            transcriber_timeout_seconds: request.transcriber_timeout_seconds,
            embedding: EmbeddingProvenance {
                provider: "hashed-text-embedder".to_string(),
                dimensions: 128,
            },
        }
    }
}

pub async fn record_ingest_report(
    database_url: &str,
    report: &IngestReport,
    provenance: &IngestProvenance,
) -> anyhow::Result<()> {
    let pool = crate::db::connect(database_url).await?;
    let processing_config = serde_json::to_value(provenance)?;

    for item in &report.items {
        let Some(video_id) = item.video_id else {
            continue;
        };
        record_video_provenance(&pool, video_id, &processing_config).await?;
    }
    Ok(())
}

async fn record_video_provenance(
    pool: &sqlx::PgPool,
    video_id: Uuid,
    processing_config: &serde_json::Value,
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
    .bind(processing_config)
    .execute(pool)
    .await?;

    sqlx::query(
        "UPDATE transcript_streams
         SET processor = 'youtube-corpus',
             processor_version = $2,
             processing_config = $3,
             retrieved_at = now()
         WHERE video_id = $1",
    )
    .bind(video_id)
    .bind(env!("CARGO_PKG_VERSION"))
    .bind(processing_config)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::IngestProvenance;
    use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
    use crate::ingest::IngestRequest;

    #[test]
    fn captures_processing_inputs_without_execution_state() {
        let request = IngestRequest {
            run_id: None,
            database_url: "postgres://unused".to_string(),
            source: CorpusSource::YoutubeUrl {
                url: "https://youtu.be/example".to_string(),
            },
            work_dir: PathBuf::from("use-case-output/test"),
            caption: CaptionConfig::default(),
            yt_dlp: YtDlpConfig::default(),
            asr_enabled: false,
            transcriber_command: None,
            transcriber_args: Vec::new(),
            transcriber_timeout_seconds: None,
            max_items: None,
            title_contains: None,
            title_excludes: Vec::new(),
            duration_min: None,
            duration_max: None,
            migrate: false,
        };
        let provenance = IngestProvenance::from_request(&request);
        assert_eq!(provenance.source_url, "https://youtu.be/example");
        assert_eq!(provenance.embedding.provider, "hashed-text-embedder");
        assert_eq!(provenance.embedding.dimensions, 128);
    }
}
