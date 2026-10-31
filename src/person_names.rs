//! Evidence-grounded names for inferred corpus-local people (Decision Atlas 4.7, issues #65, #135).
//!
//! A name is attached to a person from #64 face matching or #134 voice matching only from
//! time-aligned evidence that points at that person:
//!
//! - an **on-screen name tag**: the canonical nlp-stack producer found a person mention leading
//!   a short name-tag-shaped visual-analysis OCR track, and exactly one face track was visible
//!   while the tag was shown;
//! - a **spoken self-introduction** ("I'm Ada Lovelace", "my name is …", "Hi, this is …", and the
//!   German "ich bin …", "mein Name ist …", "hier ist …"): a spoken nlp-stack person mention
//!   directly follows the cue in a transcript segment, and one voice track clearly speaks during
//!   that segment.
//!
//! A host introducing a guest ("joining me is …", "my guest is …", "I'm joined by …") names
//! another speaker: it never names the speaking person and stops that person from taking the
//! name from a self-introduction. Other mentions, co-occurrence, face/voice similarity,
//! conflicting names and names supported for several people never assign a name. Names stay
//! separate from observations and matching links; researcher confirmations, replacements and
//! rejections survive reruns.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

pub const ALGORITHM: &str = "youtube-corpus/person-name-evidence-v2";
/// OCR roles that can carry a name tag; end credits and slides list names without pointing at
/// a visible person.
const NAME_TAG_ROLES: [&str; 2] = ["subtitle", "scene_text"];
/// Words allowed after the name and a separator ("Ada Lovelace, Mathematician").
const MAX_TRAILING_WORDS: usize = 6;
/// Share of the tag's display time during which the single face must be visible.
const MIN_FACE_OVERLAP: f64 = 0.5;
/// Share of the introducing transcript segment the speaking voice track must cover.
const MIN_VOICE_COVERAGE: f64 = 0.5;
/// Largest share of that segment any other voice track may cover.
const MAX_OTHER_VOICE_COVERAGE: f64 = 0.2;
/// Self-introduction cues (lowercase words) that must directly precede the spoken mention.
const SELF_CUES: [&str; 10] = [
    "i'm",
    "i am",
    "my name is",
    "my name's",
    "this is",
    "ich bin",
    "ich heiße",
    "mein name ist",
    "hier ist",
    "hier spricht",
];
/// Cues that count only at the start of a sentence or after a greeting ("Hi, this is Ada"):
/// "and this is Ada" points at someone else.
const SENTENCE_START_CUES: [&str; 3] = ["this is", "hier ist", "hier spricht"];
const GREETINGS: [&str; 16] = [
    "hi",
    "hey",
    "hello",
    "hi everyone",
    "hey everyone",
    "hello everyone",
    "hi everybody",
    "hey everybody",
    "hello everybody",
    "hi guys",
    "hey guys",
    "hi there",
    "hey there",
    "hallo",
    "hallo zusammen",
    "servus",
];
/// Words that turn a following "I am X" into reported or hypothetical speech ("evidence that I
/// am God", "if I'm Ada").
const REPORTING_WORDS: [&str; 17] = [
    "that", "if", "whether", "because", "unless", "say", "says", "said", "saying", "think",
    "thinks", "claim", "claims", "claimed", "believe", "pretend", "dass",
];
/// Cues introducing someone else: the mention names another speaker, not the one talking.
const GUEST_CUES: [&str; 22] = [
    "joining me is",
    "joining me today is",
    "joining us is",
    "joining us today is",
    "joined by",
    "here with",
    "with me is",
    "with me today is",
    "my guest is",
    "my guest today is",
    "our guest is",
    "our guest today is",
    "welcome",
    "please welcome",
    "say hi to",
    "say hello to",
    "introducing",
    "bei mir ist",
    "zu gast ist",
    "mein gast ist",
    "unser gast ist",
    "begrüße",
];

