//! Corpus persistence around canonical model-backed speaker diarization.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use audio_analysis_core::{interleaved_to_mono, AudioClip, ChannelMix};
use audio_analysis_io::{AudioFrameSource, FfmpegAudioSource};
use audio_analysis_processing::OfflineAudioProcessor;
use audio_analysis_speakers::{
    DiarizationSegment, DiarizedSpeaker, EnergyVoiceActivityDetector, OnnxSpeakerEmbedder,
    OnnxSpeakerEmbeddingConfig, SpeakerAudio, SpeakerDiarizer, SpeakerEmbedding,
    SpeakerEmbeddingExtractor, VadOptions, WindowedSpeakerDiarizer, DEFAULT_CLUSTER_THRESHOLD,
    DEFAULT_EMBEDDING_WINDOW_SECONDS, DEFAULT_MIN_CLUSTER_FRACTION, DEFAULT_MIN_CLUSTER_SECONDS,
    DEFAULT_MIN_SPEAKER_EVIDENCE_SECONDS,
};
use model_runtime::{HuggingFaceDownloader, HuggingFaceModelSpec, ModelBundleStore, ModelTask};
use runtime_core::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::multimodal::{
    begin_processing_run_with, link_voice_observation_to_track_with, upsert_voice_observation_with,
    upsert_voice_track_with, BeginProcessingRunRequest, MediaModality, ProcessingProvenance,
    UpsertVoiceObservationRequest, UpsertVoiceTrackRequest,
};

pub const AUDIO_ANALYSIS_REVISION: &str = "c058cac6235c6471779bd0a87f5076549c47958b";
pub const VOICE_PROCESSOR: &str = "youtube-corpus/voices";
pub const VOICE_MODEL: &str = "Wespeaker/wespeaker-voxceleb-resnet34-LM";
pub const VOICE_MODEL_VERSION: &str = "f0c48c298fd835726c27956a5d617bad7115627e";
const MODEL_NAME: &str = "wespeaker-resnet34-lm";
const MODEL_FILE: &str = "voxceleb_resnet34_LM.onnx";
const MODEL_SHA256: &str = "7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068";

#[derive(Debug, Clone)]
pub struct VoiceAnalysisRequest {
    pub video_id: Uuid,
    pub model_dir: PathBuf,
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAnalysisReport {
    pub video_id: Uuid,
    pub run_id: Uuid,
    pub observations: u64,
    pub tracks: u64,
    pub reused: bool,
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// Shortest speech span that is embedded. WeSpeaker ResNet34 returns non-finite embeddings
/// below ~150 ms (its statistics pooling sees a single frame), which failed whole real videos;
/// spans under half a second also carry too little voice for cross-video matching.
const MIN_SPEECH_SECONDS: f64 = 0.5;

fn vad_options() -> VadOptions {
    VadOptions {
        min_speech_seconds: MIN_SPEECH_SECONDS,
        ..VadOptions::unpadded()
    }
}

pub(crate) fn settings() -> Value {
    let vad = vad_options();
    json!({"sampleRate":16000,"channelMix":"average","diarizer":"audio-analysis/WindowedSpeakerDiarizer",
        "clustering":"average-linkage","clusterThreshold":DEFAULT_CLUSTER_THRESHOLD,
        "embeddingWindowSeconds":DEFAULT_EMBEDDING_WINDOW_SECONDS,"minClusterSeconds":DEFAULT_MIN_CLUSTER_SECONDS,
        "minClusterFraction":DEFAULT_MIN_CLUSTER_FRACTION,"minSpeakerEvidenceSeconds":DEFAULT_MIN_SPEAKER_EVIDENCE_SECONDS,"vad":{"kind":"energy","rmsThreshold":vad.rms_threshold,"frameSeconds":vad.frame_seconds,
        "hopSeconds":vad.hop_seconds,"minSpeechSeconds":vad.min_speech_seconds,"mergeGapSeconds":vad.merge_gap_seconds}})
}

fn provenance(input_hash: String) -> anyhow::Result<ProcessingProvenance> {
    Ok(ProcessingProvenance {
        processor: VOICE_PROCESSOR.into(),
        processor_version: crate::provenance::PROCESSOR_SOURCE_REVISION.into(),
        model: VOICE_MODEL.into(),
        model_version: VOICE_MODEL_VERSION.into(),
        input_hash,
        config_hash: hash_bytes(&serde_json::to_vec(&settings())?),
        processing_config: json!({"settings":settings(),"audioAnalysisRevision":AUDIO_ANALYSIS_REVISION,
            "foundationRevision":"8255347717ca7b8943ede7c435d0c468e5a107c5","runtime":"onnx/cpu",
            "model":{"id":VOICE_MODEL,"revision":VOICE_MODEL_VERSION,"file":MODEL_FILE,"sha256":MODEL_SHA256,
                "license":"CC-BY-4.0","attribution":"WeSpeaker, https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34-LM"}}),
    })
}

async fn media_path(pool: &PgPool, video: Uuid) -> anyhow::Result<PathBuf> {
    let path: Option<String> =
        sqlx::query_scalar("SELECT local_video_path FROM videos WHERE id=$1")
            .bind(video)
            .fetch_optional(pool)
            .await?
            .context("video not found")?;
    let path=path.filter(|p|!p.trim().is_empty()).map(PathBuf::from)
        .context("voices needs retained local media with an audio stream; download media before running this detector")?;
    anyhow::ensure!(
        path.is_file(),
        "retained voice-analysis media does not exist: {}",
        path.display()
    );
    Ok(path)
}

async fn input_hash(path: PathBuf) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || {
        let identity = crate::detector_status::file_identity(&path)?;
        Ok(format!(
            "sha256:{}",
            identity["sha256"]
                .as_str()
                .context("media fingerprint missing")?
        ))
    })
    .await?
}

