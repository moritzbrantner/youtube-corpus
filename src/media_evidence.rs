use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::sponsorblock::{
    latest_sponsorblock_snapshot, SPONSORBLOCK_ATTRIBUTION, SPONSORBLOCK_DATA_LICENSE,
};

pub const MEDIA_EVIDENCE_SCHEMA: &str = "media_evidence";
pub const MEDIA_EVIDENCE_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaEvidenceBatchV1 {
    pub schema: String,
    pub schema_version: u32,
    pub producer: EvidenceProducerV1,
    pub video: EvidenceVideoV1,
    pub revision: String,
    pub scenes: Vec<SceneEvidenceV1>,
    pub ocr_observations: Vec<OcrObservationEvidenceV1>,
    pub ocr_tracks: Vec<OcrTrackEvidenceV1>,
    pub sponsorblock: Option<SponsorBlockEvidenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceProducerV1 {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceVideoV1 {
    pub id: String,
    pub youtube_id: Option<String>,
    pub source_url: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingEvidenceV1 {
    pub run_id: String,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub config_hash: String,
    pub processing_config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SceneEvidenceV1 {
    pub id: String,
    pub scene_index: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub metadata: Value,
    pub provenance: ProcessingEvidenceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrObservationEvidenceV1 {
    pub id: String,
    pub text: String,
    pub language: Option<String>,
    pub frame_index: Option<u64>,
    pub timestamp_seconds: Option<f64>,
    pub scene_index: Option<u64>,
    pub region: Option<BoundingBoxV1>,
    pub confidence: Option<f64>,
    pub attributes: Value,
    pub provenance: ProcessingEvidenceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBoxV1 {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrTrackEvidenceV1 {
    pub id: String,
    pub text: String,
    pub role: String,
    pub language: Option<String>,
    pub sample_count: u32,
    pub start_frame: Option<u64>,
    pub end_frame: Option<u64>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub region: Option<BoundingBoxV1>,
    pub metadata: Value,
    pub observation_ids: Vec<String>,
    pub scene_ids: Vec<String>,
    pub provenance: ProcessingEvidenceV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockEvidenceV1 {
    pub snapshot_id: String,
    pub response_hash: String,
    pub data_license: String,
    pub attribution: String,
    pub categories: Vec<String>,
    pub segments: Vec<SponsorBlockSegmentEvidenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockSegmentEvidenceV1 {
    pub uuid: String,
    pub category: String,
    pub action_type: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub video_duration: Option<f64>,
    pub metadata: Value,
}

pub async fn export_media_evidence_batch(
    pool: &PgPool,
    video_id: Uuid,
    producer_revision: &str,
) -> anyhow::Result<MediaEvidenceBatchV1> {
    if producer_revision.trim().is_empty() {
        anyhow::bail!("media-evidence producer revision is required");
    }

    let row = sqlx::query("SELECT youtube_id, source_url, title FROM videos WHERE id = $1")
        .bind(video_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("video {video_id} not found"))?;
    let video = EvidenceVideoV1 {
        id: video_id.to_string(),
        youtube_id: row.try_get("youtube_id")?,
        source_url: row.try_get("source_url")?,
        title: row.try_get("title")?,
    };

    let scenes = scene_evidence(pool, video_id).await?;
    let ocr_observations = ocr_observation_evidence(pool, video_id).await?;
    let ocr_tracks = ocr_track_evidence(pool, video_id).await?;
    let sponsorblock =
        latest_sponsorblock_snapshot(pool, video_id)
            .await?
            .map(|(snapshot_id, snapshot)| SponsorBlockEvidenceV1 {
                snapshot_id: snapshot_id.to_string(),
                response_hash: snapshot.response_hash,
                data_license: SPONSORBLOCK_DATA_LICENSE.to_string(),
                attribution: SPONSORBLOCK_ATTRIBUTION.to_string(),
                categories: snapshot.categories,
                segments: snapshot
                    .segments
                    .into_iter()
                    .map(|segment| SponsorBlockSegmentEvidenceV1 {
                        uuid: segment.uuid,
                        category: segment.category,
                        action_type: segment.action_type,
                        start_seconds: segment.start_seconds,
                        end_seconds: segment.end_seconds,
                        video_duration: segment.video_duration,
                        metadata: segment.metadata,
                    })
                    .collect(),
            });

    let revision = media_evidence_revision(
        &video,
        &scenes,
        &ocr_observations,
        &ocr_tracks,
        &sponsorblock,
    )?;

    Ok(MediaEvidenceBatchV1 {
        schema: MEDIA_EVIDENCE_SCHEMA.to_string(),
        schema_version: MEDIA_EVIDENCE_VERSION_V1,
        producer: EvidenceProducerV1 {
            name: "youtube-corpus".to_string(),
            revision: producer_revision.to_string(),
        },
        video,
        revision,
        scenes,
        ocr_observations,
        ocr_tracks,
        sponsorblock,
    })
}

/// Explicitly export one scene producer run while retaining ordinary v1 OCR
/// and SponsorBlock evidence. Useful when older seconds-only scene runs coexist
/// with an opt-in exact run: an exact sidecar may claim the selected scene
/// modality without pretending the earlier runs acquired original PTS.
/// The existing all-runs exporter and v1 schema remain unchanged.
pub async fn export_media_evidence_batch_for_scene_run(
    pool: &PgPool,
    video_id: Uuid,
    producer_revision: &str,
    scene_run_id: Uuid,
) -> anyhow::Result<MediaEvidenceBatchV1> {
    let batch = export_media_evidence_batch(pool, video_id, producer_revision).await?;
    select_scene_run(batch, scene_run_id)
}

fn select_scene_run(
    mut batch: MediaEvidenceBatchV1,
    scene_run_id: Uuid,
) -> anyhow::Result<MediaEvidenceBatchV1> {
    let run_id = scene_run_id.to_string();
    batch
        .scenes
        .retain(|scene| scene.provenance.run_id == run_id);
    if batch.scenes.is_empty() {
        anyhow::bail!("scene run {scene_run_id} has no exported scene evidence");
    }
    let selected: BTreeSet<_> = batch.scenes.iter().map(|scene| scene.id.as_str()).collect();
    for track in &mut batch.ocr_tracks {
        track.scene_ids.retain(|id| selected.contains(id.as_str()));
    }
    batch.revision = media_evidence_revision(
        &batch.video,
        &batch.scenes,
        &batch.ocr_observations,
        &batch.ocr_tracks,
        &batch.sponsorblock,
    )?;
    Ok(batch)
}

fn media_evidence_revision(
    video: &EvidenceVideoV1,
    scenes: &[SceneEvidenceV1],
    ocr_observations: &[OcrObservationEvidenceV1],
    ocr_tracks: &[OcrTrackEvidenceV1],
    sponsorblock: &Option<SponsorBlockEvidenceV1>,
) -> anyhow::Result<String> {
    let material = serde_json::json!({
        "video": video,
        "scenes": scenes,
        "ocrObservations": ocr_observations,
        "ocrTracks": ocr_tracks,
        "sponsorBlock": sponsorblock,
    });
    Ok(format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(&material)?)
    ))
}

async fn scene_evidence(pool: &PgPool, video_id: Uuid) -> anyhow::Result<Vec<SceneEvidenceV1>> {
    let rows = sqlx::query(
        "SELECT s.id, s.scene_index, s.start_frame, s.end_frame, s.start_seconds, s.end_seconds,
                s.metadata, r.id AS run_id, r.processor, r.processor_version, r.model,
                r.model_version, r.input_hash, r.config_hash, r.processing_config
         FROM video_scenes s
         JOIN media_processing_runs r ON r.id = s.run_id
         WHERE s.video_id = $1
         ORDER BY s.start_seconds, s.scene_index, s.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(SceneEvidenceV1 {
                id: row.try_get::<Uuid, _>("id")?.to_string(),
                scene_index: non_negative_u64(row.try_get("scene_index")?, "scene index")?,
                start_frame: non_negative_u64(row.try_get("start_frame")?, "scene start frame")?,
                end_frame: non_negative_u64(row.try_get("end_frame")?, "scene end frame")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                metadata: row.try_get("metadata")?,
                provenance: provenance_from_row(&row)?,
            })
        })
        .collect()
}

async fn ocr_observation_evidence(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<OcrObservationEvidenceV1>> {
    let rows = sqlx::query(
        "SELECT o.id, o.text, o.language, o.frame_index, o.timestamp_seconds, o.scene_index,
                o.bbox_x, o.bbox_y, o.bbox_width, o.bbox_height, o.confidence, o.attributes,
                r.id AS run_id, r.processor, r.processor_version, r.model, r.model_version,
                r.input_hash, r.config_hash, r.processing_config
         FROM visual_text_observations o
         JOIN media_processing_runs r ON r.id = o.run_id
         WHERE o.video_id = $1
         ORDER BY o.timestamp_seconds NULLS LAST, o.frame_index NULLS LAST, o.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(OcrObservationEvidenceV1 {
                id: row.try_get::<Uuid, _>("id")?.to_string(),
                text: row.try_get("text")?,
                language: row.try_get("language")?,
                frame_index: optional_u64(row.try_get("frame_index")?, "OCR frame index")?,
                timestamp_seconds: row.try_get("timestamp_seconds")?,
                scene_index: optional_u64(row.try_get("scene_index")?, "OCR scene index")?,
                region: region_from_row(&row)?,
                confidence: row.try_get("confidence")?,
                attributes: row.try_get("attributes")?,
                provenance: provenance_from_row(&row)?,
            })
        })
        .collect()
}

async fn ocr_track_evidence(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Vec<OcrTrackEvidenceV1>> {
    let rows = sqlx::query(
        "SELECT t.id, t.text, t.role, t.language, t.sample_count, t.start_frame, t.end_frame,
                t.start_seconds, t.end_seconds, t.bbox_x, t.bbox_y, t.bbox_width, t.bbox_height,
                t.metadata, r.id AS run_id, r.processor, r.processor_version, r.model,
                r.model_version, r.input_hash, r.config_hash, r.processing_config
         FROM visual_text_tracks t
         JOIN media_processing_runs r ON r.id = t.run_id
         WHERE t.video_id = $1
         ORDER BY t.start_seconds NULLS LAST, t.track_key, t.id",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;

    let mut tracks = Vec::with_capacity(rows.len());
    for row in rows {
        let track_id: Uuid = row.try_get("id")?;
        let observation_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT observation_id FROM visual_text_track_observations
             WHERE track_id = $1 ORDER BY observation_id",
        )
        .bind(track_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|id| id.to_string())
        .collect();
        let scene_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT scene_id FROM visual_text_track_scenes
             WHERE track_id = $1 ORDER BY scene_id",
        )
        .bind(track_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|id| id.to_string())
        .collect();

        let role: String = row.try_get("role")?;
        tracks.push(OcrTrackEvidenceV1 {
            id: track_id.to_string(),
            text: row.try_get("text")?,
            role,
            language: row.try_get("language")?,
            sample_count: u32::try_from(row.try_get::<i32, _>("sample_count")?)
                .map_err(|_| anyhow::anyhow!("stored OCR sample count is negative"))?,
            start_frame: optional_u64(row.try_get("start_frame")?, "OCR track start frame")?,
            end_frame: optional_u64(row.try_get("end_frame")?, "OCR track end frame")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
            region: region_from_row(&row)?,
            metadata: row.try_get("metadata")?,
            observation_ids,
            scene_ids,
            provenance: provenance_from_row(&row)?,
        });
    }
    Ok(tracks)
}

fn provenance_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<ProcessingEvidenceV1> {
    Ok(ProcessingEvidenceV1 {
        run_id: row.try_get::<Uuid, _>("run_id")?.to_string(),
        processor: row.try_get("processor")?,
        processor_version: row.try_get("processor_version")?,
        model: row.try_get("model")?,
        model_version: row.try_get("model_version")?,
        input_hash: row.try_get("input_hash")?,
        config_hash: row.try_get("config_hash")?,
        processing_config: row.try_get("processing_config")?,
    })
}

fn region_from_row(row: &sqlx::postgres::PgRow) -> anyhow::Result<Option<BoundingBoxV1>> {
    let values: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = (
        row.try_get("bbox_x")?,
        row.try_get("bbox_y")?,
        row.try_get("bbox_width")?,
        row.try_get("bbox_height")?,
    );
    match values {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) => Ok(Some(BoundingBoxV1 {
            x: u32::try_from(x)?,
            y: u32::try_from(y)?,
            width: u32::try_from(width)?,
            height: u32::try_from(height)?,
        })),
        _ => anyhow::bail!("partially null OCR bounding box"),
    }
}

fn non_negative_u64(value: i64, name: &str) -> anyhow::Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("stored {name} is negative"))
}

fn optional_u64(value: Option<i64>, name: &str) -> anyhow::Result<Option<u64>> {
    value.map(|value| non_negative_u64(value, name)).transpose()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    crate::lower_hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_schema_is_explicitly_versioned() {
        assert_eq!(MEDIA_EVIDENCE_SCHEMA, "media_evidence");
        assert_eq!(MEDIA_EVIDENCE_VERSION_V1, 1);
    }

    #[test]
    fn revisions_are_sha256_addressed() {
        assert_eq!(sha256_hex(b"evidence").len(), 64);
    }

    #[test]
    fn selecting_exact_scene_run_preserves_v1_and_rehashes_its_actual_contents() {
        let legacy_run = Uuid::new_v4();
        let exact_run = Uuid::new_v4();
        let provenance = |run_id: Uuid| ProcessingEvidenceV1 {
            run_id: run_id.to_string(),
            processor: "visual-analysis".into(),
            processor_version: "revision".into(),
            model: "scene".into(),
            model_version: "1".into(),
            input_hash: "sha256:source".into(),
            config_hash: run_id.to_string(),
            processing_config: serde_json::json!({}),
        };
        let scene = |id: &str, run_id: Uuid| SceneEvidenceV1 {
            id: id.into(),
            scene_index: 0,
            start_frame: 0,
            end_frame: 7,
            start_seconds: 0.0,
            end_seconds: 0.4,
            metadata: serde_json::json!({}),
            provenance: provenance(run_id),
        };
        let batch = MediaEvidenceBatchV1 {
            schema: MEDIA_EVIDENCE_SCHEMA.into(),
            schema_version: MEDIA_EVIDENCE_VERSION_V1,
            producer: EvidenceProducerV1 {
                name: "youtube-corpus".into(),
                revision: "git:revision".into(),
            },
            video: EvidenceVideoV1 {
                id: Uuid::new_v4().to_string(),
                youtube_id: None,
                source_url: "https://example.invalid/test".into(),
                title: None,
            },
            revision: "sha256:old".into(),
            scenes: vec![scene("legacy", legacy_run), scene("exact", exact_run)],
            ocr_observations: vec![],
            ocr_tracks: vec![OcrTrackEvidenceV1 {
                id: "track".into(),
                text: "text".into(),
                role: "presentation_slide".into(),
                language: None,
                sample_count: 1,
                start_frame: None,
                end_frame: None,
                start_seconds: None,
                end_seconds: None,
                region: None,
                metadata: serde_json::json!({}),
                observation_ids: vec![],
                scene_ids: vec!["legacy".into(), "exact".into()],
                provenance: provenance(Uuid::new_v4()),
            }],
            sponsorblock: None,
        };
        let selected = select_scene_run(batch.clone(), exact_run).unwrap();
        assert_eq!(selected.scenes.len(), 1);
        assert_eq!(selected.scenes[0].id, "exact");
        assert_eq!(selected.ocr_tracks[0].scene_ids, vec!["exact"]);
        assert_eq!(selected.schema_version, batch.schema_version);
        assert_ne!(selected.revision, batch.revision);
        assert_eq!(
            select_scene_run(selected.clone(), exact_run)
                .unwrap()
                .revision,
            selected.revision,
        );
        assert!(select_scene_run(batch, Uuid::new_v4()).is_err());
    }
}
