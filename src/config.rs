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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    CaptionManual,
    CaptionAuto,
    Asr,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CaptionManual => "caption_manual",
            Self::CaptionAuto => "caption_auto",
            Self::Asr => "asr",
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
