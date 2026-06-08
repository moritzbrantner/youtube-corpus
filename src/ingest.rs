use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use text_embeddings::{HashedTextEmbedder, TextEmbeddingConfig};
use text_lexical::CorpusOptions;
use uuid::Uuid;

use crate::captions::TranscriptStream;
use crate::config::{CaptionConfig, CorpusSource};
use crate::youtube::{stable_child_id, stable_video_id, VideoItem};

#[derive(Debug, Clone)]
pub struct IngestRequest {
    pub database_url: String,
    pub source: CorpusSource,
    pub work_dir: PathBuf,
    pub caption: CaptionConfig,
    pub asr_enabled: bool,
    pub transcriber_command: Option<PathBuf>,
    pub transcriber_args: Vec<String>,
    pub max_items: Option<u64>,
    pub title_contains: Option<String>,
    pub title_excludes: Vec<String>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub migrate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub workflow: String,
    pub run_id: Uuid,
    pub videos_seen: u64,
    pub videos_indexed: u64,
    pub segments_indexed: u64,
    pub items: Vec<IngestItemReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestItemReport {
    pub video_id: Option<Uuid>,
    pub source_url: String,
    pub title: Option<String>,
    pub status: String,
    pub streams_indexed: u64,
    pub segments_indexed: u64,
    pub message: Option<String>,
}

pub async fn ingest_corpus(request: IngestRequest) -> anyhow::Result<IngestReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }

    tokio::fs::create_dir_all(&request.work_dir).await?;
    let items = resolve_items(&request).await?;
    ingest_resolved_items(&pool, request, items).await
}

pub async fn ingest_video_items(
    request: IngestRequest,
    items: Vec<VideoItem>,
) -> anyhow::Result<IngestReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }
    tokio::fs::create_dir_all(&request.work_dir).await?;
    ingest_resolved_items(&pool, request, items).await
}

async fn ingest_resolved_items(
    pool: &PgPool,
    request: IngestRequest,
    mut items: Vec<VideoItem>,
) -> anyhow::Result<IngestReport> {
    let run_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, request.source.source_url().as_bytes());
    let videos_seen = items.len() as u64;
    items.retain(|item| {
        crate::youtube::filter_item(
            item,
            &request.title_contains,
            &request.title_excludes,
            request.duration_min,
            request.duration_max,
        )
    });

    let mut report_items = Vec::new();
    let mut videos_indexed = 0;
    let mut segments_indexed = 0;

    for item in items {
        match ingest_item(&pool, &request, &item).await {
            Ok(item_report) => {
                if item_report.status == "indexed" {
                    videos_indexed += 1;
                    segments_indexed += item_report.segments_indexed;
                }
                report_items.push(item_report);
            }
            Err(error) => report_items.push(IngestItemReport {
                video_id: None,
                source_url: item.source_url.clone(),
                title: item.title.clone(),
                status: "failed".to_string(),
                streams_indexed: 0,
                segments_indexed: 0,
                message: Some(error.to_string()),
            }),
        }
    }

    let report = IngestReport {
        workflow: "youtube_corpus_ingest".to_string(),
        run_id,
        videos_seen,
        videos_indexed,
        segments_indexed,
        items: report_items,
    };
    sqlx::query(
        "INSERT INTO ingest_runs (id, source_url, status, videos_seen, videos_indexed, segments_indexed, report)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status, videos_seen = EXCLUDED.videos_seen,
           videos_indexed = EXCLUDED.videos_indexed, segments_indexed = EXCLUDED.segments_indexed,
           report = EXCLUDED.report",
    )
    .bind(run_id)
    .bind(request.source.source_url())
    .bind("completed")
    .bind(report.videos_seen as i64)
    .bind(report.videos_indexed as i64)
    .bind(report.segments_indexed as i64)
    .bind(serde_json::to_value(&report)?)
    .execute(pool)
    .await?;
    Ok(report)
}

