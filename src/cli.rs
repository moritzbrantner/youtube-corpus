use std::ffi::OsString;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use clap::{ArgGroup, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::benchmark::{BenchmarkRequest, DISTINGUO_SEARCH_QUERIES};
use crate::config::{
    AppConfig, BrowserCookieSource, CaptionConfig, CorpusSource, SearchMode, SourceKind,
    YtDlpConfig,
};
use crate::ingest::IngestRequest;
use crate::search::SearchRequest;
use crate::status::{CorpusStatusRequest, ListVideosRequest};
use crate::subscriptions::{
    AddSubscriptionRequest, CheckSubscriptionsRequest, SubscriptionSourceKind,
};

pub const CONFIG_FILE_NAME: &str = "youtube-corpus.conf";

#[derive(Debug, Parser)]
#[command(
    name = "youtube-corpus",
    version,
    about = "Build and search a Postgres-backed YouTube transcript corpus",
    args_override_self = true
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
    #[arg(
        long = "yt-dlp-arg",
        global = true,
        value_name = "ARG",
        num_args = 1,
        allow_hyphen_values = true
    )]
    pub yt_dlp_args: Vec<String>,
    #[arg(long, global = true)]
    pub yt_dlp_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub yt_dlp_cookies_from_browser: Option<String>,
    #[arg(long, global = true)]
    pub yt_dlp_cookie_profile: Option<String>,
    #[arg(long, global = true)]
    pub yt_dlp_cookie_keyring: Option<String>,
    #[arg(long, global = true)]
    pub yt_dlp_cache_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub yt_dlp_user_agent: Option<String>,
    #[arg(long, global = true)]
    pub yt_dlp_sleep_requests_seconds: Option<f64>,
    #[arg(long, global = true)]
    pub yt_dlp_sleep_interval_seconds: Option<f64>,
    #[arg(long, global = true)]
    pub yt_dlp_max_sleep_interval_seconds: Option<f64>,
    #[arg(long, global = true)]
    pub yt_dlp_socket_timeout_seconds: Option<f64>,
    #[arg(long, global = true)]
    pub yt_dlp_retry_sleep: Option<String>,
    #[arg(long, global = true)]
    pub yt_dlp_retries: Option<u32>,
    #[arg(long, global = true)]
    pub yt_dlp_fragment_retries: Option<u32>,
    #[arg(long, global = true)]
    pub yt_dlp_format: Option<String>,
    #[arg(long)]
    pub migrate: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn parse_with_config_file() -> anyhow::Result<Self> {
        let args = args_with_config_file(std::env::args_os(), Path::new(CONFIG_FILE_NAME))?;
        Ok(Self::parse_from(args))
    }

    pub fn yt_dlp_config(&self) -> YtDlpConfig {
        yt_dlp_config(self)
    }
}

pub fn args_with_config_file<I, T>(args: I, config_path: &Path) -> anyhow::Result<Vec<OsString>>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let Some((program, runtime_args)) = args.split_first() else {
        return Ok(args);
    };
    if !config_path.exists() {
        return Ok(args);
    }
    let config = std::fs::read_to_string(config_path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", config_path.display()))?;
    let mut config_args = shlex::split(&config).ok_or_else(|| {
        anyhow::anyhow!(
            "failed to parse {}: unmatched quote or trailing escape",
            config_path.display()
        )
    })?;
    let mut merged = Vec::with_capacity(1 + config_args.len() + runtime_args.len());
    merged.push(program.clone());
    merged.extend(config_args.drain(..).map(OsString::from));
    merged.extend(runtime_args.iter().cloned());
    Ok(merged)
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
    /// Pins a citation: the segments overlapping `[start, end)` and their `span_sha256`.
    Span(SpanArgs),
    /// Opt-in Tantivy literal search pilot index (#76).
    SearchIndex(SearchIndexArgs),
    Diagnostics(DiagnosticsArgs),
    ApiSchema(ApiSchemaArgs),
    /// Writes a deterministic, versioned downstream export of the corpus.
    Export(ExportArgs),
    /// Extracts source-grounded entity mentions from transcripts and visible text.
    Entities(EntitiesArgs),
    /// Suggests unreviewed dated historical event claims from transcripts and visible text.
    Events(EventsArgs),
}

#[derive(Debug, Parser)]
pub struct EventsArgs {
    #[command(subcommand)]
    pub command: EventsCommand,
}

