//! Automatically suggested, unreviewed historical event claims (Atlas 7.1; issue #107).
//!
//! Dated expressions in transcript segments and visual text (OCR) tracks come from the nlp-stack
//! temporal normalizer (`text_linguistics::normalize_temporal_expressions`, nlp-stack#156).
//! This module turns them into claims: it names the event each date is about, and it pairs dates
//! with participants and places from the nlp-stack entity mentions already linked to canonical
//! identities (#104). Claims, events and review belong to `crate::events` (#106).
//!
//! A suggestion is always `unverified`: it records its producer, run, the exact matched text and
//! byte range, and a confidence, and is never presented as an established fact. Reruns keep
//! reviewed claims (confirmed or rejected) and unchanged suggestions in place and drop only
//! suggestions the new run no longer produces.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use text_linguistics::{
    normalize_temporal_expressions, HistoricalDate, TemporalCertainty, TemporalExpression,
    TemporalPolarity, TEMPORAL_MIN_CONFIDENCE, TEMPORAL_NORMALIZER_PRODUCER,
};
use uuid::Uuid;

use crate::entities::{video_text_sources, EntityModality};
use crate::events::{insert_claim, ClaimExtraction, ClaimInput, ClaimPredicate, EventDate};

/// Producer identity recorded on every extraction run and suggested claim.
pub const EVENT_CLAIM_PRODUCER: &str = TEMPORAL_NORMALIZER_PRODUCER;
/// The pinned nlp-stack revision of the producer (`text-linguistics` in Cargo.toml).
pub const EVENT_CLAIM_PRODUCER_VERSION: &str = crate::entities::ENTITY_PRODUCER_VERSION;
/// Longest fallback event label taken from the evidence text.
const LABEL_CHARS: usize = 72;

static NAMED_EVENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(?:(?:first|second|third|fourth|great|holy)\s+)?(?:battle|siege|treaty|council|fall|sack|war|revolution|conquest|crusade|edict|peace|massacre|coronation|crucifixion|destruction|founding|invasion|discovery|death|birth|assassination|schism|exile|burning|schlacht|belagerung|konzil|vertrag|frieden|zerstörung|eroberung|tod|geburt)\s+(?:of|at|on|von|der|des|bei|zu)\s+(?:the\s+)?[\p{L}'’-]+(?:\s+[\p{L}'’-]+){0,2}|world\s+war\s+(?:one|two|i{1,2}|1|2)|(?:the\s+)?(?:protestant\s+)?reformation|(?:the\s+)?(?:french|american|russian|industrial|glorious)\s+revolution|(?:the\s+)?great\s+schism|(?:the\s+)?black\s+death|(?:the\s+)?holocaust|(?:the\s+)?cold\s+war)",
    )
    .expect("valid named-event pattern")
});

fn event_date(date: &HistoricalDate) -> EventDate {
    EventDate {
        start_year: date.start_year,
        start_month: date.start_month.map(i16::from),
        start_day: date.start_day.map(i16::from),
        end_year: date.end_year,
        end_month: date.end_month.map(i16::from),
        end_day: date.end_day.map(i16::from),
        approximate: date.approximate,
    }
}

fn certainty(certainty: TemporalCertainty) -> &'static str {
    match certainty {
        TemporalCertainty::Stated => "stated",
        TemporalCertainty::Hedged => "hedged",
    }
}

fn polarity(polarity: TemporalPolarity) -> &'static str {
    match polarity {
        TemporalPolarity::Affirms => "affirms",
        TemporalPolarity::Denies => "denies",
    }
}

/// Words that end a named-event phrase (`Battle of Hastings in 1066` → `Battle of Hastings`).
const LABEL_STOP: &[&str] = &[
    "in", "on", "at", "was", "were", "is", "and", "by", "from", "of", "the", "a", "an", "to",
    "which", "that", "when", "where", "who", "happened", "took", "im", "um", "war", "ist", "und",
    "fand", "statt", "am", "wurde", "die", "das", "der", "around", "about", "during", "after",
    "before", "since", "until", "between",
];
/// Lower-case connecting words of a label.
const LABEL_LOWER: &[&str] = &["of", "the", "at", "on", "von", "der", "des", "bei", "zu"];

