use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const SOURCE_SPAN_INTERCHANGE_SCHEMA: &str = "source_span_interchange";
pub const SOURCE_SPAN_INTERCHANGE_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpanBatchV1 {
    pub schema: String,
    pub schema_version: u32,
    pub producer: SourceProducerV1,
    pub sources: Vec<SourceRecordV1>,
    pub spans: Vec<SourceSpanRecordV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceProducerV1 {
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecordV1 {
    pub id: String,
    pub kind: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creators: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpanRecordV1 {
    pub id: String,
    pub source_id: String,
    pub sequence: u64,
    pub text: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub locator: SourceLocatorV1,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum SourceLocatorV1 {
    Text {
        byte_start: usize,
        byte_end: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        paragraph_ordinal: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        section: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_selector: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        heading_path: Vec<String>,
    },
    Timed {
        segment_index: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_seconds: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        end_seconds: Option<f64>,
    },
}

#[derive(Debug, Clone)]
struct TranscriptSpanExportInput {
    segment_id: Uuid,
    stream_id: Uuid,
    video_id: Uuid,
    youtube_id: Option<String>,
    source_url: String,
    title: Option<String>,
    channel: Option<String>,
    channel_id: Option<String>,
    uploader: Option<String>,
    uploader_id: Option<String>,
    source_kind: String,
    stream_language: Option<String>,
    full_text: Option<String>,
    segment_index: u64,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    text: String,
    language: Option<String>,
}

pub async fn export_source_span_batch(
    pool: &PgPool,
    producer_revision: &str,
    video_id: Option<Uuid>,
) -> anyhow::Result<SourceSpanBatchV1> {
    ensure!(
        !producer_revision.trim().is_empty(),
        "a producer revision is required for source-span export"
    );

    let rows = sqlx::query(
        "SELECT ts.id AS segment_id, ts.stream_id, ts.video_id, ts.segment_index,
                ts.start_seconds, ts.end_seconds, ts.text, ts.language,
                st.source_kind, st.language AS stream_language, st.full_text,
                v.youtube_id, v.source_url, v.title, v.channel, v.channel_id,
                v.uploader, v.uploader_id
         FROM transcript_segments ts
         JOIN transcript_streams st ON st.id = ts.stream_id
         JOIN videos v ON v.id = ts.video_id
         WHERE ($1::uuid IS NULL OR v.id = $1)
         ORDER BY ts.stream_id, ts.segment_index",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await?;

    let mut inputs = Vec::with_capacity(rows.len());
    for row in rows {
        let segment_index: i64 = row.try_get("segment_index")?;
        inputs.push(TranscriptSpanExportInput {
            segment_id: row.try_get("segment_id")?,
            stream_id: row.try_get("stream_id")?,
            video_id: row.try_get("video_id")?,
            youtube_id: row.try_get("youtube_id")?,
            source_url: row.try_get("source_url")?,
            title: row.try_get("title")?,
            channel: row.try_get("channel")?,
            channel_id: row.try_get("channel_id")?,
            uploader: row.try_get("uploader")?,
            uploader_id: row.try_get("uploader_id")?,
            source_kind: row.try_get("source_kind")?,
            stream_language: row.try_get("stream_language")?,
            full_text: row.try_get("full_text")?,
            segment_index: segment_index
                .try_into()
                .context("transcript segment index must be non-negative")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
            text: row.try_get("text")?,
            language: row.try_get("language")?,
        });
    }

    build_source_span_batch(inputs, producer_revision)
}

fn build_source_span_batch(
    mut inputs: Vec<TranscriptSpanExportInput>,
    producer_revision: &str,
) -> anyhow::Result<SourceSpanBatchV1> {
    ensure!(
        !producer_revision.trim().is_empty(),
        "a producer revision is required for source-span export"
    );

    inputs.sort_by_key(|input| (input.stream_id, input.segment_index));

    let mut grouped = BTreeMap::<String, Vec<&TranscriptSpanExportInput>>::new();
    for input in &inputs {
        validate_timed_range(input)?;
        grouped
            .entry(input.stream_id.to_string())
            .or_default()
            .push(input);
    }

    let mut sources = Vec::with_capacity(grouped.len());
    for (stream_id, segments) in &grouped {
        let first = segments
            .first()
            .context("transcript stream group unexpectedly empty")?;
        let stream_text = first
            .full_text
            .as_deref()
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        let source_hash = sha256(&stream_text);
        let revision_material = serde_json::json!({
            "contentHash": &source_hash,
            "sourceKind": &first.source_kind,
            "language": &first.stream_language,
            "spans": segments
                .iter()
                .map(|segment| serde_json::json!({
                    "id": segment.segment_id,
                    "sequence": segment.segment_index,
                    "textHash": sha256(&segment.text),
                    "startSeconds": segment.start_seconds,
                    "endSeconds": segment.end_seconds,
                    "language": &segment.language,
                }))
                .collect::<Vec<_>>(),
        });
        let revision = sha256(&serde_json::to_string(&revision_material)?);
        let mut metadata = BTreeMap::new();
        metadata.insert("videoId".to_string(), Value::String(first.video_id.to_string()));
        metadata.insert("streamId".to_string(), Value::String(stream_id.clone()));
        metadata.insert(
            "transcriptSource".to_string(),
            Value::String(first.source_kind.clone()),
        );
        if let Some(youtube_id) = &first.youtube_id {
            metadata.insert("youtubeId".to_string(), Value::String(youtube_id.clone()));
        }
        if let Some(channel_id) = &first.channel_id {
            metadata.insert("channelId".to_string(), Value::String(channel_id.clone()));
        }
        if let Some(uploader_id) = &first.uploader_id {
            metadata.insert("uploaderId".to_string(), Value::String(uploader_id.clone()));
        }
        let mut creators = Vec::new();
        for creator in [&first.channel, &first.uploader].into_iter().flatten() {
            if !creator.trim().is_empty() && !creators.contains(creator) {
                creators.push(creator.clone());
            }
        }

        sources.push(SourceRecordV1 {
            id: stream_id.clone(),
            kind: "youtube_transcript".to_string(),
            revision,
            uri: Some(first.source_url.clone()),
            title: first.title.clone(),
            creators,
            language: first.stream_language.clone(),
            content_hash: source_hash,
            metadata,
        });
    }

    let spans = inputs
        .into_iter()
        .map(|input| {
            let mut metadata = BTreeMap::new();
            metadata.insert("videoId".to_string(), Value::String(input.video_id.to_string()));
            metadata.insert(
                "streamId".to_string(),
                Value::String(input.stream_id.to_string()),
            );
            metadata.insert(
                "transcriptSource".to_string(),
                Value::String(input.source_kind),
            );
            SourceSpanRecordV1 {
                id: input.segment_id.to_string(),
                source_id: input.stream_id.to_string(),
                sequence: input.segment_index,
                content_hash: sha256(&input.text),
                text: input.text,
                language: input.language.or(input.stream_language),
                locator: SourceLocatorV1::Timed {
                    segment_index: input.segment_index,
                    start_seconds: input.start_seconds,
                    end_seconds: input.end_seconds,
                },
                metadata,
            }
        })
        .collect();

    Ok(SourceSpanBatchV1 {
        schema: SOURCE_SPAN_INTERCHANGE_SCHEMA.to_string(),
        schema_version: SOURCE_SPAN_INTERCHANGE_VERSION_V1,
        producer: SourceProducerV1 {
            name: "youtube-corpus".to_string(),
            revision: producer_revision.to_string(),
        },
        sources,
        spans,
    })
}

fn validate_timed_range(input: &TranscriptSpanExportInput) -> anyhow::Result<()> {
    ensure!(
        input
            .start_seconds
            .is_none_or(|value| value.is_finite() && value >= 0.0),
        "transcript segment {} has invalid start time",
        input.segment_id
    );
    ensure!(
        input
            .end_seconds
            .is_none_or(|value| value.is_finite() && value >= 0.0),
        "transcript segment {} has invalid end time",
        input.segment_id
    );
    ensure!(
        !input
            .start_seconds
            .zip(input.end_seconds)
            .is_some_and(|(start, end)| end < start),
        "transcript segment {} has reversed timestamps",
        input.segment_id
    );
    Ok(())
}

fn sha256(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(segment_index: u64, text: &str, start: f64, end: f64) -> TranscriptSpanExportInput {
        TranscriptSpanExportInput {
            segment_id: Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("segment:{segment_index}").as_bytes(),
            ),
            stream_id: Uuid::new_v5(&Uuid::NAMESPACE_URL, b"stream"),
            video_id: Uuid::new_v5(&Uuid::NAMESPACE_URL, b"video"),
            youtube_id: Some("abc123".to_string()),
            source_url: "https://www.youtube.com/watch?v=abc123".to_string(),
            title: Some("Lecture".to_string()),
            channel: Some("Lecture Channel".to_string()),
            channel_id: Some("channel-1".to_string()),
            uploader: Some("Lecturer".to_string()),
            uploader_id: Some("uploader-1".to_string()),
            source_kind: "caption_manual".to_string(),
            stream_language: Some("en".to_string()),
            full_text: Some("First claim. Second claim.".to_string()),
            segment_index,
            start_seconds: Some(start),
            end_seconds: Some(end),
            text: text.to_string(),
            language: Some("en".to_string()),
        }
    }

    #[test]
    fn builds_one_revisioned_source_with_timestamped_spans() {
        let batch = build_source_span_batch(
            vec![
                input(1, "Second claim.", 3.0, 5.0),
                input(0, "First claim.", 0.0, 2.5),
            ],
            "git:abc123",
        )
        .unwrap();

        assert_eq!(batch.schema, SOURCE_SPAN_INTERCHANGE_SCHEMA);
        assert_eq!(batch.schema_version, 1);
        assert_eq!(batch.producer.revision, "git:abc123");
        assert_eq!(batch.sources.len(), 1);
        assert!(batch.sources[0].content_hash.starts_with("sha256:"));
        assert!(batch.sources[0].revision.starts_with("sha256:"));
        assert_ne!(batch.sources[0].revision, batch.sources[0].content_hash);
        assert_eq!(
            batch.spans.iter().map(|span| span.sequence).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(matches!(
            batch.spans[0].locator,
            SourceLocatorV1::Timed {
                segment_index: 0,
                start_seconds: Some(0.0),
                end_seconds: Some(2.5),
            }
        ));
        assert!(batch
            .spans
            .iter()
            .all(|span| span.content_hash.starts_with("sha256:")));
    }

    #[test]
    fn source_revision_changes_when_timing_changes_but_text_does_not() {
        let first = build_source_span_batch(
            vec![input(0, "Claim.", 1.0, 2.0)],
            "git:abc123",
        )
        .unwrap();
        let second = build_source_span_batch(
            vec![input(0, "Claim.", 1.5, 2.5)],
            "git:abc123",
        )
        .unwrap();

        assert_eq!(first.sources[0].content_hash, second.sources[0].content_hash);
        assert_ne!(first.sources[0].revision, second.sources[0].revision);
    }

    #[test]
    fn refuses_reversed_timestamp_ranges() {
        let error =
            build_source_span_batch(vec![input(0, "Claim.", 3.0, 2.0)], "git:abc123").unwrap_err();
        assert!(error.to_string().contains("reversed timestamps"));
    }

    #[test]
    fn serialized_contract_uses_shared_camel_case_field_names() {
        let batch =
            build_source_span_batch(vec![input(0, "Claim.", 1.0, 2.0)], "git:abc123").unwrap();
        let value = serde_json::to_value(batch).unwrap();

        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["spans"][0]["sourceId"].as_str(), value["sources"][0]["id"].as_str());
        assert_eq!(value["spans"][0]["locator"]["kind"], "timed");
        assert_eq!(value["spans"][0]["locator"]["segmentIndex"], 0);
        assert_eq!(value["spans"][0]["locator"]["startSeconds"], 1.0);
    }
}
