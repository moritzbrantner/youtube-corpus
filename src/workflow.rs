//! One reusable ingest-to-export workflow (#128).
//!
//! A JSON plan names a corpus, a list of YouTube videos and the stages to run. `process` takes
//! every video through metadata and caption ingest, joins it to the corpus, runs the corpus's
//! effective detector plan (which acquires only the media the stages need and deletes it once
//! they stored their output), and writes the corpus's citable exports. It composes the existing
//! corpus operations; it adds no processing of its own.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::cli::ExportFormat;
use crate::config::{CaptionConfig, CorpusSource, YtDlpConfig};
use crate::detectors::{
    DetectorSetting, MediaCleanup, PlanRunRequest, ScopeKind, StageOutcome, StageReport,
    TranscriberConfig,
};
use crate::ingest::IngestRequest;
use crate::media_extracts::{ExtractKind, ExtractRequest, ExtractSelection, SkippedExtract};

/// Licence text a video must state unless the plan sets `requireLicense` (empty disables it).
pub const DEFAULT_REQUIRED_LICENSE: &str = "Creative Commons";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPlan {
    /// Corpus slug; the corpus is created on first use.
    pub corpus: String,
    /// Display name for a newly created corpus (defaults to the slug).
    #[serde(default)]
    pub corpus_name: Option<String>,
    /// YouTube video URLs.
    pub videos: Vec<String>,
    /// The corpus's default detector selection; detectors left out are off.
    pub detectors: Vec<PlanDetector>,
    /// Case-insensitive text the video's stated YouTube licence must contain; videos without
    /// it are not ingested.
    #[serde(default = "default_required_license")]
    pub require_license: String,
    #[serde(default = "default_caption_languages")]
    pub caption_languages: Vec<String>,
    /// Transcriber for ASR of videos no tracked source configures one for.
    #[serde(default)]
    pub transcriber: Option<TranscriberConfig>,
    #[serde(default = "default_exports")]
    pub exports: Vec<ExportFormat>,
    /// Keep downloaded media instead of deleting it once every media stage stored output.
    #[serde(default)]
    pub keep_media: bool,
    /// Media extracts to cut (#116), from the media the stages acquired, before it is
    /// released.
    #[serde(default)]
    pub extracts: Vec<PlanExtract>,
}

/// A scene clip, audio excerpt and keyframes of a span (or of stored scenes) of a plan video.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanExtract {
    /// One of the plan's video URLs.
    pub video: String,
    #[serde(default)]
    pub start_seconds: Option<f64>,
    #[serde(default)]
    pub end_seconds: Option<f64>,
    #[serde(default)]
    pub scene_ids: Vec<Uuid>,
    #[serde(default = "default_extract_kinds")]
    pub kinds: Vec<ExtractKind>,
    #[serde(default = "default_extract_height")]
    pub max_height: u32,
}

fn default_extract_kinds() -> Vec<ExtractKind> {
    ExtractKind::ALL.to_vec()
}

