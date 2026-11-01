//! Spoken-word transcription for corpus videos (#73).
//!
//! Default ASR goes through the pinned native-whisperx library (the audio/ASR capability that
//! owns Whisper decoding), never through a command resolved from `PATH`. One process-wide
//! [`TranscriptionSession`] keeps the Whisper model loaded across videos. With the `cuda` build
//! feature the model runs on the GPU when one is present, and a failed GPU run is retried on the
//! CPU. An external Whisper-compatible command runs only when a source configures one explicitly.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Context;
use native_whisperx::{
    AlignmentConfig, AsrConfig, DevicePreference, DiarizationConfig, InputSource,
    NativeWhisperxConfig, NativeWhisperxError, NativeWhisperxReport, OutputConfig,
    TranscriptionSession, TranslationConfig, VadConfig,
};
use serde_json::{json, Value};

use crate::captions::TranscriptStream;
use crate::config::SourceKind;
use crate::translation::NATIVE_WHISPERX_REVISION;

/// Whisper checkpoint of the default native provider (multilingual).
pub const NATIVE_ASR_MODEL: &str = "small";

/// The language native ASR decodes in: the source's first requested caption language
/// (`en-US` → `en`). Native language detection returns punctuation-only transcripts at the
/// pinned revision (native-whisperx#434), so the corpus always passes a language.
pub fn native_language(caption_languages: &[String]) -> Option<String> {
    caption_languages
        .iter()
        .map(|language| language.split(['-', '_']).next().unwrap_or("").trim())
        .find(|language| !language.is_empty())
        .map(str::to_ascii_lowercase)
}

/// A transcript plus the producer facts recorded in the stream's provenance.
#[derive(Debug)]
pub struct AsrOutput {
    pub stream: TranscriptStream,
    /// What ran: provider, exact revision, model, device, and whether the CPU fallback was used.
    pub run: Value,
}

/// Stable identity of the ASR producer a request selects. Stored with every ASR stream and
/// compared on reuse, so a changed revision, model or command invalidates earlier output.
pub fn producer_identity(
    command: Option<&Path>,
    caption_languages: &[String],
) -> anyhow::Result<Value> {
    Ok(match command {
        Some(command) => json!({
            "provider": "command",
            "executable": crate::detector_status::file_identity(command)?,
        }),
        None => json!({
            "provider": "native-whisperx",
            "revision": NATIVE_WHISPERX_REVISION,
            "config": native_config_identity(native_language(caption_languages).as_deref()),
        }),
    })
}

fn native_config_identity(language: Option<&str>) -> Value {
    json!({ "modelId": NATIVE_ASR_MODEL, "language": language, "alignment": false,
        "vad": "energy", "diarization": false })
}

/// Transcribes `media_path` with the explicitly configured command, else the native provider.
pub async fn transcribe(
    media_path: &Path,
    work_dir: &Path,
    command: Option<&PathBuf>,
    args: &[String],
    timeout_seconds: Option<u64>,
    caption_languages: &[String],
) -> anyhow::Result<AsrOutput> {
    let output_dir = work_dir.join("asr");
    tokio::fs::create_dir_all(&output_dir).await?;
    match command {
        Some(command) => {
            transcribe_with_command(media_path, &output_dir, command, args, timeout_seconds).await
        }
        None => {
            let (reply, answer) = tokio::sync::oneshot::channel();
            NATIVE_WORKER
                .send(NativeJob {
                    media_path: media_path.to_path_buf(),
                    output_dir,
                    language: native_language(caption_languages),
                    reply,
                })
                .context("native ASR worker stopped")?;
            answer.await.context("native ASR worker stopped")?
        }
    }
}

struct NativeJob {
    media_path: PathBuf,
    output_dir: PathBuf,
    language: Option<String>,
    reply: tokio::sync::oneshot::Sender<anyhow::Result<AsrOutput>>,
}