#[derive(Debug, Subcommand)]
pub enum EventsCommand {
    /// Suggests claims for every video of a corpus; reviewed claims are never changed.
    Extract(ExtractEventsArgs),
}

#[derive(Debug, Parser)]
pub struct ExtractEventsArgs {
    /// Corpus id or slug.
    #[arg(long)]
    pub corpus: String,
    #[arg(long)]
    pub migrate: bool,
}

#[derive(Debug, Parser)]
pub struct EntitiesArgs {
    #[command(subcommand)]
    pub command: EntitiesCommand,
}

#[derive(Debug, Subcommand)]
pub enum EntitiesCommand {
    /// Runs the canonical nlp-stack NER model over every video of a corpus.
    Extract(ExtractEntitiesArgs),
}

#[derive(Debug, Parser)]
pub struct ExtractEntitiesArgs {
    /// Corpus id or slug.
    #[arg(long)]
    pub corpus: String,
    /// Directory holding (or receiving) the bert-base-NER model bundle.
    #[arg(long, default_value = crate::entities::DEFAULT_MODEL_DIR)]
    pub model_dir: PathBuf,
    /// Fail instead of downloading a missing model bundle.
    #[arg(long)]
    pub no_download: bool,
    #[arg(long)]
    pub migrate: bool,
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct ServeArgs {
    #[arg(long)]
    pub migrate: bool,
    /// Opt-in Tantivy literal search pilot (#76): rebuild this local index from Postgres at
    /// launch and answer `/api/search` with it. Omit to keep the default Postgres search.
    #[arg(long)]
    pub tantivy_index_dir: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct SearchIndexArgs {
    #[command(subcommand)]
    pub command: SearchIndexCommand,
}

/// Opt-in Tantivy literal search pilot (#76). Postgres remains canonical and the default.
#[derive(Debug, Subcommand)]
pub enum SearchIndexCommand {
    /// Rebuilds the local index from Postgres (edits get new revisions, removals tombstones).
    Sync {
        #[arg(long)]
        index_dir: PathBuf,
    },
    /// Runs one query on both providers and reports shared/missing hits and latency.
    /// Requires `--tantivy-index-dir` and `--mode fts`.
    Compare(Box<SearchArgs>),
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
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
    #[arg(long)]
    pub no_asr: bool,
    #[arg(long)]
    pub transcriber_command: Option<PathBuf>,
    #[arg(long = "transcriber-arg")]
    pub transcriber_args: Vec<String>,
    #[arg(long)]
    pub transcriber_timeout_seconds: Option<u64>,
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
    pub fn try_into_request(
        self,
        config: &AppConfig,
        yt_dlp: YtDlpConfig,
    ) -> anyhow::Result<IngestRequest> {
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
        validate_positive_option("--max-items", self.max_items)?;
        validate_non_negative_range(
            "--duration-min",
            self.duration_min,
            "--duration-max",
            self.duration_max,
        )?;
        Ok(IngestRequest {
            run_id: None,
            database_url: config.database_url.clone(),
            source,
            work_dir: self.work_dir,
            caption: CaptionConfig {
                enabled: !self.no_captions,
                include_auto_captions: !self.no_auto_captions,
                languages,
            },
            yt_dlp,
            asr_enabled: !self.no_asr,
            transcriber_command: self.transcriber_command,
            transcriber_args: self.transcriber_args,
            transcriber_timeout_seconds: self.transcriber_timeout_seconds,
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
#[command(args_override_self = true)]
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
    #[arg(long)]
    pub no_asr: bool,
    #[arg(long)]
    pub transcriber_command: Option<PathBuf>,
    #[arg(long = "transcriber-arg")]
    pub transcriber_args: Vec<String>,
    #[arg(long)]
    pub transcriber_timeout_seconds: Option<u64>,
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
    /// Analyze new videos automatically, or hold them for review first.
    #[arg(long, value_enum, default_value_t = crate::subscriptions::ProcessingPolicy::Automatic)]
    pub processing_policy: crate::subscriptions::ProcessingPolicy,
    /// How often the running server checks this source (default: 6 hours).
    #[arg(long)]
    pub check_interval_seconds: Option<u64>,
    #[arg(long)]
    pub disabled: bool,
    #[arg(long)]
    pub migrate: bool,
}

impl SubscribeArgs {
    pub fn try_into_request(
        self,
        config: &AppConfig,
        yt_dlp: YtDlpConfig,
    ) -> anyhow::Result<AddSubscriptionRequest> {
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
        validate_positive_option("--max-items", self.max_items)?;
        validate_non_negative_range(
            "--duration-min",
            self.duration_min,
            "--duration-max",
            self.duration_max,
        )?;
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
            yt_dlp,
            asr_enabled: !self.no_asr,
            transcriber_command: self.transcriber_command,
            transcriber_args: self.transcriber_args,
            transcriber_timeout_seconds: self.transcriber_timeout_seconds,
            max_items: self.max_items,
            title_contains: self.title_contains,
            title_excludes: self.title_excludes,
            duration_min: self.duration_min,
            duration_max: self.duration_max,
            processing_policy: self.processing_policy,
            check_interval_seconds: self.check_interval_seconds,
            migrate: self.migrate,
        })
    }
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct SubscriptionsArgs {
    #[command(subcommand)]
    pub command: SubscriptionsCommand,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum SubscriptionsCommand {
    Add(SubscribeArgs),
    List(ListSubscriptionsArgs),
    Check(CheckSubscriptionsArgs),
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct ListSubscriptionsArgs {
    #[arg(long)]
    pub include_disabled: bool,
    #[arg(long)]
    pub migrate: bool,
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
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
            dispatch: Default::default(),
        })
    }
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
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
#[command(args_override_self = true)]
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
#[command(args_override_self = true)]
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
    #[arg(long)]
    pub with_asr: bool,
    #[arg(long)]
    pub transcriber_timeout_seconds: Option<u64>,
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
    pub fn try_into_request(
        self,
        config: &AppConfig,
        yt_dlp: YtDlpConfig,
    ) -> anyhow::Result<BenchmarkRequest> {
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
                yt_dlp,
                asr_enabled: self.with_asr,
                transcriber_timeout_seconds: self.transcriber_timeout_seconds,
                migrate: self.migrate,
                search_queries,
                top_k: self.top_k,
            }),
        }
    }
}