pub fn configuration() -> Value {
    json!({
        "algorithm": ALGORITHM,
        "processorRevision": crate::provenance::PROCESSOR_SOURCE_REVISION,
        "evidence": [
            "nlp-stack person mention in a visual-analysis OCR track",
            "nlp-stack spoken person mention in a self-introduction attributed to one voice track",
        ],
        "nameTagRoles": NAME_TAG_ROLES,
        "nameTagShape": "the person name leads the text; at most one person name; at most 6 further words, after a separator",
        "visibleFaces": "exactly one current face track overlaps the name tag",
        "minFaceOverlapOfTag": MIN_FACE_OVERLAP,
        "faceIdentity": crate::face_matching::ALGORITHM,
        "selfIntroductionCues": SELF_CUES,
        "sentenceStartCues": SENTENCE_START_CUES,
        "guestIntroductionCues": GUEST_CUES,
        "reportedSpeechWords": REPORTING_WORDS,
        "selfIntroductionShape": "the cue directly precedes the person mention; the segment continues after the name and not with a possessive",
        "minSpeakerCoverageOfSegment": MIN_VOICE_COVERAGE,
        "maxOtherSpeakerCoverageOfSegment": MAX_OTHER_VOICE_COVERAGE,
        "voiceIdentity": crate::voice_matching::ALGORITHM,
        "guestIntroductions": "never name the speaker; block that name for the introducing person",
        "otherSpokenMentions": "never used",
        "conflictingNames": "unresolved",
        "nameSupportedForSeveralPeople": "unresolved",
        "ownerApprovedThresholds": false,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameEvidence {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    /// `visible_text` (a name tag aligned with a face) or `spoken` (a self-introduction
    /// aligned with a voice).
    pub modality: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub surface_text: String,
    /// The full on-screen text or transcript segment the name was read from.
    pub source_text: String,
    pub byte_start: i32,
    pub byte_end: i32,
    pub confidence: Option<f64>,
    pub entity_mention_id: Uuid,
    pub visual_text_track_id: Option<Uuid>,
    pub face_track_id: Option<Uuid>,
    pub transcript_segment_id: Option<Uuid>,
    pub voice_track_id: Option<Uuid>,
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

/// One spoken person mention in a transcript segment.
#[derive(Debug, Clone)]
pub struct SpokenNameCandidate {
    pub evidence: NameEvidence,
    pub entity_key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct VisibleFace {
    pub track_id: Uuid,
    pub video_id: Uuid,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// One speech span of a current voice track.
#[derive(Debug, Clone, Copy)]
pub struct VoiceSpan {
    pub track_id: Uuid,
    pub video_id: Uuid,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Everything name planning looks at.
#[derive(Debug, Clone, Default)]
pub struct NameEvidenceInputs {
    /// Face tracks of each corpus person.
    pub face_people: BTreeMap<Uuid, BTreeSet<Uuid>>,
    /// Voice tracks of each corpus person.
    pub voice_people: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub faces: Vec<VisibleFace>,
    pub voices: Vec<VoiceSpan>,
    pub tags: Vec<NameTagCandidate>,
    pub spoken: Vec<SpokenNameCandidate>,
    /// Researcher-rejected (person, name key) pairs.
    pub rejected: BTreeSet<(Uuid, String)>,
    /// Keys already owned by a confirmed or researcher-provided name of a person.
    pub protected: BTreeMap<Uuid, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NamePlan {
    Named {
        name: String,
        key: String,
        confidence: Option<f64>,
        evidence: Vec<NameEvidence>,
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

/// How a spoken person mention is introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Introduction {
    /// The speaker introduces themself with this cue.
    Own(&'static str),
    /// The speaker introduces someone else with this cue.
    Guest(&'static str),
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('’', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The cue of `cues` that the words before a mention end with.
fn ends_with_cue<'a>(before: &[String], cues: &[&'a str]) -> Option<&'a str> {
    cues.iter()
        .filter(|cue| {
            let cue: Vec<&str> = cue.split(' ').collect();
            before.len() >= cue.len() && before[before.len() - cue.len()..] == cue[..]
        })
        .max_by_key(|cue| cue.len())
        .copied()
}

/// Whether the person mention at `start..end` of a transcript segment is a self-introduction
/// or a guest introduction.
pub fn introduction(text: &str, start: usize, end: usize) -> Option<Introduction> {
    let (Some(before), Some(after)) = (text.get(..start), text.get(end..)) else {
        return None;
    };
    // The cue must run straight into the name ("I'm Ada", not "I'm, uh, Ada").
    let tail = before.trim_end();
    if tail.len() == before.len() || !tail.ends_with(|c: char| c.is_alphanumeric() || c == '\'') {
        return None;
    }
    let preceding = words(tail);
    if let Some(cue) = ends_with_cue(&preceding, &GUEST_CUES) {
        return Some(Introduction::Guest(cue));
    }
    let cue = ends_with_cue(&preceding, &SELF_CUES)?;
    let cue_words = cue.split(' ').count();
    if preceding.len() > cue_words
        && REPORTING_WORDS.contains(&preceding[preceding.len() - cue_words - 1].as_str())
    {
        return None;
    }
    // The segment must show what follows the name: a cut-off "This is Ada" may continue as
    // "Ada's argument", and a possessive never introduces the speaker.
    let after = after.replace('’', "'");
    let after = after.trim_start();
    if after.is_empty() || after.starts_with("'s") || after.starts_with('\'') {
        return None;
    }
    if SENTENCE_START_CUES.contains(&cue) {
        let sentence = tail
            .rsplit(['.', '!', '?', ';', ':'])
            .next()
            .unwrap_or_default();
        let lead = words(sentence);
        let lead = lead[..lead.len() - cue_words].join(" ");
        if !lead.is_empty() && !GREETINGS.contains(&lead.as_str()) {
            return None;
        }
    }
    Some(Introduction::Own(cue))
}

fn overlap(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.1.min(b.1) - a.0.max(b.0)).max(0.0)
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpeakerAlignment {
    /// One voice track clearly speaks during the span, covering this share of it.
    Speaker(Uuid, f64),
    /// Several voice tracks speak during the span.
    Several(Vec<Uuid>),
    None,
}

/// The voice track speaking during `span` of `video`.
pub fn speaking_track(voices: &[VoiceSpan], video: Uuid, span: (f64, f64)) -> SpeakerAlignment {
    let duration = span.1 - span.0;
    if duration <= 0.0 {
        return SpeakerAlignment::None;
    }
    let mut coverage: BTreeMap<Uuid, f64> = BTreeMap::new();
    for voice in voices.iter().filter(|v| v.video_id == video) {
        let covered = overlap(span, (voice.start_seconds, voice.end_seconds));
        if covered > 0.0 {
            *coverage.entry(voice.track_id).or_default() += covered / duration;
        }
    }
    let mut ranked: Vec<(Uuid, f64)> = coverage.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let others = ranked
        .iter()
        .filter(|(_, share)| *share > MAX_OTHER_VOICE_COVERAGE)
        .count();
    match ranked.first() {
        Some(&(track, share)) if share >= MIN_VOICE_COVERAGE && others <= 1 => {
            SpeakerAlignment::Speaker(track, share.min(1.0))
        }
        _ if others > 1 => SpeakerAlignment::Several(
            ranked
                .iter()
                .filter(|(_, share)| *share > MAX_OTHER_VOICE_COVERAGE)
                .map(|(track, _)| *track)
                .collect(),
        ),
        _ => SpeakerAlignment::None,
    }
}

fn owners(people: &BTreeMap<Uuid, BTreeSet<Uuid>>) -> BTreeMap<Uuid, Uuid> {
    people
        .iter()
        .flat_map(|(person, tracks)| tracks.iter().map(move |t| (*t, *person)))
        .collect()
}

fn note(
    notes: &mut BTreeMap<Uuid, (&'static str, BTreeSet<String>)>,
    person: Uuid,
    reason: &'static str,
    name: &str,
) {
    notes
        .entry(person)
        .or_insert((reason, BTreeSet::new()))
        .1
        .insert(name.trim().to_string());
}

/// Pure planning step: aligns name tags with faces and self-introductions with voices, and
/// decides each person's inferred name.
pub fn plan_names(inputs: &NameEvidenceInputs) -> BTreeMap<Uuid, NamePlan> {
    let face_owner = owners(&inputs.face_people);
    let voice_owner = owners(&inputs.voice_people);
    // (person, key, evidence) pairs clearly connected to that person.
    let mut strong: Vec<(Uuid, &str, NameEvidence)> = Vec::new();
    let mut ambiguous: BTreeMap<Uuid, (&'static str, BTreeSet<String>)> = BTreeMap::new();
    // Names each person introduced as somebody else.
    let mut introduced: BTreeSet<(Uuid, &str)> = BTreeSet::new();
    for tag in &inputs.tags {
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
        let visible: Vec<&VisibleFace> = inputs
            .faces
            .iter()
            .filter(|f| {
                f.video_id == e.video_id && f.start_seconds <= span.1 && span.0 <= f.end_seconds
            })
            .collect();
        match visible.as_slice() {
            [face] => {
                let duration = span.1 - span.0;
                let covered = overlap(span, (face.start_seconds, face.end_seconds));
                let aligned = duration <= 0.0 || covered / duration >= MIN_FACE_OVERLAP;
                if let (true, Some(person)) = (aligned, face_owner.get(&face.track_id)) {
                    let mut evidence = e.clone();
                    evidence.face_track_id = Some(face.track_id);
                    strong.push((*person, &tag.entity_key, evidence));
                }
            }
            [] => {}
            several => {
                for face in several {
                    if let Some(person) = face_owner.get(&face.track_id) {
                        note(
                            &mut ambiguous,
                            *person,
                            "Name tags appeared while several faces were visible",
                            &e.surface_text,
                        );
                    }
                }
            }
        }
    }
    for spoken in &inputs.spoken {
        let e = &spoken.evidence;
        let Some(kind) = introduction(&e.source_text, e.byte_start as usize, e.byte_end as usize)
        else {
            continue;
        };
        match speaking_track(&inputs.voices, e.video_id, (e.start_seconds, e.end_seconds)) {
            SpeakerAlignment::Speaker(track, share) => {
                let Some(&person) = voice_owner.get(&track) else {
                    continue;
                };
                match kind {
                    Introduction::Guest(_) => {
                        introduced.insert((person, &spoken.entity_key));
                    }
                    Introduction::Own(cue) => {
                        let mut evidence = e.clone();
                        evidence.voice_track_id = Some(track);
                        if let Value::Object(map) = &mut evidence.provenance {
                            map.insert(
                                "selfIntroduction".into(),
                                json!({"cue": cue, "speakerCoverage": share}),
                            );
                        }
                        strong.push((person, &spoken.entity_key, evidence));
                    }
                }
            }
            SpeakerAlignment::Several(tracks) if matches!(kind, Introduction::Own(_)) => {
                for track in tracks {
                    if let Some(person) = voice_owner.get(&track) {
                        note(
                            &mut ambiguous,
                            *person,
                            "A self-introduction was spoken while several voices overlapped",
                            &e.surface_text,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    // Keys supported (or already owned) per person, before the homonym check.
    let mut keys: BTreeMap<Uuid, BTreeMap<&str, Vec<NameEvidence>>> = BTreeMap::new();
    let mut blocked: BTreeMap<Uuid, (&'static str, BTreeSet<String>)> = BTreeMap::new();
    for (person, key, evidence) in strong {
        if inputs.rejected.contains(&(person, key.to_string())) {
            note(
                &mut blocked,
                person,
                "A researcher rejected the only supported name",
                &evidence.surface_text,
            );
        } else if introduced.contains(&(person, key)) {
            note(
                &mut blocked,
                person,
                "This person introduced the only supported name as another speaker",
                &evidence.surface_text,
            );
        } else {
            keys.entry(person)
                .or_default()
                .entry(key)
                .or_default()
                .push(evidence);
        }
    }
    let mut claims: BTreeMap<&str, BTreeSet<Uuid>> = BTreeMap::new();
    for (person, by_key) in &keys {
        if by_key.len() == 1 {
            claims
                .entry(by_key.keys().next().unwrap())
                .or_default()
                .insert(*person);
        }
    }
    for (person, key) in &inputs.protected {
        claims.entry(key.as_str()).or_default().insert(*person);
    }
    let display = |evidence: &[NameEvidence]| -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for e in evidence {
            *counts.entry(e.surface_text.trim()).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(name, _)| name.to_string())
            .unwrap_or_default()
    };
    let people: BTreeSet<Uuid> = inputs
        .face_people
        .keys()
        .chain(inputs.voice_people.keys())
        .copied()
        .collect();
    let mut result = BTreeMap::new();
    for person in people {
        let plan = match keys.get(&person) {
            Some(by_key) if by_key.len() > 1 => NamePlan::Unresolved {
                reason: "Conflicting names are linked to this person".into(),
                candidates: by_key.values().map(|v| display(v)).collect(),
            },
            Some(by_key) => {
                let (key, evidence) = by_key.iter().next().unwrap();
                if claims.get(key).is_some_and(|p| p.len() > 1) {
                    NamePlan::Unresolved {
                        reason: "The same name is linked to several corpus people".into(),
                        candidates: vec![display(evidence)],
                    }
                } else {
                    NamePlan::Named {
                        name: display(evidence),
                        key: key.to_string(),
                        confidence: evidence
                            .iter()
                            .filter_map(|e| e.confidence)
                            .reduce(f64::min),
                        evidence: evidence.clone(),
                    }
                }
            }
            None => match blocked.get(&person).or_else(|| ambiguous.get(&person)) {
                Some((reason, names)) => NamePlan::Unresolved {
                    reason: (*reason).into(),
                    candidates: names.iter().cloned().collect(),
                },
                None => continue,
            },
        };
        result.insert(person, plan);
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
    evidence: NameEvidenceInputs,
    rejected_names: BTreeMap<Uuid, Vec<String>>,
    /// Current attribution per person: (status, key).
    current: BTreeMap<Uuid, (String, String)>,
}

impl Inputs {
    fn plan(&self) -> BTreeMap<Uuid, NamePlan> {
        plan_names(&self.evidence)
    }
}

fn entity_provenance(row: &sqlx::postgres::PgRow) -> anyhow::Result<Value> {
    Ok(json!({
        "producer": row.try_get::<String, _>("producer")?,
        "producerVersion": row.try_get::<String, _>("producer_version")?,
        "model": row.try_get::<String, _>("model")?,
        "configHash": row.try_get::<String, _>("entity_config")?,
    }))
}

async fn load(connection: &mut PgConnection, corpus: Uuid) -> anyhow::Result<Inputs> {
    let videos: Vec<Uuid> = crate::corpora::corpus_video_ids_with(&mut *connection, corpus)
        .await?
        .into_iter()
        .collect();
    let mut labels = BTreeMap::new();
    let mut face_people: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    let mut voice_people: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for row in sqlx::query(
        "SELECT p.id, p.label, 'face' AS kind, m.track_id FROM people p
         LEFT JOIN person_face_clusters pc ON pc.person_id = p.id
         LEFT JOIN face_cluster_members m ON m.cluster_id = pc.cluster_id
         WHERE p.corpus_id = $1
         UNION ALL
         SELECT p.id, p.label, 'voice', m.track_id FROM people p
         JOIN person_voice_clusters pc ON pc.person_id = p.id
         JOIN voice_cluster_members m ON m.cluster_id = pc.cluster_id
         WHERE p.corpus_id = $1",
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
        let people = if row.try_get::<String, _>("kind")? == "face" {
            &mut face_people
        } else {
            &mut voice_people
        };
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
                visual_text_track_id: Some(row.try_get("track_id")?),
                face_track_id: None,
                transcript_segment_id: None,
                voice_track_id: None,
                provenance: json!({
                    "entity": entity_provenance(&row)?,
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
    // Speech spans of each video's current complete voice run (the run #134 matches against).
    let voices = sqlx::query(
        "SELECT l.track_id, t.video_id, o.start_seconds, o.end_seconds FROM voice_tracks t
         JOIN voice_track_observations l ON l.track_id = t.id
         JOIN voice_observations o ON o.id = l.observation_id
         WHERE t.video_id = ANY($1) AND t.run_id = (
           SELECT r.id FROM media_processing_runs r WHERE r.video_id = t.video_id
             AND r.modality = 'voice' AND r.processor = $2 AND r.processing_config->>'complete' = 'true'
             ORDER BY r.updated_at DESC, r.id DESC LIMIT 1)",
    )
    .bind(&videos)
    .bind(crate::voice_processing::VOICE_PROCESSOR)
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(|row| {
        Ok(VoiceSpan {
            track_id: row.try_get("track_id")?,
            video_id: row.try_get("video_id")?,
            start_seconds: row.try_get("start_seconds")?,
            end_seconds: row.try_get("end_seconds")?,
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;
    let spoken = sqlx::query(
        "SELECT m.id, m.video_id, m.surface_text, m.entity_key, m.byte_start, m.byte_end,
                m.confidence, s.id AS segment_id, s.stream_id, s.segment_index, s.text,
                s.start_seconds, s.end_seconds, v.title, v.source_url, er.producer,
                er.producer_version, er.model, er.config_hash AS entity_config
         FROM entity_mentions m
         JOIN entity_extraction_runs er ON er.id = m.run_id
         JOIN transcript_segments s ON s.id = m.transcript_segment_id
         JOIN videos v ON v.id = m.video_id
         WHERE m.video_id = ANY($1) AND m.entity_type = 'person' AND m.modality = 'spoken'
           AND s.start_seconds IS NOT NULL AND s.end_seconds IS NOT NULL
         ORDER BY m.video_id, s.start_seconds, m.id",
    )
    .bind(&videos)
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(|row| {
        Ok(SpokenNameCandidate {
            entity_key: row.try_get("entity_key")?,
            evidence: NameEvidence {
                video_id: row.try_get("video_id")?,
                title: row.try_get("title")?,
                source_url: row.try_get("source_url")?,
                modality: "spoken".into(),
                start_seconds: row.try_get("start_seconds")?,
                end_seconds: row.try_get("end_seconds")?,
                surface_text: row.try_get("surface_text")?,
                source_text: row.try_get("text")?,
                byte_start: row.try_get("byte_start")?,
                byte_end: row.try_get("byte_end")?,
                confidence: row.try_get("confidence")?,
                entity_mention_id: row.try_get("id")?,
                visual_text_track_id: None,
                face_track_id: None,
                transcript_segment_id: Some(row.try_get("segment_id")?),
                voice_track_id: None,
                provenance: json!({
                    "entity": entity_provenance(&row)?,
                    "transcript": {
                        "streamId": row.try_get::<Uuid, _>("stream_id")?,
                        "segmentIndex": row.try_get::<i64, _>("segment_index")?,
                    },
                    "voice": {
                        "processor": crate::voice_processing::VOICE_PROCESSOR,
                        "matching": crate::voice_matching::ALGORITHM,
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
    .collect::<anyhow::Result<BTreeMap<Uuid, (String, String)>>>()?;
    let protected = current
        .iter()
        .filter(|(_, (status, _))| status != "inferred")
        .map(|(person, (_, key))| (*person, key.clone()))
        .collect();
    Ok(Inputs {
        labels,
        evidence: NameEvidenceInputs {
            face_people,
            voice_people,
            faces,
            voices,
            tags,
            spoken,
            rejected,
            protected,
        },
        rejected_names,
        current,
    })
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
    .bind(inputs.labels.keys().copied().collect::<Vec<_>>())
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
                transcript_segment_id: row.try_get("transcript_segment_id")?,
                voice_track_id: row.try_get("voice_track_id")?,
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
    for person in inputs.labels.keys() {
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
        for e in evidence {
            sqlx::query(
                "INSERT INTO person_name_evidence
                   (id, person_id, video_id, modality, entity_mention_id, visual_text_track_id,
                    face_track_id, transcript_segment_id, voice_track_id, start_seconds,
                    end_seconds, surface_text, source_text, byte_start, byte_end, confidence,
                    provenance)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            )
            .bind(Uuid::new_v4())
            .bind(person)
            .bind(e.video_id)
            .bind(&e.modality)
            .bind(e.entity_mention_id)
            .bind(e.visual_text_track_id)
            .bind(e.face_track_id)
            .bind(e.transcript_segment_id)
            .bind(e.voice_track_id)
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

    fn evidence(
        video: Uuid,
        modality: &str,
        text: &str,
        name: &str,
        span: (f64, f64),
    ) -> NameEvidence {
        let byte_start = text.find(name).unwrap();
        NameEvidence {
            video_id: video,
            title: None,
            source_url: String::new(),
            modality: modality.into(),
            start_seconds: span.0,
            end_seconds: span.1,
            surface_text: name.into(),
            source_text: text.into(),
            byte_start: byte_start as i32,
            byte_end: (byte_start + name.len()) as i32,
            confidence: Some(0.9),
            entity_mention_id: Uuid::new_v4(),
            visual_text_track_id: None,
            face_track_id: None,
            transcript_segment_id: None,
            voice_track_id: None,
            provenance: json!({}),
        }
    }
    fn tag(video: Uuid, text: &str, name: &str, start: f64, end: f64) -> NameTagCandidate {
        let mut evidence = evidence(video, "visible_text", text, name, (start, end));
        evidence.visual_text_track_id = Some(Uuid::new_v4());
        NameTagCandidate {
            entity_key: crate::entities::entity_key(name),
            role: "scene_text".into(),
            person_mentions_in_track: 1,
            evidence,
        }
    }
    fn said(video: Uuid, text: &str, name: &str, start: f64, end: f64) -> SpokenNameCandidate {
        let mut evidence = evidence(video, "spoken", text, name, (start, end));
        evidence.transcript_segment_id = Some(Uuid::new_v4());
        SpokenNameCandidate {
            entity_key: crate::entities::entity_key(name),
            evidence,
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
    fn voice(track: Uuid, video: Uuid, start: f64, end: f64) -> VoiceSpan {
        VoiceSpan {
            track_id: track,
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
    fn faces_plan(
        people: BTreeMap<Uuid, BTreeSet<Uuid>>,
        faces: &[VisibleFace],
        tags: Vec<NameTagCandidate>,
        rejected: BTreeSet<(Uuid, String)>,
    ) -> BTreeMap<Uuid, NamePlan> {
        plan_names(&NameEvidenceInputs {
            face_people: people,
            faces: faces.to_vec(),
            tags,
            rejected,
            ..Default::default()
        })
    }
    fn voices_plan(
        people: &[(Uuid, &[Uuid])],
        voices: Vec<VoiceSpan>,
        spoken: Vec<SpokenNameCandidate>,
    ) -> BTreeMap<Uuid, NamePlan> {
        plan_names(&NameEvidenceInputs {
            voice_people: people
                .iter()
                .map(|(p, tracks)| (*p, tracks.iter().copied().collect()))
                .collect(),
            voices,
            spoken,
            ..Default::default()
        })
    }
    fn intro(text: &str, name: &str) -> Option<Introduction> {
        let start = text.find(name).unwrap();
        introduction(text, start, start + name.len())
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
    fn self_introduction_shape() {
        use Introduction::{Guest, Own};
        let own = |text, name| assert!(matches!(intro(text, name), Some(Own(_))), "{text}");
        let none = |text, name| assert_eq!(intro(text, name), None, "{text}");
        own(
            "welcome back to the Majesty of reason I'm Joe Schmid and today",
            "Joe Schmid",
        );
        own(
            "Hoi hoi, I'm Planarwalk, and recently I came across a video",
            "Planarwalk",
        );
        own("My name is Ada Lovelace.", "Ada Lovelace");
        own(
            "Hi everyone, this is Ada Lovelace, and today",
            "Ada Lovelace",
        );
        own("I’m Ada.", "Ada");
        own(
            "Hallo zusammen, ich bin Ada Lovelace und heute",
            "Ada Lovelace",
        );
        own("Mein Name ist Ada Lovelace.", "Ada Lovelace");
        // Possessives, cut-off segments and pointing at someone else.
        none(
            "So this is Emanuel Ruten's uh atomistic argument",
            "Emanuel Ruten",
        );
        none(
            "the probabilistic formulation. This is Christopher",
            "Christopher",
        );
        none("and this is Walter Sto's argument", "Walter Sto");
        none("and this is Ada, who", "Ada");
        none("I'm, uh, Ada.", "Ada");
        none("I talked to Ada Lovelace yesterday.", "Ada Lovelace");
        none(
            "evidence that I am God you see how that's a non-sequitur",
            "God",
        );
        // Hosts introducing guests.
        assert_eq!(
            intro("Joining me today is Ada Lovelace.", "Ada Lovelace"),
            Some(Guest("joining me today is"))
        );
        assert_eq!(
            intro("I'm joined by Ada Lovelace.", "Ada Lovelace"),
            Some(Guest("joined by"))
        );
        assert_eq!(
            intro("I'm here with Ada Lovelace.", "Ada Lovelace"),
            Some(Guest("here with"))
        );
        assert_eq!(
            intro("Please welcome Ada Lovelace!", "Ada Lovelace"),
            Some(Guest("please welcome"))
        );
        assert_eq!(
            intro("Heute zu Gast ist Ada Lovelace.", "Ada Lovelace"),
            Some(Guest("zu gast ist"))
        );
    }

    #[test]
    fn speaking_track_needs_one_clear_voice() {
        let video = Uuid::new_v4();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let alone = [voice(a, video, 0.0, 3.0), voice(a, video, 3.2, 4.0)];
        assert!(matches!(
            speaking_track(&alone, video, (0.0, 4.0)),
            SpeakerAlignment::Speaker(track, _) if track == a
        ));
        let brief_other = [voice(a, video, 0.0, 3.5), voice(b, video, 3.5, 4.0)];
        assert!(matches!(
            speaking_track(&brief_other, video, (0.0, 4.0)),
            SpeakerAlignment::Speaker(track, _) if track == a
        ));
        let shared = [voice(a, video, 0.0, 2.2), voice(b, video, 2.2, 4.0)];
        assert!(matches!(
            speaking_track(&shared, video, (0.0, 4.0)),
            SpeakerAlignment::Several(tracks) if tracks.len() == 2
        ));
        let sparse = [voice(a, video, 0.0, 1.0)];
        assert_eq!(
            speaking_track(&sparse, video, (0.0, 4.0)),
            SpeakerAlignment::None
        );
        assert_eq!(
            speaking_track(&alone, Uuid::new_v4(), (0.0, 4.0)),
            SpeakerAlignment::None
        );
    }

    #[test]
    fn self_introduction_names_the_speaking_voice_person() {
        let video = Uuid::new_v4();
        let (host, guest) = (Uuid::new_v4(), Uuid::new_v4());
        let (h, g) = (Uuid::new_v4(), Uuid::new_v4());
        let voices = vec![voice(h, video, 0.0, 10.0), voice(g, video, 10.0, 20.0)];
        let spoken = vec![
            said(
                video,
                "Hi everyone, I'm Ada Lovelace, and joining me is Charles Babbage.",
                "Ada Lovelace",
                0.0,
                5.0,
            ),
            said(
                video,
                "Hi everyone, I'm Ada Lovelace, and joining me is Charles Babbage.",
                "Charles Babbage",
                0.0,
                5.0,
            ),
            said(
                video,
                "Thanks Ada. I'm Charles Babbage, and I build engines.",
                "Charles Babbage",
                10.0,
                14.0,
            ),
        ];
        let plan = voices_plan(&[(host, &[h]), (guest, &[g])], voices, spoken);
        let named = |person| match &plan[&person] {
            NamePlan::Named { name, evidence, .. } => (name.clone(), evidence[0].voice_track_id),
            other => panic!("{other:?}"),
        };
        assert_eq!(named(host), ("Ada Lovelace".into(), Some(h)));
        assert_eq!(named(guest), ("Charles Babbage".into(), Some(g)));
    }

    #[test]
    fn guest_introductions_and_shared_speech_never_name_the_speaker() {
        let video = Uuid::new_v4();
        let host = Uuid::new_v4();
        let h = Uuid::new_v4();
        // Over-split diarization put the guest's "I'm Charles" on the host's track: the host
        // introduced that name as another speaker, so it stays unresolved.
        let voices = vec![voice(h, video, 0.0, 20.0)];
        let spoken = vec![
            said(
                video,
                "Joining me is Charles Babbage.",
                "Charles Babbage",
                0.0,
                3.0,
            ),
            said(
                video,
                "Thanks. I'm Charles Babbage, hello.",
                "Charles Babbage",
                10.0,
                13.0,
            ),
        ];
        let plan = voices_plan(&[(host, &[h])], voices, spoken);
        assert!(
            matches!(&plan[&host], NamePlan::Unresolved { reason, .. } if reason.contains("another speaker"))
        );
        // A guest introduction alone names nobody.
        let voices = vec![voice(h, video, 0.0, 20.0)];
        let spoken = vec![said(
            video,
            "Joining me is Charles Babbage.",
            "Charles Babbage",
            0.0,
            3.0,
        )];
        assert!(voices_plan(&[(host, &[h])], voices, spoken).is_empty());
        // Two voices during the introduction: ambiguous.
        let g = Uuid::new_v4();
        let voices = vec![voice(h, video, 0.0, 1.5), voice(g, video, 1.5, 3.0)];
        let spoken = vec![said(video, "I'm Ada Lovelace.", "Ada Lovelace", 0.0, 3.0)];
        let plan = voices_plan(&[(host, &[h])], voices, spoken);
        assert!(
            matches!(&plan[&host], NamePlan::Unresolved { reason, .. } if reason.contains("several voices"))
        );
        // A track outside every person names nobody.
        let voices = vec![voice(g, video, 0.0, 3.0)];
        let spoken = vec![said(video, "I'm Ada Lovelace.", "Ada Lovelace", 0.0, 3.0)];
        assert!(voices_plan(&[(host, &[h])], voices, spoken).is_empty());
    }

    #[test]
    fn spoken_and_on_screen_names_combine_and_conflict() {
        let video = Uuid::new_v4();
        let person = Uuid::new_v4();
        let f = face(video, 0.0, 10.0);
        let v = Uuid::new_v4();
        let inputs = |spoken_name: &str| NameEvidenceInputs {
            face_people: people(&[(person, &[f])]),
            voice_people: BTreeMap::from([(person, BTreeSet::from([v]))]),
            faces: vec![f],
            voices: vec![voice(v, video, 0.0, 10.0)],
            tags: vec![tag(video, "Ada Lovelace", "Ada Lovelace", 1.0, 3.0)],
            spoken: vec![said(
                video,
                &format!("I'm {spoken_name}."),
                spoken_name,
                4.0,
                6.0,
            )],
            ..Default::default()
        };
        let plan = plan_names(&inputs("Ada Lovelace"));
        assert!(matches!(&plan[&person], NamePlan::Named { evidence, .. } if evidence.len() == 2));
        let plan = plan_names(&inputs("Ada Byron"));
        assert!(
            matches!(&plan[&person], NamePlan::Unresolved { candidates, .. } if candidates.len() == 2)
        );
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
        let plan = faces_plan(people(&[(person, &[f])]), &[f], tags, BTreeSet::new());
        assert!(
            matches!(&plan[&person], NamePlan::Named { name, evidence, .. }
            if name == "Ada Lovelace" && evidence[0].face_track_id == Some(f.track_id))
        );
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
        assert!(faces_plan(people(&[(person, &[f])]), &[f], mention, BTreeSet::new()).is_empty());
        // A tag shown while two faces are visible is ambiguous.
        let tags = vec![tag(video, "Ada Lovelace", "Ada Lovelace", 11.0, 15.0)];
        let plan = faces_plan(all, &[f, g], tags, BTreeSet::new());
        assert!(plan
            .values()
            .all(|p| matches!(p, NamePlan::Unresolved { .. })));
        // A tag mostly outside the face's time is not aligned.
        let late = vec![tag(video, "Ada Lovelace", "Ada Lovelace", 18.0, 30.0)];
        assert!(faces_plan(people(&[(person, &[f])]), &[f], late, BTreeSet::new()).is_empty());
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
        let plan = faces_plan(
            people(&[(person, &[f1, f2])]),
            &[f1, f2],
            conflicting,
            BTreeSet::new(),
        );
        assert!(
            matches!(&plan[&person], NamePlan::Unresolved { candidates, .. } if candidates.len() == 2)
        );

        let homonyms = vec![
            tag(v1, "John Smith", "John Smith", 1.0, 4.0),
            tag(v2, "John Smith", "John Smith", 1.0, 4.0),
        ];
        let plan = faces_plan(
            people(&[(person, &[f1]), (other, &[f2])]),
            &[f1, f2],
            homonyms.clone(),
            BTreeSet::new(),
        );
        assert!(plan
            .values()
            .all(|p| matches!(p, NamePlan::Unresolved { .. })));

        let rejected = BTreeSet::from([(person, "john smith".to_string())]);
        let plan = faces_plan(
            people(&[(person, &[f1])]),
            &[f1],
            homonyms[..1].to_vec(),
            rejected,
        );
        assert!(
            matches!(&plan[&person], NamePlan::Unresolved { reason, .. } if reason.contains("rejected"))
        );
    }
}