/// One worker thread owns the process's [`TranscriptionSession`] (it is not `Send`,
/// native-whisperx#432), so the
/// Whisper model stays loaded between videos and jobs run one at a time.
static NATIVE_WORKER: LazyLock<std::sync::mpsc::Sender<NativeJob>> = LazyLock::new(|| {
    let (jobs, queue) = std::sync::mpsc::channel::<NativeJob>();
    std::thread::Builder::new()
        .name("native-asr".to_string())
        .spawn(move || {
            let mut session = TranscriptionSession::new();
            for job in queue {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    transcribe_native(
                        &mut session,
                        &job.media_path,
                        &job.output_dir,
                        job.language.as_deref(),
                        cfg!(feature = "cuda"),
                    )
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("native-whisperx panicked")));
                let _ = job.reply.send(result);
            }
        })
        .expect("start the native ASR worker thread");
    jobs
});

/// The native ASR seam: the real [`TranscriptionSession`] in production, a fake in tests.
pub trait NativeAsrBackend {
    fn run(
        &mut self,
        config: NativeWhisperxConfig,
    ) -> Result<NativeWhisperxReport, NativeWhisperxError>;
}

impl NativeAsrBackend for TranscriptionSession {
    fn run(
        &mut self,
        config: NativeWhisperxConfig,
    ) -> Result<NativeWhisperxReport, NativeWhisperxError> {
        TranscriptionSession::run(self, config)
    }
}

/// Runs the native provider. `gpu_capable` (the `cuda` build) lets the first attempt pick the
/// GPU automatically; if that attempt fails the video is transcribed again on the CPU.
pub fn transcribe_native(
    backend: &mut dyn NativeAsrBackend,
    media_path: &Path,
    output_dir: &Path,
    language: Option<&str>,
    gpu_capable: bool,
) -> anyhow::Result<AsrOutput> {
    let first = if gpu_capable {
        DevicePreference::Auto
    } else {
        DevicePreference::Cpu
    };
    let config = |device| native_config(media_path, output_dir, language, device);
    let (report, gpu_error) = match backend.run(config(first)) {
        Ok(report) => (report, None),
        // A configuration error fails the same way on the CPU.
        Err(error) if gpu_capable && !matches!(error, NativeWhisperxError::InvalidConfig(_)) => {
            let report = backend.run(config(DevicePreference::Cpu)).map_err(|cpu| {
                anyhow::anyhow!(
                    "native-whisperx failed on the GPU ({error}) and on the CPU ({cpu})"
                )
            })?;
            (report, Some(error.to_string()))
        }
        Err(error) => return Err(anyhow::anyhow!("native-whisperx failed: {error}")),
    };

    let json_path = whisper_json_path(media_path, output_dir)?;
    let whisper_json = native_whisperx::whisperx_json_value(&report.transcript, false);
    std::fs::write(&json_path, serde_json::to_vec_pretty(&whisper_json)?)?;
    let mut parsed = text_transcripts::parse_whisper_json(&serde_json::to_vec(&whisper_json)?)?;
    parsed.source = Some(json_path.to_string_lossy().into_owned());

    let provenance = &report.provenance;
    let device = provenance.device.clone();
    let mut message = format!(
        "ASR generated by native-whisperx {} ({} on {})",
        &NATIVE_WHISPERX_REVISION[..7],
        provenance.model_id,
        device.as_deref().unwrap_or("unreported device"),
    );
    if let Some(error) = &gpu_error {
        message.push_str(&format!("; GPU run failed, used the CPU: {error}"));
    }
    Ok(AsrOutput {
        stream: TranscriptStream {
            source_kind: SourceKind::Asr,
            language: parsed.language.clone(),
            source_path: Some(json_path),
            text: parsed.text.clone(),
            segments: parsed.segments.into_iter().map(Into::into).collect(),
            source_cues: Vec::new(),
            message: Some(message),
        },
        run: json!({
            "provider": "native-whisperx",
            "revision": NATIVE_WHISPERX_REVISION,
            "executionProvider": provenance.provider,
            "modelId": provenance.model_id,
            "device": device,
            "cpuFallback": gpu_error,
            "config": native_config_identity(language),
        }),
    })
}

