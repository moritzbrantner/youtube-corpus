//! Corpus storage and findings statistics per video length (#200).
//!
//! `youtube-corpus stats` measures what the corpus costs and yields, normalised by video
//! length. It reads the Postgres catalog (table, TOAST and index sizes), sums
//! `pg_column_size` per row for every table that links rows to a video, walks the media and
//! output directory on disk, and folds in the latest SponsorBlock snapshot of each video.
//!
//! The command is read-only on the database except for the opt-in SponsorBlock fetch, which
//! stores snapshots through the existing [`crate::sponsorblock`] code path.
//!
//! Two byte measures are reported, and they answer different questions:
//!
//! - **Physical** bytes are what Postgres allocates (heap pages, TOAST, indexes). They are
//!   exact per table but cannot be split per video, so a table's heap and TOAST are divided
//!   between artifacts (row data, embedding vectors, tsvectors) in proportion to the column
//!   bytes, and each index goes to the artifact it serves.
//! - **Attributed** bytes are `pg_column_size` sums of the rows a video owns plus its files on
//!   disk. They are comparable between videos and buckets but leave out page overhead,
//!   indexes and tables without a `video_id`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::sponsorblock::{
    latest_sponsorblock_snapshot, refresh_sponsorblock_for_video, SponsorBlockSnapshot,
    DEFAULT_SPONSORBLOCK_CATEGORIES, SPONSORBLOCK_API_BASE, SPONSORBLOCK_ATTRIBUTION,
    SPONSORBLOCK_DATA_LICENSE,
};

pub const STATS_SCHEMA: &str = "youtube_corpus_stats@1";

/// Storage artifacts, in report order.
pub const ARTIFACTS: &[&str] = &[
    MEDIA,
    TRANSCRIPTS,
    EMBEDDINGS,
    SEARCH_INDEX,
    OCR,
    ANNOTATIONS,
    TRANSLATIONS,
    VISUAL_VOICE,
    SPONSORBLOCK,
    METADATA,
];
pub const MEDIA: &str = "media";
pub const TRANSCRIPTS: &str = "transcripts";
pub const EMBEDDINGS: &str = "embeddings";
pub const SEARCH_INDEX: &str = "search_index";
pub const OCR: &str = "ocr";
pub const ANNOTATIONS: &str = "annotations";
pub const TRANSLATIONS: &str = "translations";
pub const VISUAL_VOICE: &str = "visual_voice";
pub const SPONSORBLOCK: &str = "sponsorblock";
pub const METADATA: &str = "metadata";

/// Length buckets in minutes: `[lower, upper)`.
pub const LENGTH_BUCKETS: &[(&str, f64, f64)] = &[
    ("<10 min", 0.0, 10.0),
    ("10–30 min", 10.0, 30.0),
    ("30–60 min", 30.0, 60.0),
    ("60+ min", 60.0, f64::INFINITY),
];

/// Findings shown in the markdown report, with labels; the JSON carries every video-linked table.
pub const KEY_FINDINGS: &[(&str, &str)] = &[
    ("transcript_segments", "transcript segments"),
    ("transcript_streams", "transcript streams"),
    ("entity_mentions", "entity mentions"),
    ("visual_text_tracks", "OCR text tracks"),
    ("visual_text_observations", "OCR observations"),
    ("research_annotations", "research annotations"),
    ("historical_event_claims", "event claims"),
    ("person_name_evidence", "spoken-name evidence"),
    ("translated_segments", "translated segments"),
    ("video_scenes", "scenes"),
    ("face_tracks", "face tracks"),
    ("voice_tracks", "voice tracks"),
    ("sponsorblock_segments", "SponsorBlock segments"),
];

/// Which videos get a fresh SponsorBlock snapshot before the report is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SponsorBlockFetch {
    /// Use stored snapshots only (no network, no writes).
    #[default]
    Off,
    /// Fetch for videos without a stored snapshot.
    Missing,
    /// Fetch for every video with a YouTube id.
    All,
}

