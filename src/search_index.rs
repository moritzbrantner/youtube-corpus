//! Opt-in Tantivy lexical search pilot (#76) through Search Engine.
//!
//! Postgres stays the canonical corpus and the default search provider. This module keeps a
//! derived, rebuildable projection of `corpus_search_segments` plus video metadata facets in a
//! local Tantivy index and answers **literal substring** queries over it: every whitespace
//! separated query term must occur, case-folded, in the passage text. There is no stemming,
//! token boundary or relevance ranking; hits are ordered by video and timestamp. This is not
//! equivalent to the Postgres FTS ranking (owner question #76.Q1, option A).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Context;
use search_engine::indexed::{Mutation, TantivyIndex};
use search_engine::{
    Document, FieldType, Predicate, Schema, SearchRequest as EngineRequest, Value,
};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::config::SearchMode;
use crate::search::{
    attach_ocr_evidence, attach_translation_evidence, corpus_ocr_coverage, SearchModality,
    SearchProviderKind, SearchReport, SearchRequest, SearchResult,
};

/// Search Engine bounds one page at 1,000 hits.
const PAGE: usize = 1_000;
const LIST_SEPARATOR: &str = "\u{1f}";

/// A local Tantivy index of the corpus' searchable passages. One process owns the directory.
pub struct TantivySearch {
    index: Mutex<TantivyIndex>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub live_passages: usize,
    pub videos: usize,
    pub upserted: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub elapsed_ms: u128,
}

impl TantivySearch {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        let index = TantivyIndex::open(dir, schema())
            .with_context(|| format!("failed to open Tantivy index at {}", dir.display()))?;
        Ok(Self {
            index: Mutex::new(index),
        })
    }

    /// Rebuilds the projection from the current Postgres snapshot. Unchanged passages keep their
    /// revision, edited ones get the next revision and passages gone from Postgres are tombstoned,
    /// so repeating a sync over the same data is a no-op.
    pub async fn sync(&self, pool: &PgPool) -> anyhow::Result<SyncReport> {
        let started = Instant::now();
        let snapshot = load_snapshot(pool).await?;
        let mut index = self.index.lock().expect("search index lock poisoned");
        let mut report = SyncReport {
            live_passages: snapshot
                .iter()
                .filter(|d| !d.id.starts_with("video:"))
                .count(),
            videos: snapshot
                .iter()
                .filter(|d| d.id.starts_with("video:"))
                .count(),
            ..SyncReport::default()
        };
        let mut seen = BTreeSet::new();
        let mut mutations = Vec::with_capacity(snapshot.len());
        for mut document in snapshot {
            seen.insert(document.id.clone());
            match index.document(&document.id) {
                Some(existing) if existing.fields == document.fields => {
                    document.revision = existing.revision;
                    report.unchanged += 1;
                }
                _ => {
                    document.revision = index.latest_revision(&document.id).map_or(1, |r| r + 1);
                    report.upserted += 1;
                }
            }
            mutations.push(Mutation::Upsert { document });
        }
        for id in live_ids(&index)? {
            if !seen.contains(&id) {
                let revision = index.latest_revision(&id).map_or(1, |r| r + 1);
                mutations.push(Mutation::Delete { id, revision });
                report.deleted += 1;
            }
        }
        index.rebuild(&mutations)?;
        report.elapsed_ms = started.elapsed().as_millis();
        Ok(report)
    }

    /// Runs a literal lexical query with the supported metadata filters. Provenance (translation
    /// and OCR evidence) is attached from Postgres, the same as the default provider.
    pub async fn search(
        &self,
        pool: &PgPool,
        request: &SearchRequest,
    ) -> anyhow::Result<SearchReport> {
        let plan = plan(request)?;
        let (results, scope_videos) = {
            let index = self.index.lock().expect("search index lock poisoned");
            let hits = index.search(
                &EngineRequest::new(plan.query)
                    .sort_by("order", false)
                    .page(0, plan.limit),
            )?;
            let results = hits
                .hits
                .iter()
                .filter_map(|hit| index.document(&hit.id))
                .map(to_result)
                .collect::<anyhow::Result<Vec<_>>>()?;
            // Videos matching the video-level filters (not text or modality), for OCR coverage.
            let scope = index.search_with_facets(
                &EngineRequest::new(plan.video_scope).page(0, 0),
                &["video_id"],
            )?;
            let videos: HashSet<Uuid> = scope
                .facets
                .get("video_id")
                .into_iter()
                .flat_map(|counts| counts.keys())
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect();
            (results, videos)
        };
        let mut results = results;
        attach_translation_evidence(pool, &mut results).await?;
        attach_ocr_evidence(pool, &mut results).await?;
        let ocr_coverage = corpus_ocr_coverage(pool, &scope_videos).await?;
        Ok(SearchReport {
            query: request.query.clone(),
            mode: SearchMode::Fts,
            provider: SearchProviderKind::TantivyLiteral,
            results,
            ocr_coverage: Some(ocr_coverage),
        })
    }
}