fn native_config(
    media_path: &Path,
    output_dir: &Path,
    language: Option<&str>,
    device: DevicePreference,
) -> NativeWhisperxConfig {
    NativeWhisperxConfig {
        input: InputSource::Path {
            path: media_path.to_path_buf(),
        },
        asr: AsrConfig {
            model_id: NATIVE_ASR_MODEL.to_string(),
            language: language.map(str::to_string),
            device,
            ..AsrConfig::default()
        },
        translation: TranslationConfig::default(),
        vad: VadConfig::default(),
        // Corpus segments are retrieval passages; word alignment is not used.
        alignment: AlignmentConfig {
            enabled: false,
            ..AlignmentConfig::default()
        },
        diarization: DiarizationConfig::default(),
        // native-whisperx's own JSON report stays beside the corpus' Whisper JSON as evidence.
        output: OutputConfig {
            output_dir: Some(output_dir.join("native-whisperx")),
            ..OutputConfig::default()
        },
    }
}

/// The explicit compatibility provider: a Whisper-CLI-compatible command.
async fn transcribe_with_command(
    media_path: &Path,
    output_dir: &Path,
    command: &Path,
    args: &[String],
    timeout_seconds: Option<u64>,
) -> anyhow::Result<AsrOutput> {
    let output_path = whisper_json_path(media_path, output_dir)?;
    if tokio::fs::try_exists(&output_path).await? {
        tokio::fs::remove_file(&output_path).await?;
    }

    run_whisper_command(command, media_path, output_dir, args, timeout_seconds).await?;

    let bytes = tokio::fs::read(&output_path).await.with_context(|| {
        format!(
            "transcriber succeeded but did not create expected Whisper JSON at {}",
            output_path.display()
        )
    })?;
    let mut parsed = text_transcripts::parse_whisper_json(&bytes)?;
    parsed.source = Some(output_path.to_string_lossy().into_owned());

    Ok(AsrOutput {
        stream: TranscriptStream {
            source_kind: SourceKind::Asr,
            language: parsed.language.clone(),
            source_path: parsed.source.as_deref().map(PathBuf::from),
            text: parsed.text.clone(),
            segments: parsed.segments.into_iter().map(Into::into).collect(),
            source_cues: Vec::new(),
            message: Some(format!("ASR generated by {}", command.display())),
        },
        run: json!({ "provider": "command", "command": command, "args": args }),
    })
}

