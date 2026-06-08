use std::net::IpAddr;
use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::benchmark::{BenchmarkRequest, DISTINGUO_SEARCH_QUERIES};
use crate::config::{AppConfig, CaptionConfig, CorpusSource, SearchMode, SourceKind, YtDlpConfig};
use crate::ingest::IngestRequest;
use crate::search::SearchRequest;
use crate::status::{CorpusStatusRequest, ListVideosRequest};
use crate::subscriptions::{
    AddSubscriptionRequest, CheckSubscriptionsRequest, SubscriptionSourceKind,
};

#[derive(Debug, Parser)]
#[command(
    name = "youtube-corpus",
    version,
    about = "Build and search a Postgres-backed YouTube transcript corpus"
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "URL")]
    pub database_url: Option<String>,
    #[arg(long, global = true, default_value = "127.0.0.1")]
    pub host: IpAddr,
    #[arg(long, global = true, default_value_t = 1420)]
    pub port: u16,
    #[arg(long, global = true)]
    pub no_open: bool,
    #[arg(long)]
    pub migrate: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Serve(ServeArgs),
    Migrate,
    Ingest(IngestArgs),
    Subscribe(SubscribeArgs),
    Subscriptions(SubscriptionsArgs),
    Videos(ListVideosArgs),
    Status(StatusArgs),
    Benchmark(BenchmarkArgs),
    Search(SearchArgs),
}

#[derive(Debug, Parser)]
pub struct ServeArgs {
    #[arg(long)]
    pub migrate: bool,
}

#[derive(Debug, Parser)]
#[command(group(
    ArgGroup::new("source")
        .args(["url", "playlist_url", "channel_url", "input"])
        .required(true)
))]
pub struct IngestArgs {
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub playlist_url: Option<String>,
    #[arg(long)]
    pub channel_url: Option<String>,
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long, default_value = "use-case-output/youtube-corpus")]
    pub work_dir: PathBuf,
    #[arg(long)]
    pub caption_language: Vec<String>,
    #[arg(long)]
    pub no_captions: bool,
    #[arg(long)]
    pub no_auto_captions: bool,
    #[arg(
        long = "yt-dlp-arg",
        value_name = "ARG",
        num_args = 1,
        allow_hyphen_values = true
    )]
    pub yt_dlp_args: Vec<String>,
    #[arg(long)]
    pub no_asr: bool,
    #[arg(long)]
    pub transcriber_command: Option<PathBuf>,
    #[arg(long = "transcriber-arg")]
    pub transcriber_args: Vec<String>,
    #[arg(long)]
    pub max_items: Option<u64>,
    #[arg(long)]
    pub title_contains: Option<String>,
    #[arg(long = "title-excludes")]
    pub title_excludes: Vec<String>,
    #[arg(long)]
    pub duration_min: Option<f64>,
    #[arg(long)]
    pub duration_max: Option<f64>,
    #[arg(long)]
    pub migrate: bool,
}

impl IngestArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<IngestRequest> {
        let source = if let Some(url) = self.url {
            CorpusSource::YoutubeUrl { url }
        } else if let Some(url) = self.playlist_url {
            CorpusSource::PlaylistUrl { url }
        } else if let Some(url) = self.channel_url {
            CorpusSource::ChannelUrl { url }
        } else if let Some(path) = self.input {
            CorpusSource::LocalFile { path }
        } else {
            anyhow::bail!("one input source is required");
        };
        let languages = if self.caption_language.is_empty() {
            vec!["en".to_string()]
        } else {
            self.caption_language
        };
        Ok(IngestRequest {
            database_url: config.database_url.clone(),
            source,
            work_dir: self.work_dir,
            caption: CaptionConfig {
                enabled: !self.no_captions,
                include_auto_captions: !self.no_auto_captions,
                languages,
            },
            yt_dlp: yt_dlp_config(self.yt_dlp_args),
            asr_enabled: !self.no_asr,
            transcriber_command: self.transcriber_command,
            transcriber_args: self.transcriber_args,
            max_items: self.max_items,
            title_contains: self.title_contains,
            title_excludes: self.title_excludes,
            duration_min: self.duration_min,
            duration_max: self.duration_max,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
#[command(group(
    ArgGroup::new("source")
        .args(["playlist_url", "channel_url"])
        .required(true)
))]
pub struct SubscribeArgs {
    #[arg(long)]
    pub playlist_url: Option<String>,
    #[arg(long)]
    pub channel_url: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long, default_value = "use-case-output/youtube-corpus")]
    pub work_dir: PathBuf,
    #[arg(long)]
    pub caption_language: Vec<String>,
    #[arg(long)]
    pub no_captions: bool,
    #[arg(long)]
    pub no_auto_captions: bool,
    #[arg(
        long = "yt-dlp-arg",
        value_name = "ARG",
        num_args = 1,
        allow_hyphen_values = true
    )]
    pub yt_dlp_args: Vec<String>,
    #[arg(long)]
    pub no_asr: bool,
    #[arg(long)]
    pub transcriber_command: Option<PathBuf>,
    #[arg(long = "transcriber-arg")]
    pub transcriber_args: Vec<String>,
    #[arg(long)]
    pub max_items: Option<u64>,
    #[arg(long)]
    pub title_contains: Option<String>,
    #[arg(long = "title-excludes")]
    pub title_excludes: Vec<String>,
    #[arg(long)]
    pub duration_min: Option<f64>,
    #[arg(long)]
    pub duration_max: Option<f64>,
    #[arg(long)]
    pub disabled: bool,
    #[arg(long)]
    pub migrate: bool,
}

