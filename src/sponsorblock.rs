use std::process::Command;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use url::Url;
use uuid::Uuid;

pub const SPONSORBLOCK_API_BASE: &str = "https://sponsor.ajay.app";
pub const SPONSORBLOCK_DATA_LICENSE: &str = "CC BY-NC-SA 4.0";
pub const SPONSORBLOCK_ATTRIBUTION: &str = "SponsorBlock by Ajay Ramachandran and contributors";

pub const DEFAULT_SPONSORBLOCK_CATEGORIES: &[&str] = &[
    "sponsor",
    "intro",
    "outro",
    "interaction",
    "selfpromo",
    "preview",
    "music_offtopic",
    "poi_highlight",
    "filler",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockSnapshot {
    pub youtube_id: String,
    pub video_hash: String,
    pub hash_prefix: String,
    pub categories: Vec<String>,
    pub response_hash: String,
    pub segments: Vec<SponsorBlockSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockSegment {
    pub uuid: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_type: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_duration: Option<f64>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Deserialize)]
struct HashMatch {
    #[serde(rename = "videoID")]
    video_id: String,
    hash: String,
    segments: Vec<ApiSegment>,
}

#[derive(Debug, Deserialize)]
struct ApiSegment {
    #[serde(rename = "UUID")]
    uuid: String,
    category: String,
    #[serde(rename = "actionType", default)]
    action_type: Option<String>,
    segment: Vec<f64>,
    #[serde(rename = "videoDuration", default)]
    video_duration: Option<f64>,
}

pub fn fetch_sponsorblock_snapshot(
    youtube_id: &str,
    categories: &[String],
) -> anyhow::Result<SponsorBlockSnapshot> {
    let youtube_id = youtube_id.trim();
    ensure!(!youtube_id.is_empty(), "YouTube video id is required");
    let categories = normalized_categories(categories);
    ensure!(!categories.is_empty(), "at least one SponsorBlock category is required");

    let video_hash = sha256_hex(youtube_id.as_bytes());
    let hash_prefix = video_hash[..4].to_string();
    let mut url = Url::parse(&format!(
        "{SPONSORBLOCK_API_BASE}/api/skipSegments/{hash_prefix}"
    ))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("categories", &serde_json::to_string(&categories)?);
        query.append_pair("service", "YouTube");
    }

    let output = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .arg(url.as_str())
        .output()
        .context("failed to execute curl for SponsorBlock")?;
    ensure!(
        output.status.success(),
        "SponsorBlock request failed with status {}",
        output.status
    );

    parse_hash_response(youtube_id, &categories, &output.stdout)
}

pub fn parse_hash_response(
    youtube_id: &str,
    categories: &[String],
    bytes: &[u8],
) -> anyhow::Result<SponsorBlockSnapshot> {
    let expected_hash = sha256_hex(youtube_id.as_bytes());
    let matches: Vec<HashMatch> = serde_json::from_slice(bytes)?;
    let selected = matches
        .into_iter()
        .find(|candidate| candidate.video_id == youtube_id && candidate.hash == expected_hash);

    let segments = selected
        .map(|entry| entry.segments)
        .unwrap_or_default()
        .into_iter()
        .map(segment_from_api)
        .collect::<anyhow::Result<Vec<_>>>()?;

    let canonical = serde_json::to_vec(&segments)?;
    Ok(SponsorBlockSnapshot {
        youtube_id: youtube_id.to_string(),
        video_hash: expected_hash.clone(),
        hash_prefix: expected_hash[..4].to_string(),
        categories: normalized_categories(categories),
        response_hash: format!("sha256:{}", sha256_hex(&canonical)),
        segments,
    })
}

pub async fn persist_sponsorblock_snapshot(
    pool: &PgPool,
    video_id: Uuid,
    snapshot: &SponsorBlockSnapshot,
) -> anyhow::Result<Uuid> {
    let expected_hash = sha256_hex(snapshot.youtube_id.as_bytes());
    ensure!(
        expected_hash == snapshot.video_hash,
        "SponsorBlock snapshot video hash does not match YouTube id"
    );
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "youtube-corpus:sponsorblock:{}:{}",
            video_id, snapshot.response_hash
        )
        .as_bytes(),
    );

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO sponsorblock_snapshots
         (id, video_id, youtube_id, hash_prefix, categories, response_hash, data_license, attribution)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (video_id, response_hash) DO UPDATE SET
           categories = EXCLUDED.categories,
           fetched_at = now()
         RETURNING id",
    )
    .bind(id)
    .bind(video_id)
    .bind(&snapshot.youtube_id)
    .bind(&snapshot.hash_prefix)
    .bind(serde_json::to_value(&snapshot.categories)?)
    .bind(&snapshot.response_hash)
    .bind(SPONSORBLOCK_DATA_LICENSE)
    .bind(SPONSORBLOCK_ATTRIBUTION)
    .fetch_one(&mut *tx)
    .await?;

    for segment in &snapshot.segments {
        let segment_id = Uuid::new_v5(&id, segment.uuid.as_bytes());
        sqlx::query(
            "INSERT INTO sponsorblock_segments
             (id, snapshot_id, video_id, segment_uuid, category, action_type,
              start_seconds, end_seconds, video_duration, metadata)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (snapshot_id, segment_uuid) DO UPDATE SET
               category = EXCLUDED.category,
               action_type = EXCLUDED.action_type,
               start_seconds = EXCLUDED.start_seconds,
               end_seconds = EXCLUDED.end_seconds,
               video_duration = EXCLUDED.video_duration,
               metadata = EXCLUDED.metadata",
        )
        .bind(segment_id)
        .bind(id)
        .bind(video_id)
        .bind(&segment.uuid)
        .bind(&segment.category)
        .bind(&segment.action_type)
        .bind(segment.start_seconds)
        .bind(segment.end_seconds)
        .bind(segment.video_duration)
        .bind(&segment.metadata)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(id)
}

