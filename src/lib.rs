pub mod asr;
pub mod benchmark;
pub mod captions;
pub mod cli;
pub mod config;
pub mod db;
pub mod ingest;
pub mod search;
pub mod status;
pub mod subscriptions;
pub mod youtube;

pub use config::{AppConfig, CaptionConfig, CorpusSource, SearchMode, SourceKind};
pub use ingest::{ingest_corpus, IngestReport};
pub use search::{search_corpus, SearchReport, SearchRequest};
pub use status::{corpus_status, list_videos, CorpusStatusReport, VideoStatus};
pub use subscriptions::{
    add_subscription, check_subscriptions, list_subscriptions, CheckSubscriptionsReport,
    Subscription,
};
