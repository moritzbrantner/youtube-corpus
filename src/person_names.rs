//! Evidence-grounded names for inferred corpus-local people (Decision Atlas 4.7, issue #65).
//!
//! A name is attached to a person from #64 face matching only when an on-screen name tag is
//! clearly connected to that person's face: the canonical nlp-stack producer found a person
//! mention leading a short name-tag-shaped visual-analysis OCR track, and exactly one face track
//! was visible while the tag was shown. Spoken mentions, co-occurrence, face similarity,
//! conflicting tags and names supported for several people never assign a name. Names stay
//! separate from face observations and matching links; researcher confirmations, replacements
//! and rejections survive reruns.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

pub const ALGORITHM: &str = "youtube-corpus/on-screen-name-tag-v1";
/// OCR roles that can carry a name tag; end credits and slides list names without pointing at
/// a visible person.
const NAME_TAG_ROLES: [&str; 2] = ["subtitle", "scene_text"];
/// Words allowed after the name and a separator ("Ada Lovelace, Mathematician").
const MAX_TRAILING_WORDS: usize = 6;
/// Share of the tag's display time during which the single face must be visible.
const MIN_FACE_OVERLAP: f64 = 0.5;

pub fn configuration() -> Value {
    json!({
        "algorithm": ALGORITHM,
        "processorRevision": crate::provenance::PROCESSOR_SOURCE_REVISION,
        "evidence": "nlp-stack person mention in a visual-analysis OCR track",
        "nameTagRoles": NAME_TAG_ROLES,
        "nameTagShape": "the person name leads the text; at most one person name; at most 6 further words, after a separator",
        "visibleFaces": "exactly one current face track overlaps the name tag",
        "minFaceOverlapOfTag": MIN_FACE_OVERLAP,
        "faceIdentity": crate::face_matching::ALGORITHM,
        "spokenMentions": "never used",
        "conflictingNames": "unresolved",
        "nameSupportedForSeveralPeople": "unresolved",
        "ownerApprovedThresholds": false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameEvidence {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub modality: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub surface_text: String,
    /// The full on-screen text the name was read from.
    pub source_text: String,
    pub byte_start: i32,
    pub byte_end: i32,
    pub confidence: Option<f64>,
    pub entity_mention_id: Uuid,
    pub visual_text_track_id: Uuid,
    pub face_track_id: Uuid,
    pub provenance: Value,
}

/// One on-screen person mention in a visible-text track, with what is needed to align it.
#[derive(Debug, Clone)]
pub struct NameTagCandidate {
    pub evidence: NameEvidence,
    pub entity_key: String,
    pub role: String,
    /// Person mentions found in the same visible-text track.
    pub person_mentions_in_track: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleFace {
    pub track_id: Uuid,
    pub video_id: Uuid,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NamePlan {
    Named {
        name: String,
        key: String,
        confidence: Option<f64>,
        evidence: Vec<usize>,
    },
    Unresolved {
        reason: String,
        candidates: Vec<String>,
    },
}

/// Whether `text` reads as a name tag for the mention at `start..end`.
pub fn is_name_tag(text: &str, start: usize, end: usize, person_mentions: usize) -> bool {
    if person_mentions != 1 || end > text.len() || start >= end {
        return false;
    }
    let (Some(before), Some(after)) = (text.get(..start), text.get(end..)) else {
        return false;
    };
    if before.chars().any(char::is_alphanumeric) {
        return false;
    }
    let after = after.trim();
    if after.is_empty() {
        return true;
    }
    let separated = after.starts_with([',', '-', '–', '—', '|', ':', '/', '(', '·']);
    let trailing = after
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .count();
    separated && trailing <= MAX_TRAILING_WORDS
}

fn overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

/// Pure planning step: aligns name tags with faces and decides each person's inferred name.
/// `people` maps each person to its face tracks; `rejected` holds researcher-rejected keys and
/// `protected` keys already owned by a confirmed or researcher-provided name of a person.
pub fn plan_names(
    people: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    faces: &[VisibleFace],
    tags: &[NameTagCandidate],
    rejected: &BTreeSet<(Uuid, String)>,
    protected: &BTreeMap<Uuid, String>,
) -> BTreeMap<Uuid, NamePlan> {
    let owner: BTreeMap<Uuid, Uuid> = people
        .iter()
        .flat_map(|(person, tracks)| tracks.iter().map(move |t| (*t, *person)))
        .collect();
    let mut strong: BTreeMap<Uuid, Vec<usize>> = BTreeMap::new();
    let mut ambiguous: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
    for (index, tag) in tags.iter().enumerate() {
        let e = &tag.evidence;
        if !NAME_TAG_ROLES.contains(&tag.role.as_str())
            || !is_name_tag(
                &e.source_text,
                e.byte_start as usize,
                e.byte_end as usize,
                tag.person_mentions_in_track,
            )
        {
            continue;
        }
        let span = (e.start_seconds, e.end_seconds);
        let visible: Vec<&VisibleFace> = faces
            .iter()
            .filter(|f| {
                f.video_id == e.video_id && f.start_seconds <= span.1 && span.0 <= f.end_seconds
            })
            .collect();
        match visible.as_slice() {
            [face] => {
                let duration = span.1 - span.0;
                let covered = overlap(span, (face.start_seconds, face.end_seconds));
                let aligned = if duration > 0.0 {
                    covered / duration >= MIN_FACE_OVERLAP
                } else {
                    true
                };
                if let (true, Some(person)) = (aligned, owner.get(&face.track_id)) {
                    strong.entry(*person).or_default().push(index);
                }
            }
            [] => {}
            several => {
                for face in several {
                    if let Some(person) = owner.get(&face.track_id) {
                        ambiguous
                            .entry(*person)
                            .or_default()
                            .insert(e.surface_text.clone());
                    }
                }
            }
        }
    }
    // Keys supported (or already owned) per person, before the homonym check.
    let mut keys: BTreeMap<Uuid, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
    let mut rejected_only: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
    for (person, indices) in &strong {
        for &index in indices {
            let tag = &tags[index];
            if rejected.contains(&(*person, tag.entity_key.clone())) {
                rejected_only
                    .entry(*person)
                    .or_default()
                    .insert(tag.evidence.surface_text.clone());
                continue;
            }
            keys.entry(*person)
                .or_default()
                .entry(tag.entity_key.clone())
                .or_default()
                .push(index);
        }
    }
    let mut claims: BTreeMap<&str, BTreeSet<Uuid>> = BTreeMap::new();
    for (person, by_key) in &keys {
        if by_key.len() == 1 {
            claims
                .entry(by_key.keys().next().unwrap().as_str())
                .or_default()
                .insert(*person);
        }
    }
    for (person, key) in protected {
        claims.entry(key.as_str()).or_default().insert(*person);
    }
    let display = |indices: &[usize]| -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for &i in indices {
            *counts
                .entry(tags[i].evidence.surface_text.trim())
                .or_default() += 1;
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(name, _)| name.to_string())
            .unwrap_or_default()
    };
    let mut result = BTreeMap::new();
    for person in people.keys() {
        let plan = match keys.get(person) {
            Some(by_key) if by_key.len() > 1 => NamePlan::Unresolved {
                reason: "Conflicting on-screen names are linked to this person".into(),
                candidates: by_key.values().map(|v| display(v)).collect(),
            },
            Some(by_key) => {
                let (key, indices) = by_key.iter().next().unwrap();
                if claims.get(key.as_str()).is_some_and(|p| p.len() > 1) {
                    NamePlan::Unresolved {
                        reason: "The same name is linked to several corpus people".into(),
                        candidates: vec![display(indices)],
                    }
                } else {
                    NamePlan::Named {
                        name: display(indices),
                        key: key.clone(),
                        confidence: indices
                            .iter()
                            .filter_map(|&i| tags[i].evidence.confidence)
                            .reduce(f64::min),
                        evidence: indices.clone(),
                    }
                }
            }
            None => {
                if let Some(names) = rejected_only.get(person) {
                    NamePlan::Unresolved {
                        reason: "A researcher rejected the only supported name".into(),
                        candidates: names.iter().cloned().collect(),
                    }
                } else if let Some(names) = ambiguous.get(person) {
                    NamePlan::Unresolved {
                        reason: "Name tags appeared while several faces were visible".into(),
                        candidates: names.iter().cloned().collect(),
                    }
                } else {
                    continue;
                }
            }
        };
        result.insert(*person, plan);
    }
    result
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonNameAttribution {
    pub name: String,
    /// `inferred`, `confirmed` or `user_provided`.
    pub status: String,
    pub confidence: Option<f64>,
    pub algorithm: String,
    pub algorithm_version: String,
    pub configuration: Value,
    pub evidence: Vec<NameEvidence>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedName {
    pub reason: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonNameReview {
    pub person_id: Uuid,
    pub label: String,
    pub name: Option<PersonNameAttribution>,
    pub unresolved: Option<UnresolvedName>,
    pub rejected_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonNamesReport {
    pub configuration: Value,
    pub people: Vec<PersonNameReview>,
}

struct Inputs {
    labels: BTreeMap<Uuid, String>,
    people: BTreeMap<Uuid, BTreeSet<Uuid>>,
    faces: Vec<VisibleFace>,
    tags: Vec<NameTagCandidate>,
    rejected: BTreeSet<(Uuid, String)>,
    rejected_names: BTreeMap<Uuid, Vec<String>>,
    /// Current attribution per person: (status, key).
    current: BTreeMap<Uuid, (String, String)>,
}

impl Inputs {
    fn protected(&self) -> BTreeMap<Uuid, String> {
        self.current
            .iter()
            .filter(|(_, (status, _))| status != "inferred")
            .map(|(person, (_, key))| (*person, key.clone()))
            .collect()
    }
    fn plan(&self) -> BTreeMap<Uuid, NamePlan> {
        plan_names(
            &self.people,
            &self.faces,
            &self.tags,
            &self.rejected,
            &self.protected(),
        )
    }
}

async fn load(connection: &mut PgConnection, corpus: Uuid) -> anyhow::Result<Inputs> {
    let videos: Vec<Uuid> = crate::corpora::corpus_video_ids_with(&mut *connection, corpus)
        .await?
        .into_iter()
        .collect();
    let mut labels = BTreeMap::new();
    let mut people: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT p.id, p.label, m.track_id FROM people p
         LEFT JOIN person_face_clusters pc ON pc.person_id = p.id
         LEFT JOIN face_cluster_members m ON m.cluster_id = pc.cluster_id
         WHERE p.corpus_id = $1 ORDER BY p.created_at, p.id",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?
    {
        let id: Uuid = row.try_get("id")?;
        labels.entry(id).or_insert_with(|| {
            row.try_get::<Option<String>, _>("label")
                .ok()
                .flatten()
                .unwrap_or_else(|| format!("Inferred person {}", &id.simple().to_string()[..8]))
        });
        let tracks = people.entry(id).or_default();
        if let Some(track) = row.try_get::<Option<Uuid>, _>("track_id")? {
            tracks.insert(track);
        }
    }
    // Faces of each video's current complete face run (the run #64 matches against).
    let faces = sqlx::query(
        "SELECT t.id, t.video_id, t.start_seconds, t.end_seconds FROM face_tracks t
         WHERE t.video_id = ANY($1) AND t.run_id = (
           SELECT r.id FROM media_processing_runs r WHERE r.video_id = t.video_id
             AND r.modality = 'face' AND r.processor = $2 AND r.processing_config->>'complete' = 'true'
             ORDER BY r.updated_at DESC, r.id DESC LIMIT 1)",
    )
    .bind(&videos)
    .bind(crate::face_processing::FACE_PROCESSOR)
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(|row| {
        Ok(VisibleFace {
            track_id: row.try_get("id")?,
            video_id: row.try_get("video_id")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;
    let tags = sqlx::query(
        "SELECT m.id, m.video_id, m.surface_text, m.entity_key, m.byte_start, m.byte_end,
                m.confidence, t.id AS track_id, t.text, t.role, t.start_seconds, t.end_seconds,
                v.title, v.source_url, er.producer, er.producer_version, er.model,
                er.config_hash AS entity_config, ocr.processor AS ocr_processor,
                ocr.processor_version AS ocr_processor_version, ocr.model AS ocr_model,
                ocr.model_version AS ocr_model_version,
                (SELECT count(*) FROM entity_mentions o WHERE o.visual_text_track_id = t.id
                   AND o.entity_type = 'person') AS person_mentions
         FROM entity_mentions m
         JOIN entity_extraction_runs er ON er.id = m.run_id
         JOIN visual_text_tracks t ON t.id = m.visual_text_track_id
         JOIN media_processing_runs ocr ON ocr.id = t.run_id
         JOIN videos v ON v.id = m.video_id
         WHERE m.video_id = ANY($1) AND m.entity_type = 'person' AND m.modality = 'visible_text'
           AND t.start_seconds IS NOT NULL AND t.end_seconds IS NOT NULL
         ORDER BY m.video_id, t.start_seconds, m.id",
    )
    .bind(&videos)
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(|row| {
        Ok(NameTagCandidate {
            entity_key: row.try_get("entity_key")?,
            role: row.try_get("role")?,
            person_mentions_in_track: usize::try_from(row.try_get::<i64, _>("person_mentions")?)
                .unwrap_or(0),
            evidence: NameEvidence {
                video_id: row.try_get("video_id")?,
                title: row.try_get("title")?,
                source_url: row.try_get("source_url")?,
                modality: "visible_text".into(),
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                surface_text: row.try_get("surface_text")?,
                source_text: row.try_get("text")?,
                byte_start: row.try_get("byte_start")?,
                byte_end: row.try_get("byte_end")?,
                confidence: row.try_get("confidence")?,
                entity_mention_id: row.try_get("id")?,
                visual_text_track_id: row.try_get("track_id")?,
                face_track_id: Uuid::nil(),
                provenance: json!({
                    "entity": {
                        "producer": row.try_get::<String, _>("producer")?,
                        "producerVersion": row.try_get::<String, _>("producer_version")?,
                        "model": row.try_get::<String, _>("model")?,
                        "configHash": row.try_get::<String, _>("entity_config")?,
                    },
                    "ocr": {
                        "processor": row.try_get::<String, _>("ocr_processor")?,
                        "processorVersion": row.try_get::<String, _>("ocr_processor_version")?,
                        "model": row.try_get::<String, _>("ocr_model")?,
                        "modelVersion": row.try_get::<String, _>("ocr_model_version")?,
                        "role": row.try_get::<String, _>("role")?,
                    },
                }),
            },
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;
    let mut rejected = BTreeSet::new();
    let mut rejected_names: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT person_id, name_key, name FROM person_name_rejections
         WHERE corpus_id = $1 ORDER BY created_at, name_key",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?
    {
        let person: Uuid = row.try_get("person_id")?;
        rejected.insert((person, row.try_get("name_key")?));
        rejected_names
            .entry(person)
            .or_default()
            .push(row.try_get("name")?);
    }
    let current = sqlx::query(
        "SELECT person_id, status, name_key FROM person_name_attributions WHERE corpus_id = $1",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            row.try_get("person_id")?,
            (row.try_get("status")?, row.try_get("name_key")?),
        ))
    })
    .collect::<anyhow::Result<_>>()?;
    Ok(Inputs {
        labels,
        people,
        faces,
        tags,
        rejected,
        rejected_names,
        current,
    })
}

/// Face track each supporting tag was aligned with (the single visible face).
fn aligned_face(inputs: &Inputs, tag: &NameTagCandidate) -> Option<Uuid> {
    let e = &tag.evidence;
    let mut visible = inputs.faces.iter().filter(|f| {
        f.video_id == e.video_id
            && f.start_seconds <= e.end_seconds
            && e.start_seconds <= f.end_seconds
    });
    let face = visible.next()?;
    visible.next().is_none().then_some(face.track_id)
}

async fn lock(tx: &mut Transaction<'_, Postgres>, corpus: Uuid) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("person-names:{corpus}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn report(connection: &mut PgConnection, corpus: Uuid) -> anyhow::Result<PersonNamesReport> {
    let inputs = load(&mut *connection, corpus).await?;
    let plan = inputs.plan();
    let mut evidence: BTreeMap<Uuid, Vec<NameEvidence>> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT e.*, v.title, v.source_url FROM person_name_evidence e
         JOIN videos v ON v.id = e.video_id WHERE e.person_id = ANY($1)
         ORDER BY e.video_id, e.start_seconds, e.id",
    )
    .bind(inputs.people.keys().copied().collect::<Vec<_>>())
    .fetch_all(&mut *connection)
    .await?
    {
        evidence
            .entry(row.try_get("person_id")?)
            .or_default()
            .push(NameEvidence {
                video_id: row.try_get("video_id")?,
                title: row.try_get("title")?,
                source_url: row.try_get("source_url")?,
                modality: row.try_get("modality")?,
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                surface_text: row.try_get("surface_text")?,
                source_text: row.try_get("source_text")?,
                byte_start: row.try_get("byte_start")?,
                byte_end: row.try_get("byte_end")?,
                confidence: row.try_get("confidence")?,
                entity_mention_id: row.try_get("entity_mention_id")?,
                visual_text_track_id: row.try_get("visual_text_track_id")?,
                face_track_id: row.try_get("face_track_id")?,
                provenance: row.try_get("provenance")?,
            });
    }
    let mut names: BTreeMap<Uuid, PersonNameAttribution> = BTreeMap::new();
    for row in sqlx::query("SELECT * FROM person_name_attributions WHERE corpus_id = $1")
        .bind(corpus)
        .fetch_all(&mut *connection)
        .await?
    {
        let person: Uuid = row.try_get("person_id")?;
        names.insert(
            person,
            PersonNameAttribution {
                name: row.try_get("name")?,
                status: row.try_get("status")?,
                confidence: row.try_get("confidence")?,
                algorithm: row.try_get("algorithm")?,
                algorithm_version: row.try_get("algorithm_version")?,
                configuration: row.try_get("configuration")?,
                evidence: evidence.remove(&person).unwrap_or_default(),
            },
        );
    }
    let people = inputs
        .labels
        .iter()
        .map(|(person, label)| {
            let name = names.remove(person);
            let unresolved = match (&name, plan.get(person)) {
                (None, Some(NamePlan::Unresolved { reason, candidates })) => Some(UnresolvedName {
                    reason: reason.clone(),
                    candidates: candidates.clone(),
                }),
                _ => None,
            };
            PersonNameReview {
                person_id: *person,
                label: label.clone(),
                name,
                unresolved,
                rejected_names: inputs
                    .rejected_names
                    .get(person)
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect();
    Ok(PersonNamesReport {
        configuration: configuration(),
        people,
    })
}

pub async fn review_person_names(pool: &PgPool, corpus: Uuid) -> anyhow::Result<PersonNamesReport> {
    let mut connection = pool.acquire().await?;
    report(&mut connection, corpus).await
}

/// Recomputes inferred names from the current evidence. Confirmed and researcher-provided
/// names are left untouched; rejected names are never re-inferred for that person.
pub async fn infer_person_names(pool: &PgPool, corpus: Uuid) -> anyhow::Result<PersonNamesReport> {
    let mut tx = pool.begin().await?;
    lock(&mut tx, corpus).await?;
    let inputs = load(&mut tx, corpus).await?;
    let plan = inputs.plan();
    for person in inputs.people.keys() {
        if inputs
            .current
            .get(person)
            .is_some_and(|(status, _)| status != "inferred")
        {
            continue;
        }
        sqlx::query(
            "DELETE FROM person_name_attributions WHERE person_id = $1 AND status = 'inferred'",
        )
        .bind(person)
        .execute(&mut *tx)
        .await?;
        let Some(NamePlan::Named {
            name,
            key,
            confidence,
            evidence,
        }) = plan.get(person)
        else {
            continue;
        };
        sqlx::query(
            "INSERT INTO person_name_attributions
               (person_id, corpus_id, name, name_key, status, confidence, algorithm,
                algorithm_version, configuration)
             VALUES ($1, $2, $3, $4, 'inferred', $5, $6, $7, $8)",
        )
        .bind(person)
        .bind(corpus)
        .bind(name)
        .bind(key)
        .bind(confidence)
        .bind(ALGORITHM)
        .bind(crate::provenance::PROCESSOR_SOURCE_REVISION)
        .bind(configuration())
        .execute(&mut *tx)
        .await?;
        for &index in evidence {
            let tag = &inputs.tags[index];
            let Some(face) = aligned_face(&inputs, tag) else {
                continue;
            };
            let e = &tag.evidence;
            sqlx::query(
                "INSERT INTO person_name_evidence
                   (id, person_id, video_id, modality, entity_mention_id, visual_text_track_id,
                    face_track_id, start_seconds, end_seconds, surface_text, source_text,
                    byte_start, byte_end, confidence, provenance)
                 VALUES ($1, $2, $3, 'visible_text', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(Uuid::new_v4())
            .bind(person)
            .bind(e.video_id)
            .bind(e.entity_mention_id)
            .bind(e.visual_text_track_id)
            .bind(face)
            .bind(e.start_seconds)
            .bind(e.end_seconds)
            .bind(&e.surface_text)
            .bind(&e.source_text)
            .bind(e.byte_start)
            .bind(e.byte_end)
            .bind(e.confidence)
            .bind(&e.provenance)
            .execute(&mut *tx)
            .await?;
        }
    }
    let result = report(&mut tx, corpus).await?;
    tx.commit().await?;
    crate::identities::resolve_identities(pool).await?;
    Ok(result)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameCorrectionAction {
    Confirm,
    Reject,
    Replace,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameCorrectionRequest {
    pub person_id: Uuid,
    pub action: NameCorrectionAction,
    pub name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NameCorrectionInputError(String);

pub async fn correct_person_name(
    pool: &PgPool,
    corpus: Uuid,
    request: NameCorrectionRequest,
) -> anyhow::Result<PersonNamesReport> {
    let input = |message: &str| anyhow::Error::from(NameCorrectionInputError(message.into()));
    let mut tx = pool.begin().await?;
    lock(&mut tx, corpus).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM people WHERE id = $1 AND corpus_id = $2)")
            .bind(request.person_id)
            .bind(corpus)
            .fetch_one(&mut *tx)
            .await?;
    anyhow::ensure!(exists, "person not found in corpus");
    let current: Option<(String, String)> =
        sqlx::query_as("SELECT name, name_key FROM person_name_attributions WHERE person_id = $1")
            .bind(request.person_id)
            .fetch_optional(&mut *tx)
            .await?;
    let reject = |name: String, key: String| {
        sqlx::query(
            "INSERT INTO person_name_rejections (corpus_id, person_id, name_key, name)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(corpus)
        .bind(request.person_id)
        .bind(key)
        .bind(name)
    };
    let (action, new_name) = match request.action {
        NameCorrectionAction::Confirm => {
            if current.is_none() {
                return Err(input("this person has no name to confirm"));
            }
            sqlx::query(
                "UPDATE person_name_attributions SET status = 'confirmed', updated_at = now()
                 WHERE person_id = $1",
            )
            .bind(request.person_id)
            .execute(&mut *tx)
            .await?;
            ("confirm", current.as_ref().map(|c| c.0.clone()))
        }
        NameCorrectionAction::Reject => {
            let Some((name, key)) = current.clone() else {
                return Err(input("this person has no name to reject"));
            };
            reject(name, key).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM person_name_attributions WHERE person_id = $1")
                .bind(request.person_id)
                .execute(&mut *tx)
                .await?;
            ("reject", None)
        }
        NameCorrectionAction::Replace => {
            let name = request
                .name
                .as_deref()
                .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|n| !n.is_empty())
                .ok_or_else(|| input("enter a name"))?;
            let key = crate::entities::entity_key(&name);
            if key.is_empty() {
                return Err(input("enter a name"));
            }
            if let Some((old, old_key)) = current.clone().filter(|(_, k)| *k != key) {
                reject(old, old_key).execute(&mut *tx).await?;
            }
            sqlx::query(
                "DELETE FROM person_name_rejections WHERE person_id = $1 AND name_key = $2",
            )
            .bind(request.person_id)
            .bind(&key)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM person_name_attributions WHERE person_id = $1")
                .bind(request.person_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO person_name_attributions
                   (person_id, corpus_id, name, name_key, status, confidence, algorithm,
                    algorithm_version, configuration)
                 VALUES ($1, $2, $3, $4, 'user_provided', NULL, 'researcher-provided name', $5, $6)",
            )
            .bind(request.person_id)
            .bind(corpus)
            .bind(&name)
            .bind(&key)
            .bind(crate::provenance::PROCESSOR_SOURCE_REVISION)
            .bind(json!({"source": "researcher"}))
            .execute(&mut *tx)
            .await?;
            ("replace", Some(name))
        }
    };
    sqlx::query(
        "INSERT INTO person_name_corrections (id, corpus_id, person_id, action, previous_name, name)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(corpus)
    .bind(request.person_id)
    .bind(action)
    .bind(current.map(|c| c.0))
    .bind(new_name)
    .execute(&mut *tx)
    .await?;
    let result = report(&mut tx, corpus).await?;
    tx.commit().await?;
    crate::identities::resolve_identities(pool).await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(video: Uuid, text: &str, name: &str, start: f64, end: f64) -> NameTagCandidate {
        let byte_start = text.find(name).unwrap();
        NameTagCandidate {
            entity_key: crate::entities::entity_key(name),
            role: "scene_text".into(),
            person_mentions_in_track: 1,
            evidence: NameEvidence {
                video_id: video,
                title: None,
                source_url: String::new(),
                modality: "visible_text".into(),
                start_seconds: start,
                end_seconds: end,
                surface_text: name.into(),
                source_text: text.into(),
                byte_start: byte_start as i32,
                byte_end: (byte_start + name.len()) as i32,
                confidence: Some(0.9),
                entity_mention_id: Uuid::new_v4(),
                visual_text_track_id: Uuid::new_v4(),
                face_track_id: Uuid::nil(),
                provenance: Value::Null,
            },
        }
    }
    fn face(video: Uuid, start: f64, end: f64) -> VisibleFace {
        VisibleFace {
            track_id: Uuid::new_v4(),
            video_id: video,
            start_seconds: start,
            end_seconds: end,
        }
    }
    fn people(entries: &[(Uuid, &[VisibleFace])]) -> BTreeMap<Uuid, BTreeSet<Uuid>> {
        entries
            .iter()
            .map(|(p, faces)| (*p, faces.iter().map(|f| f.track_id).collect()))
            .collect()
    }

    #[test]
    fn name_tag_shape() {
        assert!(is_name_tag("Ada Lovelace", 0, 12, 1));
        assert!(is_name_tag("Ada Lovelace, Mathematician", 0, 12, 1));
        assert!(is_name_tag(
            "  Ada Lovelace | Analytical Engine team",
            2,
            14,
            1
        ));
        assert!(!is_name_tag("Ada Lovelace explains the engine", 0, 12, 1));
        assert!(!is_name_tag("Interview with Ada Lovelace", 15, 27, 1));
        assert!(!is_name_tag("Ada Lovelace, Charles Babbage", 0, 12, 2));
    }

    #[test]
    fn single_visible_face_with_name_tag_names_the_person() {
        let video = Uuid::new_v4();
        let person = Uuid::new_v4();
        let f = face(video, 10.0, 20.0);
        let tags = vec![tag(
            video,
            "Ada Lovelace, Mathematician",
            "Ada Lovelace",
            11.0,
            15.0,
        )];
        let plan = plan_names(
            &people(&[(person, &[f])]),
            &[f],
            &tags,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert!(matches!(&plan[&person], NamePlan::Named { name, .. } if name == "Ada Lovelace"));
    }

    #[test]
    fn weak_alignment_never_names() {
        let video = Uuid::new_v4();
        let (person, other) = (Uuid::new_v4(), Uuid::new_v4());
        let f = face(video, 10.0, 20.0);
        let g = face(video, 10.0, 20.0);
        let all = people(&[(person, &[f]), (other, &[g])]);
        // A sentence mentioning an absent person is not a name tag.
        let mention = vec![tag(
            video,
            "Ada Lovelace wrote the first program",
            "Ada Lovelace",
            11.0,
            15.0,
        )];
        assert!(plan_names(
            &people(&[(person, &[f])]),
            &[f],
            &mention,
            &BTreeSet::new(),
            &BTreeMap::new()
        )
        .is_empty());
        // A tag shown while two faces are visible is ambiguous.
        let tags = vec![tag(video, "Ada Lovelace", "Ada Lovelace", 11.0, 15.0)];
        let plan = plan_names(&all, &[f, g], &tags, &BTreeSet::new(), &BTreeMap::new());
        assert!(plan
            .values()
            .all(|p| matches!(p, NamePlan::Unresolved { .. })));
        // A tag mostly outside the face's time is not aligned.
        let late = vec![tag(video, "Ada Lovelace", "Ada Lovelace", 18.0, 30.0)];
        assert!(plan_names(
            &people(&[(person, &[f])]),
            &[f],
            &late,
            &BTreeSet::new(),
            &BTreeMap::new()
        )
        .is_empty());
    }

    #[test]
    fn conflicts_homonyms_and_rejections_stay_unresolved() {
        let (v1, v2) = (Uuid::new_v4(), Uuid::new_v4());
        let (person, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (f1, f2) = (face(v1, 0.0, 10.0), face(v2, 0.0, 10.0));
        let conflicting = vec![
            tag(v1, "Ada Lovelace", "Ada Lovelace", 1.0, 4.0),
            tag(v2, "Ada Byron", "Ada Byron", 1.0, 4.0),
        ];
        let plan = plan_names(
            &people(&[(person, &[f1, f2])]),
            &[f1, f2],
            &conflicting,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert!(
            matches!(&plan[&person], NamePlan::Unresolved { candidates, .. } if candidates.len() == 2)
        );

        let homonyms = vec![
            tag(v1, "John Smith", "John Smith", 1.0, 4.0),
            tag(v2, "John Smith", "John Smith", 1.0, 4.0),
        ];
        let plan = plan_names(
            &people(&[(person, &[f1]), (other, &[f2])]),
            &[f1, f2],
            &homonyms,
            &BTreeSet::new(),
            &BTreeMap::new(),
        );
        assert!(plan
            .values()
            .all(|p| matches!(p, NamePlan::Unresolved { .. })));

        let rejected = BTreeSet::from([(person, "john smith".to_string())]);
        let plan = plan_names(
            &people(&[(person, &[f1])]),
            &[f1],
            &homonyms[..1],
            &rejected,
            &BTreeMap::new(),
        );
        assert!(
            matches!(&plan[&person], NamePlan::Unresolved { reason, .. } if reason.contains("rejected"))
        );
    }
}