fn schema() -> Schema {
    Schema::new()
        .field("kind", FieldType::Keyword)
        .field("doc_id", FieldType::Keyword)
        .field("text", FieldType::Text)
        .field("video_id", FieldType::Keyword)
        .field("stream_id", FieldType::Keyword)
        .field("source_kind", FieldType::Keyword)
        .field("language", FieldType::Keyword)
        .field("start_ms", FieldType::Integer)
        .field("end_ms", FieldType::Integer)
        .field("translated", FieldType::Boolean)
        .field("source_url", FieldType::Keyword)
        .field("title", FieldType::Text)
        .field("channel", FieldType::Text)
        .field("categories", FieldType::Text)
        .field("tags", FieldType::Text)
        .field("upload_date", FieldType::Integer)
        .field("duration_ms", FieldType::Integer)
        .field("view_count", FieldType::Integer)
        .field("order", FieldType::Keyword)
}

macro_rules! video_columns {
    () => {
        "v.source_url, v.title, v.channel, v.channel_id, v.uploader, v.uploader_id,
         v.categories, v.tags, v.upload_date, v.duration_seconds, v.view_count"
    };
}

/// Passage documents (`kind = passage`) plus one `kind = video` document per video, so the OCR
/// coverage scope of video-level filters never has to materialize every passage.
async fn load_snapshot(pool: &PgPool) -> anyhow::Result<Vec<Document>> {
    let passages = sqlx::query(concat!(
        "SELECT s.id, s.video_id, s.stream_id, s.source_kind, s.language, s.start_seconds,
                s.end_seconds, s.text, s.translated, ",
        video_columns!(),
        " FROM corpus_search_segments s
          JOIN videos v ON v.id = s.video_id
          ORDER BY s.id"
    ))
    .fetch_all(pool)
    .await?;
    let videos = sqlx::query(concat!(
        "SELECT v.id, ",
        video_columns!(),
        " FROM videos v ORDER BY v.id"
    ))
    .fetch_all(pool)
    .await?;
    let mut documents = passages
        .iter()
        .map(project_passage)
        .collect::<anyhow::Result<Vec<_>>>()?;
    for row in &videos {
        let video_id: Uuid = row.try_get("id")?;
        let id = format!("video:{video_id}");
        let mut document = Document::new(id.clone(), 0)
            .field("kind", Value::Keyword("video".into()))
            .field("doc_id", Value::Keyword(id.clone()))
            .field("video_id", Value::Keyword(video_id.to_string()))
            .field("order", Value::Keyword(id));
        project_video(row, &mut document)?;
        documents.push(document);
    }
    Ok(documents)
}

fn project_passage(row: &sqlx::postgres::PgRow) -> anyhow::Result<Document> {
    let id: Uuid = row.try_get("id")?;
    let video_id: Uuid = row.try_get("video_id")?;
    let stream_id: Uuid = row.try_get("stream_id")?;
    let start_ms = row.try_get::<Option<f64>, _>("start_seconds")?.map(millis);
    let mut document = Document::new(id.to_string(), 0)
        .field("kind", Value::Keyword("passage".into()))
        .field("doc_id", Value::Keyword(id.to_string()))
        .field("text", Value::Text(row.try_get("text")?))
        .field("video_id", Value::Keyword(video_id.to_string()))
        .field("stream_id", Value::Keyword(stream_id.to_string()))
        .field("source_kind", Value::Keyword(row.try_get("source_kind")?))
        .field("translated", Value::Boolean(row.try_get("translated")?))
        .field(
            "order",
            Value::Keyword(format!(
                "{video_id}:{:015}:{id}",
                start_ms.unwrap_or(0).max(0)
            )),
        );
    put(&mut document, "start_ms", start_ms.map(Value::Integer));
    put(
        &mut document,
        "language",
        row.try_get::<Option<String>, _>("language")?
            .map(Value::Keyword),
    );
    put(
        &mut document,
        "end_ms",
        row.try_get::<Option<f64>, _>("end_seconds")?
            .map(|seconds| Value::Integer(millis(seconds))),
    );
    project_video(row, &mut document)?;
    Ok(document)
}

