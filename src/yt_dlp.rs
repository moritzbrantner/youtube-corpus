use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;

use crate::config::{BrowserCookieSource, YtDlpConfig};

type CommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CommandRunnerOutput, CommandRunnerError>> + Send + 'a>>;

#[derive(Clone)]
pub struct YtDlpClient {
    config: YtDlpConfig,
    runner: Arc<dyn CommandRunner>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtDlpCommandSpec {
    pub label: String,
    pub args: Vec<String>,
    pub timeout_seconds: Option<u64>,
    pub redact_args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YtDlpOperation {
    DiscoverCollection,
    FetchMetadata,
    DownloadCaptions,
    DownloadMedia,
    ProbeCookies,
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YtDlpErrorKind {
    MissingBinary,
    Timeout,
    AuthOrCookies,
    RateLimited,
    UnsupportedOrUnavailable,
    NoRequestedFormat,
    ParseJson,
    CommandFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtDlpRunReport {
    pub operation: YtDlpOperation,
    pub yt_dlp_version: Option<String>,
    pub args_redacted: Vec<String>,
    pub exit_status: Option<i32>,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRunnerOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<i32>,
}

#[derive(Debug, Error)]
pub enum CommandRunnerError {
    #[error("required command `yt-dlp` was not found on PATH")]
    MissingBinary,
    #[error("{label} timed out after {seconds} seconds")]
    Timeout { label: String, seconds: u64 },
    #[error("failed to run yt-dlp: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
#[error("{kind:?}: {message}")]
pub struct YtDlpError {
    pub kind: YtDlpErrorKind,
    pub message: String,
    pub report: YtDlpRunReport,
}

pub trait CommandRunner: Send + Sync {
    fn run<'a>(&'a self, spec: YtDlpCommandSpec) -> CommandFuture<'a>;
}

#[derive(Debug, Default)]
pub struct TokioCommandRunner;

impl CommandRunner for TokioCommandRunner {
    fn run<'a>(&'a self, spec: YtDlpCommandSpec) -> CommandFuture<'a> {
        Box::pin(async move {
            let mut command = Command::new("yt-dlp");
            command.args(&spec.args).stdin(Stdio::null());
            let output = command.output();
            let output = if let Some(seconds) = spec.timeout_seconds {
                tokio::time::timeout(Duration::from_secs(seconds), output)
                    .await
                    .map_err(|_| CommandRunnerError::Timeout {
                        label: spec.label.clone(),
                        seconds,
                    })?
            } else {
                output.await
            };
            match output {
                Ok(output) => Ok(CommandRunnerOutput {
                    stdout: output.stdout,
                    stderr: output.stderr,
                    exit_status: output.status.code(),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Err(CommandRunnerError::MissingBinary)
                }
                Err(error) => Err(CommandRunnerError::Io(error)),
            }
        })
    }
}

impl YtDlpClient {
    pub fn new(config: YtDlpConfig) -> Self {
        Self {
            config,
            runner: Arc::new(TokioCommandRunner),
        }
    }

    pub fn with_runner(config: YtDlpConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self { config, runner }
    }

    pub fn config(&self) -> &YtDlpConfig {
        &self.config
    }

    pub async fn version(&self) -> Result<(String, YtDlpRunReport), YtDlpError> {
        let (output, report) = self
            .run_spec(
                YtDlpOperation::Version,
                YtDlpCommandSpec {
                    label: "yt-dlp version".to_string(),
                    args: vec!["--version".to_string()],
                    timeout_seconds: self.config.timeout_seconds,
                    redact_args: Vec::new(),
                },
            )
            .await?;
        let version = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        Ok((version, report))
    }

    pub async fn discover_collection_json(
        &self,
        url: &str,
        max_items: Option<u64>,
    ) -> Result<(Vec<u8>, YtDlpRunReport), YtDlpError> {
        let mut args = vec!["--flat-playlist".to_string()];
        if let Some(max_items) = max_items.filter(|value| *value > 0) {
            args.push("--playlist-end".to_string());
            args.push(max_items.to_string());
        }
        args.extend(self.global_args().args);
        args.push("-J".to_string());
        args.push(url.to_string());
        self.run_args(
            YtDlpOperation::DiscoverCollection,
            "yt-dlp collection discovery",
            args,
            Vec::new(),
        )
        .await
    }

    pub async fn fetch_metadata_json(
        &self,
        url: &str,
    ) -> Result<(Vec<u8>, YtDlpRunReport), YtDlpError> {
        let mut args = vec![
            "--no-playlist".to_string(),
            "--skip-download".to_string(),
            "--dump-json".to_string(),
        ];
        args.extend(self.global_args().args);
        args.push(url.to_string());
        self.run_args(
            YtDlpOperation::FetchMetadata,
            "yt-dlp metadata download",
            args,
            Vec::new(),
        )
        .await
    }

    pub async fn download_captions(
        &self,
        url: &str,
        template: impl Into<PathBuf>,
        languages: &str,
        auto: bool,
    ) -> Result<YtDlpRunReport, YtDlpError> {
        let mut args = vec![
            "--skip-download".to_string(),
            "--no-playlist".to_string(),
            "--sub-format".to_string(),
            "vtt/srt/best".to_string(),
            "--sub-langs".to_string(),
            languages.to_string(),
            "-o".to_string(),
            template.into().to_string_lossy().into_owned(),
        ];
        if auto {
            args.push("--write-auto-subs".to_string());
        } else {
            args.push("--write-subs".to_string());
        }
        args.extend(self.global_args().args);
        args.push(url.to_string());
        self.run_args(
            YtDlpOperation::DownloadCaptions,
            "yt-dlp subtitle download",
            args,
            Vec::new(),
        )
        .await
        .map(|(_, report)| report)
    }

    pub async fn download_media(
        &self,
        url: &str,
        output_template: impl Into<PathBuf>,
        download_archive: Option<PathBuf>,
    ) -> Result<(Vec<u8>, YtDlpRunReport), YtDlpError> {
        let mut args = vec![
            "--no-playlist".to_string(),
            "--merge-output-format".to_string(),
            "mp4".to_string(),
            "--write-info-json".to_string(),
            "--print".to_string(),
            "after_move:filepath".to_string(),
            "-o".to_string(),
            output_template.into().to_string_lossy().into_owned(),
        ];
        if let Some(archive) = download_archive {
            args.push("--download-archive".to_string());
            args.push(archive.to_string_lossy().into_owned());
        }
        args.extend(self.global_args().args);
        args.push(url.to_string());
        self.run_args(
            YtDlpOperation::DownloadMedia,
            "yt-dlp media download",
            args,
            Vec::new(),
        )
        .await
    }

    pub async fn probe_cookies(&self, url: &str) -> Result<(Vec<u8>, YtDlpRunReport), YtDlpError> {
        let mut args = vec!["--simulate".to_string(), "--dump-json".to_string()];
        args.extend(self.global_args().args);
        args.push(url.to_string());
        self.run_args(
            YtDlpOperation::ProbeCookies,
            "yt-dlp cookie probe",
            args,
            Vec::new(),
        )
        .await
    }

    async fn run_args(
        &self,
        operation: YtDlpOperation,
        label: &str,
        args: Vec<String>,
        mut redact_args: Vec<String>,
    ) -> Result<(Vec<u8>, YtDlpRunReport), YtDlpError> {
        redact_args.extend(self.global_args().redact_args);
        let spec = YtDlpCommandSpec {
            label: label.to_string(),
            args,
            timeout_seconds: self.config.timeout_seconds,
            redact_args,
        };
        self.run_spec(operation, spec)
            .await
            .map(|(output, report)| (output.stdout, report))
    }

    async fn run_spec(
        &self,
        operation: YtDlpOperation,
        spec: YtDlpCommandSpec,
    ) -> Result<(CommandRunnerOutput, YtDlpRunReport), YtDlpError> {
        let args_redacted = redact_args(&spec.args, &spec.redact_args);
        let result = self.runner.run(spec).await;
        match result {
            Ok(output) if output.exit_status == Some(0) => {
                let report = YtDlpRunReport {
                    operation,
                    yt_dlp_version: None,
                    args_redacted,
                    exit_status: output.exit_status,
                    stderr_tail: stderr_tail(&output.stderr),
                };
                Ok((output, report))
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let report = YtDlpRunReport {
                    operation,
                    yt_dlp_version: None,
                    args_redacted,
                    exit_status: output.exit_status,
                    stderr_tail: stderr_tail(&output.stderr),
                };
                Err(YtDlpError {
                    kind: classify_failure(&stderr),
                    message: stderr,
                    report,
                })
            }
            Err(CommandRunnerError::MissingBinary) => Err(self.runner_error(
                operation,
                args_redacted,
                YtDlpErrorKind::MissingBinary,
                "required command `yt-dlp` was not found on PATH".to_string(),
            )),
            Err(CommandRunnerError::Timeout { label, seconds }) => Err(self.runner_error(
                operation,
                args_redacted,
                YtDlpErrorKind::Timeout,
                format!("{label} timed out after {seconds} seconds"),
            )),
            Err(CommandRunnerError::Io(error)) => Err(self.runner_error(
                operation,
                args_redacted,
                YtDlpErrorKind::CommandFailed,
                error.to_string(),
            )),
        }
    }

    fn runner_error(
        &self,
        operation: YtDlpOperation,
        args_redacted: Vec<String>,
        kind: YtDlpErrorKind,
        message: String,
    ) -> YtDlpError {
        YtDlpError {
            kind,
            message,
            report: YtDlpRunReport {
                operation,
                yt_dlp_version: None,
                args_redacted,
                exit_status: None,
                stderr_tail: String::new(),
            },
        }
    }

    fn global_args(&self) -> BuiltArgs {
        build_global_args(&self.config)
    }
}

#[derive(Debug, Default)]
struct BuiltArgs {
    args: Vec<String>,
    redact_args: Vec<String>,
}

fn build_global_args(config: &YtDlpConfig) -> BuiltArgs {
    let mut built = BuiltArgs::default();
    if let Some(source) = &config.cookies_from_browser {
        let value = browser_cookie_arg(source);
        built.args.push("--cookies-from-browser".to_string());
        built.redact_args.push(value.clone());
        built.args.push(value);
    }
    if let Some(cache_dir) = &config.cache_dir {
        built.args.push("--cache-dir".to_string());
        built.args.push(cache_dir.to_string_lossy().into_owned());
    }
    push_value(
        &mut built.args,
        "--user-agent",
        config.user_agent.as_deref(),
    );
    push_display(
        &mut built.args,
        "--sleep-requests",
        config.sleep_requests_seconds,
    );
    push_display(
        &mut built.args,
        "--sleep-interval",
        config.sleep_interval_seconds,
    );
    push_display(
        &mut built.args,
        "--max-sleep-interval",
        config.max_sleep_interval_seconds,
    );
    push_display(
        &mut built.args,
        "--socket-timeout",
        config.socket_timeout_seconds,
    );
    push_value(
        &mut built.args,
        "--retry-sleep",
        config.retry_sleep.as_deref(),
    );
    push_display(&mut built.args, "--retries", config.retries);
    push_display(
        &mut built.args,
        "--fragment-retries",
        config.fragment_retries,
    );
    push_value(&mut built.args, "--format", config.format.as_deref());
    built
        .args
        .extend(config.args.iter().filter_map(|arg| clean_arg(arg)));
    built
}

fn browser_cookie_arg(source: &BrowserCookieSource) -> String {
    let browser = source.browser.trim();
    let browser = if browser.is_empty() { "brave" } else { browser };
    let mut value = browser.to_string();
    if let Some(keyring) = source
        .keyring
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        value.push('+');
        value.push_str(keyring);
    }
    if let Some(profile) = source
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        value.push(':');
        value.push_str(profile);
    }
    value
}

fn push_value(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

fn push_display<T: ToString>(args: &mut Vec<String>, flag: &str, value: Option<T>) {
    if let Some(value) = value {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

fn clean_arg(arg: &str) -> Option<String> {
    let arg = arg.trim();
    (!arg.is_empty()).then(|| arg.to_string())
}

fn redact_args(args: &[String], explicit_redactions: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next || explicit_redactions.iter().any(|value| value == arg) {
            redacted.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        redacted.push(redact_single_arg(arg));
        redact_next = matches!(
            arg.as_str(),
            "--cookies" | "--cookies-from-browser" | "--proxy" | "--user-agent"
        );
    }
    redacted
}

fn redact_single_arg(arg: &str) -> String {
    if arg.contains("://") && arg.contains('@') {
        "[redacted-url]".to_string()
    } else {
        arg.to_string()
    }
}

pub fn classify_failure(stderr: &str) -> YtDlpErrorKind {
    let stderr = stderr.to_ascii_lowercase();
    if stderr.contains("sign in")
        || stderr.contains("cookies")
        || stderr.contains("login")
        || stderr.contains("private video")
        || stderr.contains("authentication")
        || stderr.contains("confirm your age")
    {
        YtDlpErrorKind::AuthOrCookies
    } else if stderr.contains("429")
        || stderr.contains("too many requests")
        || stderr.contains("rate-limit")
        || stderr.contains("rate limit")
    {
        YtDlpErrorKind::RateLimited
    } else if stderr.contains("requested format is not available")
        || stderr.contains("no video formats")
        || stderr.contains("no requested formats")
    {
        YtDlpErrorKind::NoRequestedFormat
    } else if stderr.contains("video unavailable")
        || stderr.contains("unavailable")
        || stderr.contains("unsupported url")
        || stderr.contains("not available")
        || stderr.contains("removed")
    {
        YtDlpErrorKind::UnsupportedOrUnavailable
    } else {
        YtDlpErrorKind::CommandFailed
    }
}

fn stderr_tail(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let lines = stderr.lines().rev().take(20).collect::<Vec<_>>();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<YtDlpCommandSpec>>,
        output: CommandRunnerOutput,
    }

    impl FakeRunner {
        fn new(output: CommandRunnerOutput) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                output,
            }
        }

        fn calls(&self) -> Vec<YtDlpCommandSpec> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run<'a>(&'a self, spec: YtDlpCommandSpec) -> CommandFuture<'a> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(spec);
                Ok(self.output.clone())
            })
        }
    }

    impl Default for CommandRunnerOutput {
        fn default() -> Self {
            Self {
                stdout: b"{}".to_vec(),
                stderr: Vec::new(),
                exit_status: Some(0),
            }
        }
    }

    #[tokio::test]
    async fn builds_metadata_args_with_typed_options_before_raw_args() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(
            YtDlpConfig {
                args: vec![
                    "--extractor-args".to_string(),
                    "youtube:player_client=ios".to_string(),
                ],
                timeout_seconds: Some(30),
                cookies_from_browser: Some(BrowserCookieSource {
                    browser: "brave".to_string(),
                    profile: Some("/home/me/Profile 1".to_string()),
                    keyring: None,
                }),
                retries: Some(4),
                fragment_retries: Some(5),
                sleep_interval_seconds: Some(1.5),
                max_sleep_interval_seconds: Some(4.0),
                format: Some("bv*+ba/b".to_string()),
                ..YtDlpConfig::default()
            },
            runner.clone(),
        );

        client
            .fetch_metadata_json("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            .await
            .unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].args,
            vec![
                "--no-playlist",
                "--skip-download",
                "--dump-json",
                "--cookies-from-browser",
                "brave:/home/me/Profile 1",
                "--sleep-interval",
                "1.5",
                "--max-sleep-interval",
                "4",
                "--retries",
                "4",
                "--fragment-retries",
                "5",
                "--format",
                "bv*+ba/b",
                "--extractor-args",
                "youtube:player_client=ios",
                "https://www.youtube.com/watch?v=jNQXAC9IVRw"
            ]
        );
        assert_eq!(calls[0].timeout_seconds, Some(30));
    }

    #[tokio::test]
    async fn builds_caption_args_for_manual_and_auto() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner.clone());

        client
            .download_captions(
                "https://www.youtube.com/watch?v=jNQXAC9IVRw",
                "/tmp/%(id)s.%(ext)s",
                "en,de",
                false,
            )
            .await
            .unwrap();
        client
            .download_captions(
                "https://www.youtube.com/watch?v=jNQXAC9IVRw",
                "/tmp/%(id)s.%(ext)s",
                "en,de",
                true,
            )
            .await
            .unwrap();

        let calls = runner.calls();
        assert!(calls[0].args.contains(&"--write-subs".to_string()));
        assert!(calls[1].args.contains(&"--write-auto-subs".to_string()));
        assert_eq!(calls[0].args[0], "--skip-download");
    }

    #[tokio::test]
    async fn builds_discovery_args_with_playlist_limit() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(YtDlpConfig::default(), runner.clone());

        client
            .discover_collection_json("https://www.youtube.com/@Distinguo/videos", Some(3))
            .await
            .unwrap();

        assert_eq!(
            runner.calls()[0].args,
            vec![
                "--flat-playlist",
                "--playlist-end",
                "3",
                "-J",
                "https://www.youtube.com/@Distinguo/videos"
            ]
        );
    }

    #[tokio::test]
    async fn builds_media_download_args_with_archive_and_after_move_print() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(
            YtDlpConfig {
                format: Some("ba".to_string()),
                ..YtDlpConfig::default()
            },
            runner.clone(),
        );

        client
            .download_media(
                "https://www.youtube.com/watch?v=jNQXAC9IVRw",
                "/tmp/%(id)s.%(ext)s",
                Some("/tmp/archive.txt".into()),
            )
            .await
            .unwrap();

        assert_eq!(
            runner.calls()[0].args,
            vec![
                "--no-playlist",
                "--merge-output-format",
                "mp4",
                "--write-info-json",
                "--print",
                "after_move:filepath",
                "-o",
                "/tmp/%(id)s.%(ext)s",
                "--download-archive",
                "/tmp/archive.txt",
                "--format",
                "ba",
                "https://www.youtube.com/watch?v=jNQXAC9IVRw"
            ]
        );
    }

    #[tokio::test]
    async fn builds_cookie_probe_args() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(
            YtDlpConfig {
                cookies_from_browser: Some(BrowserCookieSource {
                    browser: "brave".to_string(),
                    profile: None,
                    keyring: None,
                }),
                ..YtDlpConfig::default()
            },
            runner.clone(),
        );

        client
            .probe_cookies("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            .await
            .unwrap();

        assert_eq!(
            runner.calls()[0].args,
            vec![
                "--simulate",
                "--dump-json",
                "--cookies-from-browser",
                "brave",
                "https://www.youtube.com/watch?v=jNQXAC9IVRw"
            ]
        );
    }

    #[tokio::test]
    async fn redacts_cookie_browser_profile_in_reports() {
        let runner = Arc::new(FakeRunner::new(CommandRunnerOutput::default()));
        let client = YtDlpClient::with_runner(
            YtDlpConfig {
                cookies_from_browser: Some(BrowserCookieSource {
                    browser: "brave".to_string(),
                    profile: Some("/secret/profile".to_string()),
                    keyring: Some("kwallet".to_string()),
                }),
                ..YtDlpConfig::default()
            },
            runner,
        );

        let (_, report) = client
            .fetch_metadata_json("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            .await
            .unwrap();

        assert_eq!(
            report.args_redacted,
            vec![
                "--no-playlist",
                "--skip-download",
                "--dump-json",
                "--cookies-from-browser",
                "[redacted]",
                "https://www.youtube.com/watch?v=jNQXAC9IVRw"
            ]
        );
    }

    #[test]
    fn classifies_common_failures() {
        assert_eq!(
            classify_failure("ERROR: Sign in to confirm you're not a bot. Use --cookies"),
            YtDlpErrorKind::AuthOrCookies
        );
        assert_eq!(
            classify_failure("HTTP Error 429: Too Many Requests"),
            YtDlpErrorKind::RateLimited
        );
        assert_eq!(
            classify_failure("ERROR: requested format is not available"),
            YtDlpErrorKind::NoRequestedFormat
        );
        assert_eq!(
            classify_failure("ERROR: Video unavailable"),
            YtDlpErrorKind::UnsupportedOrUnavailable
        );
    }
}
