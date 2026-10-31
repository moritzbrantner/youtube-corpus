//! Historical events and their separate source-backed claims (Atlas 7.2, 7.3; #106).
//!
//! An event groups claims researchers believe refer to one occurrence. It holds no date, place
//! or participants of its own: every such statement is a claim (assertion) anchored to one
//! original video moment, with its own typed date reading, certainty, qualification and review
//! status, in the shape of the historical-source-atlas v2 assertion model. Contradictory claims
//! stay side by side; a reviewed preferred date reading marks one claim without touching the
//! alternatives. Merging and splitting events only move claims between events, so source
//! evidence and corpus filtering (by the claim's video) are unaffected. Participants and places
//! are global canonical identities (#104), not per-corpus names. Automatically suggested claims
//! come from `crate::event_extraction` (#107) and stay unverified until reviewed here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::identities::{
    list_identities, live_identity, CorpusRef, IdentityFilter, IdentitySummary,
};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EventInputError(String);

fn input(message: &str) -> anyhow::Error {
    EventInputError(message.into()).into()
}

/// A typed historical date or range. Years are historical: negative is BC, there is no year 0.
/// A missing end is a single point at the start's precision (year, month or day).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDate {
    pub start_year: i32,
    pub start_month: Option<i16>,
    pub start_day: Option<i16>,
    pub end_year: Option<i32>,
    pub end_month: Option<i16>,
    pub end_day: Option<i16>,
    #[serde(default)]
    pub approximate: bool,
}

