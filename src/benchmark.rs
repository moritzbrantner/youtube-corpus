use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::{CaptionConfig, CorpusSource, SearchMode, YtDlpConfig};
use crate::ingest::{ingest_corpus, IngestReport};
use crate::search::{search_corpus, SearchReport, SearchRequest};

pub const DISTINGUO_VIDEOS_URL: &str = "https://www.youtube.com/@Distinguo/videos";

pub const DISTINGUO_SEARCH_QUERIES: &[&str] = &["faith alone", "justification", "church history"];

#[derive(Debug, Clone)]
pub struct BenchmarkRequest {
    pub database_url: String,
    pub work_dir: PathBuf,
    pub max_items: u64,
    pub caption: CaptionConfig,
    pub yt_dlp: YtDlpConfig,
    pub asr_enabled: bool,
    pub migrate: bool,
    pub search_queries: Vec<String>,
    pub top_k: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReport {
    pub benchmark: String,
    pub source_url: String,
    pub max_items: u64,
    pub ingest_elapsed_ms: u128,
    pub search_elapsed_ms: u128,
    pub ingest: IngestReport,
    pub searches: Vec<TimedSearchReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedSearchReport {
    pub elapsed_ms: u128,
    pub report: SearchReport,
}

pub async fn run_distinguo_benchmark(request: BenchmarkRequest) -> anyhow::Result<BenchmarkReport> {
    let ingest_request = crate::ingest::IngestRequest {
        database_url: request.database_url.clone(),
        source: CorpusSource::ChannelUrl {
            url: DISTINGUO_VIDEOS_URL.to_string(),
        },
        work_dir: request.work_dir,
        caption: request.caption,
        yt_dlp: request.yt_dlp,
        asr_enabled: request.asr_enabled,
        transcriber_command: None,
        transcriber_args: Vec::new(),
        max_items: Some(request.max_items),
        title_contains: None,
        title_excludes: Vec::new(),
        duration_min: None,
        duration_max: None,
        migrate: request.migrate,
    };

    let ingest_start = Instant::now();
    let ingest = ingest_corpus(ingest_request).await?;
    let ingest_elapsed_ms = ingest_start.elapsed().as_millis();

    let mut searches = Vec::new();
    let search_start = Instant::now();
    for query in request.search_queries {
        let item_start = Instant::now();
        let report = search_corpus(SearchRequest {
            database_url: request.database_url.clone(),
            query,
            top_k: request.top_k,
            mode: SearchMode::Hybrid,
            source_kind: None,
            video_id: None,
            language: None,
            transcript_start_min: None,
            transcript_start_max: None,
            upload_date_from: None,
            upload_date_to: None,
            duration_min: None,
            duration_max: None,
            channel_query: None,
            title_query: None,
            category_query: None,
            tag_query: None,
            metadata_query: None,
            view_count_min: None,
            view_count_max: None,
        })
        .await?;
        searches.push(TimedSearchReport {
            elapsed_ms: item_start.elapsed().as_millis(),
            report,
        });
    }

    Ok(BenchmarkReport {
        benchmark: "distinguo".to_string(),
        source_url: DISTINGUO_VIDEOS_URL.to_string(),
        max_items: request.max_items,
        ingest_elapsed_ms,
        search_elapsed_ms: search_start.elapsed().as_millis(),
        ingest,
        searches,
    })
}