async fn resolve_items(request: &IngestRequest) -> anyhow::Result<Vec<VideoItem>> {
    match &request.source {
        CorpusSource::YoutubeUrl { url } => Ok(vec![crate::youtube::single_url_item(url.clone())]),
        CorpusSource::PlaylistUrl { url } | CorpusSource::ChannelUrl { url } => {
            crate::youtube::discover_collection(url, request.max_items).await
        }
        CorpusSource::LocalFile { path } => Ok(vec![crate::youtube::local_file_item(path.clone())]),
    }
}

async fn ingest_item(
    pool: &PgPool,
    request: &IngestRequest,
    item: &VideoItem,
) -> anyhow::Result<IngestItemReport> {
    let mut item = item.clone();
    let item_dir = request.work_dir.join(&item.item_id);
    let metadata_dir = item_dir.join("metadata");
    let caption_dir = item_dir.join("captions");
    let mut video_path = item.local_video_path.clone();

    if let Err(error) = crate::youtube::enrich_video_metadata(&mut item, &metadata_dir).await {
        tracing::warn!(
            source_url = %item.source_url,
            error = %error,
            "video metadata download failed"
        );
    }

    let mut streams =
        crate::captions::download_and_parse_captions(&item, &caption_dir, &request.caption)
            .await
            .unwrap_or_default();

    if request.asr_enabled {
        let media_path = match &video_path {
            Some(path) => path.clone(),
            None => {
                let path = crate::youtube::download_video(&item, &item_dir.join("media")).await?;
                video_path = Some(path.clone());
                path
            }
        };
        match crate::asr::transcribe_with_command(
            &media_path,
            &item_dir,
            request.transcriber_command.as_ref(),
            &request.transcriber_args,
        )
        .await
        {
            Ok(Some(stream)) => streams.push(stream),
            Ok(None) => {}
            Err(error) => streams.push(TranscriptStream {
                source_kind: crate::config::SourceKind::Asr,
                language: None,
                source_path: None,
                text: None,
                segments: Vec::new(),
                message: Some(error.to_string()),
            }),
        }
    }

    let video_id = stable_video_id(&item.source_url);
    let mut tx = pool.begin().await?;
    upsert_video(&mut tx, video_id, &item, video_path.as_deref()).await?;

    let embedder =
        HashedTextEmbedder::new(TextEmbeddingConfig::default(), CorpusOptions::default())?;
    let mut streams_indexed = 0;
    let mut segments_indexed = 0;
    for stream in streams {
        if stream.segments.is_empty() {
            continue;
        }
        let indexed =
            upsert_stream(&mut tx, video_id, &item.source_url, &stream, &embedder).await?;
        if indexed > 0 {
            streams_indexed += 1;
            segments_indexed += indexed;
        }
    }
    tx.commit().await?;

    Ok(IngestItemReport {
        video_id: Some(video_id),
        source_url: item.source_url.clone(),
        title: item.title.clone(),
        status: if segments_indexed > 0 {
            "indexed"
        } else {
            "no_transcript"
        }
        .to_string(),
        streams_indexed,
        segments_indexed,
        message: None,
    })
}