pub async fn refresh_sponsorblock_for_video(
    pool: &PgPool,
    video_id: Uuid,
    categories: &[String],
) -> anyhow::Result<SponsorBlockSnapshot> {
    let youtube_id = sqlx::query_scalar::<_, Option<String>>(
        "SELECT youtube_id FROM videos WHERE id = $1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .ok_or_else(|| anyhow::anyhow!("video {video_id} has no YouTube id"))?;

    let snapshot = fetch_sponsorblock_snapshot(&youtube_id, categories)?;
    persist_sponsorblock_snapshot(pool, video_id, &snapshot).await?;
    Ok(snapshot)
}

pub async fn latest_sponsorblock_snapshot(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<Option<(Uuid, SponsorBlockSnapshot)>> {
    let Some(row) = sqlx::query(
        "SELECT id, youtube_id, hash_prefix, categories, response_hash
         FROM sponsorblock_snapshots
         WHERE video_id = $1
         ORDER BY fetched_at DESC, id DESC
         LIMIT 1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let snapshot_id: Uuid = row.try_get("id")?;
    let youtube_id: String = row.try_get("youtube_id")?;
    let categories: Value = row.try_get("categories")?;
    let rows = sqlx::query(
        "SELECT segment_uuid, category, action_type, start_seconds, end_seconds,
                video_duration, metadata
         FROM sponsorblock_segments
         WHERE snapshot_id = $1
         ORDER BY start_seconds, end_seconds, segment_uuid",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?;
    let segments = rows
        .into_iter()
        .map(|row| {
            Ok(SponsorBlockSegment {
                uuid: row.try_get("segment_uuid")?,
                category: row.try_get("category")?,
                action_type: row.try_get("action_type")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                video_duration: row.try_get("video_duration")?,
                metadata: row.try_get("metadata")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Some((
        snapshot_id,
        SponsorBlockSnapshot {
            video_hash: sha256_hex(youtube_id.as_bytes()),
            youtube_id,
            hash_prefix: row.try_get("hash_prefix")?,
            categories: serde_json::from_value(categories)?,
            response_hash: row.try_get("response_hash")?,
            segments,
        },
    )))
}

fn segment_from_api(value: ApiSegment) -> anyhow::Result<SponsorBlockSegment> {
    ensure!(value.segment.len() == 2, "SponsorBlock segment must contain start and end");
    let start = value.segment[0];
    let end = value.segment[1];
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0.0 && end >= start,
        "SponsorBlock segment timestamps must be finite, non-negative, and ordered"
    );
    ensure!(
        value
            .video_duration
            .is_none_or(|duration| duration.is_finite() && duration >= 0.0),
        "SponsorBlock video duration must be finite and non-negative"
    );
    Ok(SponsorBlockSegment {
        uuid: value.uuid,
        category: value.category,
        action_type: value.action_type,
        start_seconds: start,
        end_seconds: end,
        video_duration: value.video_duration,
        metadata: serde_json::json!({
            "source": "SponsorBlock",
            "dataLicense": SPONSORBLOCK_DATA_LICENSE,
            "attribution": SPONSORBLOCK_ATTRIBUTION,
        }),
    })
}

fn normalized_categories(categories: &[String]) -> Vec<String> {
    let mut values = categories
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_k_anonymous_match_and_preserves_license() {
        let youtube_id = "ifI_fwg55k8";
        let hash = sha256_hex(youtube_id.as_bytes());
        let json = serde_json::json!([{
            "videoID": youtube_id,
            "hash": hash,
            "segments": [{
                "category": "intro",
                "segment": [0.0, 2.8],
                "UUID": "segment-1",
                "videoDuration": 1163.0
            }]
        }]);
        let snapshot = parse_hash_response(
            youtube_id,
            &["intro".to_string(), "sponsor".to_string()],
            &serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();

        assert_eq!(snapshot.hash_prefix.len(), 4);
        assert_eq!(snapshot.segments.len(), 1);
        assert_eq!(snapshot.segments[0].category, "intro");
        assert_eq!(
            snapshot.segments[0].metadata["dataLicense"],
            SPONSORBLOCK_DATA_LICENSE
        );
    }

    #[test]
    fn unrelated_hash_prefix_matches_are_ignored() {
        let youtube_id = "target-video";
        let json = serde_json::json!([{
            "videoID": "other-video",
            "hash": sha256_hex(b"other-video"),
            "segments": [{
                "category": "sponsor",
                "segment": [1.0, 2.0],
                "UUID": "other"
            }]
        }]);
        let snapshot = parse_hash_response(
            youtube_id,
            &["sponsor".to_string()],
            &serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(snapshot.segments.is_empty());
    }

    #[test]
    fn invalid_ranges_fail_closed() {
        let youtube_id = "target-video";
        let hash = sha256_hex(youtube_id.as_bytes());
        let json = serde_json::json!([{
            "videoID": youtube_id,
            "hash": hash,
            "segments": [{
                "category": "sponsor",
                "segment": [8.0, 2.0],
                "UUID": "bad"
            }]
        }]);
        assert!(
            parse_hash_response(
                youtube_id,
                &["sponsor".to_string()],
                &serde_json::to_vec(&json).unwrap(),
            )
            .is_err()
        );
    }
}