#[derive(Debug, Clone)]
pub struct CorpusStatsRequest {
    /// Per-item media and output directory written by ingest (`use-case-output/youtube-corpus`).
    pub work_dir: PathBuf,
    /// Shared directories measured as a whole (model bundles, search index), by label.
    pub shared_dirs: Vec<(String, PathBuf)>,
    pub sponsorblock: SponsorBlockFetch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStatsReport {
    pub schema: String,
    pub generated_at: String,
    pub sample: SampleStats,
    pub database: DatabaseStats,
    pub disk: DiskStats,
    pub artifacts: Vec<ArtifactStats>,
    pub embeddings: Vec<EmbeddingStats>,
    pub findings: Vec<FindingStats>,
    pub buckets: Vec<BucketStats>,
    pub sponsorblock: SponsorBlockStats,
    pub videos: Vec<VideoStats>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleStats {
    pub videos: usize,
    pub videos_with_duration: usize,
    pub videos_with_transcripts: usize,
    pub total_minutes: f64,
    pub transcribed_minutes: f64,
    pub duration_minutes: Spread,
}

/// Minimum, median and maximum of a per-video value.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spread {
    pub min: Option<f64>,
    pub median: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStats {
    pub database_bytes: i64,
    /// Sum of `pg_total_relation_size` over the public tables.
    pub table_bytes: i64,
    pub tables: Vec<TableStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStats {
    pub table: String,
    pub artifact: String,
    pub rows: i64,
    /// Dead tuples not yet vacuumed (`pg_stat_user_tables.n_dead_tup`); their bytes are in
    /// the heap and TOAST sizes.
    pub dead_rows: i64,
    pub links_video: bool,
    /// Main fork plus free-space and visibility maps.
    pub heap_bytes: i64,
    /// TOAST table and its index.
    pub toast_bytes: i64,
    pub index_bytes: i64,
    pub total_bytes: i64,
    /// `pg_column_size` of all rows (TOASTed values counted at their stored size).
    pub row_bytes: i64,
    pub vector_bytes: i64,
    pub tsvector_bytes: i64,
    pub indexes: Vec<IndexStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub name: String,
    pub method: String,
    pub artifact: String,
    pub bytes: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskStats {
    pub work_dir: String,
    pub work_dir_exists: bool,
    pub bytes: u64,
    pub files: u64,
    /// Bytes per file kind (audio, video, captions, metadata, media_other, other).
    pub bytes_by_kind: BTreeMap<String, u64>,
    /// Item directories that match no video row.
    pub unattributed_items: Vec<String>,
    pub unattributed_bytes: u64,
    pub shared: Vec<SharedDirStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedDirStats {
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactStats {
    pub artifact: String,
    /// Postgres bytes allocated to this artifact (heap/TOAST share plus its indexes).
    pub db_physical_bytes: i64,
    /// Row bytes of video-linked rows.
    pub db_attributed_bytes: i64,
    pub disk_bytes: u64,
    pub total_bytes: i64,
    pub videos_with_artifact: usize,
    /// `total_bytes` over all video minutes.
    pub bytes_per_video_minute: Option<f64>,
    /// `total_bytes` over the minutes of videos that have this artifact.
    pub bytes_per_covered_minute: Option<f64>,
    /// Attributed bytes per minute across videos that have this artifact.
    pub attributed_bytes_per_minute: Spread,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStats {
    pub table: String,
    pub column: String,
    pub column_type: String,
    pub model: String,
    pub vectors: i64,
    pub bytes: i64,
    /// Rows of the table without a vector in this column.
    pub rows_without_vector: i64,
    pub index_bytes: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingStats {
    pub table: String,
    pub label: String,
    pub total: i64,
    pub videos_with_rows: usize,
    pub per_video_mean: Option<f64>,
    pub per_video_minute: Option<f64>,
    pub per_minute: Spread,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketStats {
    pub bucket: String,
    pub videos: usize,
    pub minutes: f64,
    pub attributed_bytes_per_minute: Option<f64>,
    pub attributed_bytes_per_minute_by_artifact: BTreeMap<String, f64>,
    pub findings_per_minute: BTreeMap<String, f64>,
    pub sponsorblock_skippable_share: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockStats {
    pub source: String,
    pub license: String,
    pub attribution: String,
    pub fetch: SponsorBlockFetch,
    pub fetched: usize,
    pub fetch_errors: Vec<String>,
    pub videos_with_snapshot: usize,
    pub videos_with_segments: usize,
    /// Minutes of the videos that have a snapshot.
    pub snapshot_minutes: f64,
    /// Union of all skip segments, clipped to the video duration.
    pub skippable_seconds: f64,
    /// `skippable_seconds` over the duration of the videos with a snapshot.
    pub skippable_share: Option<f64>,
    pub categories: Vec<SponsorBlockCategoryStats>,
    pub savings: SponsorBlockSavings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockCategoryStats {
    pub category: String,
    pub segments: usize,
    pub seconds: f64,
    pub share: Option<f64>,
}

/// What skipping the SponsorBlock skip segments would have saved.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SponsorBlockSavings {
    /// Audio seconds that would not need transcription.
    pub transcription_seconds: f64,
    /// Transcript segments that lie mostly (≥ 50 %) inside a skip segment.
    pub transcript_segments: i64,
    pub transcript_segment_share: Option<f64>,
    /// Row bytes of those segments, including their vectors.
    pub transcript_row_bytes: i64,
    /// Embedding vectors (and their bytes) of those segments: embedding work saved.
    pub embedding_vectors: i64,
    pub embedding_bytes: i64,
    /// Media bytes on disk in proportion to the skipped share of each video.
    pub media_bytes_estimate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStats {
    pub video_id: Uuid,
    pub youtube_id: Option<String>,
    pub title: Option<String>,
    pub duration_seconds: Option<f64>,
    pub bucket: Option<String>,
    /// Attributed bytes (DB rows plus files on disk) per artifact.
    pub bytes_by_artifact: BTreeMap<String, i64>,
    pub attributed_bytes: i64,
    pub attributed_bytes_per_minute: Option<f64>,
    pub disk_bytes_by_kind: BTreeMap<String, u64>,
    /// Rows per video-linked table.
    pub counts: BTreeMap<String, i64>,
    pub sponsorblock: Option<VideoSponsorBlock>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSponsorBlock {
    pub segments: usize,
    pub skippable_seconds: f64,
    pub skippable_share: Option<f64>,
    pub seconds_by_category: BTreeMap<String, f64>,
    pub transcript_segments_skippable: i64,
}

/// The artifact a table's row data belongs to.
pub fn table_artifact(table: &str) -> &'static str {
    let starts = |prefix: &str| table.starts_with(prefix);
    if starts("transcript_") || table == "preferred_transcript_streams" {
        TRANSCRIPTS
    } else if starts("visual_text_") {
        OCR
    } else if starts("entity_")
        || starts("research_annotation")
        || starts("historical_event")
        || starts("person_name_")
    {
        ANNOTATIONS
    } else if starts("translated_") || starts("translation_") {
        TRANSLATIONS
    } else if starts("face_")
        || starts("voice_")
        || starts("person_face")
        || starts("person_voice")
        || starts("canonical_identit")
        || table == "people"
        || table == "video_scenes"
    {
        VISUAL_VOICE
    } else if starts("sponsorblock_") {
        SPONSORBLOCK
    } else {
        METADATA
    }
}

/// The artifact an index serves.
pub fn index_artifact(table: &str, method: &str, covers_tsvector: bool) -> &'static str {
    match method {
        "hnsw" | "ivfflat" => EMBEDDINGS,
        _ if covers_tsvector => SEARCH_INDEX,
        _ => table_artifact(table),
    }
}

/// The file kind of a path below an item directory.
pub fn disk_kind(relative: &Path) -> &'static str {
    let components = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>();
    let extension = relative
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let audio = [
        "m4a", "mp3", "opus", "ogg", "oga", "wav", "flac", "aac", "weba",
    ];
    let video = ["mp4", "webm", "mkv", "mov", "avi", "m4v"];
    let in_dir = |name: &str| components.iter().rev().skip(1).any(|c| *c == name);
    if audio.contains(&extension.as_str()) {
        "audio"
    } else if video.contains(&extension.as_str()) {
        "video"
    } else if in_dir("media") {
        "media_other"
    } else if in_dir("captions")
        || ["vtt", "srt", "ttml", "srv3", "json3"].contains(&extension.as_str())
    {
        "captions"
    } else if in_dir("metadata") {
        "metadata"
    } else {
        "other"
    }
}

/// The artifact a file kind counts towards.
pub fn disk_artifact(kind: &str) -> &'static str {
    match kind {
        "audio" | "video" | "media_other" => MEDIA,
        "captions" => TRANSCRIPTS,
        _ => METADATA,
    }
}

pub fn length_bucket(minutes: f64) -> &'static str {
    LENGTH_BUCKETS
        .iter()
        .find(|(_, lower, upper)| minutes >= *lower && minutes < *upper)
        .map(|(label, _, _)| *label)
        .unwrap_or(LENGTH_BUCKETS[LENGTH_BUCKETS.len() - 1].0)
}

pub fn spread(values: &[f64]) -> Spread {
    let mut values = values
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Spread::default();
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let median = if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    };
    Spread {
        min: Some(values[0]),
        median: Some(median),
        max: Some(values[n - 1]),
    }
}

/// Sorted, non-overlapping union of `[start, end)` ranges clipped to `[0, limit]`.
pub fn merge_ranges(ranges: &[(f64, f64)], limit: Option<f64>) -> Vec<(f64, f64)> {
    let mut clipped = ranges
        .iter()
        .filter_map(|&(start, end)| {
            let start = start.max(0.0);
            let end = limit.map_or(end, |limit| end.min(limit));
            (end > start).then_some((start, end))
        })
        .collect::<Vec<_>>();
    clipped.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in clipped {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

pub fn ranges_seconds(ranges: &[(f64, f64)]) -> f64 {
    non_negative_zero(ranges.iter().map(|(s, e)| e - s).sum())
}

/// Seconds of `[start, end)` covered by the merged `ranges`.
pub fn overlap_seconds(ranges: &[(f64, f64)], start: f64, end: f64) -> f64 {
    ranges
        .iter()
        .map(|&(s, e)| (end.min(e) - start.max(s)).max(0.0))
        .sum()
}

/// SponsorBlock segments a player skips: skip actions (or unlabelled), not points or labels.
pub fn is_skip_segment(category: &str, action_type: Option<&str>) -> bool {
    category != "poi_highlight" && matches!(action_type, None | Some("skip"))
}

/// `x + 0.0` turns the `-0.0` an empty `f64` sum yields into `0.0`.
fn non_negative_zero(value: f64) -> f64 {
    value + 0.0
}

fn ratio(numerator: f64, denominator: f64) -> Option<f64> {
    (denominator > 0.0).then(|| numerator / denominator)
}

/// Dynamic SQL in this module only interpolates catalog identifiers quoted here.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

struct Column {
    name: String,
    type_name: String,
    formatted: String,
}

#[derive(Default)]
struct VideoRows {
    rows: i64,
    row_bytes: i64,
    vector_bytes: i64,
    tsvector_bytes: i64,
}

/// A SponsorBlock range of one video: `(video, start, end)`.
type VideoRange = (Uuid, f64, f64);

struct VideoRow {
    id: Uuid,
    youtube_id: Option<String>,
    source_url: String,
    title: Option<String>,
    duration_seconds: Option<f64>,
}

pub async fn collect_corpus_stats(
    pool: &PgPool,
    request: &CorpusStatsRequest,
) -> anyhow::Result<CorpusStatsReport> {
    let video_rows = sqlx::query(
        "SELECT id, youtube_id, source_url, title, duration_seconds FROM videos ORDER BY id",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(VideoRow {
            id: row.try_get("id")?,
            youtube_id: row.try_get("youtube_id")?,
            source_url: row.try_get("source_url")?,
            title: row.try_get("title")?,
            duration_seconds: row
                .try_get::<Option<f64>, _>("duration_seconds")?
                .filter(|d| d.is_finite() && *d > 0.0),
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;

    let mut sponsorblock = SponsorBlockStats {
        source: SPONSORBLOCK_API_BASE.to_string(),
        license: SPONSORBLOCK_DATA_LICENSE.to_string(),
        attribution: SPONSORBLOCK_ATTRIBUTION.to_string(),
        fetch: request.sponsorblock,
        ..Default::default()
    };
    let categories = DEFAULT_SPONSORBLOCK_CATEGORIES
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>();
    let mut snapshots: HashMap<Uuid, SponsorBlockSnapshot> = HashMap::new();
    for video in &video_rows {
        let stored = latest_sponsorblock_snapshot(pool, video.id).await?;
        let has_youtube_id = video
            .youtube_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty());
        let fetch = has_youtube_id
            && match request.sponsorblock {
                SponsorBlockFetch::Off => false,
                SponsorBlockFetch::Missing => stored.is_none(),
                SponsorBlockFetch::All => true,
            };
        if fetch {
            match refresh_sponsorblock_for_video(pool, video.id, &categories).await {
                Ok(snapshot) => {
                    sponsorblock.fetched += 1;
                    snapshots.insert(video.id, snapshot);
                    continue;
                }
                Err(error) => sponsorblock.fetch_errors.push(format!(
                    "{}: {error:#}",
                    video.youtube_id.as_deref().unwrap_or_default()
                )),
            }
        }
        if let Some((_, snapshot)) = stored {
            snapshots.insert(video.id, snapshot);
        }
    }

    // Catalog: columns, sizes, indexes.
    let mut columns: BTreeMap<String, Vec<Column>> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT c.relname::text AS table_name, a.attname::text AS column_name,
                t.typname::text AS type_name, format_type(a.atttypid, a.atttypmod) AS formatted
         FROM pg_attribute a
         JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_type t ON t.oid = a.atttypid
         WHERE n.nspname = 'public' AND c.relkind = 'r' AND a.attnum > 0 AND NOT a.attisdropped
         ORDER BY c.relname, a.attnum",
    )
    .fetch_all(pool)
    .await?
    {
        columns
            .entry(row.try_get("table_name")?)
            .or_default()
            .push(Column {
                name: row.try_get("column_name")?,
                type_name: row.try_get("type_name")?,
                formatted: row.try_get("formatted")?,
            });
    }

    let mut tables: BTreeMap<String, TableStats> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT c.relname::text AS table_name,
                pg_total_relation_size(c.oid)::bigint AS total_bytes,
                coalesce(pg_total_relation_size(nullif(c.reltoastrelid, 0)), 0)::bigint AS toast_bytes,
                pg_indexes_size(c.oid)::bigint AS index_bytes,
                coalesce(s.n_dead_tup, 0)::bigint AS dead_rows
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid
         WHERE n.nspname = 'public' AND c.relkind = 'r'",
    )
    .fetch_all(pool)
    .await?
    {
        let table: String = row.try_get("table_name")?;
        let total_bytes: i64 = row.try_get("total_bytes")?;
        let toast_bytes: i64 = row.try_get("toast_bytes")?;
        let index_bytes: i64 = row.try_get("index_bytes")?;
        tables.insert(
            table.clone(),
            TableStats {
                artifact: table_artifact(&table).to_string(),
                table,
                heap_bytes: total_bytes - toast_bytes - index_bytes,
                toast_bytes,
                index_bytes,
                total_bytes,
                dead_rows: row.try_get("dead_rows")?,
                ..Default::default()
            },
        );
    }
    for row in sqlx::query(
        "SELECT t.relname::text AS table_name, ic.relname::text AS index_name,
                am.amname::text AS method, pg_relation_size(ic.oid)::bigint AS bytes,
                EXISTS (
                  SELECT 1 FROM unnest(i.indkey::int2[]) AS k(attnum)
                  JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
                  WHERE a.atttypid = 'tsvector'::regtype
                ) AS covers_tsvector
         FROM pg_index i
         JOIN pg_class ic ON ic.oid = i.indexrelid
         JOIN pg_class t ON t.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         JOIN pg_am am ON am.oid = ic.relam
         WHERE n.nspname = 'public' AND t.relkind = 'r'
         ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await?
    {
        let table: String = row.try_get("table_name")?;
        let method: String = row.try_get("method")?;
        let covers_tsvector: bool = row.try_get("covers_tsvector")?;
        if let Some(stats) = tables.get_mut(&table) {
            stats.indexes.push(IndexStats {
                name: row.try_get("index_name")?,
                artifact: index_artifact(&table, &method, covers_tsvector).to_string(),
                method,
                bytes: row.try_get("bytes")?,
            });
        }
    }
    let has_embedding_models = tables.contains_key("embedding_models");

    // Row bytes per table, grouped by video where the table links one.
    let mut per_video: BTreeMap<Uuid, BTreeMap<String, VideoRows>> = BTreeMap::new();
    let mut embeddings = Vec::new();
    for (table, stats) in tables.iter_mut() {
        let table_columns = columns.get(table).map(Vec::as_slice).unwrap_or(&[]);
        let links_video = table_columns
            .iter()
            .any(|c| c.name == "video_id" && c.type_name == "uuid");
        stats.links_video = links_video;
        let vectors = table_columns
            .iter()
            .filter(|c| c.type_name == "vector")
            .collect::<Vec<_>>();
        let tsvectors = table_columns
            .iter()
            .filter(|c| c.type_name == "tsvector")
            .collect::<Vec<_>>();
        let sum_columns = |cols: &[&Column]| {
            if cols.is_empty() {
                "0::bigint".to_string()
            } else {
                cols.iter()
                    .map(|c| {
                        format!(
                            "coalesce(sum(pg_column_size(t.{})), 0)",
                            quote_ident(&c.name)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" + ")
                    + "::bigint"
            }
        };
        let group = if links_video {
            "t.video_id"
        } else {
            "NULL::uuid"
        };
        let sql = format!(
            "SELECT {group} AS video_id, count(*)::bigint AS rows,
                    coalesce(sum(pg_column_size(t.*)), 0)::bigint AS row_bytes,
                    ({})::bigint AS vector_bytes, ({})::bigint AS tsvector_bytes
             FROM {} t GROUP BY 1",
            sum_columns(&vectors),
            sum_columns(&tsvectors),
            quote_ident(table),
        );
        for row in sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await?
        {
            let video_id: Option<Uuid> = row.try_get("video_id")?;
            let rows: i64 = row.try_get("rows")?;
            let row_bytes: i64 = row.try_get("row_bytes")?;
            let vector_bytes: i64 = row.try_get("vector_bytes")?;
            let tsvector_bytes: i64 = row.try_get("tsvector_bytes")?;
            stats.rows += rows;
            stats.row_bytes += row_bytes;
            stats.vector_bytes += vector_bytes;
            stats.tsvector_bytes += tsvector_bytes;
            if let Some(video_id) = video_id {
                per_video.entry(video_id).or_default().insert(
                    table.clone(),
                    VideoRows {
                        rows,
                        row_bytes,
                        vector_bytes,
                        tsvector_bytes,
                    },
                );
            }
        }

        for column in vectors {
            let col = quote_ident(&column.name);
            let labelled =
                has_embedding_models && table_columns.iter().any(|c| c.name == "embedding_model");
            let (model_expr, join) = if labelled {
                (
                    "coalesce(m.model_id || '@' || m.revision || ' (' || m.pooling || ')', 'no model row')",
                    "LEFT JOIN embedding_models m ON m.id = t.embedding_model",
                )
            } else {
                ("'unlabelled'", "")
            };
            let sql = format!(
                "SELECT {model_expr} AS model, count(*)::bigint AS vectors,
                        coalesce(sum(pg_column_size(t.{col})), 0)::bigint AS bytes
                 FROM {} t {join} WHERE t.{col} IS NOT NULL GROUP BY 1 ORDER BY 1",
                quote_ident(table)
            );
            let mut column_vectors = 0;
            let mut models = Vec::new();
            for row in sqlx::query(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await?
            {
                let vectors: i64 = row.try_get("vectors")?;
                column_vectors += vectors;
                models.push(EmbeddingStats {
                    table: table.clone(),
                    column: column.name.clone(),
                    column_type: column.formatted.clone(),
                    model: row.try_get("model")?,
                    vectors,
                    bytes: row.try_get("bytes")?,
                    ..Default::default()
                });
            }
            let index_bytes = stats
                .indexes
                .iter()
                .filter(|i| i.artifact == EMBEDDINGS)
                .map(|i| i.bytes)
                .sum::<i64>();
            if models.is_empty() {
                models.push(EmbeddingStats {
                    table: table.clone(),
                    column: column.name.clone(),
                    column_type: column.formatted.clone(),
                    model: "none".to_string(),
                    ..Default::default()
                });
            }
            for model in &mut models {
                model.rows_without_vector = stats.rows - column_vectors;
                model.index_bytes = index_bytes;
            }
            embeddings.extend(models);
        }
    }
    // Physical bytes per artifact.
    let mut physical: BTreeMap<&'static str, i64> = BTreeMap::new();
    for stats in tables.values() {
        let data_bytes = stats.heap_bytes + stats.toast_bytes;
        let artifact = table_artifact(&stats.table);
        if stats.row_bytes > 0 {
            let vector_part = (data_bytes as f64 * stats.vector_bytes as f64
                / stats.row_bytes as f64)
                .round() as i64;
            let tsvector_part = (data_bytes as f64 * stats.tsvector_bytes as f64
                / stats.row_bytes as f64)
                .round() as i64;
            *physical.entry(EMBEDDINGS).or_default() += vector_part;
            *physical.entry(SEARCH_INDEX).or_default() += tsvector_part;
            *physical.entry(artifact).or_default() += data_bytes - vector_part - tsvector_part;
        } else {
            *physical.entry(artifact).or_default() += data_bytes;
        }
        for index in &stats.indexes {
            let artifact = ARTIFACTS
                .iter()
                .find(|a| **a == index.artifact)
                .copied()
                .unwrap_or(METADATA);
            *physical.entry(artifact).or_default() += index.bytes;
        }
        // pg_indexes_size can include bytes not listed per index (none in practice).
        let listed = stats.indexes.iter().map(|i| i.bytes).sum::<i64>();
        *physical.entry(artifact).or_default() += stats.index_bytes - listed;
    }

    // Disk.
    let mut item_to_video: HashMap<String, Uuid> = HashMap::new();
    for video in &video_rows {
        item_to_video.insert(crate::youtube::sanitize_id(&video.source_url), video.id);
        match video
            .youtube_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
        {
            Some(youtube_id) => {
                item_to_video.insert(crate::youtube::sanitize_id(youtube_id), video.id);
            }
            // Local-file ingests write under the sanitized file stem (`local_file_item`).
            None => {
                let item_id =
                    crate::youtube::local_file_item(PathBuf::from(&video.source_url)).item_id;
                item_to_video.entry(item_id).or_insert(video.id);
            }
        }
    }
    let mut disk = DiskStats {
        work_dir: display_path(&request.work_dir),
        work_dir_exists: request.work_dir.is_dir(),
        ..Default::default()
    };
    let mut disk_per_video: HashMap<Uuid, BTreeMap<String, u64>> = HashMap::new();
    if disk.work_dir_exists {
        let mut entries = std::fs::read_dir(&request.work_dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let video = item_to_video.get(&name).copied();
            let mut files = Vec::new();
            walk_files(&entry.path(), &mut files)?;
            let mut item_bytes = 0;
            for (path, bytes) in files {
                let relative = path.strip_prefix(entry.path()).unwrap_or(&path);
                let kind = disk_kind(relative);
                disk.files += 1;
                disk.bytes += bytes;
                item_bytes += bytes;
                *disk.bytes_by_kind.entry(kind.to_string()).or_default() += bytes;
                if let Some(video) = video {
                    *disk_per_video
                        .entry(video)
                        .or_default()
                        .entry(kind.to_string())
                        .or_default() += bytes;
                }
            }
            if video.is_none() {
                disk.unattributed_items.push(name);
                disk.unattributed_bytes += item_bytes;
            }
        }
    }
    for (label, path) in &request.shared_dirs {
        let mut files = Vec::new();
        let exists = path.is_dir();
        if exists {
            walk_files(path, &mut files)?;
        }
        disk.shared.push(SharedDirStats {
            label: label.clone(),
            path: display_path(path),
            exists,
            files: files.len() as u64,
            bytes: files.iter().map(|(_, b)| b).sum(),
        });
    }

    // SponsorBlock ranges and per-segment savings.
    let mut skip_ranges: HashMap<Uuid, Vec<(f64, f64)>> = HashMap::new();
    let mut category_ranges: BTreeMap<String, (usize, Vec<VideoRange>)> = BTreeMap::new();
    for video in &video_rows {
        let Some(snapshot) = snapshots.get(&video.id) else {
            continue;
        };
        // Without a duration the ranges cannot be clipped or put in proportion; such videos
        // stay out of every SponsorBlock numerator and denominator.
        if video.duration_seconds.is_none() {
            continue;
        }
        let skips = snapshot
            .segments
            .iter()
            .filter(|s| is_skip_segment(&s.category, s.action_type.as_deref()))
            .collect::<Vec<_>>();
        for segment in &skips {
            let entry = category_ranges.entry(segment.category.clone()).or_default();
            entry.0 += 1;
            entry
                .1
                .push((video.id, segment.start_seconds, segment.end_seconds));
        }
        let ranges = skips
            .iter()
            .map(|s| (s.start_seconds, s.end_seconds))
            .collect::<Vec<_>>();
        skip_ranges.insert(video.id, merge_ranges(&ranges, video.duration_seconds));
    }
    let mut segment_savings: HashMap<Uuid, (i64, i64, i64, i64)> = HashMap::new();
    let skip_video_ids = skip_ranges
        .iter()
        .filter(|(_, r)| !r.is_empty())
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if !skip_video_ids.is_empty() && tables.contains_key("transcript_segments") {
        let has_embedding = columns
            .get("transcript_segments")
            .is_some_and(|cols| cols.iter().any(|c| c.name == "embedding"));
        let embedding_expr = if has_embedding {
            "coalesce(pg_column_size(s.embedding), 0)::bigint"
        } else {
            "0::bigint"
        };
        let sql = format!(
            "SELECT s.video_id, s.start_seconds, s.end_seconds,
                    pg_column_size(s.*)::bigint AS row_bytes, {embedding_expr} AS embedding_bytes
             FROM transcript_segments s WHERE s.video_id = ANY($1)"
        );
        for row in sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&skip_video_ids)
            .fetch_all(pool)
            .await?
        {
            let video_id: Uuid = row.try_get("video_id")?;
            let (Some(start), Some(end)) = (
                row.try_get::<Option<f64>, _>("start_seconds")?,
                row.try_get::<Option<f64>, _>("end_seconds")?,
            ) else {
                continue;
            };
            let ranges = &skip_ranges[&video_id];
            let length = end - start;
            let covered = overlap_seconds(ranges, start, end);
            let inside = if length > 0.0 {
                covered / length >= 0.5
            } else {
                covered > 0.0 || ranges.iter().any(|&(s, e)| start >= s && start < e)
            };
            if inside {
                let embedding_bytes: i64 = row.try_get("embedding_bytes")?;
                let entry = segment_savings.entry(video_id).or_default();
                entry.0 += 1;
                entry.1 += row.try_get::<i64, _>("row_bytes")?;
                entry.2 += i64::from(embedding_bytes > 0);
                entry.3 += embedding_bytes;
            }
        }
    }

    // Per-video assembly.
    let mut videos = Vec::new();
    for video in &video_rows {
        let rows = per_video.remove(&video.id).unwrap_or_default();
        let disk_kinds = disk_per_video.remove(&video.id).unwrap_or_default();
        let mut bytes_by_artifact: BTreeMap<String, i64> = BTreeMap::new();
        let mut counts = BTreeMap::new();
        for (table, r) in &rows {
            counts.insert(table.clone(), r.rows);
            *bytes_by_artifact
                .entry(table_artifact(table).to_string())
                .or_default() += r.row_bytes - r.vector_bytes - r.tsvector_bytes;
            *bytes_by_artifact.entry(EMBEDDINGS.to_string()).or_default() += r.vector_bytes;
            *bytes_by_artifact
                .entry(SEARCH_INDEX.to_string())
                .or_default() += r.tsvector_bytes;
        }
        for (kind, bytes) in &disk_kinds {
            *bytes_by_artifact
                .entry(disk_artifact(kind).to_string())
                .or_default() += *bytes as i64;
        }
        bytes_by_artifact.retain(|_, b| *b > 0);
        let attributed_bytes = bytes_by_artifact.values().sum::<i64>();
        let minutes = video.duration_seconds.map(|d| d / 60.0);
        let sponsorblock = snapshots.get(&video.id).map(|snapshot| {
            let ranges = skip_ranges.get(&video.id).cloned().unwrap_or_default();
            let skippable_seconds = ranges_seconds(&ranges);
            let mut seconds_by_category = BTreeMap::new();
            for (category, (_, all)) in &category_ranges {
                let own = all
                    .iter()
                    .filter(|(id, _, _)| *id == video.id)
                    .map(|(_, s, e)| (*s, *e))
                    .collect::<Vec<_>>();
                if !own.is_empty() {
                    seconds_by_category.insert(
                        category.clone(),
                        ranges_seconds(&merge_ranges(&own, video.duration_seconds)),
                    );
                }
            }
            VideoSponsorBlock {
                segments: snapshot.segments.len(),
                skippable_seconds,
                skippable_share: video
                    .duration_seconds
                    .and_then(|d| ratio(skippable_seconds, d)),
                seconds_by_category,
                transcript_segments_skippable: segment_savings.get(&video.id).map_or(0, |s| s.0),
            }
        });
        videos.push(VideoStats {
            video_id: video.id,
            youtube_id: video.youtube_id.clone(),
            title: video.title.clone(),
            duration_seconds: video.duration_seconds,
            bucket: minutes.map(|m| length_bucket(m).to_string()),
            attributed_bytes_per_minute: minutes.and_then(|m| ratio(attributed_bytes as f64, m)),
            bytes_by_artifact,
            attributed_bytes,
            disk_bytes_by_kind: disk_kinds,
            counts,
            sponsorblock,
        });
    }
    videos.sort_by(|a, b| {
        a.duration_seconds
            .unwrap_or(0.0)
            .total_cmp(&b.duration_seconds.unwrap_or(0.0))
            .then(a.video_id.cmp(&b.video_id))
    });

    let minutes_of = |v: &VideoStats| v.duration_seconds.map(|d| d / 60.0);
    let total_minutes = non_negative_zero(videos.iter().filter_map(minutes_of).sum::<f64>());
    let transcribed =
        |v: &VideoStats| v.counts.get("transcript_segments").copied().unwrap_or(0) > 0;
    let sample = SampleStats {
        videos: videos.len(),
        videos_with_duration: videos
            .iter()
            .filter(|v| v.duration_seconds.is_some())
            .count(),
        videos_with_transcripts: videos.iter().filter(|v| transcribed(v)).count(),
        total_minutes,
        transcribed_minutes: videos
            .iter()
            .filter(|v| transcribed(v))
            .filter_map(minutes_of)
            .sum(),
        duration_minutes: spread(&videos.iter().filter_map(minutes_of).collect::<Vec<_>>()),
    };

    let artifacts = ARTIFACTS
        .iter()
        .map(|artifact| {
            let covered = videos
                .iter()
                .filter(|v| v.bytes_by_artifact.contains_key(*artifact))
                .collect::<Vec<_>>();
            let db_attributed_bytes = videos
                .iter()
                .flat_map(|v| v.bytes_by_artifact.get(*artifact))
                .sum::<i64>()
                - videos
                    .iter()
                    .flat_map(|v| v.disk_bytes_by_kind.iter())
                    .filter(|(kind, _)| disk_artifact(kind) == *artifact)
                    .map(|(_, b)| *b as i64)
                    .sum::<i64>();
            let disk_bytes = disk
                .bytes_by_kind
                .iter()
                .filter(|(kind, _)| disk_artifact(kind) == *artifact)
                .map(|(_, b)| *b)
                .sum::<u64>();
            let db_physical_bytes = physical.get(artifact).copied().unwrap_or(0);
            let total_bytes = db_physical_bytes + disk_bytes as i64;
            let covered_minutes = covered.iter().filter_map(|v| minutes_of(v)).sum::<f64>();
            ArtifactStats {
                artifact: artifact.to_string(),
                db_physical_bytes,
                db_attributed_bytes,
                disk_bytes,
                total_bytes,
                videos_with_artifact: covered.len(),
                bytes_per_video_minute: ratio(total_bytes as f64, total_minutes),
                bytes_per_covered_minute: ratio(total_bytes as f64, covered_minutes),
                attributed_bytes_per_minute: spread(
                    &covered
                        .iter()
                        .filter_map(|v| {
                            minutes_of(v)
                                .and_then(|m| ratio(v.bytes_by_artifact[*artifact] as f64, m))
                        })
                        .collect::<Vec<_>>(),
                ),
            }
        })
        .collect::<Vec<_>>();

    let video_tables = tables
        .values()
        .filter(|t| t.links_video)
        .map(|t| t.table.clone())
        .collect::<BTreeSet<_>>();
    let label_of = |table: &str| {
        KEY_FINDINGS
            .iter()
            .find(|(t, _)| *t == table)
            .map(|(_, l)| l.to_string())
            .unwrap_or_else(|| table.replace('_', " "))
    };
    let with_duration = videos
        .iter()
        .filter(|v| v.duration_seconds.is_some())
        .collect::<Vec<_>>();
    let findings = video_tables
        .iter()
        .map(|table| {
            let total = videos
                .iter()
                .map(|v| v.counts.get(table).copied().unwrap_or(0))
                .sum::<i64>();
            FindingStats {
                table: table.clone(),
                label: label_of(table),
                total,
                videos_with_rows: videos
                    .iter()
                    .filter(|v| v.counts.get(table).copied().unwrap_or(0) > 0)
                    .count(),
                per_video_mean: ratio(total as f64, videos.len() as f64),
                per_video_minute: ratio(
                    with_duration
                        .iter()
                        .map(|v| v.counts.get(table).copied().unwrap_or(0))
                        .sum::<i64>() as f64,
                    total_minutes,
                ),
                per_minute: spread(
                    &with_duration
                        .iter()
                        .filter_map(|v| {
                            ratio(
                                v.counts.get(table).copied().unwrap_or(0) as f64,
                                minutes_of(v)?,
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            }
        })
        .collect::<Vec<_>>();

    let buckets = LENGTH_BUCKETS
        .iter()
        .map(|(label, _, _)| {
            let members = with_duration
                .iter()
                .filter(|v| v.bucket.as_deref() == Some(*label))
                .collect::<Vec<_>>();
            let minutes =
                non_negative_zero(members.iter().filter_map(|v| minutes_of(v)).sum::<f64>());
            let mut by_artifact = BTreeMap::new();
            for artifact in ARTIFACTS {
                let bytes = members
                    .iter()
                    .flat_map(|v| v.bytes_by_artifact.get(*artifact))
                    .sum::<i64>();
                if bytes > 0 {
                    if let Some(rate) = ratio(bytes as f64, minutes) {
                        by_artifact.insert(artifact.to_string(), rate);
                    }
                }
            }
            let mut findings_per_minute = BTreeMap::new();
            for table in &video_tables {
                let count = members
                    .iter()
                    .map(|v| v.counts.get(table).copied().unwrap_or(0))
                    .sum::<i64>();
                if count > 0 {
                    if let Some(rate) = ratio(count as f64, minutes) {
                        findings_per_minute.insert(table.clone(), rate);
                    }
                }
            }
            let snapshot_members = members
                .iter()
                .filter(|v| v.sponsorblock.is_some())
                .collect::<Vec<_>>();
            BucketStats {
                bucket: label.to_string(),
                videos: members.len(),
                minutes,
                attributed_bytes_per_minute: ratio(
                    members.iter().map(|v| v.attributed_bytes).sum::<i64>() as f64,
                    minutes,
                ),
                attributed_bytes_per_minute_by_artifact: by_artifact,
                findings_per_minute,
                sponsorblock_skippable_share: ratio(
                    snapshot_members
                        .iter()
                        .filter_map(|v| v.sponsorblock.as_ref())
                        .map(|s| s.skippable_seconds)
                        .sum(),
                    snapshot_members
                        .iter()
                        .filter_map(|v| v.duration_seconds)
                        .sum(),
                ),
            }
        })
        .collect::<Vec<_>>();

    // SponsorBlock totals.
    let snapshot_videos = videos
        .iter()
        .filter(|v| v.sponsorblock.is_some() && v.duration_seconds.is_some())
        .collect::<Vec<_>>();
    let undated_snapshots = videos
        .iter()
        .filter(|v| v.sponsorblock.is_some() && v.duration_seconds.is_none())
        .count();
    let snapshot_seconds = snapshot_videos
        .iter()
        .filter_map(|v| v.duration_seconds)
        .sum::<f64>();
    sponsorblock.videos_with_snapshot = snapshot_videos.len();
    sponsorblock.videos_with_segments = snapshot_videos
        .iter()
        .filter(|v| v.sponsorblock.as_ref().is_some_and(|s| s.segments > 0))
        .count();
    let snapshot_seconds = non_negative_zero(snapshot_seconds);
    sponsorblock.snapshot_minutes = snapshot_seconds / 60.0;
    sponsorblock.skippable_seconds = non_negative_zero(
        snapshot_videos
            .iter()
            .filter_map(|v| v.sponsorblock.as_ref())
            .map(|s| s.skippable_seconds)
            .sum(),
    );
    sponsorblock.skippable_share = ratio(sponsorblock.skippable_seconds, snapshot_seconds);
    sponsorblock.categories = category_ranges
        .iter()
        .map(|(category, (segments, _))| {
            let seconds = snapshot_videos
                .iter()
                .filter_map(|v| v.sponsorblock.as_ref())
                .filter_map(|s| s.seconds_by_category.get(category))
                .sum::<f64>();
            SponsorBlockCategoryStats {
                category: category.clone(),
                segments: *segments,
                seconds,
                share: ratio(seconds, snapshot_seconds),
            }
        })
        .collect();
    let snapshot_segments = snapshot_videos
        .iter()
        .map(|v| v.counts.get("transcript_segments").copied().unwrap_or(0))
        .sum::<i64>();
    let mut savings = SponsorBlockSavings {
        transcription_seconds: sponsorblock.skippable_seconds,
        ..Default::default()
    };
    for (rows, bytes, vectors, vector_bytes) in segment_savings.values() {
        savings.transcript_segments += rows;
        savings.transcript_row_bytes += bytes;
        savings.embedding_vectors += vectors;
        savings.embedding_bytes += vector_bytes;
    }
    savings.transcript_segment_share =
        ratio(savings.transcript_segments as f64, snapshot_segments as f64);
    savings.media_bytes_estimate = non_negative_zero(
        snapshot_videos
            .iter()
            .map(|v| {
                let media = v.bytes_by_artifact.get(MEDIA).copied().unwrap_or(0) as f64;
                media
                    * v.sponsorblock
                        .as_ref()
                        .and_then(|s| s.skippable_share)
                        .unwrap_or(0.0)
            })
            .sum::<f64>(),
    );
    sponsorblock.savings = savings;

    let mut notes = Vec::new();
    if undated_snapshots > 0 {
        notes.push(format!(
            "{undated_snapshots} videos with a SponsorBlock snapshot have no known duration and are left out of the SponsorBlock shares and savings."
        ));
    }
    if sample.videos < 30 {
        notes.push(format!(
            "Small sample: {} videos ({:.1} h). Per-minute averages and bucket figures are indicative only; buckets with one or two videos describe those videos, not the bucket.",
            sample.videos,
            sample.total_minutes / 60.0
        ));
    }
    if sample.videos_with_transcripts < sample.videos {
        notes.push(format!(
            "{} of {} videos have no transcript segments; per-video-minute figures divide by all {:.1} video minutes, per-covered-minute figures only by the minutes of videos that have the artifact.",
            sample.videos - sample.videos_with_transcripts,
            sample.videos,
            sample.total_minutes
        ));
    }
    if disk.bytes_by_kind.keys().all(|k| disk_artifact(k) != MEDIA) {
        notes.push("No media (audio/video) files are on disk: ingest keeps captions and metadata, and workflow runs release downloaded media after processing.".to_string());
    }
    let dead_rows = tables.values().map(|t| t.dead_rows).sum::<i64>();
    let live_rows = tables.values().map(|t| t.rows).sum::<i64>();
    if dead_rows * 20 > live_rows {
        notes.push(format!(
            "{dead_rows} dead tuples (vs {live_rows} live rows) are not vacuumed yet; physical heap and TOAST bytes include them until autovacuum or VACUUM reclaims the space."
        ));
    }
    let database_bytes: i64 =
        sqlx::query_scalar("SELECT pg_database_size(current_database())::bigint")
            .fetch_one(pool)
            .await?;
    let table_bytes = tables.values().map(|t| t.total_bytes).sum();

    Ok(CorpusStatsReport {
        schema: STATS_SCHEMA.to_string(),
        generated_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        sample,
        database: DatabaseStats {
            database_bytes,
            table_bytes,
            tables: tables.into_values().collect(),
        },
        disk,
        artifacts,
        embeddings,
        findings,
        buckets,
        sponsorblock,
        videos,
        notes,
    })
}

/// A path for the report, with the home directory shown as `~`.
fn display_path(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if !home.as_os_str().is_empty() => match path.strip_prefix(&home) {
            Ok(rest) => Path::new("~").join(rest).display().to_string(),
            Err(_) => path.display().to_string(),
        },
        _ => path.display().to_string(),
    }
}

fn walk_files(dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> anyhow::Result<()> {
    let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk_files(&entry.path(), out)?;
        } else if file_type.is_file() {
            out.push((entry.path(), entry.metadata()?.len()));
        }
    }
    Ok(())
}

/// `1234567` → `1.18 MiB`.
pub fn human_bytes(bytes: f64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes;
    let mut unit = 0;
    while value.abs() >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn opt_bytes(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_string(), human_bytes)
}

fn opt_num(value: Option<f64>, digits: usize) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{v:.digits$}"))
}

fn opt_pct(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{:.1} %", v * 100.0))
}

fn spread_bytes(spread: &Spread) -> String {
    match (spread.min, spread.median, spread.max) {
        (Some(min), Some(median), Some(max)) => format!(
            "{} / {} / {}",
            human_bytes(min),
            human_bytes(median),
            human_bytes(max)
        ),
        _ => "–".to_string(),
    }
}

fn spread_num(spread: &Spread, digits: usize) -> String {
    match (spread.min, spread.median, spread.max) {
        (Some(min), Some(median), Some(max)) => {
            format!("{min:.digits$} / {median:.digits$} / {max:.digits$}")
        }
        _ => "–".to_string(),
    }
}

/// Renders the report as the markdown committed in `docs/corpus-stats.md`.
pub fn render_markdown(report: &CorpusStatsReport) -> String {
    let mut md = String::new();
    let sample = &report.sample;
    let _ = writeln!(md, "# Corpus storage and findings statistics\n");
    let _ = writeln!(
        md,
        "Generated by `youtube-corpus stats` at {} (schema `{}`). Re-run it to refresh these numbers; see [How to re-run](#how-to-re-run).\n",
        report.generated_at, report.schema
    );
    let _ = writeln!(md, "## Sample\n");
    let _ = writeln!(
        md,
        "- {} videos, {:.1} minutes ({:.2} h) in total; {} with a known duration.",
        sample.videos,
        sample.total_minutes,
        sample.total_minutes / 60.0,
        sample.videos_with_duration
    );
    let _ = writeln!(
        md,
        "- {} videos ({:.1} minutes) have transcript segments.",
        sample.videos_with_transcripts, sample.transcribed_minutes
    );
    let _ = writeln!(
        md,
        "- Duration min / median / max: {} minutes.",
        spread_num(&sample.duration_minutes, 1)
    );
    for note in &report.notes {
        let _ = writeln!(md, "- {note}");
    }

    let _ = writeln!(md, "\n## Storage per artifact\n");
    let _ = writeln!(
        md,
        "Physical bytes are what Postgres allocates (heap and TOAST split between row data, vectors and tsvectors by column bytes; each index goes to the artifact it serves) plus files on disk. Attributed bytes are `pg_column_size` sums of rows linked to a video plus its files; they leave out page overhead, indexes and tables without `video_id`.\n"
    );
    let _ = writeln!(
        md,
        "| Artifact | DB physical | Disk | Total | Per video-minute | Per covered minute | Videos | Attributed per minute (min / median / max) |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---|");
    for a in &report.artifacts {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            a.artifact,
            human_bytes(a.db_physical_bytes as f64),
            human_bytes(a.disk_bytes as f64),
            human_bytes(a.total_bytes as f64),
            opt_bytes(a.bytes_per_video_minute),
            opt_bytes(a.bytes_per_covered_minute),
            a.videos_with_artifact,
            spread_bytes(&a.attributed_bytes_per_minute)
        );
    }
    let total = report.artifacts.iter().map(|a| a.total_bytes).sum::<i64>();
    let _ = writeln!(
        md,
        "| **all** | {} | {} | {} | {} | | | |",
        human_bytes(
            report
                .artifacts
                .iter()
                .map(|a| a.db_physical_bytes)
                .sum::<i64>() as f64
        ),
        human_bytes(report.disk.bytes as f64),
        human_bytes(total as f64),
        opt_bytes(ratio(total as f64, sample.total_minutes))
    );
    let _ = writeln!(
        md,
        "\nDatabase size (`pg_database_size`, includes catalogs and free space): {}; public tables: {}.",
        human_bytes(report.database.database_bytes as f64),
        human_bytes(report.database.table_bytes as f64)
    );

    let _ = writeln!(md, "\n### Embeddings\n");
    let _ = writeln!(
        md,
        "| Table.column | Type | Model | Vectors | Rows without | Vector bytes | Vector index |"
    );
    let _ = writeln!(md, "|---|---|---|---:|---:|---:|---:|");
    for e in &report.embeddings {
        let _ = writeln!(
            md,
            "| {}.{} | {} | {} | {} | {} | {} | {} |",
            e.table,
            e.column,
            e.column_type,
            e.model,
            e.vectors,
            e.rows_without_vector,
            human_bytes(e.bytes as f64),
            human_bytes(e.index_bytes as f64)
        );
    }

    let _ = writeln!(md, "\n### Largest tables\n");
    let _ = writeln!(
        md,
        "| Table | Artifact | Rows | Dead rows | Heap | TOAST | Indexes | Total |"
    );
    let _ = writeln!(md, "|---|---|---:|---:|---:|---:|---:|---:|");
    let mut tables = report.database.tables.iter().collect::<Vec<_>>();
    tables.sort_by(|a, b| {
        b.total_bytes
            .cmp(&a.total_bytes)
            .then(a.table.cmp(&b.table))
    });
    for t in tables.iter().filter(|t| t.rows > 0).take(15) {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            t.table,
            t.artifact,
            t.rows,
            t.dead_rows,
            human_bytes(t.heap_bytes as f64),
            human_bytes(t.toast_bytes as f64),
            human_bytes(t.index_bytes as f64),
            human_bytes(t.total_bytes as f64)
        );
    }
    let empty = tables.iter().filter(|t| t.rows == 0).collect::<Vec<_>>();
    let _ = writeln!(
        md,
        "\n{} empty tables take {} (page and index stubs).",
        empty.len(),
        human_bytes(empty.iter().map(|t| t.total_bytes).sum::<i64>() as f64)
    );

    let _ = writeln!(md, "\n### Disk\n");
    let _ = writeln!(
        md,
        "Work directory `{}`: {} in {} files.\n",
        report.disk.work_dir,
        human_bytes(report.disk.bytes as f64),
        report.disk.files
    );
    for (kind, bytes) in &report.disk.bytes_by_kind {
        let _ = writeln!(md, "- {kind}: {}", human_bytes(*bytes as f64));
    }
    if !report.disk.unattributed_items.is_empty() {
        let _ = writeln!(
            md,
            "- Not linked to a video row: {} ({}).",
            report.disk.unattributed_items.join(", "),
            human_bytes(report.disk.unattributed_bytes as f64)
        );
    }
    for shared in &report.disk.shared {
        let _ = writeln!(
            md,
            "- Shared, not per video — {} `{}`: {}",
            shared.label,
            shared.path,
            if shared.exists {
                format!(
                    "{} in {} files",
                    human_bytes(shared.bytes as f64),
                    shared.files
                )
            } else {
                "absent".to_string()
            }
        );
    }

    let _ = writeln!(md, "\n## Findings per video\n");
    let _ = writeln!(md, "| Finding | Total | Videos with rows | Mean per video | Per video-minute | Per minute (min / median / max) |");
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---|");
    let mut not_run = Vec::new();
    for (table, _) in KEY_FINDINGS {
        let Some(f) = report.findings.iter().find(|f| f.table == *table) else {
            continue;
        };
        if f.total == 0 {
            not_run.push(f.label.clone());
            continue;
        }
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} |",
            f.label,
            f.total,
            f.videos_with_rows,
            opt_num(f.per_video_mean, 1),
            opt_num(f.per_video_minute, 2),
            spread_num(&f.per_minute, 2)
        );
    }
    if !not_run.is_empty() {
        let _ = writeln!(
            md,
            "\nNo rows yet (stage not run on this corpus): {}.",
            not_run.join(", ")
        );
    }

    let _ = writeln!(md, "\n## By length\n");
    let _ = writeln!(
        md,
        "Attributed bytes (rows plus files) and findings per video-minute, by length bucket.\n"
    );
    let _ = writeln!(md, "| Bucket | Videos | Minutes | Attributed bytes / min | Transcripts / min | Embeddings / min | Search / min | Segments / min | SponsorBlock skippable |");
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for b in &report.buckets {
        let artifact =
            |name: &str| opt_bytes(b.attributed_bytes_per_minute_by_artifact.get(name).copied());
        let _ = writeln!(
            md,
            "| {} | {} | {:.1} | {} | {} | {} | {} | {} | {} |",
            b.bucket,
            b.videos,
            b.minutes,
            opt_bytes(b.attributed_bytes_per_minute),
            artifact(TRANSCRIPTS),
            artifact(EMBEDDINGS),
            artifact(SEARCH_INDEX),
            opt_num(b.findings_per_minute.get("transcript_segments").copied(), 2),
            opt_pct(b.sponsorblock_skippable_share)
        );
    }

    let sb = &report.sponsorblock;
    let _ = writeln!(md, "\n## SponsorBlock savings\n");
    let _ = writeln!(
        md,
        "{} of {} videos have a SponsorBlock snapshot ({:.1} minutes); {} have at least one segment. Skip segments only (`actionType` skip; highlights, full-video labels, chapters and mutes excluded), merged across categories and clipped to the video duration.\n",
        sb.videos_with_snapshot, sample.videos, sb.snapshot_minutes, sb.videos_with_segments
    );
    let _ = writeln!(
        md,
        "**Skippable: {:.0} s of {:.0} s ({}).**\n",
        sb.skippable_seconds,
        sb.snapshot_minutes * 60.0,
        opt_pct(sb.skippable_share)
    );
    if !sb.categories.is_empty() {
        let _ = writeln!(md, "| Category | Segments | Seconds | Share of duration |");
        let _ = writeln!(md, "|---|---:|---:|---:|");
        for c in &sb.categories {
            let _ = writeln!(
                md,
                "| {} | {} | {:.0} | {} |",
                c.category,
                c.segments,
                c.seconds,
                opt_pct(c.share)
            );
        }
        let _ = writeln!(md);
    }
    let s = &sb.savings;
    let _ = writeln!(md, "Skipping these ranges would save:\n");
    let _ = writeln!(
        md,
        "- transcription: {:.0} s of audio ({:.1} min);",
        s.transcription_seconds,
        s.transcription_seconds / 60.0
    );
    let _ = writeln!(
        md,
        "- transcript storage: {} segments ({} of the segments of these videos), {} of row data;",
        s.transcript_segments,
        opt_pct(s.transcript_segment_share),
        human_bytes(s.transcript_row_bytes as f64)
    );
    let _ = writeln!(
        md,
        "- embedding work: {} vectors ({});",
        s.embedding_vectors,
        human_bytes(s.embedding_bytes as f64)
    );
    let _ = writeln!(
        md,
        "- media: {} (estimate, in proportion to the skipped share).",
        human_bytes(s.media_bytes_estimate)
    );
    if !sb.fetch_errors.is_empty() {
        let _ = writeln!(md, "\nFetch errors: {}.", sb.fetch_errors.join("; "));
    }
    let _ = writeln!(
        md,
        "\nSegment data from [SponsorBlock]({}) — {} — licensed under {}.",
        sb.source, sb.attribution, sb.license
    );

    let _ = writeln!(md, "\n## Per video\n");
    let _ = writeln!(
        md,
        "| Video | Minutes | Bucket | Segments | Attributed | Per minute | Skippable |"
    );
    let _ = writeln!(md, "|---|---:|---|---:|---:|---:|---:|");
    for v in &report.videos {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} |",
            v.youtube_id.as_deref().unwrap_or("–"),
            opt_num(v.duration_seconds.map(|d| d / 60.0), 1),
            v.bucket.as_deref().unwrap_or("–"),
            v.counts.get("transcript_segments").copied().unwrap_or(0),
            human_bytes(v.attributed_bytes as f64),
            opt_bytes(v.attributed_bytes_per_minute),
            opt_pct(v.sponsorblock.as_ref().and_then(|s| s.skippable_share))
        );
    }

    let _ = writeln!(md, "\n## How to re-run\n");
    let _ = writeln!(md, "```bash");
    let _ = writeln!(md, "# Read-only: stored SponsorBlock snapshots only.");
    let _ = writeln!(md, "youtube-corpus stats --markdown-out docs/corpus-stats.md --json-out use-case-output/corpus-stats.json");
    let _ = writeln!(
        md,
        "# Also fetch SponsorBlock for videos without a snapshot (stores snapshots)."
    );
    let _ = writeln!(
        md,
        "youtube-corpus stats --sponsorblock missing --markdown-out docs/corpus-stats.md"
    );
    let _ = writeln!(md, "```");
    align_tables(&md)
}

/// Collapses blank-line runs and pads markdown table columns the way the repository formatter (oxfmt) lays them out, so
/// a regenerated report passes `bun run format:check` unchanged.
pub fn align_tables(markdown: &str) -> String {
    fn cells(line: &str) -> Vec<String> {
        let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
        inner.split('|').map(|c| c.trim().to_string()).collect()
    }
    fn width(cell: &str) -> usize {
        cell.chars().count()
    }
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with('|') {
            // The formatter collapses runs of blank lines.
            if !(lines[i].is_empty() && out.last().is_some_and(|l: &String| l.is_empty())) {
                out.push(lines[i].to_string());
            }
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].starts_with('|') {
            i += 1;
        }
        let mut rows = lines[start..i].iter().map(|l| cells(l)).collect::<Vec<_>>();
        if rows.len() < 2 {
            out.extend(lines[start..i].iter().map(|l| l.to_string()));
            continue;
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut rows {
            row.resize(columns, String::new());
        }
        let right = rows[1]
            .iter()
            .map(|c| c.ends_with(':') && !c.starts_with(':'))
            .collect::<Vec<_>>();
        let widths = (0..columns)
            .map(|c| {
                rows.iter()
                    .enumerate()
                    .filter(|(r, _)| *r != 1)
                    .map(|(_, row)| width(&row[c]))
                    .max()
                    .unwrap_or(0)
                    .max(3)
            })
            .collect::<Vec<_>>();
        for (r, row) in rows.iter().enumerate() {
            let rendered = (0..columns)
                .map(|c| {
                    let w = widths[c];
                    if r == 1 {
                        if right[c] {
                            format!("{}:", "-".repeat(w - 1))
                        } else {
                            "-".repeat(w)
                        }
                    } else {
                        let pad = " ".repeat(w - width(&row[c]));
                        if right[c] {
                            format!("{pad}{}", row[c])
                        } else {
                            format!("{}{pad}", row[c])
                        }
                    }
                })
                .collect::<Vec<_>>();
            out.push(format!("| {} |", rendered.join(" | ")));
        }
    }
    let mut joined = out.join("\n");
    if markdown.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_half_open() {
        assert_eq!(length_bucket(0.0), "<10 min");
        assert_eq!(length_bucket(9.99), "<10 min");
        assert_eq!(length_bucket(10.0), "10–30 min");
        assert_eq!(length_bucket(59.9), "30–60 min");
        assert_eq!(length_bucket(60.0), "60+ min");
        assert_eq!(length_bucket(600.0), "60+ min");
    }

    #[test]
    fn spread_has_even_and_odd_medians() {
        assert_eq!(spread(&[]), Spread::default());
        let odd = spread(&[3.0, 1.0, 2.0]);
        assert_eq!(
            (odd.min, odd.median, odd.max),
            (Some(1.0), Some(2.0), Some(3.0))
        );
        assert_eq!(spread(&[4.0, 1.0, 2.0, 3.0]).median, Some(2.5));
    }

    #[test]
    fn ranges_merge_clip_and_overlap() {
        let merged = merge_ranges(
            &[
                (5.0, 10.0),
                (0.0, 2.0),
                (8.0, 12.0),
                (-1.0, 1.0),
                (95.0, 120.0),
            ],
            Some(100.0),
        );
        assert_eq!(merged, vec![(0.0, 2.0), (5.0, 12.0), (95.0, 100.0)]);
        assert_eq!(ranges_seconds(&merged), 14.0);
        assert_eq!(overlap_seconds(&merged, 1.0, 6.0), 2.0);
        assert!(merge_ranges(&[(3.0, 3.0)], None).is_empty());
    }

    #[test]
    fn skip_segments_exclude_points_labels_and_mutes() {
        assert!(is_skip_segment("sponsor", Some("skip")));
        assert!(is_skip_segment("intro", None));
        assert!(!is_skip_segment("poi_highlight", Some("poi")));
        assert!(!is_skip_segment("sponsor", Some("full")));
        assert!(!is_skip_segment("sponsor", Some("mute")));
        assert!(!is_skip_segment("chapter", Some("chapter")));
    }

    #[test]
    fn tables_indexes_and_files_map_to_artifacts() {
        assert_eq!(table_artifact("transcript_segments"), TRANSCRIPTS);
        assert_eq!(table_artifact("preferred_transcript_streams"), TRANSCRIPTS);
        assert_eq!(table_artifact("visual_text_tracks"), OCR);
        assert_eq!(table_artifact("entity_mentions"), ANNOTATIONS);
        assert_eq!(table_artifact("historical_event_claims"), ANNOTATIONS);
        assert_eq!(table_artifact("translated_segments"), TRANSLATIONS);
        assert_eq!(table_artifact("face_tracks"), VISUAL_VOICE);
        assert_eq!(table_artifact("sponsorblock_segments"), SPONSORBLOCK);
        assert_eq!(table_artifact("videos"), METADATA);
        assert_eq!(
            index_artifact("transcript_segments", "hnsw", false),
            EMBEDDINGS
        );
        assert_eq!(
            index_artifact("transcript_segments", "gin", true),
            SEARCH_INDEX
        );
        assert_eq!(index_artifact("videos", "gin", false), METADATA);
        assert_eq!(disk_kind(Path::new("media/item.m4a")), "audio");
        assert_eq!(disk_kind(Path::new("media/item.webm")), "video");
        assert_eq!(disk_kind(Path::new("media/item.info.json")), "media_other");
        assert_eq!(disk_kind(Path::new("captions/x.en.vtt")), "captions");
        assert_eq!(disk_kind(Path::new("metadata/x.metadata.json")), "metadata");
        assert_eq!(disk_kind(Path::new("notes.txt")), "other");
        assert_eq!(disk_artifact("audio"), MEDIA);
        assert_eq!(disk_artifact("captions"), TRANSCRIPTS);
    }

    #[test]
    fn tables_are_padded_like_the_formatter() {
        let md = "x\n| A | Bee |\n|---|---:|\n| long cell | 1 |\n| – | |\ny\n";
        assert_eq!(
            align_tables(md),
            "x\n| A         | Bee |\n| --------- | --: |\n| long cell |   1 |\n| –         |     |\ny\n"
        );
    }

    #[test]
    fn human_bytes_uses_binary_units() {
        assert_eq!(human_bytes(512.0), "512 B");
        assert_eq!(human_bytes(1536.0), "1.50 KiB");
        assert_eq!(human_bytes(3.0 * 1024.0 * 1024.0), "3.00 MiB");
    }

    #[test]
    fn markdown_states_sample_and_attribution() {
        let report = CorpusStatsReport {
            schema: STATS_SCHEMA.to_string(),
            generated_at: "2026-11-04T00:00:00Z".to_string(),
            sample: SampleStats {
                videos: 2,
                total_minutes: 30.0,
                ..Default::default()
            },
            database: DatabaseStats::default(),
            disk: DiskStats::default(),
            artifacts: Vec::new(),
            embeddings: Vec::new(),
            findings: Vec::new(),
            buckets: Vec::new(),
            sponsorblock: SponsorBlockStats {
                attribution: SPONSORBLOCK_ATTRIBUTION.to_string(),
                license: SPONSORBLOCK_DATA_LICENSE.to_string(),
                ..Default::default()
            },
            videos: Vec::new(),
            notes: vec!["Small sample".to_string()],
        };
        let md = render_markdown(&report);
        assert!(md.contains("2 videos, 30.0 minutes"));
        assert!(md.contains("Small sample"));
        assert!(md.contains(SPONSORBLOCK_DATA_LICENSE));
        assert!(md.contains(SPONSORBLOCK_ATTRIBUTION));
    }
}