async fn existing_run(
    pool: &PgPool,
    video: Uuid,
    identity: &ProcessingProvenance,
) -> anyhow::Result<Option<VoiceAnalysisReport>> {
    let row=sqlx::query("SELECT r.id,
        (SELECT count(*) FROM voice_observations o WHERE o.run_id=r.id) observations,
        (SELECT count(*) FROM voice_tracks t WHERE t.run_id=r.id) tracks
        FROM media_processing_runs r WHERE video_id=$1 AND modality='voice' AND processor=$2
        AND processor_version=$3 AND model=$4 AND model_version=$5 AND input_hash=$6 AND config_hash=$7
        AND processing_config->>'complete'='true'")
        .bind(video).bind(&identity.processor).bind(&identity.processor_version).bind(&identity.model)
        .bind(&identity.model_version).bind(&identity.input_hash).bind(&identity.config_hash).fetch_optional(pool).await?;
    row.map(|r| {
        Ok(VoiceAnalysisReport {
            video_id: video,
            run_id: r.try_get("id")?,
            observations: u64::try_from(r.try_get::<i64, _>("observations")?)?,
            tracks: u64::try_from(r.try_get::<i64, _>("tracks")?)?,
            reused: true,
        })
    })
    .transpose()
}

pub(crate) async fn has_current_voice_output(pool: &PgPool, video: Uuid) -> anyhow::Result<bool> {
    let Some(hash) = crate::media_acquisition::source_input_hash(pool, video).await? else {
        return Ok(false);
    };
    let identity = provenance(hash)?;
    Ok(existing_run(pool, video, &identity).await?.is_some())
}

pub async fn analyze_stored_video_voices(
    pool: &PgPool,
    request: VoiceAnalysisRequest,
) -> anyhow::Result<VoiceAnalysisReport> {
    let path = media_path(pool, request.video_id).await?;
    let identity = provenance(input_hash(path.clone()).await?)?;
    if !request.refresh {
        if let Some(report) = existing_run(pool, request.video_id, &identity).await? {
            return Ok(report);
        }
    }
    let token = CancellationToken::new();
    let _guard = CancelOnDrop(token.clone());
    let worker_path = path.clone();
    let models = request.model_dir.join("voices");
    let batch =
        tokio::task::spawn_blocking(move || analyze_media(&worker_path, &models, &token)).await??;
    anyhow::ensure!(
        input_hash(path).await? == identity.input_hash,
        "voice-analysis media changed during processing; retry"
    );
    persist_batch(pool, request.video_id, identity, batch).await
}

struct Observation {
    start: f64,
    end: f64,
    speaker: String,
    activity_score: f32,
    embedding: Vec<f32>,
}
struct Batch {
    observations: Vec<Observation>,
}

/// Record the actual canonical extractor outputs, with each embedded window's sample count,
/// without running inference twice or reconstructing speaker embeddings in this application.
struct RecordingEmbedder {
    inner: OnnxSpeakerEmbedder,
    embeddings: Vec<(SpeakerEmbedding, usize)>,
    token: CancellationToken,
}
impl SpeakerEmbeddingExtractor for RecordingEmbedder {
    fn model_info(&self) -> audio_analysis_speakers::SpeakerEmbeddingModel {
        self.inner.model_info()
    }
    fn embed_speaker(
        &mut self,
        audio: &SpeakerAudio<'_>,
    ) -> audio_contracts::Result<SpeakerEmbedding> {
        if self.token.is_cancelled() {
            return Err(audio_contracts::DetectError::InvalidArgument(
                "voice analysis cancelled".into(),
            ));
        }
        let embedding = self.inner.embed_speaker(audio)?;
        self.embeddings
            .push((embedding.clone(), audio.samples_per_channel()));
        Ok(embedding)
    }
}

fn analyze_media(path: &Path, models: &Path, token: &CancellationToken) -> anyhow::Result<Batch> {
    // Probe/decode before model acquisition; canonical FFmpeg ownership ends on drop.
    let mut source = FfmpegAudioSource::open(path)
        .context("cannot decode retained media for voices; an audio stream is required")?;
    let rate = source.metadata().sample_rate;
    let mut samples = Vec::new();
    while let Some(frame) = source.next_audio_frame()? {
        anyhow::ensure!(!token.is_cancelled(), "voice analysis cancelled");
        samples.extend(interleaved_to_mono(
            &frame.data,
            frame.channels,
            ChannelMix::Average,
        )?);
    }
    drop(source);
    anyhow::ensure!(
        !samples.is_empty(),
        "retained media has no decodable audio samples"
    );
    let clip = AudioClip::new(rate, 1, samples)?;
    let clip = OfflineAudioProcessor::new()
        .resample(16000)
        .process_clip(clip)?;
    anyhow::ensure!(!token.is_cancelled(), "voice analysis cancelled");
    let store =
        ModelBundleStore::new(models).downloader(HuggingFaceDownloader::new().max_retries(2));
    let spec = HuggingFaceModelSpec::new(VOICE_MODEL, ModelTask::AudioEmbedding)
        .name(MODEL_NAME)
        .revision(VOICE_MODEL_VERSION)
        .file(MODEL_FILE)
        .file("README.md");
    let bundle = if store.bundle_dir(&spec).join("manifest.json").exists() {
        store.load(MODEL_NAME, VOICE_MODEL_VERSION)?
    } else {
        store.download_cancellable(&spec, Some(token))?
    };
    let model = bundle
        .file_path(MODEL_FILE)
        .context("speaker model bundle is missing ONNX file")?;
    anyhow::ensure!(
        crate::detector_status::file_identity(&model)?["sha256"].as_str() == Some(MODEL_SHA256),
        "speaker model checksum does not match pinned model; repair its local bundle"
    );
    let embedder = OnnxSpeakerEmbedder::from_config(OnnxSpeakerEmbeddingConfig {
        bundle_path: model,
        embedding_dimension: 256,
        ..Default::default()
    })?;
    let recorder = RecordingEmbedder {
        inner: embedder,
        embeddings: Vec::new(),
        token: token.clone(),
    };
    let mut diarizer = WindowedSpeakerDiarizer::new(recorder, EnergyVoiceActivityDetector)
        .vad_options(vad_options())?;
    let audio = SpeakerAudio::mono(&clip.samples, 16000)?;
    let result = diarizer.diarize(&audio)?;
    anyhow::ensure!(!token.is_cancelled(), "voice analysis cancelled");
    let observations = window_observations(result.segments, diarizer.extractor.embeddings, 16000)?;
    Ok(Batch { observations })
}

/// One observation per embedded diarizer window. The diarizer splits long speech spans into
/// windows and rejoins neighbouring windows that land on one speaker, so a segment covers one or
/// more consecutive recorded windows; each window keeps its own embedding and time range.
/// Unresolved spans were never embedded and yield no observation.
fn window_observations(
    segments: Vec<DiarizationSegment>,
    windows: Vec<(SpeakerEmbedding, usize)>,
    sample_rate: u32,
) -> anyhow::Result<Vec<Observation>> {
    let rate = f64::from(sample_rate);
    // Window boundaries are rounded to samples, so allow a little slack per segment.
    let slack = rate / 10.0;
    let mut windows = windows.into_iter();
    let mut observations = Vec::new();
    for segment in segments {
        let speaker = match segment.speaker {
            DiarizedSpeaker::Unknown(label) => label,
            DiarizedSpeaker::Known(id) => id.as_str().into(),
            DiarizedSpeaker::Unresolved => continue,
        };
        let length = (segment.end_seconds - segment.start_seconds) * rate;
        let mut covered = 0.0;
        loop {
            let (embedding, samples) = windows
                .next()
                .context("canonical speaker segment has no recorded embedding")?;
            let start = segment.start_seconds + covered / rate;
            covered += samples as f64;
            let last = length - covered <= slack;
            observations.push(Observation {
                start,
                end: if last {
                    segment.end_seconds
                } else {
                    segment.start_seconds + covered / rate
                },
                speaker: speaker.clone(),
                activity_score: segment.score,
                embedding: embedding.values().to_vec(),
            });
            if last {
                break;
            }
        }
    }
    anyhow::ensure!(
        windows.next().is_none(),
        "canonical speaker result/embedding count differs"
    );
    Ok(observations)
}

async fn persist_batch(
    pool: &PgPool,
    video: Uuid,
    mut identity: ProcessingProvenance,
    batch: Batch,
) -> anyhow::Result<VoiceAnalysisReport> {
    identity.processing_config["complete"] = json!(true);
    let mut tx = pool.begin().await?;
    let run = begin_processing_run_with(
        &mut tx,
        BeginProcessingRunRequest {
            video_id: video,
            modality: MediaModality::Voice,
            provenance: identity,
        },
    )
    .await?;
    let mut observation_ids = Vec::new();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, observation) in batch.observations.iter().enumerate() {
        let stored=upsert_voice_observation_with(&mut tx,UpsertVoiceObservationRequest{run_id:run.id,video_id:video,
            observation_key:format!("segment-{index}"),start_seconds:observation.start,end_seconds:observation.end,
            confidence:None,transcript_segment_id:None,embedding:Some(observation.embedding.clone()),
            quality:json!({"activityScore":observation.activity_score,"speakerLabel":observation.speaker,"inferred":true,"verifiedIdentity":false})}).await?;
        observation_ids.push(stored.id);
        groups
            .entry(observation.speaker.clone())
            .or_default()
            .push(index);
    }
    sqlx::query("DELETE FROM voice_track_observations l USING voice_tracks t WHERE l.track_id=t.id AND t.run_id=$1").bind(run.id).execute(&mut *tx).await?;
    let mut track_ids = Vec::new();
    for (label, members) in groups {
        let first = &batch.observations[members[0]];
        let end = members
            .iter()
            .map(|index| batch.observations[*index].end)
            .fold(first.end, f64::max);
        let track=upsert_voice_track_with(&mut tx,UpsertVoiceTrackRequest{run_id:run.id,video_id:video,track_key:label,
            start_seconds:first.start,end_seconds:end,representative_embedding:Some(first.embedding.clone()),
            quality:json!({"inferred":true,"verifiedIdentity":false,"observationCount":members.len(),"representative":"first-canonical-segment"})}).await?;
        track_ids.push(track.id);
        for index in members {
            link_voice_observation_to_track_with(&mut tx, track.id, observation_ids[index]).await?;
        }
    }
    sqlx::query("DELETE FROM voice_tracks WHERE run_id=$1 AND NOT(id=ANY($2))")
        .bind(run.id)
        .bind(&track_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM voice_observations WHERE run_id=$1 AND NOT(id=ANY($2))")
        .bind(run.id)
        .bind(&observation_ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(VoiceAnalysisReport {
        video_id: video,
        run_id: run.id,
        observations: u64::try_from(observation_ids.len())?,
        tracks: u64::try_from(track_ids.len())?,
        reused: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: f64, end: f64, speaker: DiarizedSpeaker) -> DiarizationSegment {
        DiarizationSegment {
            start_seconds: start,
            end_seconds: end,
            speaker,
            score: 1.0,
        }
    }

    fn window(value: f32, seconds: f64) -> (SpeakerEmbedding, usize) {
        let model = audio_analysis_speakers::SpeakerEmbeddingModel::spectral_baseline(2).unwrap();
        let embedding = SpeakerEmbedding::new(vec![value, 1.0], model, 100).unwrap();
        (embedding, (seconds * 100.0).round() as usize)
    }

    #[test]
    fn rejoined_windows_keep_their_own_embeddings_and_times() {
        let a = || DiarizedSpeaker::Unknown("speaker_0".into());
        // A 7.5 s span cut into three 2.5 s windows: the first two rejoined on speaker_0,
        // the third went to speaker_1. A short unresolved span carries no embedding.
        let segments = vec![
            segment(1.0, 6.0, a()),
            segment(6.0, 8.5, DiarizedSpeaker::Unknown("speaker_1".into())),
            segment(9.0, 9.15, DiarizedSpeaker::Unresolved),
            segment(10.0, 11.0, a()),
        ];
        let windows = vec![
            window(0.1, 2.5),
            window(0.2, 2.5),
            window(0.3, 2.5),
            window(0.4, 1.0),
        ];
        let observations = window_observations(segments, windows, 100).unwrap();
        let spans = observations
            .iter()
            .map(|o| (o.start, o.end, o.speaker.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            spans,
            [
                (1.0, 3.5, "speaker_0"),
                (3.5, 6.0, "speaker_0"),
                (6.0, 8.5, "speaker_1"),
                (10.0, 11.0, "speaker_0")
            ]
        );
        assert!(observations[1].embedding[0] > observations[0].embedding[0]);
    }

    #[test]
    fn missing_or_extra_windows_fail() {
        let one = vec![segment(
            0.0,
            1.0,
            DiarizedSpeaker::Unknown("speaker_0".into()),
        )];
        assert!(window_observations(one.clone(), vec![], 100).is_err());
        assert!(window_observations(one, vec![window(0.1, 1.0), window(0.2, 1.0)], 100).is_err());
    }
}