async fn run_whisper_command(
    command: &Path,
    media_path: &Path,
    output_dir: &Path,
    args: &[String],
    timeout_seconds: Option<u64>,
) -> anyhow::Result<()> {
    let mut process = tokio::process::Command::new(command);
    process
        .arg(media_path)
        .args(args)
        .arg("--output_dir")
        .arg(output_dir)
        .arg("--output_format")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = process
        .spawn()
        .with_context(|| format!("failed to start transcriber {}", command.display()))?;
    let output = if let Some(timeout_seconds) = timeout_seconds {
        tokio::time::timeout(
            Duration::from_secs(timeout_seconds),
            child.wait_with_output(),
        )
        .await
        .with_context(|| format!("transcriber timed out after {timeout_seconds} seconds"))??
    } else {
        child.wait_with_output().await?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "transcriber {} exited with {}: {}",
            command.display(),
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

fn whisper_json_path(media_path: &Path, output_dir: &Path) -> anyhow::Result<PathBuf> {
    let file_stem = media_path.file_stem().with_context(|| {
        format!(
            "cannot derive Whisper output name from media path {}",
            media_path.display()
        )
    })?;
    let mut output_name = file_stem.to_os_string();
    output_name.push(".json");
    Ok(output_dir.join(output_name))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use native_whisperx::{
        DevicePreference, NativeWhisperxConfig, NativeWhisperxError, NativeWhisperxReport,
    };

    use super::{producer_identity, transcribe_native, whisper_json_path, NativeAsrBackend};

    /// Fails on anything but the CPU when `gpu_fails`, and records every requested device.
    struct FakeBackend {
        gpu_fails: bool,
        devices: Vec<DevicePreference>,
    }

    impl NativeAsrBackend for FakeBackend {
        fn run(
            &mut self,
            config: NativeWhisperxConfig,
        ) -> Result<NativeWhisperxReport, NativeWhisperxError> {
            self.devices.push(config.asr.device);
            assert_eq!(config.asr.model_id, "small");
            assert!(!config.alignment.enabled);
            assert_eq!(config.asr.language.as_deref(), Some("en"));
            if self.gpu_fails && config.asr.device != DevicePreference::Cpu {
                return Err(NativeWhisperxError::Transcription(
                    "CUDA out of memory".to_string(),
                ));
            }
            let device = if config.asr.device == DevicePreference::Cpu {
                "cpu"
            } else {
                "cuda:0"
            };
            Ok(serde_json::from_value(serde_json::json!({
                "transcript": {
                    "language": "en",
                    "text": "Hello corpus. Music under speech.",
                    "segments": [
                        {"index": 0, "startSeconds": 0.0, "endSeconds": 1.5,
                         "text": "Hello corpus.", "isFinal": true},
                        {"index": 1, "startSeconds": 1.5, "endSeconds": 3.0,
                         "text": "Music under speech.", "isFinal": true}
                    ]
                },
                "provenance": {"provider": "candle-whisper", "modelId": "small", "device": device}
            }))
            .expect("fake native report"))
        }
    }

    #[test]
    fn native_gpu_run_persists_segments_and_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = FakeBackend {
            gpu_fails: false,
            devices: Vec::new(),
        };
        let output = transcribe_native(
            &mut backend,
            Path::new("/media/talk.webm"),
            dir.path(),
            Some("en"),
            true,
        )
        .unwrap();
        assert_eq!(backend.devices, [DevicePreference::Auto]);
        assert_eq!(output.stream.segments.len(), 2);
        assert_eq!(output.stream.segments[1].text, "Music under speech.");
        assert_eq!(output.stream.language.as_deref(), Some("en"));
        assert!(dir.path().join("talk.json").is_file());
        assert_eq!(output.run["device"], "cuda:0");
        assert_eq!(output.run["revision"], super::NATIVE_WHISPERX_REVISION);
        assert!(output.run["cpuFallback"].is_null());
    }

    #[test]
    fn failed_gpu_run_falls_back_to_the_cpu() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = FakeBackend {
            gpu_fails: true,
            devices: Vec::new(),
        };
        let output = transcribe_native(
            &mut backend,
            Path::new("/media/talk.webm"),
            dir.path(),
            Some("en"),
            true,
        )
        .unwrap();
        assert_eq!(
            backend.devices,
            [DevicePreference::Auto, DevicePreference::Cpu]
        );
        assert_eq!(output.run["device"], "cpu");
        assert!(output.run["cpuFallback"]
            .as_str()
            .unwrap()
            .contains("CUDA out of memory"));
        assert!(output
            .stream
            .message
            .unwrap()
            .contains("GPU run failed, used the CPU"));
    }

    #[test]
    fn cpu_only_build_never_asks_for_the_gpu() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = FakeBackend {
            gpu_fails: true,
            devices: Vec::new(),
        };
        transcribe_native(
            &mut backend,
            Path::new("/media/talk.webm"),
            dir.path(),
            Some("en"),
            false,
        )
        .unwrap();
        assert_eq!(backend.devices, [DevicePreference::Cpu]);
    }

    #[test]
    fn producer_identity_pins_the_native_revision() {
        let identity = producer_identity(None, &["en-US".to_string()]).unwrap();
        assert_eq!(identity["provider"], "native-whisperx");
        assert_eq!(identity["revision"], super::NATIVE_WHISPERX_REVISION);
        assert_eq!(identity["config"]["modelId"], "small");
        assert_eq!(identity["config"]["language"], "en");
    }

    #[test]
    fn native_language_is_the_first_caption_language() {
        use super::native_language;
        assert_eq!(
            native_language(&["de_DE".into(), "en".into()]).as_deref(),
            Some("de")
        );
        assert_eq!(native_language(&[]), None);
    }

    #[test]
    fn whisper_json_path_uses_media_file_stem() {
        let path = whisper_json_path(
            Path::new("/media/lecture.audio.wav"),
            Path::new("/work/asr"),
        )
        .unwrap();
        assert_eq!(path, Path::new("/work/asr/lecture.audio.json"));
    }

    #[test]
    fn whisper_json_path_requires_a_file_name() {
        assert!(whisper_json_path(Path::new("/"), Path::new("/work/asr")).is_err());
    }
}