/// Video metadata facets shared by passage and video documents.
fn project_video(row: &sqlx::postgres::PgRow, document: &mut Document) -> anyhow::Result<()> {
    put(
        document,
        "source_url",
        Some(Value::Keyword(row.try_get("source_url")?)),
    );
    put(
        document,
        "title",
        row.try_get::<Option<String>, _>("title")?.map(Value::Text),
    );
    let channel: Vec<String> = ["channel", "channel_id", "uploader", "uploader_id"]
        .into_iter()
        .filter_map(|column| row.try_get::<Option<String>, _>(column).ok().flatten())
        .collect();
    put(document, "channel", joined(channel));
    for list in ["categories", "tags"] {
        put(
            document,
            list,
            joined(
                row.try_get::<Option<Vec<String>>, _>(list)?
                    .unwrap_or_default(),
            ),
        );
    }
    put(
        document,
        "upload_date",
        row.try_get::<Option<String>, _>("upload_date")?
            .and_then(|date| date.parse::<i64>().ok())
            .map(Value::Integer),
    );
    put(
        document,
        "duration_ms",
        row.try_get::<Option<f64>, _>("duration_seconds")?
            .map(|seconds| Value::Integer(millis(seconds))),
    );
    put(
        document,
        "view_count",
        row.try_get::<Option<i64>, _>("view_count")?
            .map(Value::Integer),
    );
    Ok(())
}

fn put(document: &mut Document, name: &str, value: Option<Value>) {
    if let Some(value) = value {
        document.fields.insert(name.to_owned(), value);
    }
}

fn millis(seconds: f64) -> i64 {
    (seconds * 1000.0).round() as i64
}

/// Joins list values so a substring filter matches any one element (as `ILIKE` over `unnest`).
fn joined(values: Vec<String>) -> Option<Value> {
    (!values.is_empty()).then(|| Value::Text(values.join(LIST_SEPARATOR)))
}

/// Every live document ID in one pass (paging `Predicate::All` would re-materialize the whole
/// index per page).
fn live_ids(index: &TantivyIndex) -> anyhow::Result<Vec<String>> {
    let all =
        index.search_with_facets(&EngineRequest::new(Predicate::All).page(0, 0), &["doc_id"])?;
    Ok(all
        .facets
        .get("doc_id")
        .into_iter()
        .flat_map(|ids| ids.keys().cloned())
        .collect())
}

fn to_result(document: &Document) -> anyhow::Result<SearchResult> {
    let keyword = |name: &str| match document.fields.get(name) {
        Some(Value::Keyword(value)) | Some(Value::Text(value)) => Some(value.clone()),
        _ => None,
    };
    let seconds = |name: &str| match document.fields.get(name) {
        Some(Value::Integer(value)) => Some(*value as f64 / 1000.0),
        _ => None,
    };
    let uuid = |name: &str| -> anyhow::Result<Uuid> {
        Ok(Uuid::parse_str(
            &keyword(name).context("missing projected id")?,
        )?)
    };
    Ok(SearchResult {
        segment_id: Uuid::parse_str(&document.id)?,
        video_id: uuid("video_id")?,
        stream_id: uuid("stream_id")?,
        source_kind: keyword("source_kind").unwrap_or_default(),
        language: keyword("language"),
        start_seconds: seconds("start_ms"),
        end_seconds: seconds("end_ms"),
        text: keyword("text").unwrap_or_default(),
        source_url: keyword("source_url").unwrap_or_default(),
        title: keyword("title"),
        score: 0.0,
        fts_score: 0.0,
        semantic_score: 0.0,
        ocr_evidence: None,
        translation: None,
    })
}

/// Fails when the request needs behavior the literal pilot does not provide.
pub fn check_supported(request: &SearchRequest) -> anyhow::Result<()> {
    plan(request).map(|_| ())
}

struct Plan {
    query: Predicate,
    video_scope: Predicate,
    limit: usize,
}