impl EventDate {
    pub(crate) fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.start_year == 0 || self.end_year == Some(0) {
            return Err(input("there is no year 0; use -1 for 1 BC"));
        }
        let month_ok = |m: Option<i16>| m.is_none_or(|m| (1..=12).contains(&m));
        let day_ok = |d: Option<i16>| d.is_none_or(|d| (1..=31).contains(&d));
        if !month_ok(self.start_month) || !month_ok(self.end_month) {
            return Err(input("months are 1-12"));
        }
        if !day_ok(self.start_day) || !day_ok(self.end_day) {
            return Err(input("days are 1-31"));
        }
        if (self.start_day.is_some() && self.start_month.is_none())
            || (self.end_day.is_some() && self.end_month.is_none())
        {
            return Err(input("a day needs its month"));
        }
        if self.end_year.is_none() && (self.end_month.is_some() || self.end_day.is_some()) {
            return Err(input("a range end needs its year"));
        }
        if let Some(end_year) = self.end_year {
            let start = (
                self.start_year,
                self.start_month.unwrap_or(1),
                self.start_day.unwrap_or(1),
            );
            let end = (
                end_year,
                self.end_month.unwrap_or(12),
                self.end_day.unwrap_or(31),
            );
            if end < start {
                return Err(input("a date range cannot end before it starts"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimPredicate {
    Date,
    Participant,
    Place,
}

impl ClaimPredicate {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Participant => "participant",
            Self::Place => "place",
        }
    }
}

/// A claim to record from one reviewed video moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimInput {
    pub video_id: Uuid,
    /// `spoken` (a transcript segment) or `visible` (an OCR text track), as in moment URLs.
    pub modality: String,
    pub evidence_id: Uuid,
    pub predicate: ClaimPredicate,
    /// For `date`; omitted when the source leaves the date unknown.
    pub date: Option<EventDate>,
    /// For `participant` and `place`: a global canonical identity.
    pub identity_id: Option<Uuid>,
    /// `affirms` (default) or `denies`.
    pub polarity: Option<String>,
    /// `stated` (default), `hedged` or `inferred`.
    pub certainty: Option<String>,
    #[serde(default)]
    pub qualification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEventRequest {
    pub label: String,
    /// Every event starts from the video evidence of its first claim.
    pub claim: ClaimInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSummary {
    pub id: Uuid,
    pub label: String,
    /// Claims, videos and date readings in the requested scope.
    pub claim_count: i64,
    pub video_count: i64,
    /// Distinct dated readings among non-rejected date claims.
    pub date_readings: i64,
    /// More than one distinct date reading, or a participant/place both affirmed and denied.
    pub disputed: bool,
    pub earliest_year: Option<i32>,
    pub latest_year: Option<i32>,
    /// The reviewed preferred date reading, if a researcher chose one (event-wide).
    pub preferred_date: Option<EventDate>,
    /// Every corpus with a claim's video, regardless of scope.
    pub corpora: Vec<CorpusRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventClaim {
    pub id: Uuid,
    pub video_id: Uuid,
    pub video_title: Option<String>,
    pub modality: String,
    pub evidence_id: Uuid,
    /// The original evidence text; `None` when reprocessing replaced the evidence.
    pub evidence_text: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub predicate: ClaimPredicate,
    pub date: Option<EventDate>,
    /// The identity as recorded; a merged identity resolves to its current one.
    pub identity_id: Option<Uuid>,
    /// `None` when the identity has no current corpus evidence.
    pub identity: Option<IdentitySummary>,
    pub polarity: String,
    pub certainty: String,
    pub qualification: String,
    pub review_status: String,
    pub preferred: bool,
    pub source_url: String,
    /// The video's publication date (`YYYY-MM-DD`), never the claimed historical date.
    pub publication_date: Option<String>,
    /// Set for an automatically suggested claim (#107); `None` for one a researcher recorded.
    pub extraction: Option<ClaimExtraction>,
}

/// Where an automatically suggested claim came from (#107).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimExtraction {
    pub run_id: Uuid,
    pub producer: String,
    pub producer_version: String,
    /// The matched expression exactly as it appears in the evidence text, and its byte range.
    pub source_text: String,
    pub source_byte_start: i32,
    pub source_byte_end: i32,
    pub confidence: f32,
    pub extracted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventProfile {
    /// The event that was asked for; differs from `event.id` after a merge.
    pub requested_id: Uuid,
    pub event: EventSummary,
    /// Claims in scope.
    pub claims: Vec<EventClaim>,
    /// Claims whose videos are outside the requested corpus.
    pub claims_outside_scope: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// Set `reviewStatus` of `claimIds`.
    Review,
    /// Make the one claim in `claimIds` the preferred date reading (confirms it).
    Prefer,
    /// Clear the preferred date reading.
    Unprefer,
    Rename,
    /// Move every claim of `otherEventId` here; the other event redirects to this one.
    Merge,
    /// Move `claimIds` into a new event named `label`.
    Split,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCorrectionRequest {
    pub action: EventAction,
    #[serde(default)]
    pub claim_ids: Vec<Uuid>,
    pub review_status: Option<String>,
    pub other_event_id: Option<Uuid>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCorrectionReport {
    pub event_id: Uuid,
    pub created_event_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub corpus: Option<Uuid>,
    pub query: String,
    pub ids: Option<Vec<Uuid>>,
    pub limit: i64,
}

fn one_of(
    value: Option<&str>,
    options: &[&'static str],
    what: &str,
) -> anyhow::Result<&'static str> {
    match value {
        None => Ok(options[0]),
        Some(value) => options
            .iter()
            .find(|option| **option == value)
            .copied()
            .ok_or_else(|| input(&format!("unknown {what} `{value}`"))),
    }
}

pub(crate) async fn insert_claim(
    tx: &mut Transaction<'_, Postgres>,
    event: Uuid,
    claim: &ClaimInput,
    extraction: Option<&ClaimExtraction>,
) -> anyhow::Result<Uuid> {
    let modality = one_of(Some(&claim.modality), &["spoken", "visible"], "modality")?;
    let polarity = one_of(
        claim.polarity.as_deref(),
        &["affirms", "denies"],
        "polarity",
    )?;
    let certainty = one_of(
        claim.certainty.as_deref(),
        &["stated", "hedged", "inferred"],
        "certainty",
    )?;
    let evidence_exists: bool = sqlx::query_scalar(if modality == "spoken" {
        "SELECT EXISTS (SELECT 1 FROM transcript_segments WHERE id = $1 AND video_id = $2)"
    } else {
        "SELECT EXISTS (SELECT 1 FROM visual_text_tracks WHERE id = $1 AND video_id = $2)"
    })
    .bind(claim.evidence_id)
    .bind(claim.video_id)
    .fetch_one(&mut **tx)
    .await?;
    if !evidence_exists {
        return Err(input("the claim's evidence is not part of that video"));
    }
    let identity = match claim.predicate {
        ClaimPredicate::Date => {
            if claim.identity_id.is_some() {
                return Err(input("a date claim names no identity"));
            }
            if let Some(date) = &claim.date {
                date.validate()?;
            }
            None
        }
        ClaimPredicate::Participant | ClaimPredicate::Place => {
            if claim.date.is_some() {
                return Err(input("participant and place claims carry no date"));
            }
            let identity = claim
                .identity_id
                .ok_or_else(|| input("participant and place claims need identityId"))?;
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM canonical_identities WHERE id = $1)",
            )
            .bind(identity)
            .fetch_one(&mut **tx)
            .await?;
            if !exists {
                return Err(input("identity not found"));
            }
            Some(identity)
        }
    };
    let date = claim.date.clone();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO historical_event_claims
           (id, event_id, video_id, modality, evidence_id, predicate,
            date_start_year, date_start_month, date_start_day,
            date_end_year, date_end_month, date_end_day, date_approximate,
            identity_id, polarity, certainty, qualification, extraction_run_id, source_text,
            source_byte_start, source_byte_end, extraction_confidence)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                 $18, $19, $20, $21, $22)",
    )
    .bind(id)
    .bind(event)
    .bind(claim.video_id)
    .bind(modality)
    .bind(claim.evidence_id)
    .bind(claim.predicate.as_str())
    .bind(date.as_ref().map(|d| d.start_year))
    .bind(date.as_ref().and_then(|d| d.start_month))
    .bind(date.as_ref().and_then(|d| d.start_day))
    .bind(date.as_ref().and_then(|d| d.end_year))
    .bind(date.as_ref().and_then(|d| d.end_month))
    .bind(date.as_ref().and_then(|d| d.end_day))
    .bind(date.as_ref().is_some_and(|d| d.approximate))
    .bind(identity)
    .bind(polarity)
    .bind(certainty)
    .bind(claim.qualification.trim())
    .bind(extraction.map(|e| e.run_id))
    .bind(extraction.map(|e| e.source_text.as_str()))
    .bind(extraction.map(|e| e.source_byte_start))
    .bind(extraction.map(|e| e.source_byte_end))
    .bind(extraction.map(|e| e.confidence))
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE historical_events SET updated_at = now() WHERE id = $1")
        .bind(event)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

async fn log(
    tx: &mut Transaction<'_, Postgres>,
    action: &str,
    event: Uuid,
    other: Option<Uuid>,
    claims: &[Uuid],
    detail: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO historical_event_corrections
           (id, action, event_id, other_event_id, claim_ids, detail)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(action)
    .bind(event)
    .bind(other)
    .bind(claims)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn clean_label(label: &str) -> anyhow::Result<String> {
    let label = label.trim();
    if label.is_empty() {
        return Err(input("an event needs a label"));
    }
    Ok(label.to_string())
}

pub async fn create_event(pool: &PgPool, request: CreateEventRequest) -> anyhow::Result<Uuid> {
    let label = clean_label(&request.label)?;
    let mut tx = pool.begin().await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO historical_events (id, label) VALUES ($1, $2)")
        .bind(id)
        .bind(&label)
        .execute(&mut *tx)
        .await?;
    insert_claim(&mut tx, id, &request.claim, None).await?;
    tx.commit().await?;
    Ok(id)
}

/// The event a (possibly merged) id currently resolves to.
pub async fn live_event(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Uuid>> {
    let mut current = id;
    for _ in 0..64 {
        let row: Option<Option<Uuid>> =
            sqlx::query_scalar("SELECT merged_into FROM historical_events WHERE id = $1")
                .bind(current)
                .fetch_optional(pool)
                .await?;
        match row {
            None => return Ok(None),
            Some(None) => return Ok(Some(current)),
            Some(Some(next)) => current = next,
        }
    }
    Ok(None)
}

pub async fn add_claim(pool: &PgPool, event: Uuid, claim: ClaimInput) -> anyhow::Result<Uuid> {
    let event = live_event(pool, event)
        .await?
        .ok_or_else(|| anyhow::anyhow!("event not found"))?;
    let mut tx = pool.begin().await?;
    let id = insert_claim(&mut tx, event, &claim, None).await?;
    tx.commit().await?;
    Ok(id)
}

/// Claims whose video is in corpus `$1` (every claim when `$1` is null).
macro_rules! scoped_claims {
    () => {
        "WITH scoped AS (
           SELECT c.* FROM historical_event_claims c
           WHERE $1::uuid IS NULL
              OR EXISTS (SELECT 1 FROM corpus_videos cv
                         WHERE cv.corpus_id = $1 AND cv.video_id = c.video_id))"
    };
}

pub(crate) fn date_from_row(
    row: &sqlx::postgres::PgRow,
    prefix: &str,
) -> anyhow::Result<Option<EventDate>> {
    let col = |name: &str| format!("{prefix}{name}");
    let Some(start_year) = row.try_get::<Option<i32>, _>(col("date_start_year").as_str())? else {
        return Ok(None);
    };
    Ok(Some(EventDate {
        start_year,
        start_month: row.try_get(col("date_start_month").as_str())?,
        start_day: row.try_get(col("date_start_day").as_str())?,
        end_year: row.try_get(col("date_end_year").as_str())?,
        end_month: row.try_get(col("date_end_month").as_str())?,
        end_day: row.try_get(col("date_end_day").as_str())?,
        approximate: row.try_get(col("date_approximate").as_str())?,
    }))
}

pub async fn list_events(pool: &PgPool, filter: &EventFilter) -> anyhow::Result<Vec<EventSummary>> {
    let rows = sqlx::query(concat!(
        scoped_claims!(),
        ",
         live AS (
           SELECT format('%s/%s/%s-%s/%s/%s~%s', c.date_start_year, c.date_start_month,
                         c.date_start_day, c.date_end_year, c.date_end_month, c.date_end_day,
                         c.date_approximate) AS reading,
                  c.*
           FROM scoped c WHERE c.review_status <> 'rejected'
         )
         SELECT e.id, e.label,
           (SELECT count(*) FROM scoped s WHERE s.event_id = e.id) AS claims,
           (SELECT count(DISTINCT s.video_id) FROM scoped s WHERE s.event_id = e.id) AS videos,
           (SELECT count(DISTINCT l.reading) FROM live l
             WHERE l.event_id = e.id AND l.predicate = 'date' AND l.date_start_year IS NOT NULL)
             AS readings,
           EXISTS (SELECT 1 FROM live a JOIN live d
                     ON d.event_id = a.event_id AND d.predicate = a.predicate
                    AND d.identity_id = a.identity_id
                    WHERE a.event_id = e.id AND a.polarity = 'affirms' AND d.polarity = 'denies')
             AS identity_disputed,
           (SELECT min(l.date_start_year) FROM live l
             WHERE l.event_id = e.id AND l.predicate = 'date') AS earliest,
           (SELECT max(coalesce(l.date_end_year, l.date_start_year)) FROM live l
             WHERE l.event_id = e.id AND l.predicate = 'date') AS latest,
           p.date_start_year AS p_date_start_year, p.date_start_month AS p_date_start_month,
           p.date_start_day AS p_date_start_day, p.date_end_year AS p_date_end_year,
           p.date_end_month AS p_date_end_month, p.date_end_day AS p_date_end_day,
           p.date_approximate AS p_date_approximate,
           (SELECT coalesce(jsonb_agg(DISTINCT jsonb_build_object('id', co.id, 'name', co.name)),
                            '[]'::jsonb)
              FROM historical_event_claims c
              JOIN corpus_videos cv ON cv.video_id = c.video_id
              JOIN corpora co ON co.id = cv.corpus_id
             WHERE c.event_id = e.id) AS corpora
         FROM historical_events e
         LEFT JOIN historical_event_claims p ON p.event_id = e.id AND p.preferred
         WHERE e.merged_into IS NULL
           AND ($2::uuid[] IS NULL OR e.id = ANY($2))
           AND ($3 = '' OR strpos(lower(e.label), $3) > 0)
           AND ($1::uuid IS NULL OR EXISTS (SELECT 1 FROM scoped s WHERE s.event_id = e.id))
         ORDER BY earliest NULLS LAST, lower(e.label), e.id
         LIMIT $4"
    ))
    .bind(filter.corpus)
    .bind(filter.ids.as_deref())
    .bind(filter.query.trim().to_lowercase())
    .bind(filter.limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let corpora: serde_json::Value = row.try_get("corpora")?;
            let mut corpora: Vec<CorpusRef> = serde_json::from_value(corpora)?;
            corpora.sort_by(|a, b| a.name.cmp(&b.name));
            let readings: i64 = row.try_get("readings")?;
            let identity_disputed: bool = row.try_get("identity_disputed")?;
            Ok(EventSummary {
                id: row.try_get("id")?,
                label: row.try_get("label")?,
                claim_count: row.try_get("claims")?,
                video_count: row.try_get("videos")?,
                date_readings: readings,
                disputed: readings > 1 || identity_disputed,
                earliest_year: row.try_get("earliest")?,
                latest_year: row.try_get("latest")?,
                preferred_date: date_from_row(&row, "p_")?,
                corpora,
            })
        })
        .collect()
}

pub async fn event_profile(
    pool: &PgPool,
    requested_id: Uuid,
    corpus: Option<Uuid>,
) -> anyhow::Result<Option<EventProfile>> {
    let Some(id) = live_event(pool, requested_id).await? else {
        return Ok(None);
    };
    let filter = EventFilter {
        corpus,
        ids: Some(vec![id]),
        limit: 1,
        ..EventFilter::default()
    };
    let scoped = list_events(pool, &filter).await?.into_iter().next();
    // An event without claims in the scope still opens; its scoped counts are zero.
    let event = match scoped {
        Some(event) => event,
        None => {
            let unscoped = EventFilter {
                corpus: None,
                ..filter
            };
            let Some(event) = list_events(pool, &unscoped).await?.into_iter().next() else {
                return Ok(None);
            };
            EventSummary {
                claim_count: 0,
                video_count: 0,
                date_readings: 0,
                disputed: false,
                earliest_year: None,
                latest_year: None,
                ..event
            }
        }
    };
    let rows = sqlx::query(concat!(
        scoped_claims!(),
        "
         SELECT c.*, v.title AS video_title, v.source_url, v.upload_date, v.timestamp,
                v.release_timestamp,
                r.producer AS extraction_producer, r.producer_version AS extraction_version,
                r.created_at AS extracted_at,
                coalesce(ts.text, vt.text) AS evidence_text,
                coalesce(ts.start_seconds, vt.start_seconds) AS start_seconds,
                coalesce(ts.end_seconds, vt.end_seconds) AS end_seconds
         FROM scoped c
         JOIN videos v ON v.id = c.video_id
         LEFT JOIN historical_event_extraction_runs r ON r.id = c.extraction_run_id
         LEFT JOIN transcript_segments ts ON c.modality = 'spoken' AND ts.id = c.evidence_id
         LEFT JOIN visual_text_tracks vt ON c.modality = 'visible' AND vt.id = c.evidence_id
         WHERE c.event_id = $2
         ORDER BY CASE c.predicate WHEN 'date' THEN 0 WHEN 'participant' THEN 1 ELSE 2 END,
                  c.date_start_year NULLS LAST, c.created_at, c.id"
    ))
    .bind(corpus)
    .bind(id)
    .fetch_all(pool)
    .await?;
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM historical_event_claims WHERE event_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;

    // Participants and places are global identities; a merged one resolves to its target.
    let mut live_ids: HashMap<Uuid, Option<Uuid>> = HashMap::new();
    for row in &rows {
        if let Some(identity) = row.try_get::<Option<Uuid>, _>("identity_id")? {
            if let std::collections::hash_map::Entry::Vacant(entry) = live_ids.entry(identity) {
                entry.insert(live_identity(pool, identity).await?);
            }
        }
    }
    let wanted: Vec<Uuid> = live_ids.values().flatten().copied().collect();
    let summaries: HashMap<Uuid, IdentitySummary> = if wanted.is_empty() {
        HashMap::new()
    } else {
        list_identities(
            pool,
            &IdentityFilter {
                ids: Some(wanted),
                limit: 500,
                ..IdentityFilter::default()
            },
        )
        .await?
        .into_iter()
        .map(|summary| (summary.id, summary))
        .collect()
    };

    let claims = rows
        .iter()
        .map(|row| {
            let predicate: String = row.try_get("predicate")?;
            let recorded: Option<Uuid> = row.try_get("identity_id")?;
            let current = recorded.and_then(|id| live_ids.get(&id).copied().flatten());
            let run: Option<Uuid> = row.try_get("extraction_run_id")?;
            let extraction = match run {
                None => None,
                Some(run_id) => Some(ClaimExtraction {
                    run_id,
                    producer: row.try_get("extraction_producer")?,
                    producer_version: row.try_get("extraction_version")?,
                    source_text: row.try_get("source_text")?,
                    source_byte_start: row.try_get("source_byte_start")?,
                    source_byte_end: row.try_get("source_byte_end")?,
                    confidence: row.try_get("extraction_confidence")?,
                    extracted_at: row.try_get("extracted_at")?,
                }),
            };
            Ok(EventClaim {
                source_url: row.try_get("source_url")?,
                publication_date: crate::entities::publication_date(row)?,
                extraction,
                id: row.try_get("id")?,
                video_id: row.try_get("video_id")?,
                video_title: row.try_get("video_title")?,
                modality: row.try_get("modality")?,
                evidence_id: row.try_get("evidence_id")?,
                evidence_text: row.try_get("evidence_text")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                predicate: match predicate.as_str() {
                    "date" => ClaimPredicate::Date,
                    "participant" => ClaimPredicate::Participant,
                    _ => ClaimPredicate::Place,
                },
                date: date_from_row(row, "")?,
                identity_id: current.or(recorded),
                identity: current.and_then(|id| summaries.get(&id).cloned()),
                polarity: row.try_get("polarity")?,
                certainty: row.try_get("certainty")?,
                qualification: row.try_get("qualification")?,
                review_status: row.try_get("review_status")?,
                preferred: row.try_get("preferred")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(EventProfile {
        requested_id,
        event,
        claims_outside_scope: total - rows.len() as i64,
        claims,
    }))
}

/// Which events a historical timeline shows: one event, the events an identity takes part in
/// (as a participant or place claim), or every event; claims are limited to `corpus`.
#[derive(Debug, Clone, Default)]
pub struct TimelineFilter {
    pub corpus: Option<Uuid>,
    pub identity: Option<Uuid>,
    pub event: Option<Uuid>,
}

/// One event on the historical timeline with its date claims (every review status).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub event: EventSummary,
    pub claims: Vec<EventClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalTimeline {
    pub events: Vec<TimelineEvent>,
    /// More events matched than the timeline holds.
    pub truncated: bool,
}

/// Events shown on one historical timeline.
const TIMELINE_EVENTS: usize = 200;

/// Historical (claimed) dates, kept apart from video publication dates (Atlas 5.5, 7.1).
pub async fn historical_timeline(
    pool: &PgPool,
    filter: &TimelineFilter,
) -> anyhow::Result<HistoricalTimeline> {
    let mut ids: Vec<Uuid> = if let Some(event) = filter.event {
        live_event(pool, event).await?.into_iter().collect()
    } else if let Some(identity) = filter.identity {
        let identity = live_identity(pool, identity).await?.unwrap_or(identity);
        // Claims keep the identity they recorded; merged identities resolve to this one.
        sqlx::query_scalar(
            "WITH RECURSIVE merged AS (
               SELECT $1::uuid AS id
               UNION SELECT c.id FROM canonical_identities c JOIN merged m ON c.merged_into = m.id
             )
             SELECT DISTINCT e.id FROM historical_event_claims c
             JOIN historical_events e ON e.id = c.event_id AND e.merged_into IS NULL
             WHERE c.identity_id IN (SELECT id FROM merged) AND c.review_status <> 'rejected'
             ORDER BY e.id",
        )
        .bind(identity)
        .fetch_all(pool)
        .await?
    } else {
        list_events(
            pool,
            &EventFilter {
                corpus: filter.corpus,
                limit: TIMELINE_EVENTS as i64 + 1,
                ..EventFilter::default()
            },
        )
        .await?
        .into_iter()
        .map(|event| event.id)
        .collect()
    };
    let truncated = ids.len() > TIMELINE_EVENTS;
    ids.truncate(TIMELINE_EVENTS);
    let mut events = Vec::new();
    for id in ids {
        let Some(profile) = event_profile(pool, id, filter.corpus).await? else {
            continue;
        };
        let claims: Vec<EventClaim> = profile
            .claims
            .into_iter()
            .filter(|claim| claim.predicate == ClaimPredicate::Date)
            .collect();
        if !claims.is_empty() {
            events.push(TimelineEvent {
                event: profile.event,
                claims,
            });
        }
    }
    events.sort_by_key(|entry| {
        (
            entry.event.earliest_year.is_none(),
            entry.event.earliest_year,
            entry.event.label.to_lowercase(),
        )
    });
    Ok(HistoricalTimeline { events, truncated })
}

async fn require_live(tx: &mut Transaction<'_, Postgres>, event: Uuid) -> anyhow::Result<String> {
    let row: Option<(String, Option<Uuid>)> =
        sqlx::query_as("SELECT label, merged_into FROM historical_events WHERE id = $1 FOR UPDATE")
            .bind(event)
            .fetch_optional(&mut **tx)
            .await?;
    match row {
        None => Err(anyhow::anyhow!("event not found")),
        Some((_, Some(_))) => Err(input(
            "that event was merged; correct the event it redirects to",
        )),
        Some((label, None)) => Ok(label),
    }
}

async fn require_claims_of(
    tx: &mut Transaction<'_, Postgres>,
    event: Uuid,
    claims: &[Uuid],
) -> anyhow::Result<()> {
    if claims.is_empty() {
        return Err(input("select at least one claim"));
    }
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM historical_event_claims WHERE event_id = $1 AND id = ANY($2)",
    )
    .bind(event)
    .bind(claims)
    .fetch_one(&mut **tx)
    .await?;
    if owned != claims.len() as i64 {
        return Err(input("every selected claim must belong to this event"));
    }
    Ok(())
}

pub async fn correct_event(
    pool: &PgPool,
    event: Uuid,
    request: EventCorrectionRequest,
) -> anyhow::Result<EventCorrectionReport> {
    let mut tx = pool.begin().await?;
    let label = require_live(&mut tx, event).await?;
    let claims = &request.claim_ids;
    let mut created = None;
    let mut other_event = None;
    let mut detail = String::new();
    let action = match request.action {
        EventAction::Review => {
            require_claims_of(&mut tx, event, claims).await?;
            let status = request
                .review_status
                .as_deref()
                .ok_or_else(|| input("reviewStatus is required"))?;
            let status = one_of(
                Some(status),
                &["unverified", "confirmed", "rejected"],
                "review status",
            )?;
            // Only a confirmed claim can be the preferred reading.
            sqlx::query(
                "UPDATE historical_event_claims
                 SET review_status = $3, preferred = preferred AND $3 = 'confirmed',
                     updated_at = now()
                 WHERE event_id = $1 AND id = ANY($2)",
            )
            .bind(event)
            .bind(claims)
            .bind(status)
            .execute(&mut *tx)
            .await?;
            detail = status.into();
            "review"
        }
        EventAction::Prefer => {
            require_claims_of(&mut tx, event, claims).await?;
            let [claim] = claims.as_slice() else {
                return Err(input("prefer exactly one date claim"));
            };
            let dated: bool = sqlx::query_scalar(
                "SELECT predicate = 'date' AND date_start_year IS NOT NULL
                 FROM historical_event_claims WHERE id = $1",
            )
            .bind(claim)
            .fetch_one(&mut *tx)
            .await?;
            if !dated {
                return Err(input(
                    "only a dated date claim can be the preferred reading",
                ));
            }
            sqlx::query(
                "UPDATE historical_event_claims SET preferred = false, updated_at = now()
                 WHERE event_id = $1 AND preferred",
            )
            .bind(event)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE historical_event_claims
                 SET preferred = true, review_status = 'confirmed', updated_at = now()
                 WHERE id = $1",
            )
            .bind(claim)
            .execute(&mut *tx)
            .await?;
            "prefer"
        }
        EventAction::Unprefer => {
            sqlx::query(
                "UPDATE historical_event_claims SET preferred = false, updated_at = now()
                 WHERE event_id = $1 AND preferred",
            )
            .bind(event)
            .execute(&mut *tx)
            .await?;
            "unprefer"
        }
        EventAction::Rename => {
            let new_label = clean_label(request.label.as_deref().unwrap_or(""))?;
            sqlx::query("UPDATE historical_events SET label = $2 WHERE id = $1")
                .bind(event)
                .bind(&new_label)
                .execute(&mut *tx)
                .await?;
            detail = format!("{label} -> {new_label}");
            "rename"
        }
        EventAction::Merge => {
            let other = request
                .other_event_id
                .ok_or_else(|| input("otherEventId is required"))?;
            if other == event {
                return Err(input("an event cannot be merged into itself"));
            }
            require_live(&mut tx, other).await?;
            // The target's preferred reading wins; the other's becomes one more alternative.
            let target_has_preferred: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM historical_event_claims
                                WHERE event_id = $1 AND preferred)",
            )
            .bind(event)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE historical_event_claims
                 SET event_id = $1, preferred = preferred AND NOT $3, updated_at = now()
                 WHERE event_id = $2",
            )
            .bind(event)
            .bind(other)
            .bind(target_has_preferred)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE historical_events SET merged_into = $1, updated_at = now()
                 WHERE id = $2 OR merged_into = $2",
            )
            .bind(event)
            .bind(other)
            .execute(&mut *tx)
            .await?;
            other_event = Some(other);
            "merge"
        }
        EventAction::Split => {
            require_claims_of(&mut tx, event, claims).await?;
            let remaining: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM historical_event_claims
                 WHERE event_id = $1 AND NOT (id = ANY($2))",
            )
            .bind(event)
            .bind(claims)
            .fetch_one(&mut *tx)
            .await?;
            if remaining == 0 {
                return Err(input("leave at least one claim in this event"));
            }
            let new_label = match request.label.as_deref() {
                Some(new_label) => clean_label(new_label)?,
                None => format!("{label} (split)"),
            };
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO historical_events (id, label) VALUES ($1, $2)")
                .bind(id)
                .bind(&new_label)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE historical_event_claims SET event_id = $1, updated_at = now()
                 WHERE event_id = $2 AND id = ANY($3)",
            )
            .bind(id)
            .bind(event)
            .bind(claims)
            .execute(&mut *tx)
            .await?;
            created = Some(id);
            other_event = Some(id);
            "split"
        }
    };
    sqlx::query("UPDATE historical_events SET updated_at = now() WHERE id = $1")
        .bind(event)
        .execute(&mut *tx)
        .await?;
    log(&mut tx, action, event, other_event, claims, &detail).await?;
    tx.commit().await?;
    Ok(EventCorrectionReport {
        event_id: event,
        created_event_id: created,
    })
}