fn default_extract_height() -> u32 {
    crate::media_acquisition::SCENES_MAX_HEIGHT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractOutcome {
    Completed,
    /// An identical extract was still intact on disk.
    Reused,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExtract {
    pub video: String,
    pub outcome: ExtractOutcome,
    pub manifest: Option<PathBuf>,
    pub manifest_sha256: Option<String>,
    pub artifacts: usize,
    pub skipped: Vec<SkippedExtract>,
    pub message: Option<String>,
}

/// A detector id, or a detector with target languages (`{"detector": "translate_spoken",
/// "targetLanguages": ["de"]}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlanDetector {
    Id(String),
    #[serde(rename_all = "camelCase")]
    WithLanguages {
        detector: String,
        #[serde(default)]
        target_languages: Vec<String>,
    },
}

impl PlanDetector {
    fn setting(&self) -> DetectorSetting {
        let (detector, target_languages) = match self {
            Self::Id(id) => (id.clone(), Vec::new()),
            Self::WithLanguages {
                detector,
                target_languages,
            } => (detector.clone(), target_languages.clone()),
        };
        DetectorSetting {
            detector,
            enabled: true,
            target_languages,
        }
    }
}

fn default_required_license() -> String {
    DEFAULT_REQUIRED_LICENSE.to_string()
}

fn default_caption_languages() -> Vec<String> {
    vec!["en".to_string()]
}

fn default_exports() -> Vec<ExportFormat> {
    vec![
        ExportFormat::SourceSpan,
        ExportFormat::CorpusCore,
        ExportFormat::BeliefEvidence,
    ]
}

impl WorkflowPlan {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read plan {}", path.display()))?;
        let plan: Self = serde_json::from_str(&json)
            .with_context(|| format!("failed to parse plan {}", path.display()))?;
        anyhow::ensure!(!plan.videos.is_empty(), "the plan lists no videos");
        for extract in &plan.extracts {
            anyhow::ensure!(
                plan.videos.contains(&extract.video),
                "extract video {} is not one of the plan's videos",
                extract.video
            );
        }
        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowRequest {
    pub database_url: String,
    pub plan: WorkflowPlan,
    pub out_dir: PathBuf,
    pub work_dir: PathBuf,
    pub model_dir: PathBuf,
    pub yt_dlp: YtDlpConfig,
    pub producer_revision: String,
    pub refresh: bool,
    pub migrate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReport {
    pub workflow: &'static str,
    pub corpus_id: Uuid,
    pub corpus: String,
    pub producer_revision: String,
    pub videos: Vec<WorkflowVideoReport>,
    pub exports: Vec<WorkflowExport>,
    /// Failed videos plus failed stages; 0 means every enabled, available stage succeeded.
    pub failures: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoOutcome {
    Processed,
    /// The video does not state the required licence; nothing was stored.
    LicenseRejected,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowVideoReport {
    pub source_url: String,
    pub video_id: Option<Uuid>,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub license: Option<String>,
    pub outcome: VideoOutcome,
    pub message: Option<String>,
    pub stages: Vec<StageReport>,
    pub media: MediaCleanup,
    /// Downloaded source media still on disk after the run.
    pub retained_media: Option<PathBuf>,
    /// The plan's media extracts of this video, cut before the media was released.
    pub extracts: Vec<WorkflowExtract>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowExport {
    pub format: ExportFormat,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// Runs the plan and writes its exports and `workflow-report.json` into `out_dir`.
pub async fn run_workflow(request: WorkflowRequest) -> anyhow::Result<WorkflowReport> {
    let pool = crate::db::connect(&request.database_url).await?;
    if request.migrate {
        crate::db::migrate(&pool).await?;
    }
    let plan = &request.plan;
    let corpus = match crate::corpora::list_corpora(&pool)
        .await?
        .into_iter()
        .find(|corpus| corpus.slug == plan.corpus)
    {
        Some(corpus) => corpus,
        None => {
            crate::corpora::create_corpus(
                &pool,
                crate::corpora::CreateCorpusRequest {
                    name: plan
                        .corpus_name
                        .clone()
                        .unwrap_or_else(|| plan.corpus.clone()),
                    slug: Some(plan.corpus.clone()),
                    description: None,
                },
            )
            .await?
        }
    };
    let settings: Vec<DetectorSetting> = plan.detectors.iter().map(PlanDetector::setting).collect();
    crate::detectors::set_scope_settings(&pool, corpus.id, ScopeKind::Corpus, corpus.id, &settings)
        .await
        .map_err(|error| anyhow::anyhow!("invalid plan detectors: {error}"))?;
    let captions = settings
        .iter()
        .any(|setting| setting.detector == "captions");
    tokio::fs::create_dir_all(&request.work_dir).await?;

    let mut videos = Vec::new();
    for url in &plan.videos {
        videos.push(process_video(&pool, &request, corpus.id, captions, url).await);
    }

    tokio::fs::create_dir_all(&request.out_dir).await?;
    let mut snapshot = crate::corpus_export::load_export_snapshot(&pool, None).await?;
    let members: BTreeSet<Uuid> = crate::corpora::corpus_video_ids(&pool, corpus.id)
        .await?
        .into_iter()
        .collect();
    snapshot.retain_videos(&members);
    let mut exports = Vec::new();
    for format in &plan.exports {
        let output = format.render(&snapshot, &request.producer_revision)?;
        let path = request.out_dir.join(format.file_name());
        tokio::fs::write(&path, output.as_bytes()).await?;
        exports.push(WorkflowExport {
            format: *format,
            path,
            bytes: output.len() as u64,
            sha256: Sha256::digest(output.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        });
    }

    let failures = videos
        .iter()
        .map(|video| {
            usize::from(video.outcome == VideoOutcome::Failed)
                + video
                    .stages
                    .iter()
                    .filter(|stage| stage.outcome == StageOutcome::Failed)
                    .count()
                + video
                    .extracts
                    .iter()
                    .filter(|extract| extract.outcome == ExtractOutcome::Failed)
                    .count()
        })
        .sum();
    let report = WorkflowReport {
        workflow: "youtube_corpus_process",
        corpus_id: corpus.id,
        corpus: corpus.slug,
        producer_revision: request.producer_revision.clone(),
        videos,
        exports,
        failures,
    };
    tokio::fs::write(
        request.out_dir.join("workflow-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )
    .await?;
    Ok(report)
}

async fn process_video(
    pool: &sqlx::PgPool,
    request: &WorkflowRequest,
    corpus_id: Uuid,
    captions: bool,
    url: &str,
) -> WorkflowVideoReport {
    let mut report = WorkflowVideoReport {
        source_url: url.to_string(),
        video_id: None,
        youtube_id: None,
        title: None,
        channel: None,
        license: None,
        outcome: VideoOutcome::Failed,
        message: None,
        stages: Vec::new(),
        media: MediaCleanup::default(),
        retained_media: None,
        extracts: Vec::new(),
    };
    if let Err(error) = process_video_inner(pool, request, corpus_id, captions, &mut report).await {
        report.outcome = VideoOutcome::Failed;
        report.message = Some(format!("{error:#}"));
    }
    report
}

async fn process_video_inner(
    pool: &sqlx::PgPool,
    request: &WorkflowRequest,
    corpus_id: Uuid,
    captions: bool,
    report: &mut WorkflowVideoReport,
) -> anyhow::Result<()> {
    let plan = &request.plan;
    let video_id = crate::youtube::stable_video_id(&report.source_url);
    // A video this corpus already holds with its licence is not ingested again (unless
    // refreshing); its plan run below reuses every stored stage.
    let stored = if request.refresh
        || !crate::corpora::corpus_video_ids(pool, corpus_id)
            .await?
            .contains(&video_id)
    {
        None
    } else {
        sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT youtube_id, title, channel, metadata->>'license' FROM videos
             WHERE id = $1 AND metadata->>'license' IS NOT NULL",
        )
        .bind(video_id)
        .fetch_optional(pool)
        .await?
    };
    if let Some((youtube_id, title, channel, license)) = stored {
        report.youtube_id = youtube_id;
        report.title = title;
        report.channel = channel;
        report.license = license;
        if !license_allowed(&plan.require_license, report.license.as_deref()) {
            reject_license(report, &plan.require_license);
            return Ok(());
        }
        report.video_id = Some(video_id);
    } else {
        ingest_video(pool, request, corpus_id, captions, report).await?;
        if report.outcome == VideoOutcome::LicenseRejected {
            return Ok(());
        }
        report.video_id = Some(video_id);
        if captions {
            // Ingest just tried captions; report that attempt instead of asking YouTube again.
            let status = crate::detector_status::video_status(pool, corpus_id, video_id).await?;
            if let Some(stage) = status.stages.iter().find(|s| s.detector == "captions") {
                use crate::detector_status::StageState;
                report.stages.push(StageReport {
                    detector: "captions".to_string(),
                    outcome: match stage.state {
                        StageState::Completed => StageOutcome::Completed,
                        StageState::Failed | StageState::Interrupted => StageOutcome::Failed,
                        _ => StageOutcome::Skipped,
                    },
                    message: stage.message.clone(),
                });
            }
        }
    }
    let ingested_captions = !report.stages.is_empty();
    let detectors = if ingested_captions {
        let plan = crate::detectors::video_plan(pool, corpus_id, video_id)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Some(
            plan.detectors
                .into_iter()
                .filter(|planned| planned.enabled && planned.detector != "captions")
                .map(|planned| planned.detector)
                .collect(),
        )
    } else {
        None
    };

    let extracts: Vec<&PlanExtract> = plan
        .extracts
        .iter()
        .filter(|extract| crate::youtube::stable_video_id(&extract.video) == video_id)
        .collect();
    let run_request = PlanRunRequest {
        database_url: request.database_url.clone(),
        corpus_id,
        video_id,
        work_dir: request.work_dir.clone(),
        yt_dlp: request.yt_dlp.clone(),
        model_dir: request.model_dir.clone(),
        refresh: request.refresh,
        detectors,
        translation_provider: None,
        keep_media: plan.keep_media,
        transcriber: plan.transcriber.clone(),
    };
    // Extracts are cut from the media the stages used, before it is released (#116).
    let run = crate::detectors::run_video_plan(PlanRunRequest {
        keep_media: plan.keep_media || !extracts.is_empty(),
        ..run_request.clone()
    })
    .await
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    report.stages.extend(run.stages);
    report.media = run.media;
    if !extracts.is_empty() {
        let client = crate::yt_dlp::YtDlpClient::new(request.yt_dlp.clone());
        for extract in extracts {
            report
                .extracts
                .push(run_extract(pool, &client, request, video_id, extract).await);
        }
        report.media = crate::detectors::release_processed_media(pool, &run_request).await?;
    }
    let local: Option<String> =
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id = $1")
            .bind(video_id)
            .fetch_one(pool)
            .await?;
    report.retained_media = local.map(PathBuf::from).filter(|path| path.exists());
    report.outcome = VideoOutcome::Processed;
    Ok(())
}

async fn run_extract(
    pool: &sqlx::PgPool,
    client: &crate::yt_dlp::YtDlpClient,
    request: &WorkflowRequest,
    video_id: Uuid,
    extract: &PlanExtract,
) -> WorkflowExtract {
    let mut extract_request = ExtractRequest::new(
        video_id,
        ExtractSelection {
            start_seconds: extract.start_seconds,
            end_seconds: extract.end_seconds,
            scene_ids: extract.scene_ids.clone(),
        },
        request.out_dir.join("extracts"),
    );
    extract_request.kinds = extract.kinds.iter().copied().collect();
    extract_request.max_height = extract.max_height;
    extract_request.work_dir = request.work_dir.clone();
    extract_request.required_license = request.plan.require_license.clone();
    extract_request.producer_revision = request.producer_revision.clone();
    extract_request.keep_media = true;
    match crate::media_extracts::extract_media(pool, client, &extract_request).await {
        Ok(report) => WorkflowExtract {
            video: extract.video.clone(),
            outcome: if report.reused {
                ExtractOutcome::Reused
            } else {
                ExtractOutcome::Completed
            },
            manifest: Some(report.manifest_path),
            manifest_sha256: Some(report.manifest_sha256),
            artifacts: report.manifest.artifacts.len(),
            skipped: report.manifest.skipped,
            message: None,
        },
        Err(error) => WorkflowExtract {
            video: extract.video.clone(),
            outcome: ExtractOutcome::Failed,
            manifest: None,
            manifest_sha256: None,
            artifacts: 0,
            skipped: Vec::new(),
            message: Some(format!("{error:#}")),
        },
    }
}

/// Whether `license` satisfies `required` (case-insensitive containment; empty allows all).
pub fn license_allowed(required: &str, license: Option<&str>) -> bool {
    let required = required.trim().to_lowercase();
    required.is_empty() || license.is_some_and(|license| license.to_lowercase().contains(&required))
}

fn reject_license(report: &mut WorkflowVideoReport, required: &str) {
    report.outcome = VideoOutcome::LicenseRejected;
    report.message = Some(format!(
        "the video's licence ({}) does not contain \"{required}\"",
        report.license.as_deref().unwrap_or("none stated"),
    ));
}

/// Downloads metadata, checks the licence, then ingests metadata and captions and joins the
/// video to the corpus. Media stages run from the corpus plan afterwards, so one acquisition
/// serves all of them.
async fn ingest_video(
    pool: &sqlx::PgPool,
    request: &WorkflowRequest,
    corpus_id: Uuid,
    captions: bool,
    report: &mut WorkflowVideoReport,
) -> anyhow::Result<()> {
    let plan = &request.plan;
    // Metadata first: the licence decides whether anything is stored.
    let mut item = crate::youtube::single_url_item(report.source_url.clone());
    let metadata_dir = request.work_dir.join(&item.item_id).join("metadata");
    crate::youtube::enrich_video_metadata(&mut item, &metadata_dir, &request.yt_dlp)
        .await
        .context("metadata download failed")?;
    report.youtube_id = item.youtube_id.clone();
    report.title = item.title.clone();
    report.channel = item.metadata.channel.clone();
    report.license = item.metadata.license.clone();
    if !license_allowed(&plan.require_license, report.license.as_deref()) {
        reject_license(report, &plan.require_license);
        return Ok(());
    }
    let ingest = crate::ingest::ingest_video_items(
        IngestRequest {
            run_id: None,
            database_url: request.database_url.clone(),
            source: CorpusSource::YoutubeUrl {
                url: report.source_url.clone(),
            },
            work_dir: request.work_dir.clone(),
            caption: CaptionConfig {
                enabled: captions,
                include_auto_captions: true,
                languages: plan.caption_languages.clone(),
            },
            yt_dlp: request.yt_dlp.clone(),
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
            keep_media: plan.keep_media,
        },
        vec![item],
    )
    .await?;
    let item = ingest
        .items
        .into_iter()
        .next()
        .context("ingest returned no item")?;
    let video_id = item
        .video_id
        .with_context(|| item.message.unwrap_or_else(|| "ingest failed".to_string()))?;
    crate::corpora::link_video(pool, corpus_id, video_id).await?;
    Ok(())
}
