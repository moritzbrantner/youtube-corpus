use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
}

impl AppConfig {
    pub fn from_env_and_cli(database_url: Option<String>) -> anyhow::Result<Self> {
        let database_url = database_url
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required or pass --database-url"))?;
        Ok(Self { database_url })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CorpusSource {
    YoutubeUrl { url: String },
    PlaylistUrl { url: String },
    ChannelUrl { url: String },
    LocalFile { path: PathBuf },
}

impl CorpusSource {
    pub fn source_url(&self) -> String {
        match self {
            Self::YoutubeUrl { url } | Self::PlaylistUrl { url } | Self::ChannelUrl { url } => {
                url.clone()
            }
            Self::LocalFile { path } => path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionConfig {
    pub enabled: bool,
    pub include_auto_captions: bool,
    pub languages: Vec<String>,
}

impl Default for CaptionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_auto_captions: true,
            languages: vec!["en".to_string()],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpConfig {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub cookies_from_browser: Option<BrowserCookieSource>,
    #[serde(default)]
    pub cache_dir: Option<PathBuf>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub sleep_requests_seconds: Option<f64>,
    #[serde(default)]
    pub sleep_interval_seconds: Option<f64>,
    #[serde(default)]
    pub max_sleep_interval_seconds: Option<f64>,
    #[serde(default)]
    pub socket_timeout_seconds: Option<f64>,
    #[serde(default)]
    pub retry_sleep: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub fragment_retries: Option<u32>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCookieSource {
    pub browser: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub keyring: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    CaptionManual,
    CaptionAuto,
    Asr,
    VisualOcr,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CaptionManual => "caption_manual",
            Self::CaptionAuto => "caption_auto",
            Self::Asr => "asr",
            Self::VisualOcr => "visual_ocr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Hybrid,
    Fts,
    Semantic,
}
