use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::config::{SearchMode, SourceKind};
use crate::ingest::IngestReport;
use crate::search::SearchResult;
use crate::status::VideoStatus;
use crate::subscriptions::Subscription;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStatus {
    pub configured: bool,
    pub database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTranscriptsInput {
    pub query: String,
    pub mode: SearchMode,
    pub top_k: i64,
    pub source_kind: Option<SourceKind>,
    pub video_id: Option<Uuid>,
    pub language: Option<String>,
    pub transcript_start_min: Option<f64>,
    pub transcript_start_max: Option<f64>,
    pub upload_date_from: Option<String>,
    pub upload_date_to: Option<String>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub channel_query: Option<String>,
    pub title_query: Option<String>,
    pub category_query: Option<String>,
    pub tag_query: Option<String>,
    pub metadata_query: Option<String>,
    pub view_count_min: Option<i64>,
    pub view_count_max: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptContextInput {
    pub segment_id: String,
    pub before: Option<i64>,
    pub after: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDownloadedFilesInput {
    pub downloaded_only: Option<bool>,
    pub parsed_only: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListIngestRunsInput {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisInput {
    pub source_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisReport {
    pub video: VideoStatus,
    pub streams: Vec<VideoAnalysisStream>,
    pub primary_stream_id: Option<Uuid>,
    pub segments: Vec<VideoAnalysisSegment>,
    pub lexical_analysis: Option<Value>,
    pub coverage: VideoAnalysisCoverage,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisStream {
    pub stream_id: Uuid,
    pub source_kind: SourceKind,
    pub language: Option<String>,
    pub status: String,
    pub segment_count: i64,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisSegment {
    pub segment_id: Uuid,
    pub stream_id: Uuid,
    pub segment_index: i64,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
    pub language: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysisCoverage {
    pub metadata: bool,
    pub transcript: bool,
    pub lexical: bool,
    pub media_retained: bool,
    pub visual_timeline: bool,
    pub audio_features: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSourceInput {
    pub source_kind: AddSourceKind,
    pub source_url: String,
    pub name: Option<String>,
    pub work_dir: Option<String>,
    pub caption_languages: Option<Vec<String>>,
    pub captions_enabled: Option<bool>,
    pub auto_captions_enabled: Option<bool>,
    pub yt_dlp_args: Option<Vec<String>>,
    pub asr_enabled: Option<bool>,
    pub transcriber_command: Option<String>,
    pub transcriber_args: Option<Vec<String>>,
    pub transcriber_timeout_seconds: Option<u64>,
    pub max_items: Option<u64>,
    pub title_contains: Option<String>,
    pub title_excludes: Option<Vec<String>>,
    pub duration_min: Option<f64>,
    pub duration_max: Option<f64>,
    pub migrate: Option<bool>,
    pub subscribe: Option<bool>,
    pub ingest_now: Option<bool>,
    #[serde(rename = "async")]
    pub async_ingest: Option<bool>,
    pub yt_dlp_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddSourceKind {
    Video,
    Channel,
    Playlist,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSourceReport {
    pub source_kind: String,
    pub source_url: String,
    pub subscription: Option<Subscription>,
    pub ingest: Option<IngestReport>,
    pub job_id: Option<Uuid>,
    pub ingest_run: Option<IngestRunStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestRunStatus {
    pub id: Uuid,
    pub source_url: Option<String>,
    pub status: String,
    pub videos_seen: i64,
    pub videos_indexed: i64,
    pub segments_indexed: i64,
    pub report: Value,
    pub created_at: String,
    pub job: Option<IngestJob>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestJob {
    pub id: jobs_core::JobId,
    pub status: jobs_core::JobStatus,
    pub progress: Option<jobs_core::JobProgress>,
    pub failure: Option<jobs_core::JobFailure>,
    pub ingest: Option<IngestReport>,
    pub source_url: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptContextReport {
    #[serde(rename = "match")]
    pub match_segment: SearchResult,
    pub segments: Vec<TranscriptContextSegment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptContextSegment {
    pub segment_id: Uuid,
    pub video_id: Uuid,
    pub stream_id: Uuid,
    pub segment_index: i64,
    pub source_kind: SourceKind,
    pub language: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
    pub is_match: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStatus {
    pub configured: bool,
    pub database_url: Option<String>,
    pub reachable: bool,
    pub schema_ready: bool,
    pub message: Option<String>,
    pub stats: Option<CorpusStats>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStats {
    pub videos: i64,
    pub streams: i64,
    pub segments: i64,
    pub subscriptions: i64,
    pub enabled_subscriptions: i64,
    pub source_kinds: Vec<SourceKindStat>,
    pub languages: Vec<LanguageStat>,
    pub last_ingest_run: Option<LastIngestRun>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceKindStat {
    pub source_kind: SourceKind,
    pub streams: i64,
    pub segments: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageStat {
    pub language: String,
    pub segments: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastIngestRun {
    pub id: String,
    pub source_url: Option<String>,
    pub status: String,
    pub videos_indexed: i64,
    pub segments_indexed: i64,
    pub created_at: String,
}

pub fn ingest_status_to_job_status(status: &str) -> Option<jobs_core::JobStatus> {
    match status {
        "running" => Some(jobs_core::JobStatus::Running),
        "completed" => Some(jobs_core::JobStatus::Succeeded),
        "failed" => Some(jobs_core::JobStatus::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ingest_status_to_job_status;

    #[test]
    fn maps_db_ingest_status_to_job_status() {
        assert_eq!(
            ingest_status_to_job_status("running"),
            Some(jobs_core::JobStatus::Running)
        );
        assert_eq!(
            ingest_status_to_job_status("completed"),
            Some(jobs_core::JobStatus::Succeeded)
        );
        assert_eq!(
            ingest_status_to_job_status("failed"),
            Some(jobs_core::JobStatus::Failed)
        );
        assert_eq!(ingest_status_to_job_status("succeeded"), None);
    }
}