impl SubscribeArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<AddSubscriptionRequest> {
        let (source_kind, source_url) = if let Some(url) = self.channel_url {
            (SubscriptionSourceKind::Channel, url)
        } else if let Some(url) = self.playlist_url {
            (SubscriptionSourceKind::Playlist, url)
        } else {
            anyhow::bail!("one subscription source is required");
        };
        let languages = if self.caption_language.is_empty() {
            vec!["en".to_string()]
        } else {
            self.caption_language
        };
        Ok(AddSubscriptionRequest {
            database_url: config.database_url.clone(),
            source_kind,
            source_url,
            name: self.name,
            enabled: !self.disabled,
            work_dir: self.work_dir,
            caption: CaptionConfig {
                enabled: !self.no_captions,
                include_auto_captions: !self.no_auto_captions,
                languages,
            },
            yt_dlp: yt_dlp_config(self.yt_dlp_args),
            asr_enabled: !self.no_asr,
            transcriber_command: self.transcriber_command,
            transcriber_args: self.transcriber_args,
            max_items: self.max_items,
            title_contains: self.title_contains,
            title_excludes: self.title_excludes,
            duration_min: self.duration_min,
            duration_max: self.duration_max,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
pub struct SubscriptionsArgs {
    #[command(subcommand)]
    pub command: SubscriptionsCommand,
}

#[derive(Debug, Subcommand)]
pub enum SubscriptionsCommand {
    Add(SubscribeArgs),
    List(ListSubscriptionsArgs),
    Check(CheckSubscriptionsArgs),
}

#[derive(Debug, Parser)]
pub struct ListSubscriptionsArgs {
    #[arg(long)]
    pub include_disabled: bool,
    #[arg(long)]
    pub migrate: bool,
}

#[derive(Debug, Parser)]
pub struct CheckSubscriptionsArgs {
    #[arg(long)]
    pub id: Option<Uuid>,
    #[arg(long)]
    pub include_disabled: bool,
    #[arg(long)]
    pub watch: bool,
    #[arg(long, default_value_t = 3600)]
    pub interval_seconds: u64,
    #[arg(long)]
    pub migrate: bool,
}

impl CheckSubscriptionsArgs {
    pub fn try_into_request(
        &self,
        config: &AppConfig,
    ) -> anyhow::Result<CheckSubscriptionsRequest> {
        if self.interval_seconds == 0 {
            anyhow::bail!("--interval-seconds must be positive");
        }
        Ok(CheckSubscriptionsRequest {
            database_url: config.database_url.clone(),
            id: self.id,
            include_disabled: self.include_disabled,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
pub struct ListVideosArgs {
    #[arg(long)]
    pub downloaded: bool,
    #[arg(long)]
    pub parsed: bool,
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub migrate: bool,
}

impl ListVideosArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<ListVideosRequest> {
        if matches!(self.limit, Some(limit) if limit <= 0) {
            anyhow::bail!("--limit must be positive");
        }
        Ok(ListVideosRequest {
            database_url: config.database_url.clone(),
            downloaded_only: self.downloaded,
            parsed_only: self.parsed,
            limit: self.limit,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
pub struct StatusArgs {
    #[arg(long)]
    pub include_disabled: bool,
    #[arg(long)]
    pub downloaded: bool,
    #[arg(long)]
    pub parsed: bool,
    #[arg(long)]
    pub limit: Option<i64>,
    #[arg(long)]
    pub migrate: bool,
}

impl StatusArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<CorpusStatusRequest> {
        if matches!(self.limit, Some(limit) if limit <= 0) {
            anyhow::bail!("--limit must be positive");
        }
        Ok(CorpusStatusRequest {
            database_url: config.database_url.clone(),
            include_disabled: self.include_disabled,
            downloaded_only: self.downloaded,
            parsed_only: self.parsed,
            limit: self.limit,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
pub struct BenchmarkArgs {
    #[arg(long, value_enum, default_value_t = BenchmarkPreset::Distinguo)]
    pub preset: BenchmarkPreset,
    #[arg(long, default_value_t = 3)]
    pub max_items: u64,
    #[arg(
        long,
        default_value = "use-case-output/youtube-corpus-benchmarks/distinguo"
    )]
    pub work_dir: PathBuf,
    #[arg(long)]
    pub caption_language: Vec<String>,
    #[arg(long)]
    pub no_captions: bool,
    #[arg(long)]
    pub no_auto_captions: bool,
    #[arg(
        long = "yt-dlp-arg",
        value_name = "ARG",
        num_args = 1,
        allow_hyphen_values = true
    )]
    pub yt_dlp_args: Vec<String>,
    #[arg(long)]
    pub with_asr: bool,
    #[arg(long)]
    pub migrate: bool,
    #[arg(long = "query")]
    pub search_queries: Vec<String>,
    #[arg(long, default_value_t = 5)]
    pub top_k: i64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BenchmarkPreset {
    Distinguo,
}

impl BenchmarkArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<BenchmarkRequest> {
        if self.max_items == 0 {
            anyhow::bail!("--max-items must be positive");
        }
        if self.top_k <= 0 {
            anyhow::bail!("--top-k must be positive");
        }
        let languages = if self.caption_language.is_empty() {
            vec!["en".to_string()]
        } else {
            self.caption_language
        };
        let search_queries = if self.search_queries.is_empty() {
            DISTINGUO_SEARCH_QUERIES
                .iter()
                .map(|query| (*query).to_string())
                .collect()
        } else {
            self.search_queries
        };
        match self.preset {
            BenchmarkPreset::Distinguo => Ok(BenchmarkRequest {
                database_url: config.database_url.clone(),
                work_dir: self.work_dir,
                max_items: self.max_items,
                caption: CaptionConfig {
                    enabled: !self.no_captions,
                    include_auto_captions: !self.no_auto_captions,
                    languages,
                },
                yt_dlp: yt_dlp_config(self.yt_dlp_args),
                asr_enabled: self.with_asr,
                migrate: self.migrate,
                search_queries,
                top_k: self.top_k,
            }),
        }
    }
}

fn yt_dlp_config(args: Vec<String>) -> YtDlpConfig {
    YtDlpConfig {
        args: args
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect(),
    }
}

#[derive(Debug, Parser)]
pub struct SearchArgs {
    #[arg(long)]
    pub query: String,
    #[arg(long, default_value_t = 10)]
    pub top_k: i64,
    #[arg(long, value_enum, default_value_t = SearchMode::Hybrid)]
    pub mode: SearchMode,
    #[arg(long, value_enum)]
    pub source_kind: Option<SourceKind>,
    #[arg(long)]
    pub video_id: Option<Uuid>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long)]
    pub transcript_start_min: Option<f64>,
    #[arg(long)]
    pub transcript_start_max: Option<f64>,
    #[arg(long)]
    pub upload_date_from: Option<String>,
    #[arg(long)]
    pub upload_date_to: Option<String>,
    #[arg(long)]
    pub duration_min: Option<f64>,
    #[arg(long)]
    pub duration_max: Option<f64>,
    #[arg(long)]
    pub channel: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub category: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub metadata: Option<String>,
    #[arg(long)]
    pub view_count_min: Option<i64>,
    #[arg(long)]
    pub view_count_max: Option<i64>,
}

impl SearchArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<SearchRequest> {
        if self.top_k <= 0 {
            anyhow::bail!("--top-k must be positive");
        }
        Ok(SearchRequest {
            database_url: config.database_url.clone(),
            query: self.query,
            top_k: self.top_k,
            mode: self.mode,
            source_kind: self.source_kind,
            video_id: self.video_id,
            language: self.language,
            transcript_start_min: self.transcript_start_min,
            transcript_start_max: self.transcript_start_max,
            upload_date_from: self.upload_date_from,
            upload_date_to: self.upload_date_to,
            duration_min: self.duration_min,
            duration_max: self.duration_max,
            channel_query: self.channel,
            title_query: self.title,
            category_query: self.category,
            tag_query: self.tag,
            metadata_query: self.metadata,
            view_count_min: self.view_count_min,
            view_count_max: self.view_count_max,
        })
    }
}