fn yt_dlp_config(cli: &Cli) -> YtDlpConfig {
    YtDlpConfig {
        args: cli
            .yt_dlp_args
            .clone()
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect(),
        timeout_seconds: cli.yt_dlp_timeout_seconds,
        cookies_from_browser: cli
            .yt_dlp_cookies_from_browser
            .as_deref()
            .map(str::trim)
            .filter(|browser| !browser.is_empty())
            .map(|browser| BrowserCookieSource {
                browser: browser.to_string(),
                profile: cli
                    .yt_dlp_cookie_profile
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                keyring: cli
                    .yt_dlp_cookie_keyring
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            }),
        cache_dir: cli.yt_dlp_cache_dir.clone(),
        user_agent: cli
            .yt_dlp_user_agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        sleep_requests_seconds: cli.yt_dlp_sleep_requests_seconds,
        sleep_interval_seconds: cli.yt_dlp_sleep_interval_seconds,
        max_sleep_interval_seconds: cli.yt_dlp_max_sleep_interval_seconds,
        socket_timeout_seconds: cli.yt_dlp_socket_timeout_seconds,
        retry_sleep: cli
            .yt_dlp_retry_sleep
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        retries: cli.yt_dlp_retries,
        fragment_retries: cli.yt_dlp_fragment_retries,
        format: cli
            .yt_dlp_format
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
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
    /// YouTube video id (for example `dQw4w9WgXcQ`), the alternative to the `--video-id` UUID.
    #[arg(long)]
    pub youtube_id: Option<String>,
    /// Corpus id or slug; hits and speaker labels are limited to this corpus.
    #[arg(long)]
    pub corpus: Option<String>,
    /// Raw diarization label of a stored speaker turn (for example `SPEAKER_00`), not an
    /// identity.
    #[arg(long)]
    pub speaker_label: Option<String>,
    /// Neighbouring transcript segments around each hit: a count per side (`2`) or seconds
    /// per side (`30s`).
    #[arg(long)]
    pub context: Option<crate::citations::ContextWindow>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
    /// Answer from this Tantivy pilot index (literal substring terms, `--mode fts` only)
    /// instead of Postgres. Run `search-index sync` first; the index is not refreshed here.
    #[arg(long)]
    pub tantivy_index_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Pretty JSON report.
    Json,
    /// `[mm:ss–mm:ss] speaker: text` lines per hit, the hit marked `>`.
    Text,
}

/// Prints the segments overlapping `[start, end)` of a video's transcript streams with the
/// `span_sha256` truth-core pins a `media_span` passage with.
#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct SpanArgs {
    #[arg(long)]
    pub youtube_id: String,
    #[arg(long)]
    pub start: f64,
    #[arg(long)]
    pub end: f64,
    /// Transcript stream UUID.
    #[arg(long)]
    pub stream: Option<Uuid>,
    #[arg(long, value_enum)]
    pub transcript_source: Option<SourceKind>,
    #[arg(long)]
    pub language: Option<String>,
    /// Revision of the producing code recorded in the citation (for example `git:<sha>`).
    #[arg(long)]
    pub producer_revision: String,
    /// Reads the corpus snapshot from this JSON file instead of Postgres.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

impl SearchArgs {
    pub fn try_into_request(self, config: &AppConfig) -> anyhow::Result<SearchRequest> {
        if self.top_k <= 0 {
            anyhow::bail!("--top-k must be positive");
        }
        validate_non_negative_range(
            "--transcript-start-min",
            self.transcript_start_min,
            "--transcript-start-max",
            self.transcript_start_max,
        )?;
        validate_non_negative_range(
            "--duration-min",
            self.duration_min,
            "--duration-max",
            self.duration_max,
        )?;
        validate_i64_range(
            "--view-count-min",
            self.view_count_min,
            "--view-count-max",
            self.view_count_max,
        )?;
        Ok(SearchRequest {
            database_url: config.database_url.clone(),
            query: self.query,
            top_k: self.top_k,
            mode: self.mode,
            source_kind: self.source_kind,
            modality: None,
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
            youtube_id: self.youtube_id,
            // Resolved from `--corpus` (an id or slug) against the database by the caller.
            corpus_id: None,
            speaker_label: self.speaker_label,
        })
    }
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct DiagnosticsArgs {
    #[arg(long)]
    pub transcriber_command: Option<String>,
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct ApiSchemaArgs {
    #[arg(long)]
    pub typescript: bool,
}

/// Downstream export formats (see `youtube_corpus::corpus_export`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    /// `source_span_interchange@1` source and span records.
    SourceSpan,
    /// `moenarch_corpus_core_batch@1` JSONL for philosophy-extractor.
    CorpusCore,
    /// `belief_evidence_interchange@1` evidence references for belief-lab.
    BeliefEvidence,
}

#[derive(Debug, Parser)]
#[command(args_override_self = true)]
pub struct ExportArgs {
    #[arg(long, value_enum)]
    pub format: ExportFormat,
    /// Revision of the producing code recorded in the export (for example `git:<sha>`).
    #[arg(long)]
    pub producer_revision: String,
    #[arg(long)]
    pub video_id: Option<Uuid>,
    /// Writes the export to this file instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Reads the corpus snapshot from this JSON file instead of Postgres.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    #[arg(long)]
    pub migrate: bool,
}

fn validate_positive_option(name: &str, value: Option<u64>) -> anyhow::Result<()> {
    if matches!(value, Some(0)) {
        anyhow::bail!("{name} must be positive");
    }
    Ok(())
}

fn validate_non_negative_range(
    min_name: &str,
    min: Option<f64>,
    max_name: &str,
    max: Option<f64>,
) -> anyhow::Result<()> {
    if matches!(min, Some(value) if value < 0.0) {
        anyhow::bail!("{min_name} must be non-negative");
    }
    if matches!(max, Some(value) if value < 0.0) {
        anyhow::bail!("{max_name} must be non-negative");
    }
    if let (Some(min), Some(max)) = (min, max) {
        if min > max {
            anyhow::bail!("{min_name} must be less than or equal to {max_name}");
        }
    }
    Ok(())
}

fn validate_i64_range(
    min_name: &str,
    min: Option<i64>,
    max_name: &str,
    max: Option<i64>,
) -> anyhow::Result<()> {
    if matches!(min, Some(value) if value < 0) {
        anyhow::bail!("{min_name} must be non-negative");
    }
    if matches!(max, Some(value) if value < 0) {
        anyhow::bail!("{max_name} must be non-negative");
    }
    if let (Some(min), Some(max)) = (min, max) {
        if min > max {
            anyhow::bail!("{min_name} must be less than or equal to {max_name}");
        }
    }
    Ok(())
}