fn title_case(phrase: &str) -> String {
    phrase
        .split_whitespace()
        .enumerate()
        .map(|(i, word)| {
            let lower = word.to_lowercase();
            if i > 0 && LABEL_LOWER.contains(&lower.as_str()) {
                lower
            } else {
                let mut chars = lower.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The event a date mention is about: the nearest named event in the same text
/// (`Battle of Hastings`), else the evidence text around the mention, quoted.
pub fn event_label(text: &str, mention: &TemporalExpression) -> String {
    let (start, end) = (mention.span.byte_start, mention.span.byte_end);
    let named = NAMED_EVENT
        .find_iter(text)
        .filter(|m| m.end() <= start || m.start() >= end)
        .min_by_key(|m| {
            if m.end() <= start {
                start - m.end()
            } else {
                m.start() - end
            }
        });
    if let Some(m) = named {
        let mut words: Vec<&str> = m.as_str().split_whitespace().collect();
        if words.first().is_some_and(|w| w.eq_ignore_ascii_case("the")) {
            words.remove(0);
        }
        // Cut the place/object after its connector at the first non-name word.
        if let Some(connector) = words
            .iter()
            .position(|w| LABEL_LOWER.contains(&w.to_lowercase().as_str()))
        {
            let mut end = connector + 1;
            while end < words.len() && words[end].eq_ignore_ascii_case("the") {
                end += 1;
            }
            let name_start = end;
            // One name word, more only while they are capitalized ("of Stamford Bridge").
            while end < words.len()
                && (end == name_start || words[end].chars().next().is_some_and(char::is_uppercase))
                && !LABEL_STOP.contains(&words[end].to_lowercase().as_str())
                && !words[end].chars().any(|c| c.is_ascii_digit())
            {
                end += 1;
            }
            if end > name_start {
                words.truncate(end);
                return title_case(&words.join(" "));
            }
        } else {
            return title_case(&words.join(" "));
        }
    }
    let collapsed: Vec<&str> = text.split_whitespace().collect();
    let joined = collapsed.join(" ");
    if joined.chars().count() <= LABEL_CHARS {
        return format!("“{joined}”");
    }
    // Keep the words around the mention.
    let target = mention.text.split_whitespace().next().unwrap_or("");
    let at = collapsed
        .iter()
        .position(|w| w.contains(target))
        .unwrap_or(0);
    let (mut from, mut to) = (at, at + 1);
    let len = |from: usize, to: usize| collapsed[from..to].join(" ").chars().count();
    while (from > 0 || to < collapsed.len()) && len(from, to) < LABEL_CHARS {
        if to < collapsed.len() && len(from, to + 1) <= LABEL_CHARS {
            to += 1;
        }
        if from > 0 && len(from - 1, to) <= LABEL_CHARS {
            from -= 1;
        } else if to >= collapsed.len() || len(from, to + 1) > LABEL_CHARS {
            break;
        }
    }
    format!(
        "“{}{}{}”",
        if from > 0 { "…" } else { "" },
        collapsed[from..to].join(" "),
        if to < collapsed.len() { "…" } else { "" }
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventClaimExtractionReport {
    pub producer: String,
    pub producer_version: String,
    pub videos_processed: usize,
    pub videos_without_text: usize,
    pub segments_analyzed: usize,
    pub visual_text_tracks_analyzed: usize,
    /// New unreviewed suggestions.
    pub claims_suggested: usize,
    /// Earlier unreviewed suggestions the run produced again (kept with their grouping).
    pub claims_kept: usize,
    /// Suggestions matching a claim a researcher already confirmed or rejected.
    pub reviewed_skipped: usize,
    /// Earlier unreviewed suggestions the run no longer produces.
    pub stale_removed: usize,
    pub low_confidence_dropped: usize,
}

impl ClaimExtraction {
    /// A suggestion's provenance; the run id is set when the run is recorded.
    fn new(source_text: &str, start: usize, end: usize, confidence: f32) -> Self {
        Self {
            run_id: Uuid::nil(),
            producer: EVENT_CLAIM_PRODUCER.into(),
            producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
            source_text: source_text.into(),
            source_byte_start: start as i32,
            source_byte_end: end as i32,
            confidence,
            extracted_at: None,
        }
    }
}

struct Suggestion {
    label: String,
    claim: ClaimInput,
    extraction: ClaimExtraction,
}

fn suggestion_key(
    evidence: Uuid,
    predicate: &str,
    date: Option<&EventDate>,
    identity: Option<Uuid>,
    source_text: &str,
) -> String {
    format!(
        "{evidence}|{predicate}|{:?}|{identity:?}|{source_text}",
        date.map(|d| (
            d.start_year,
            d.start_month,
            d.start_day,
            d.end_year,
            d.end_month,
            d.end_day,
            d.approximate
        ))
    )
}

/// Participants and places mentioned in one evidence record: nlp-stack entity mentions whose
/// corpus-local group is linked to a canonical identity.
async fn linked_mentions(
    pool: &PgPool,
    record: Uuid,
) -> anyhow::Result<Vec<(ClaimPredicate, Uuid, String, usize, usize, f32)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (l.identity_id)
                m.entity_type, m.surface_text, m.byte_start, m.byte_end, m.confidence, l.identity_id
         FROM entity_mentions m
         JOIN corpus_videos cv ON cv.video_id = m.video_id
         JOIN canonical_identity_links l
           ON l.unit_kind = 'entity_group' AND l.corpus_id = cv.corpus_id
          AND l.entity_type = m.entity_type AND l.entity_key = m.entity_key
         WHERE (m.transcript_segment_id = $1 OR m.visual_text_track_id = $1)
           AND m.entity_type IN ('person', 'organization', 'location')
         ORDER BY l.identity_id, m.byte_start",
    )
    .bind(record)
    .fetch_all(pool)
    .await?;
    let mut found = Vec::new();
    for row in rows {
        let entity_type: String = row.try_get("entity_type")?;
        let identity: Uuid = row.try_get("identity_id")?;
        let Some(identity) = crate::identities::live_identity(pool, identity).await? else {
            continue;
        };
        let start: i32 = row.try_get("byte_start")?;
        let end: i32 = row.try_get("byte_end")?;
        let confidence: Option<f64> = row.try_get("confidence")?;
        found.push((
            if entity_type == "location" {
                ClaimPredicate::Place
            } else {
                ClaimPredicate::Participant
            },
            identity,
            row.try_get("surface_text")?,
            start as usize,
            end as usize,
            confidence.unwrap_or(0.5) as f32,
        ));
    }
    Ok(found)
}

/// Suggests claims for one video, keeping reviewed claims and unchanged suggestions.
pub async fn extract_video_event_claims(
    pool: &PgPool,
    video_id: Uuid,
) -> anyhow::Result<EventClaimExtractionReport> {
    let sources = video_text_sources(pool, video_id).await?;
    let mut report = EventClaimExtractionReport {
        producer: EVENT_CLAIM_PRODUCER.into(),
        producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
        videos_processed: 1,
        segments_analyzed: sources.segments,
        visual_text_tracks_analyzed: sources.sources.len() - sources.segments,
        ..EventClaimExtractionReport::default()
    };
    if sources.sources.is_empty() {
        report.videos_without_text = 1;
    }

    let mut suggestions = Vec::new();
    for source in &sources.sources {
        let found = normalize_temporal_expressions(&source.text);
        report.low_confidence_dropped += found.low_confidence_dropped;
        if found.expressions.is_empty() {
            continue;
        }
        let modality = match source.modality {
            EntityModality::Spoken => "spoken",
            EntityModality::VisibleText => "visible",
        };
        let base = |predicate, date, identity_id, polarity: &str, certainty: &str| ClaimInput {
            video_id,
            modality: modality.into(),
            evidence_id: source.record_id,
            predicate,
            date,
            identity_id,
            polarity: Some(polarity.into()),
            certainty: Some(certainty.into()),
            qualification: String::new(),
        };
        let linked = linked_mentions(pool, source.record_id).await?;
        let mut labels = Vec::new();
        for mention in &found.expressions {
            let label = event_label(&source.text, mention);
            suggestions.push(Suggestion {
                label: label.clone(),
                claim: base(
                    ClaimPredicate::Date,
                    Some(event_date(&mention.date)),
                    None,
                    polarity(mention.polarity),
                    certainty(mention.certainty),
                ),
                extraction: ClaimExtraction::new(
                    &mention.text,
                    mention.span.byte_start,
                    mention.span.byte_end,
                    mention.confidence,
                ),
            });
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        // Who/where the dated text names, inferred from co-occurrence, for each of its events.
        for label in &labels {
            for (predicate, identity, surface, start, end, confidence) in &linked {
                suggestions.push(Suggestion {
                    label: label.clone(),
                    claim: ClaimInput {
                        qualification: "Named in the same evidence as the date".into(),
                        ..base(*predicate, None, Some(*identity), "affirms", "inferred")
                    },
                    extraction: ClaimExtraction::new(surface, *start, *end, *confidence),
                });
            }
        }
    }

    let mut tx = pool.begin().await?;
    let existing = sqlx::query(
        "SELECT id, review_status, evidence_id, predicate, identity_id, source_text,
                date_start_year, date_start_month, date_start_day,
                date_end_year, date_end_month, date_end_day, date_approximate
         FROM historical_event_claims
         WHERE video_id = $1 AND extraction_run_id IS NOT NULL
         FOR UPDATE",
    )
    .bind(video_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut by_key: HashMap<String, (Uuid, String)> = HashMap::new();
    for row in &existing {
        let date = crate::events::date_from_row(row, "")?;
        let predicate: String = row.try_get("predicate")?;
        let source_text: String = row.try_get("source_text")?;
        by_key.insert(
            suggestion_key(
                row.try_get("evidence_id")?,
                &predicate,
                date.as_ref(),
                row.try_get("identity_id")?,
                &source_text,
            ),
            (row.try_get("id")?, row.try_get("review_status")?),
        );
    }

    let run = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO historical_event_extraction_runs
           (id, video_id, producer, producer_version, config, transcript_stream_id,
            segments_analyzed, visual_text_tracks_analyzed, low_confidence_dropped,
            claims_suggested)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0)",
    )
    .bind(run)
    .bind(video_id)
    .bind(EVENT_CLAIM_PRODUCER)
    .bind(EVENT_CLAIM_PRODUCER_VERSION)
    .bind(format!("min-confidence={TEMPORAL_MIN_CONFIDENCE}"))
    .bind(sources.stream_id)
    .bind(report.segments_analyzed as i32)
    .bind(report.visual_text_tracks_analyzed as i32)
    .bind(report.low_confidence_dropped as i32)
    .execute(&mut *tx)
    .await?;

    let mut kept: Vec<Uuid> = Vec::new();
    let mut seen = HashSet::new();
    let mut events: HashMap<String, Uuid> = HashMap::new();
    for suggestion in suggestions {
        let key = suggestion_key(
            suggestion.claim.evidence_id,
            suggestion.claim.predicate.as_str(),
            suggestion.claim.date.as_ref(),
            suggestion.claim.identity_id,
            &suggestion.extraction.source_text,
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some((id, status)) = by_key.get(&key) {
            if status == "unverified" {
                sqlx::query(
                    "UPDATE historical_event_claims
                     SET extraction_run_id = $2, extraction_confidence = $3,
                         source_byte_start = $4, source_byte_end = $5
                     WHERE id = $1",
                )
                .bind(id)
                .bind(run)
                .bind(suggestion.extraction.confidence)
                .bind(suggestion.extraction.source_byte_start)
                .bind(suggestion.extraction.source_byte_end)
                .execute(&mut *tx)
                .await?;
                kept.push(*id);
                report.claims_kept += 1;
            } else {
                report.reviewed_skipped += 1;
            }
            continue;
        }
        let folded = suggestion.label.to_lowercase();
        let event = match events.get(&folded) {
            Some(event) => *event,
            None => {
                let found: Option<Uuid> = sqlx::query_scalar(
                    "SELECT id FROM historical_events
                     WHERE merged_into IS NULL AND lower(label) = $1
                     ORDER BY created_at, id LIMIT 1",
                )
                .bind(&folded)
                .fetch_optional(&mut *tx)
                .await?;
                let event = match found {
                    Some(event) => event,
                    None => {
                        let event = Uuid::new_v4();
                        sqlx::query("INSERT INTO historical_events (id, label) VALUES ($1, $2)")
                            .bind(event)
                            .bind(&suggestion.label)
                            .execute(&mut *tx)
                            .await?;
                        event
                    }
                };
                events.insert(folded, event);
                event
            }
        };
        let extraction = ClaimExtraction {
            run_id: run,
            ..suggestion.extraction
        };
        let id = insert_claim(&mut tx, event, &suggestion.claim, Some(&extraction)).await?;
        kept.push(id);
        report.claims_suggested += 1;
    }

    let emptied: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM historical_event_claims
         WHERE video_id = $1 AND extraction_run_id IS NOT NULL
           AND review_status = 'unverified' AND NOT (id = ANY($2))
         RETURNING event_id",
    )
    .bind(video_id)
    .bind(&kept)
    .fetch_all(&mut *tx)
    .await?;
    report.stale_removed = emptied.len();
    // Events that only held the removed suggestions go with them.
    sqlx::query(
        "DELETE FROM historical_events e
         WHERE e.id = ANY($1) AND e.merged_into IS NULL
           AND NOT EXISTS (SELECT 1 FROM historical_event_claims c WHERE c.event_id = e.id)
           AND NOT EXISTS (SELECT 1 FROM historical_events r WHERE r.merged_into = e.id)",
    )
    .bind(&emptied)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE historical_event_extraction_runs SET claims_suggested = $2 WHERE id = $1")
        .bind(run)
        .bind((report.claims_suggested + report.claims_kept) as i32)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(report)
}

/// Suggests claims for every video of a corpus.
pub async fn extract_corpus_event_claims(
    pool: &PgPool,
    corpus_id: Uuid,
) -> anyhow::Result<EventClaimExtractionReport> {
    let mut video_ids: Vec<Uuid> = crate::corpora::corpus_video_ids(pool, corpus_id)
        .await?
        .into_iter()
        .collect();
    video_ids.sort();
    let mut total = EventClaimExtractionReport {
        producer: EVENT_CLAIM_PRODUCER.into(),
        producer_version: EVENT_CLAIM_PRODUCER_VERSION.into(),
        ..EventClaimExtractionReport::default()
    };
    for video_id in video_ids {
        let report = extract_video_event_claims(pool, video_id).await?;
        total.videos_processed += report.videos_processed;
        total.videos_without_text += report.videos_without_text;
        total.segments_analyzed += report.segments_analyzed;
        total.visual_text_tracks_analyzed += report.visual_text_tracks_analyzed;
        total.claims_suggested += report.claims_suggested;
        total.claims_kept += report.claims_kept;
        total.reviewed_skipped += report.reviewed_skipped;
        total.stale_removed += report.stale_removed;
        total.low_confidence_dropped += report.low_confidence_dropped;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(text: &str) -> TemporalExpression {
        normalize_temporal_expressions(text).expressions.remove(0)
    }

    #[test]
    fn producer_version_is_the_pinned_nlp_stack_revision() {
        let manifest = include_str!("../Cargo.toml");
        let line = manifest
            .lines()
            .find(|l| l.starts_with("text-linguistics ="))
            .expect("text-linguistics dependency");
        assert!(
            line.contains(&format!("rev = \"{EVENT_CLAIM_PRODUCER_VERSION}\"")),
            "{line}"
        );
    }

    #[test]
    fn readings_map_to_claim_dates() {
        let mention = first("on March 15, 44 BC Caesar died");
        assert_eq!(
            event_date(&mention.date),
            EventDate {
                start_year: -44,
                start_month: Some(3),
                start_day: Some(15),
                ..EventDate::default()
            }
        );
        let mention = first("it was probably not written around 1450");
        assert_eq!(certainty(mention.certainty), "hedged");
        assert_eq!(polarity(mention.polarity), "denies");
        assert!(event_date(&mention.date).approximate);
    }

    #[test]
    fn labels_name_the_event_or_quote_the_evidence() {
        let text = "The Battle of Hastings was fought on 14 October 1066.";
        assert_eq!(event_label(text, &first(text)), "Battle of Hastings");
        let text = "the council of nicaea in 325 ad settled it";
        assert_eq!(event_label(text, &first(text)), "Council of Nicaea");
        let text = "In 2010, he did a missions trip to";
        assert_eq!(
            event_label(text, &first(text)),
            "“In 2010, he did a missions trip to”"
        );
    }
}
