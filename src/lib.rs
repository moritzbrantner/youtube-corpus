pub mod api_surface;
pub mod api_types;
pub mod asr;
pub mod benchmark;
pub mod captions;
pub mod cli;
pub mod config;
pub mod corpora;
pub mod db;
pub mod diagnostics;
pub mod ingest;
pub mod reprocessing;
pub mod research_web;
pub mod search;
pub mod status;
pub mod subscriptions;
pub mod web;
pub mod youtube;
pub mod yt_dlp;

pub use config::{
    AppConfig, BrowserCookieSource, CaptionConfig, CorpusSource, SearchMode, SourceKind,
    YtDlpConfig,
};
pub use ingest::{ingest_corpus, IngestReport};
pub use reprocessing::{reprocess_video, ReprocessReport, ReprocessRequest, ReprocessStage};
pub use search::{search_corpus, SearchReport, SearchRequest};
pub use status::{corpus_status, list_videos, CorpusStatusReport, VideoStatus};
pub use subscriptions::{
    add_subscription, check_subscriptions, list_subscriptions, CheckSubscriptionsReport,
    Subscription,
};