/// Translates the shared search request into Search Engine predicates, rejecting anything the
/// literal pilot cannot answer faithfully instead of silently ignoring it.
fn plan(request: &SearchRequest) -> anyhow::Result<Plan> {
    if request.mode != SearchMode::Fts {
        anyhow::bail!(
            "the Tantivy pilot answers literal lexical queries only; use mode `fts` \
             (semantic and hybrid stay on the default Postgres provider)"
        );
    }
    if request
        .metadata_query
        .as_deref()
        .is_some_and(|q| !q.trim().is_empty())
    {
        anyhow::bail!("the Tantivy pilot does not support the raw metadata filter");
    }
    let terms: Vec<Predicate> = request
        .query
        .split_whitespace()
        .map(|term| Predicate::TextContains("text".into(), term.to_owned()))
        .collect();
    if terms.is_empty() {
        anyhow::bail!("query is required");
    }

    // Passage-level filters narrow hits; video-level filters also define the OCR coverage scope.
    let mut filters = Vec::new();
    let mut videos = Vec::new();
    let keyword =
        |field: &str, value: String| Predicate::Equals(field.into(), Value::Keyword(value));
    if let Some(kind) = request.source_kind {
        filters.push(keyword("source_kind", kind.as_str().to_owned()));
    }
    match request.modality {
        Some(SearchModality::Visible) => filters.push(keyword("source_kind", "visual_ocr".into())),
        Some(SearchModality::Spoken) => filters.push(Predicate::Not(Box::new(keyword(
            "source_kind",
            "visual_ocr".into(),
        )))),
        None => {}
    }
    if let Some(video_id) = request.video_id {
        videos.push(keyword("video_id", video_id.to_string()));
    }
    if let Some(language) = trimmed(&request.language) {
        filters.push(keyword("language", language));
    }
    push_range(
        &mut filters,
        "start_ms",
        request
            .transcript_start_min
            .map(|s| (s * 1000.0).ceil() as i64),
        request
            .transcript_start_max
            .map(|s| (s * 1000.0).floor() as i64),
    );
    push_range(
        &mut videos,
        "duration_ms",
        request.duration_min.map(|s| (s * 1000.0).ceil() as i64),
        request.duration_max.map(|s| (s * 1000.0).floor() as i64),
    );
    push_range(
        &mut videos,
        "upload_date",
        upload_date(&request.upload_date_from)?,
        upload_date(&request.upload_date_to)?,
    );
    push_range(
        &mut videos,
        "view_count",
        request.view_count_min,
        request.view_count_max,
    );
    for (field, value) in [
        ("channel", &request.channel_query),
        ("title", &request.title_query),
        ("categories", &request.category_query),
        ("tags", &request.tag_query),
    ] {
        if let Some(value) = trimmed(value) {
            videos.push(Predicate::TextContains(field.into(), value));
        }
    }

    let mut query = vec![keyword("kind", "passage".into())];
    query.extend(terms);
    query.extend(filters);
    query.extend(videos.iter().cloned());
    videos.push(keyword("kind", "video".into()));
    Ok(Plan {
        query: Predicate::And(query),
        video_scope: Predicate::And(videos),
        limit: usize::try_from(request.top_k.max(1))
            .unwrap_or(PAGE)
            .min(PAGE),
    })
}

fn push_range(filters: &mut Vec<Predicate>, field: &str, min: Option<i64>, max: Option<i64>) {
    if min.is_some() || max.is_some() {
        filters.push(Predicate::Range {
            field: field.into(),
            minimum: min.map(Value::Integer),
            maximum: max.map(Value::Integer),
        });
    }
}