async fn upsert_video(
    tx: &mut Transaction<'_, Postgres>,
    video_id: Uuid,
    item: &VideoItem,
    video_path: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO videos
         (id, youtube_id, source_url, title, local_video_path, duration_seconds, upload_date,
          description, channel, channel_id, channel_url, uploader, uploader_id, uploader_url,
          thumbnail_url, duration_string, timestamp, release_timestamp, view_count, like_count,
          comment_count, live_status, availability, age_limit, categories, tags, metadata)
         VALUES
         ($1, $2, $3, $4, $5, $6, $7,
          $8, $9, $10, $11, $12, $13, $14,
          $15, $16, $17, $18, $19, $20,
          $21, $22, $23, $24, $25, $26, $27)
         ON CONFLICT (id) DO UPDATE SET youtube_id = EXCLUDED.youtube_id, source_url = EXCLUDED.source_url,
           title = EXCLUDED.title, local_video_path = EXCLUDED.local_video_path,
           duration_seconds = EXCLUDED.duration_seconds, upload_date = EXCLUDED.upload_date,
           description = EXCLUDED.description, channel = EXCLUDED.channel, channel_id = EXCLUDED.channel_id,
           channel_url = EXCLUDED.channel_url, uploader = EXCLUDED.uploader, uploader_id = EXCLUDED.uploader_id,
           uploader_url = EXCLUDED.uploader_url, thumbnail_url = EXCLUDED.thumbnail_url,
           duration_string = EXCLUDED.duration_string, timestamp = EXCLUDED.timestamp,
           release_timestamp = EXCLUDED.release_timestamp, view_count = EXCLUDED.view_count,
           like_count = EXCLUDED.like_count, comment_count = EXCLUDED.comment_count,
           live_status = EXCLUDED.live_status, availability = EXCLUDED.availability,
           age_limit = EXCLUDED.age_limit, categories = EXCLUDED.categories, tags = EXCLUDED.tags,
           metadata = EXCLUDED.metadata,
           updated_at = now()",
    )
    .bind(video_id)
    .bind(&item.youtube_id)
    .bind(&item.source_url)
    .bind(&item.title)
    .bind(video_path.map(|path| path.to_string_lossy().into_owned()))
    .bind(item.duration_seconds)
    .bind(&item.upload_date)
    .bind(&item.metadata.description)
    .bind(&item.metadata.channel)
    .bind(&item.metadata.channel_id)
    .bind(&item.metadata.channel_url)
    .bind(&item.metadata.uploader)
    .bind(&item.metadata.uploader_id)
    .bind(&item.metadata.uploader_url)
    .bind(&item.metadata.thumbnail_url)
    .bind(&item.metadata.duration_string)
    .bind(item.metadata.timestamp)
    .bind(item.metadata.release_timestamp)
    .bind(item.metadata.view_count)
    .bind(item.metadata.like_count)
    .bind(item.metadata.comment_count)
    .bind(&item.metadata.live_status)
    .bind(&item.metadata.availability)
    .bind(item.metadata.age_limit)
    .bind(&item.metadata.categories)
    .bind(&item.metadata.tags)
    .bind(serde_json::to_value(&item.metadata)?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_stream(
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
    let stream_id = stable_child_id(video_id, &stream_label);
    let full_text = stream.text.clone().unwrap_or_else(|| {
        stream
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    });
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, source_path, full_text, message)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status, source_path = EXCLUDED.source_path,
           full_text = EXCLUDED.full_text, message = EXCLUDED.message",
    )
    .bind(stream_id)
    .bind(video_id)
    .bind(stream.source_kind.as_str())
    .bind(&language)
    .bind("completed")
    .bind(stream.source_path.as_ref().map(|path| path.to_string_lossy().into_owned()))
    .bind(&full_text)
    .bind(&stream.message)
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM transcript_segments WHERE stream_id = $1")
        .bind(stream_id)
        .execute(&mut **tx)
        .await?;

    let mut indexed = 0;
    for segment in &stream.segments {
        let text = segment.text.trim();
        if text.is_empty() {
            continue;
        }
        let embedding = match embedder.embed_text(text) {
            Ok(vector) => Some(vector_literal(vector.as_slice())),
            Err(_) => None,
        };
        let segment_id = stable_child_id(stream_id, &segment.index.to_string());
        let metadata = serde_json::json!({
            "source_url": source_url,
            "source_kind": stream.source_kind.as_str(),
            "start_seconds": segment.start_seconds,
            "end_seconds": segment.end_seconds,
        });
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

pub fn vector_literal(values: &[f32]) -> String {
    let mut output = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&format!("{value:.8}"));
    }
    output.push(']');
    output
}
