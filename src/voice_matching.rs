//! Cross-video voice matching into corpus people and evidence-based face-to-voice links (#134).
//!
//! The voices detector (#89) stores canonical WeSpeaker embeddings per speech span. This module
//! only aggregates them per voice track (a speech-weighted centroid) and ranks corpus people with
//! the audio-analysis `SpeakerLibrary`. Every automatic association is an unverified suggestion
//! until a researcher confirms it; corrections survive reruns like face matching (#64).
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use audio_analysis_speakers::{
    SpeakerEmbedding, SpeakerEmbeddingModel, SpeakerEmbeddingModelFamily, SpeakerId,
    SpeakerIdentificationOptions, SpeakerLabel, SpeakerLibrary, SpeakerProfile,
    UnknownSpeakerPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::voice_processing::{VOICE_MODEL, VOICE_MODEL_VERSION, VOICE_PROCESSOR};

pub const ALGORITHM: &str = "youtube-corpus/voice-centroid-v1";
const EMBEDDING_DIMENSIONS: usize = 256;
/// A track needs this much speech before it is matched across videos.
const MIN_TRACK_SPEECH_SECONDS: f64 = 2.0;
/// Track centroid vs person centroid cosine similarity needed to suggest a match.
const MIN_SIMILARITY: f32 = 0.60;
/// Lead over the next-best person; closer competitors leave the track unresolved.
const MIN_MARGIN: f32 = 0.10;
/// At or above this a match is near-identical audio, typically a reused clip or intro.
const REUSED_CLIP_SIMILARITY: f64 = 0.95;
const MIN_LINK_SHARED_VIDEOS: usize = 2;
/// Share of the voice's speech (in shared videos) during which the face is on screen.
const MIN_LINK_COVERAGE: f64 = 0.5;
const MIN_LINK_OVERLAP_SECONDS: f64 = 10.0;
/// Coverage lead over the voice's next-best face person.
const MIN_LINK_MARGIN: f64 = 0.2;

fn configuration() -> Value {
    json!({"algorithm":ALGORITHM,"processorRevision":crate::provenance::PROCESSOR_SOURCE_REVISION,
        "embedder":{"processor":VOICE_PROCESSOR,"model":VOICE_MODEL,"modelVersion":VOICE_MODEL_VERSION},
        "trackEmbedding":"speech-weighted centroid of observation embeddings",
        "ranking":"audio-analysis SpeakerLibrary::identify over person centroids",
        "minTrackSpeechSeconds":MIN_TRACK_SPEECH_SECONDS,"minCosineSimilarity":MIN_SIMILARITY,
        "minAlternativeMargin":MIN_MARGIN,"newPersonMinVideos":2,"reusedClipSimilarity":REUSED_CLIP_SIMILARITY,
        "faceLinks":{"minSharedVideos":MIN_LINK_SHARED_VIDEOS,"minCoverage":MIN_LINK_COVERAGE,
            "minOverlapSeconds":MIN_LINK_OVERLAP_SECONDS,"minCoverageMargin":MIN_LINK_MARGIN,"mutualBest":true}})
}
fn hash(value: &Value) -> String {
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceMatchStatus {
    /// An automatic association that no researcher has reviewed.
    Suggested,
    /// Placed or confirmed by a researcher.
    Confirmed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEvidence {
    pub track_id: Uuid,
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub observations: usize,
    pub speech_seconds: f64,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub producer_config: Value,
    pub similarity: Option<f64>,
    pub status: Option<VoiceMatchStatus>,
    pub assignment_source: Option<String>,
    pub matching_configuration: Option<Value>,
    pub protected: bool,
    pub reason: Option<String>,
    pub note: Option<String>,
    #[serde(skip)]
    sum: Vec<f32>,
    #[serde(skip)]
    spans: Vec<(f64, f64)>,
    #[serde(skip)]
    current: bool,
    #[serde(skip)]
    decision: Option<String>,
    #[serde(skip)]
    rejected: BTreeSet<Uuid>,
}
impl VoiceEvidence {
    fn supported(&self) -> bool {
        self.processor == VOICE_PROCESSOR
            && self.model == VOICE_MODEL
            && self.model_version == VOICE_MODEL_VERSION
            && self.sum.len() == EMBEDDING_DIMENSIONS
            && self.sum.iter().all(|v| v.is_finite())
            && self.sum.iter().any(|v| *v != 0.0)
    }
    fn strong(&self) -> bool {
        self.supported() && self.speech_seconds >= MIN_TRACK_SPEECH_SECONDS
    }
    fn placement_fixed(&self) -> bool {
        self.decision.as_deref().is_some_and(|d| d != "unlink")
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoicePersonGroup {
    pub person_id: Uuid,
    pub label: String,
    pub identity_source: Option<String>,
    pub tracks: Vec<VoiceEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceVoiceLinkVideo {
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub overlap_seconds: f64,
    pub voice_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceVoiceLink {
    pub face_person_id: Uuid,
    pub face_label: String,
    pub voice_person_id: Uuid,
    pub voice_label: String,
    /// Always `unverified`: confirming merges the people, rejecting hides the suggestion.
    pub status: String,
    pub shared_videos: i32,
    pub voice_seconds: f64,
    pub overlap_seconds: f64,
    pub coverage: f64,
    pub margin: f64,
    pub videos: Vec<FaceVoiceLinkVideo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMatchReview {
    pub configuration: Value,
    pub groups: Vec<VoicePersonGroup>,
    pub unresolved: Vec<VoiceEvidence>,
    pub face_links: Vec<FaceVoiceLink>,
}

fn parse_vector(text: Option<String>) -> anyhow::Result<Vec<f32>> {
    Ok(text
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or_default())
}

async fn corpus_tracks(
    connection: &mut PgConnection,
    corpus: Uuid,
) -> anyhow::Result<Vec<VoiceEvidence>> {
    let ids: Vec<Uuid> = crate::corpora::corpus_video_ids_with(&mut *connection, corpus)
        .await?
        .into_iter()
        .collect();
    let rows = sqlx::query(
        "SELECT t.id,t.video_id,t.run_id,t.start_seconds,t.end_seconds,v.title,v.source_url,
        r.processor,r.processor_version,r.model,r.model_version,r.input_hash,r.processing_config,
        r.id=(SELECT x.id FROM media_processing_runs x WHERE x.video_id=t.video_id AND x.modality='voice'
            AND x.processor=$2 AND x.processing_config->>'complete'='true' ORDER BY x.updated_at DESC,x.id DESC LIMIT 1) current,
        (SELECT d.decision FROM voice_match_decisions d WHERE d.corpus_id=$3 AND d.track_id=t.id) decision,
        ARRAY(SELECT CASE WHEN j.track_a=t.id THEN j.track_b ELSE j.track_a END
            FROM voice_match_rejections j WHERE j.corpus_id=$3 AND (j.track_a=t.id OR j.track_b=t.id)) rejected
        FROM voice_tracks t JOIN media_processing_runs r ON r.id=t.run_id JOIN videos v ON v.id=t.video_id
        WHERE t.video_id=ANY($1) ORDER BY t.video_id,t.start_seconds,t.id",
    )
    .bind(&ids)
    .bind(VOICE_PROCESSOR)
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?;
    let mut tracks: Vec<VoiceEvidence> = rows
        .into_iter()
        .map(|r| {
            let decision: Option<String> = r.try_get("decision")?;
            Ok(VoiceEvidence {
                track_id: r.try_get("id")?,
                video_id: r.try_get("video_id")?,
                title: r.try_get("title")?,
                source_url: r.try_get("source_url")?,
                start_seconds: r.try_get("start_seconds")?,
                end_seconds: r.try_get("end_seconds")?,
                observations: 0,
                speech_seconds: 0.0,
                processor: r.try_get("processor")?,
                processor_version: r.try_get("processor_version")?,
                model: r.try_get("model")?,
                model_version: r.try_get("model_version")?,
                input_hash: r.try_get("input_hash")?,
                producer_config: r.try_get("processing_config")?,
                similarity: None,
                status: None,
                assignment_source: None,
                matching_configuration: None,
                protected: decision.is_some(),
                reason: None,
                note: None,
                sum: Vec::new(),
                spans: Vec::new(),
                current: r.try_get::<Option<bool>, _>("current")?.unwrap_or(false),
                decision,
                rejected: r.try_get::<Vec<Uuid>, _>("rejected")?.into_iter().collect(),
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let index: BTreeMap<Uuid, usize> = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.track_id, i))
        .collect();
    let observations = sqlx::query(
        "SELECT l.track_id,o.start_seconds,o.end_seconds,o.embedding::text vector
        FROM voice_track_observations l JOIN voice_observations o ON o.id=l.observation_id
        WHERE o.video_id=ANY($1) ORDER BY l.track_id,o.start_seconds,o.id",
    )
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?;
    for row in observations {
        let Some(&i) = index.get(&row.try_get::<Uuid, _>("track_id")?) else {
            continue;
        };
        let (start, end): (f64, f64) = (row.try_get("start_seconds")?, row.try_get("end_seconds")?);
        let track = &mut tracks[i];
        track.observations += 1;
        track.spans.push((start, end));
        let embedding = parse_vector(row.try_get("vector")?)?;
        let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        let seconds = end - start;
        if embedding.len() != EMBEDDING_DIMENSIONS || !norm.is_finite() || norm <= f32::EPSILON {
            continue;
        }
        track.speech_seconds += seconds;
        if track.sum.is_empty() {
            track.sum = vec![0.0; EMBEDDING_DIMENSIONS];
        }
        // Speech-weighted: a long span says more about the voice than a short one.
        for (total, value) in track.sum.iter_mut().zip(embedding) {
            *total += value / norm * seconds as f32;
        }
    }
    Ok(tracks)
}

async fn groups(
    connection: &mut PgConnection,
    corpus: Uuid,
    tracks: &[VoiceEvidence],
) -> anyhow::Result<Vec<VoicePersonGroup>> {
    let rows = sqlx::query(
        "SELECT p.id,p.label,p.identity_source,m.track_id,m.similarity,m.assignment_algorithm,config.configuration
        FROM people p JOIN person_voice_clusters pc ON pc.person_id=p.id
        JOIN voice_clusters c ON c.id=pc.cluster_id AND c.corpus_id=p.corpus_id
        LEFT JOIN voice_cluster_members m ON m.cluster_id=c.id
        LEFT JOIN voice_match_evidence edge ON edge.cluster_id=m.cluster_id AND edge.track_id=m.track_id
        LEFT JOIN voice_match_configurations config ON config.config_hash=coalesce(edge.config_hash,c.config_hash)
        WHERE p.corpus_id=$1 ORDER BY p.created_at,p.id,m.track_id",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?;
    let by_id: BTreeMap<Uuid, &VoiceEvidence> = tracks.iter().map(|t| (t.track_id, t)).collect();
    let mut result: BTreeMap<Uuid, VoicePersonGroup> = BTreeMap::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let group = result.entry(id).or_insert(VoicePersonGroup {
            person_id: id,
            label: row
                .try_get::<Option<String>, _>("label")?
                .unwrap_or_else(|| format!("Inferred person {}", &id.simple().to_string()[..8])),
            identity_source: row.try_get("identity_source")?,
            tracks: Vec::new(),
        });
        let Some(track) = row
            .try_get::<Option<Uuid>, _>("track_id")?
            .and_then(|t| by_id.get(&t))
        else {
            continue;
        };
        if group.tracks.iter().any(|t| t.track_id == track.track_id) {
            continue;
        }
        let mut track = (*track).clone();
        track.similarity = row.try_get("similarity")?;
        track.assignment_source = row.try_get("assignment_algorithm")?;
        track.matching_configuration = row.try_get("configuration")?;
        track.status = Some(if track.placement_fixed() {
            VoiceMatchStatus::Confirmed
        } else {
            VoiceMatchStatus::Suggested
        });
        if track
            .similarity
            .is_some_and(|s| s >= REUSED_CLIP_SIMILARITY)
        {
            track.note = Some(
                "Near-identical audio to this person's other tracks: possibly a reused clip, intro or music rather than separate speech"
                    .into(),
            );
        }
        group.tracks.push(track);
    }
    Ok(result.into_values().collect())
}

async fn face_links(
    connection: &mut PgConnection,
    corpus: Uuid,
) -> anyhow::Result<Vec<FaceVoiceLink>> {
    let rows = sqlx::query(
        "SELECT l.*,coalesce(f.label,'Inferred person') face_label,coalesce(v.label,'Inferred person') voice_label
        FROM person_face_voice_links l JOIN people f ON f.id=l.face_person_id JOIN people v ON v.id=l.voice_person_id
        WHERE l.corpus_id=$1 AND l.status='unverified' ORDER BY l.coverage DESC,l.face_person_id,l.voice_person_id",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(FaceVoiceLink {
                face_person_id: r.try_get("face_person_id")?,
                face_label: r.try_get("face_label")?,
                voice_person_id: r.try_get("voice_person_id")?,
                voice_label: r.try_get("voice_label")?,
                status: r.try_get("status")?,
                shared_videos: r.try_get("shared_videos")?,
                voice_seconds: r.try_get("voice_seconds")?,
                overlap_seconds: r.try_get("overlap_seconds")?,
                coverage: r.try_get("coverage")?,
                margin: r.try_get("margin")?,
                videos: serde_json::from_value(r.try_get("videos")?)?,
            })
        })
        .collect()
}

async fn review_with(
    connection: &mut PgConnection,
    corpus: Uuid,
) -> anyhow::Result<VoiceMatchReview> {
    let tracks = corpus_tracks(&mut *connection, corpus).await?;
    let groups = groups(&mut *connection, corpus, &tracks).await?;
    let assigned: BTreeSet<Uuid> = groups
        .iter()
        .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
        .collect();
    let mut unresolved: Vec<VoiceEvidence> = tracks
        .into_iter()
        .filter(|t| t.current && !assigned.contains(&t.track_id))
        .map(|mut t| {
            t.reason = Some(
                if t.protected {
                    "Prior association rejected by a researcher; no different high-confidence match"
                } else if !t.supported() {
                    "Unsupported or incompatible producer/model evidence"
                } else if !t.strong() {
                    "Less than 2 s of speech: too little voice for a cross-video match"
                } else {
                    "No unique high-confidence match in another video"
                }
                .into(),
            );
            t
        })
        .collect();
    // Matchable evidence first; the many short fragments follow.
    unresolved.sort_by(|a, b| {
        b.strong()
            .cmp(&a.strong())
            .then(b.speech_seconds.total_cmp(&a.speech_seconds))
            .then(a.track_id.cmp(&b.track_id))
    });
    Ok(VoiceMatchReview {
        configuration: configuration(),
        groups,
        unresolved,
        face_links: face_links(&mut *connection, corpus).await?,
    })
}

pub async fn review_voice_matches(pool: &PgPool, corpus: Uuid) -> anyhow::Result<VoiceMatchReview> {
    let mut connection = pool.acquire().await?;
    review_with(&mut connection, corpus).await
}

fn speaker_model() -> anyhow::Result<SpeakerEmbeddingModel> {
    Ok(SpeakerEmbeddingModel::new(
        SpeakerEmbeddingModelFamily::Custom("wespeaker".into()),
        VOICE_MODEL,
        VOICE_MODEL_VERSION,
        EMBEDDING_DIMENSIONS,
    )?)
}
fn embedding(sum: &[f32], model: &SpeakerEmbeddingModel) -> anyhow::Result<SpeakerEmbedding> {
    // `SpeakerEmbedding::new` L2-normalizes, turning a weighted sum into its centroid.
    Ok(SpeakerEmbedding::new(sum.to_vec(), model.clone(), 16000)?)
}
fn add(total: &mut [f32], sum: &[f32]) {
    for (t, v) in total.iter_mut().zip(sum) {
        *t += v;
    }
}

/// A person the plan may extend: an existing corpus person or one formed in this run.
struct Candidate {
    person: Option<Uuid>,
    sum: Vec<f32>,
    members: Vec<(usize, Option<f32>)>,
    track_ids: BTreeSet<Uuid>,
    videos: BTreeSet<Uuid>,
}
struct Assignment {
    track: Uuid,
    person: Uuid,
    score: f32,
}
struct MatchPlan {
    new_people: Vec<Uuid>,
    assignments: Vec<Assignment>,
}

fn plan_matches(
    tracks: &[VoiceEvidence],
    groups: &[VoicePersonGroup],
) -> anyhow::Result<MatchPlan> {
    let model = speaker_model()?;
    // The margin is applied below: the crate's margin check rejects a lone profile, but a
    // single existing person is no competing alternative.
    let options = SpeakerIdentificationOptions {
        min_score: MIN_SIMILARITY,
        min_margin: None,
        max_results: 2,
        unknown_policy: UnknownSpeakerPolicy::NoMatch,
    };
    let assigned: BTreeSet<Uuid> = groups
        .iter()
        .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
        .collect();
    let mut candidates: Vec<Candidate> = groups
        .iter()
        .map(|g| {
            let mut sum = vec![0.0; EMBEDDING_DIMENSIONS];
            for t in g.tracks.iter().filter(|t| t.supported()) {
                add(&mut sum, &t.sum);
            }
            Candidate {
                person: Some(g.person_id),
                sum,
                members: Vec::new(),
                track_ids: g.tracks.iter().map(|t| t.track_id).collect(),
                videos: g.tracks.iter().map(|t| t.video_id).collect(),
            }
        })
        .collect();
    // Strongest evidence first, so people form around long, clean speech.
    let mut eligible: Vec<usize> = (0..tracks.len())
        .filter(|&i| {
            let t = &tracks[i];
            t.current && !t.placement_fixed() && t.strong() && !assigned.contains(&t.track_id)
        })
        .collect();
    eligible.sort_by(|&a, &b| {
        tracks[b]
            .speech_seconds
            .total_cmp(&tracks[a].speech_seconds)
            .then(tracks[a].track_id.cmp(&tracks[b].track_id))
    });
    for index in eligible {
        let track = &tracks[index];
        let mut library = SpeakerLibrary::new();
        for (c, candidate) in candidates.iter().enumerate() {
            if candidate.sum.iter().all(|v| *v == 0.0)
                || !candidate.track_ids.is_disjoint(&track.rejected)
            {
                continue;
            }
            library.add_profile(
                SpeakerProfile::new(
                    SpeakerId::new(c.to_string())?,
                    SpeakerLabel::new(c.to_string())?,
                )
                .with_embedding(embedding(&candidate.sum, &model)?)?,
            )?;
        }
        let result = library.identify(&embedding(&track.sum, &model)?, &options)?;
        let accepted = result
            .best_match
            .filter(|_| result.margin.is_none_or(|m| m >= MIN_MARGIN));
        match accepted {
            Some(best) => {
                let c: usize = best.speaker_id.as_str().parse()?;
                let candidate = &mut candidates[c];
                add(&mut candidate.sum, &track.sum);
                candidate.members.push((index, Some(best.score)));
                candidate.track_ids.insert(track.track_id);
                candidate.videos.insert(track.video_id);
            }
            // Competing people within the margin: leave the track unresolved.
            None if result
                .ranked_matches
                .first()
                .is_some_and(|m| m.score >= MIN_SIMILARITY) => {}
            None => candidates.push(Candidate {
                person: None,
                sum: track.sum.clone(),
                members: vec![(index, None)],
                track_ids: BTreeSet::from([track.track_id]),
                videos: BTreeSet::from([track.video_id]),
            }),
        }
    }
    let mut plan = MatchPlan {
        new_people: Vec::new(),
        assignments: Vec::new(),
    };
    for candidate in candidates {
        let person = match candidate.person {
            Some(person) => person,
            // A new person needs the voice in at least two videos.
            None if candidate.videos.len() >= 2 => {
                let id = Uuid::new_v4();
                plan.new_people.push(id);
                id
            }
            None => continue,
        };
        let new = candidate.person.is_none();
        for (index, score) in &candidate.members {
            let track = &tracks[*index];
            // New people report each member against the rest of the person (leave one out).
            let score = if new {
                let mut rest = candidate.sum.clone();
                for (t, v) in rest.iter_mut().zip(&track.sum) {
                    *t -= v;
                }
                embedding(&rest, &model)?.cosine_similarity(&embedding(&track.sum, &model)?)?
            } else {
                score.unwrap_or(MIN_SIMILARITY)
            };
            plan.assignments.push(Assignment {
                track: track.track_id,
                person,
                score,
            });
        }
    }
    Ok(plan)
}

fn overlap(spans: &[(f64, f64)], visible: &[(f64, f64)]) -> f64 {
    spans
        .iter()
        .map(|&(start, end)| {
            visible
                .iter()
                .map(|&(a, b)| (end.min(b) - start.max(a)).max(0.0))
                .sum::<f64>()
                .min(end - start)
        })
        .sum()
}

/// Union of possibly overlapping intervals, so co-occurrence is never counted twice.
fn union(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

struct LinkCandidate {
    face: Uuid,
    voice: Uuid,
    coverage: f64,
    margin: f64,
    voice_seconds: f64,
    overlap_seconds: f64,
    videos: Vec<FaceVoiceLinkVideo>,
}

/// Suggests face↔voice links from co-occurrence: the share of a voice person's speech during
/// which a face person is on screen, over at least two shared videos.
async fn suggest_face_links(
    tx: &mut Transaction<'_, Postgres>,
    corpus: Uuid,
    tracks: &[VoiceEvidence],
) -> anyhow::Result<()> {
    let groups = groups(&mut *tx, corpus, tracks).await?;
    let face_rows = sqlx::query(
        "SELECT pc.person_id,ft.video_id,ft.start_seconds,ft.end_seconds
        FROM person_face_clusters pc JOIN face_clusters c ON c.id=pc.cluster_id AND c.corpus_id=$1
        JOIN face_cluster_members m ON m.cluster_id=c.id JOIN face_tracks ft ON ft.id=m.track_id",
    )
    .bind(corpus)
    .fetch_all(&mut **tx)
    .await?;
    let mut faces: BTreeMap<Uuid, BTreeMap<Uuid, Vec<(f64, f64)>>> = BTreeMap::new();
    for row in face_rows {
        faces
            .entry(row.try_get("person_id")?)
            .or_default()
            .entry(row.try_get("video_id")?)
            .or_default()
            .push((row.try_get("start_seconds")?, row.try_get("end_seconds")?));
    }
    let faces: BTreeMap<Uuid, BTreeMap<Uuid, Vec<(f64, f64)>>> = faces
        .into_iter()
        .map(|(p, v)| (p, v.into_iter().map(|(k, s)| (k, union(s))).collect()))
        .collect();
    let voices: BTreeSet<Uuid> = groups
        .iter()
        .filter(|g| !g.tracks.is_empty())
        .map(|g| g.person_id)
        .collect();
    let mut best_per_voice: Vec<LinkCandidate> = Vec::new();
    for group in groups.iter().filter(|g| !g.tracks.is_empty()) {
        // Only a voice-only person is linked to a face-only person.
        if faces.contains_key(&group.person_id) {
            continue;
        }
        let mut speech: BTreeMap<Uuid, Vec<(f64, f64)>> = BTreeMap::new();
        let mut meta: BTreeMap<Uuid, (Option<String>, String)> = BTreeMap::new();
        for track in &group.tracks {
            speech
                .entry(track.video_id)
                .or_default()
                .extend(&track.spans);
            meta.entry(track.video_id)
                .or_insert_with(|| (track.title.clone(), track.source_url.clone()));
        }
        let mut scored: Vec<LinkCandidate> = Vec::new();
        for (face, visible) in faces.iter().filter(|(p, _)| !voices.contains(p)) {
            let (mut voice_seconds, mut overlap_seconds, mut videos) = (0.0, 0.0, Vec::new());
            for (video, spans) in &speech {
                let Some(on_screen) = visible.get(video) else {
                    continue;
                };
                let seconds: f64 = spans.iter().map(|(a, b)| b - a).sum();
                let shared = overlap(spans, on_screen);
                if shared <= 0.0 {
                    continue;
                }
                voice_seconds += seconds;
                overlap_seconds += shared;
                let (title, source_url) = &meta[video];
                videos.push(FaceVoiceLinkVideo {
                    video_id: *video,
                    title: title.clone(),
                    source_url: source_url.clone(),
                    overlap_seconds: shared,
                    voice_seconds: seconds,
                });
            }
            if videos.is_empty() {
                continue;
            }
            scored.push(LinkCandidate {
                face: *face,
                voice: group.person_id,
                coverage: (overlap_seconds / voice_seconds).min(1.0),
                margin: 1.0,
                voice_seconds,
                overlap_seconds,
                videos,
            });
        }
        scored.sort_by(|a, b| b.coverage.total_cmp(&a.coverage).then(a.face.cmp(&b.face)));
        let runner_up = scored.get(1).map(|s| s.coverage);
        if let Some(mut best) = scored.into_iter().next() {
            best.margin = runner_up.map_or(1.0, |r| best.coverage - r);
            best_per_voice.push(best);
        }
    }
    // Mutual best: a face is suggested only for the voice it covers best.
    let mut best_voice_for_face: BTreeMap<Uuid, f64> = BTreeMap::new();
    for c in &best_per_voice {
        let entry = best_voice_for_face.entry(c.face).or_insert(c.coverage);
        *entry = entry.max(c.coverage);
    }
    let settings = configuration();
    let config_hash = hash(&settings);
    sqlx::query("INSERT INTO voice_match_configurations(config_hash,configuration) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(&config_hash).bind(&settings).execute(&mut **tx).await?;
    let mut kept: Vec<(Uuid, Uuid)> = Vec::new();
    for c in best_per_voice {
        if c.videos.len() < MIN_LINK_SHARED_VIDEOS
            || c.coverage < MIN_LINK_COVERAGE
            || c.overlap_seconds < MIN_LINK_OVERLAP_SECONDS
            || c.margin < MIN_LINK_MARGIN
            || best_voice_for_face.get(&c.face) != Some(&c.coverage)
        {
            continue;
        }
        kept.push((c.face, c.voice));
        sqlx::query(
            "INSERT INTO person_face_voice_links(corpus_id,face_person_id,voice_person_id,status,shared_videos,
                voice_seconds,overlap_seconds,coverage,margin,videos,config_hash)
            VALUES($1,$2,$3,'unverified',$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT(face_person_id,voice_person_id) DO UPDATE SET shared_videos=EXCLUDED.shared_videos,
                voice_seconds=EXCLUDED.voice_seconds,overlap_seconds=EXCLUDED.overlap_seconds,coverage=EXCLUDED.coverage,
                margin=EXCLUDED.margin,videos=EXCLUDED.videos,config_hash=EXCLUDED.config_hash,updated_at=now()
            WHERE person_face_voice_links.status='unverified'",
        )
        .bind(corpus)
        .bind(c.face)
        .bind(c.voice)
        .bind(i32::try_from(c.videos.len())?)
        .bind(c.voice_seconds)
        .bind(c.overlap_seconds)
        .bind(c.coverage)
        .bind(c.margin)
        .bind(serde_json::to_value(&c.videos)?)
        .bind(&config_hash)
        .execute(&mut **tx)
        .await?;
    }
    // Suggestions the evidence no longer supports disappear; rejections stay.
    let (faces_kept, voices_kept): (Vec<Uuid>, Vec<Uuid>) = kept.into_iter().unzip();
    sqlx::query(
        "DELETE FROM person_face_voice_links l WHERE l.corpus_id=$1 AND l.status='unverified'
        AND NOT EXISTS(SELECT 1 FROM unnest($2::uuid[],$3::uuid[]) k(f,v) WHERE k.f=l.face_person_id AND k.v=l.voice_person_id)",
    )
    .bind(corpus)
    .bind(&faces_kept)
    .bind(&voices_kept)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn lock_corpus(tx: &mut Transaction<'_, Postgres>, corpus: Uuid) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("voice-matches:{corpus}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn create_person(
    tx: &mut Transaction<'_, Postgres>,
    corpus: Uuid,
    id: Uuid,
    source: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO people(id,corpus_id,label,identity_source) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(corpus)
        .bind(format!("Inferred person {}", &id.simple().to_string()[..8]))
        .bind(source)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn cluster_for(
    tx: &mut Transaction<'_, Postgres>,
    corpus: Uuid,
    person: Uuid,
    source: &str,
) -> anyhow::Result<Uuid> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM people WHERE id=$1 AND corpus_id=$2)")
            .bind(person)
            .bind(corpus)
            .fetch_one(&mut **tx)
            .await?;
    anyhow::ensure!(exists, "person not found in corpus");
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM voice_clusters c JOIN person_voice_clusters pc ON pc.cluster_id=c.id
        WHERE c.corpus_id=$1 AND pc.person_id=$2 AND c.algorithm=$3 ORDER BY c.created_at,c.id LIMIT 1",
    )
    .bind(corpus)
    .bind(person)
    .bind(ALGORITHM)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let settings = configuration();
    sqlx::query("INSERT INTO voice_match_configurations(config_hash,configuration) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(hash(&settings)).bind(&settings).execute(&mut **tx).await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO voice_clusters(id,corpus_id,algorithm,algorithm_version,config_hash) VALUES($1,$2,$3,$4,$5)")
        .bind(id).bind(corpus).bind(ALGORITHM).bind(crate::provenance::PROCESSOR_SOURCE_REVISION).bind(hash(&settings))
        .execute(&mut **tx).await?;
    sqlx::query("INSERT INTO person_voice_clusters(person_id,cluster_id,source) VALUES($1,$2,$3)")
        .bind(person)
        .bind(id)
        .bind(source)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}
async fn assign(
    tx: &mut Transaction<'_, Postgres>,
    cluster: Uuid,
    track: Uuid,
    score: Option<f64>,
    source: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO voice_cluster_members(cluster_id,track_id,similarity,assignment_algorithm) VALUES($1,$2,$3,$4)
        ON CONFLICT(cluster_id,track_id) DO UPDATE SET similarity=EXCLUDED.similarity,assignment_algorithm=EXCLUDED.assignment_algorithm,assigned_at=now()")
        .bind(cluster).bind(track).bind(score).bind(source).execute(&mut **tx).await?;
    let settings = if score.is_some() {
        configuration()
    } else {
        json!({"algorithm":"researcher-corrected association","processorRevision":crate::provenance::PROCESSOR_SOURCE_REVISION,"source":source})
    };
    let config_hash = hash(&settings);
    sqlx::query("INSERT INTO voice_match_configurations(config_hash,configuration) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(&config_hash).bind(settings).execute(&mut **tx).await?;
    sqlx::query(
        "INSERT INTO voice_match_evidence(cluster_id,track_id,config_hash) VALUES($1,$2,$3)
        ON CONFLICT(cluster_id,track_id) DO UPDATE SET config_hash=EXCLUDED.config_hash",
    )
    .bind(cluster)
    .bind(track)
    .bind(config_hash)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Suggests cross-video voice matches and face↔voice links for a corpus. Researcher decisions
/// are never changed; suggestions stay unverified until confirmed.
pub async fn match_corpus_voices(pool: &PgPool, corpus: Uuid) -> anyhow::Result<VoiceMatchReview> {
    let mut tx = pool.begin().await?;
    lock_corpus(&mut tx, corpus).await?;
    let tracks = corpus_tracks(&mut tx, corpus).await?;
    let groups = groups(&mut tx, corpus, &tracks).await?;
    let (tracks, plan) = tokio::task::spawn_blocking(move || {
        let plan = plan_matches(&tracks, &groups);
        (tracks, plan)
    })
    .await?;
    let plan = plan?;
    for person in &plan.new_people {
        create_person(&mut tx, corpus, *person, "automatic-inferred-voice").await?;
    }
    let source = format!(
        "{ALGORITHM}@{}",
        crate::provenance::PROCESSOR_SOURCE_REVISION
    );
    for assignment in plan.assignments {
        let cluster = cluster_for(&mut tx, corpus, assignment.person, &source).await?;
        assign(
            &mut tx,
            cluster,
            assignment.track,
            Some(f64::from(assignment.score)),
            &source,
        )
        .await?;
        sqlx::query("UPDATE people SET identity_confidence=least(coalesce(identity_confidence,$2),$2),updated_at=now() WHERE id=$1")
            .bind(assignment.person).bind(f64::from(assignment.score).clamp(0.0, 1.0)).execute(&mut *tx).await?;
    }
    // Once a current run's track joins a person, drop that person's automatic members from
    // older runs of the same video so one utterance is not counted twice.
    sqlx::query("DELETE FROM voice_cluster_members old USING voice_clusters c,voice_tracks previous
        WHERE old.cluster_id=c.id AND previous.id=old.track_id AND c.corpus_id=$1 AND c.algorithm=$2
        AND NOT EXISTS(SELECT 1 FROM voice_match_decisions d WHERE d.corpus_id=$1 AND d.track_id=previous.id)
        AND EXISTS(SELECT 1 FROM voice_cluster_members fresh JOIN voice_tracks current ON current.id=fresh.track_id
            WHERE fresh.cluster_id=old.cluster_id AND current.video_id=previous.video_id AND current.run_id<>previous.run_id
            AND current.run_id=(SELECT r.id FROM media_processing_runs r WHERE r.video_id=previous.video_id
                AND r.modality='voice' AND r.processor=$3 AND r.processing_config->>'complete'='true'
                ORDER BY r.updated_at DESC,r.id DESC LIMIT 1))")
        .bind(corpus).bind(ALGORITHM).bind(VOICE_PROCESSOR).execute(&mut *tx).await?;
    suggest_face_links(&mut tx, corpus, &tracks).await?;
    let review = review_with(&mut tx, corpus).await?;
    tx.commit().await?;
    Ok(review)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCorrectionAction {
    Unlink,
    Split,
    Link,
    Confirm,
}
impl VoiceCorrectionAction {
    fn name(self) -> &'static str {
        match self {
            Self::Unlink => "unlink",
            Self::Split => "split",
            Self::Link => "link",
            Self::Confirm => "confirm",
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct VoiceCorrectionInputError(String);
fn input_error(message: &str) -> anyhow::Error {
    VoiceCorrectionInputError(message.into()).into()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceCorrectionRequest {
    pub action: VoiceCorrectionAction,
    pub track_ids: Vec<Uuid>,
    pub person_id: Option<Uuid>,
}

pub async fn correct_voice_matches(
    pool: &PgPool,
    corpus: Uuid,
    request: VoiceCorrectionRequest,
) -> anyhow::Result<VoiceMatchReview> {
    let selected: BTreeSet<_> = request.track_ids.iter().copied().collect();
    if selected.is_empty() {
        return Err(input_error("select at least one voice track"));
    }
    let mut tx = pool.begin().await?;
    lock_corpus(&mut tx, corpus).await?;
    let tracks = corpus_tracks(&mut tx, corpus).await?;
    anyhow::ensure!(
        selected
            .iter()
            .all(|id| tracks.iter().any(|t| t.track_id == *id)),
        "voice track not found in corpus"
    );
    let previous = groups(&mut tx, corpus, &tracks).await?;
    let in_group = |id: &Uuid| {
        previous
            .iter()
            .any(|g| g.tracks.iter().any(|t| t.track_id == *id))
    };
    if matches!(request.action, VoiceCorrectionAction::Unlink) && !selected.iter().all(in_group) {
        return Err(input_error(
            "only currently assigned voice tracks can be unlinked",
        ));
    }
    let person = match request.action {
        VoiceCorrectionAction::Unlink => None,
        VoiceCorrectionAction::Split => {
            let id = Uuid::new_v4();
            create_person(&mut tx, corpus, id, "manual-inferred-voice").await?;
            Some(id)
        }
        VoiceCorrectionAction::Link | VoiceCorrectionAction::Confirm => Some(
            request
                .person_id
                .ok_or_else(|| input_error("choose a corpus person"))?,
        ),
    };
    if let VoiceCorrectionAction::Confirm = request.action {
        let group = previous
            .iter()
            .find(|g| Some(g.person_id) == person)
            .context("person not found in corpus")?;
        let members: BTreeSet<_> = group.tracks.iter().map(|t| t.track_id).collect();
        if selected != members {
            return Err(input_error(
                "confirmation must include the person's current voice tracks",
            ));
        }
    } else {
        // Moving tracks out of a person keeps them apart from the members left behind.
        for group in &previous {
            let (removed, retained): (Vec<_>, Vec<_>) = group
                .tracks
                .iter()
                .map(|t| t.track_id)
                .partition(|id| selected.contains(id));
            for a in &removed {
                for b in &retained {
                    sqlx::query("INSERT INTO voice_match_rejections(corpus_id,track_a,track_b) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                        .bind(corpus).bind(a.min(b)).bind(a.max(b)).execute(&mut *tx).await?;
                }
            }
        }
    }
    if let Some(person) = person {
        // An explicit link supersedes earlier rejections between the selection and its target.
        let target: Vec<Uuid> = previous
            .iter()
            .filter(|g| g.person_id == person)
            .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
            .chain(selected.iter().copied())
            .collect();
        sqlx::query("DELETE FROM voice_match_rejections WHERE corpus_id=$1 AND track_a=ANY($2) AND track_b=ANY($2) AND (track_a=ANY($3) OR track_b=ANY($3))")
            .bind(corpus).bind(&target).bind(selected.iter().copied().collect::<Vec<_>>()).execute(&mut *tx).await?;
    }
    let cluster = match person {
        Some(person) => Some(
            cluster_for(
                &mut tx,
                corpus,
                person,
                "researcher-corrected voice association",
            )
            .await?,
        ),
        None => None,
    };
    for track in &selected {
        sqlx::query("DELETE FROM voice_cluster_members m USING voice_clusters c WHERE m.cluster_id=c.id AND c.corpus_id=$1 AND m.track_id=$2")
            .bind(corpus).bind(track).execute(&mut *tx).await?;
        if let Some(cluster) = cluster {
            let source = if matches!(request.action, VoiceCorrectionAction::Confirm) {
                "user-confirmed corpus association"
            } else {
                "researcher-corrected inferred association"
            };
            assign(&mut tx, cluster, *track, None, source).await?;
        }
        sqlx::query("INSERT INTO voice_match_decisions(corpus_id,track_id,person_id,decision) VALUES($1,$2,$3,$4)
            ON CONFLICT(corpus_id,track_id) DO UPDATE SET person_id=EXCLUDED.person_id,decision=EXCLUDED.decision,updated_at=now()")
            .bind(corpus).bind(track).bind(person).bind(request.action.name()).execute(&mut *tx).await?;
    }
    if matches!(request.action, VoiceCorrectionAction::Confirm) {
        sqlx::query("UPDATE people SET identity_source='user-confirmed corpus association',identity_confidence=NULL,updated_at=now() WHERE id=$1 AND corpus_id=$2")
            .bind(person).bind(corpus).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO voice_match_corrections(id,corpus_id,decision,track_ids,person_id) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(corpus).bind(request.action.name())
        .bind(selected.into_iter().collect::<Vec<_>>()).bind(person).execute(&mut *tx).await?;
    let review = review_with(&mut tx, corpus).await?;
    tx.commit().await?;
    Ok(review)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceLinkAction {
    /// Merge the voice person into the face person.
    Confirm,
    /// Keep the two people apart across reruns.
    Reject,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceLinkRequest {
    pub action: FaceLinkAction,
    pub face_person_id: Uuid,
    pub voice_person_id: Uuid,
}

pub async fn review_face_voice_link(
    pool: &PgPool,
    corpus: Uuid,
    request: FaceLinkRequest,
) -> anyhow::Result<VoiceMatchReview> {
    let mut tx = pool.begin().await?;
    lock_corpus(&mut tx, corpus).await?;
    let suggested: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM person_face_voice_links WHERE corpus_id=$1 AND face_person_id=$2
        AND voice_person_id=$3 AND status='unverified')",
    )
    .bind(corpus)
    .bind(request.face_person_id)
    .bind(request.voice_person_id)
    .fetch_one(&mut *tx)
    .await?;
    if !suggested {
        return Err(input_error(
            "no open face-to-voice suggestion for these people",
        ));
    }
    let (decision, (face, voice)) = (
        match request.action {
            FaceLinkAction::Confirm => "confirm_face_link",
            FaceLinkAction::Reject => "reject_face_link",
        },
        (request.face_person_id, request.voice_person_id),
    );
    match request.action {
        FaceLinkAction::Reject => {
            sqlx::query("UPDATE person_face_voice_links SET status='rejected',updated_at=now() WHERE face_person_id=$1 AND voice_person_id=$2")
                .bind(face).bind(voice).execute(&mut *tx).await?;
        }
        FaceLinkAction::Confirm => {
            // The voice person's clusters and decisions move to the face person; the now-empty
            // voice person is removed, so the corpus has one person with face and voice.
            sqlx::query(
                "UPDATE person_voice_clusters SET person_id=$1,linked_at=now() WHERE person_id=$2",
            )
            .bind(face)
            .bind(voice)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE voice_match_decisions SET person_id=$1,updated_at=now() WHERE person_id=$2",
            )
            .bind(face)
            .bind(voice)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM people WHERE id=$1 AND corpus_id=$2")
                .bind(voice)
                .bind(corpus)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("INSERT INTO voice_match_corrections(id,corpus_id,decision,face_person_id,voice_person_id) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(corpus).bind(decision).bind(face).bind(voice).execute(&mut *tx).await?;
    let review = review_with(&mut tx, corpus).await?;
    tx.commit().await?;
    Ok(review)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_counts_each_spoken_second_once() {
        let visible = union(vec![(0.0, 4.0), (2.0, 6.0), (10.0, 12.0)]);
        assert_eq!(visible, vec![(0.0, 6.0), (10.0, 12.0)]);
        assert_eq!(overlap(&[(5.0, 11.0)], &visible), 2.0);
    }
}