fn trimmed(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn upload_date(value: &Option<String>) -> anyhow::Result<Option<i64>> {
    Ok(crate::search::normalize_upload_date_filter(value)?
        .map(|date| date.parse::<i64>())
        .transpose()?)
}

/// One query answered by both providers, for recording match, order and latency differences.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderComparison {
    pub query: String,
    pub postgres: ProviderRun,
    pub tantivy: ProviderRun,
    pub shared: usize,
    pub only_postgres: Vec<Uuid>,
    pub only_tantivy: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRun {
    pub elapsed_ms: f64,
    pub hits: Vec<ComparedHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_coverage: Option<crate::search::OcrCoverage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparedHit {
    pub segment_id: Uuid,
    pub video_id: Uuid,
    pub source_kind: String,
    pub start_seconds: Option<f64>,
}

pub async fn compare(
    search: &TantivySearch,
    pool: &PgPool,
    request: SearchRequest,
) -> anyhow::Result<ProviderComparison> {
    let started = Instant::now();
    let tantivy = search.search(pool, &request).await?;
    let tantivy_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let postgres = crate::search::search_corpus(request.clone()).await?;
    let postgres_ms = started.elapsed().as_secs_f64() * 1000.0;

    let ids = |report: &SearchReport| -> BTreeMap<Uuid, usize> {
        report
            .results
            .iter()
            .enumerate()
            .map(|(rank, result)| (result.segment_id, rank))
            .collect()
    };
    let (pg_ids, tv_ids) = (ids(&postgres), ids(&tantivy));
    let run = |report: SearchReport, elapsed_ms: f64| ProviderRun {
        elapsed_ms,
        ocr_coverage: report.ocr_coverage,
        hits: report
            .results
            .into_iter()
            .map(|result| ComparedHit {
                segment_id: result.segment_id,
                video_id: result.video_id,
                source_kind: result.source_kind,
                start_seconds: result.start_seconds,
            })
            .collect(),
    };
    Ok(ProviderComparison {
        query: request.query,
        shared: pg_ids.keys().filter(|id| tv_ids.contains_key(id)).count(),
        only_postgres: pg_ids
            .keys()
            .filter(|id| !tv_ids.contains_key(id))
            .copied()
            .collect(),
        only_tantivy: tv_ids
            .keys()
            .filter(|id| !pg_ids.contains_key(id))
            .copied()
            .collect(),
        postgres: run(postgres, postgres_ms),
        tantivy: run(tantivy, tantivy_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(query: &str) -> SearchRequest {
        SearchRequest {
            database_url: String::new(),
            query: query.into(),
            top_k: 10,
            mode: SearchMode::Fts,
            source_kind: None,
            modality: None,
            video_id: None,
            language: None,
            transcript_start_min: None,
            transcript_start_max: None,
            upload_date_from: None,
            upload_date_to: None,
            duration_min: None,
            duration_max: None,
            channel_query: None,
            title_query: None,
            category_query: None,
            tag_query: None,
            metadata_query: None,
            view_count_min: None,
            view_count_max: None,
        }
    }

    #[test]
    fn rejects_semantic_modes_and_raw_metadata() {
        let mut hybrid = request("planet");
        hybrid.mode = SearchMode::Hybrid;
        assert!(plan(&hybrid).is_err());
        let mut metadata = request("planet");
        metadata.metadata_query = Some("x".into());
        assert!(plan(&metadata).is_err());
        assert!(plan(&request("  ")).is_err());
    }

    #[test]
    fn literal_terms_match_substrings_with_filters_and_tombstones() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = TantivyIndex::open(dir.path(), schema()).unwrap();
        let video = Uuid::new_v4();
        let passage = |id: Uuid, text: &str, kind: &str, start: i64| {
            Document::new(id.to_string(), 1)
                .field("kind", Value::Keyword("passage".into()))
                .field("doc_id", Value::Keyword(id.to_string()))
                .field("text", Value::Text(text.into()))
                .field("video_id", Value::Keyword(video.to_string()))
                .field("stream_id", Value::Keyword(Uuid::new_v4().to_string()))
                .field("source_kind", Value::Keyword(kind.into()))
                .field("translated", Value::Boolean(false))
                .field("source_url", Value::Keyword("https://youtu.be/x".into()))
                .field("start_ms", Value::Integer(start))
                .field("order", Value::Keyword(format!("{video}:{start:015}:{id}")))
        };
        let (spoken, visible) = (Uuid::new_v4(), Uuid::new_v4());
        index
            .apply_batch(&[
                Mutation::Upsert {
                    document: passage(spoken, "The Planetary orbit", "caption_manual", 7_000),
                },
                Mutation::Upsert {
                    document: passage(visible, "planet orbit slide", "visual_ocr", 12_000),
                },
            ])
            .unwrap();

        let ids = |index: &TantivyIndex, request: &SearchRequest| -> Vec<String> {
            let plan = plan(request).unwrap();
            index
                .search(&EngineRequest::new(plan.query).sort_by("order", false))
                .unwrap()
                .hits
                .into_iter()
                .map(|hit| hit.id)
                .collect()
        };
        // Literal substring: `planet` also matches `Planetary`, in timestamp order.
        assert_eq!(
            ids(&index, &request("PLANET orbit")),
            [spoken.to_string(), visible.to_string()]
        );
        let mut visible_only = request("planet");
        visible_only.modality = Some(SearchModality::Visible);
        assert_eq!(ids(&index, &visible_only), [visible.to_string()]);
        let mut late = request("orbit");
        late.transcript_start_min = Some(10.0);
        assert_eq!(ids(&index, &late), [visible.to_string()]);

        index.delete(&visible.to_string(), 2).unwrap();
        assert_eq!(ids(&index, &request("orbit")), [spoken.to_string()]);
        let result = to_result(index.document(&spoken.to_string()).unwrap()).unwrap();
        assert_eq!(result.start_seconds, Some(7.0));
        assert_eq!(result.video_id, video);
    }
}
