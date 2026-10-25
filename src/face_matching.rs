//! Reversible corpus-local identity associations; visual producers own biometric extraction.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

const ALGORITHM: &str = "youtube-corpus/face-complete-link-v1";
const MIN_SIMILARITY: f32 = 0.75;
const MIN_MARGIN: f32 = 0.10;

fn configuration() -> Value {
    json!({"algorithm":ALGORITHM,"processorRevision":crate::provenance::PROCESSOR_SOURCE_REVISION,
        "minCosineSimilarity":MIN_SIMILARITY,"minAlternativeMargin":MIN_MARGIN,
        "minObservations":2,"minDetectionScore":0.9,"sameRunConcurrentTracks":"incompatible"})
}
fn config_hash() -> String {
    Sha256::digest(configuration().to_string().as_bytes())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceEvidence {
    pub track_id: Uuid,
    pub video_id: Uuid,
    pub title: Option<String>,
    pub source_url: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub observations: i64,
    pub detection_score: Option<f64>,
    pub processor: String,
    pub processor_version: String,
    pub model: String,
    pub model_version: String,
    pub input_hash: String,
    pub producer_config: Value,
    pub similarity: Option<f64>,
    pub assignment_source: Option<String>,
    pub matching_algorithm: Option<String>,
    pub matching_version: Option<String>,
    pub matching_configuration: Option<Value>,
    pub protected: bool,
    pub reason: Option<String>,
    #[serde(skip)]
    run_id: Uuid,
    #[serde(skip)]
    embedding: Vec<f32>,
    #[serde(skip)]
    current: bool,
    #[serde(skip)]
    placement_fixed: bool,
    #[serde(skip)]
    rejected: BTreeSet<Uuid>,
}
impl FaceEvidence {
    fn supported(&self) -> bool {
        self.processor == crate::face_processing::FACE_PROCESSOR
            && self.model == crate::face_processing::FACE_MODEL
            && self.model_version == crate::face_processing::FACE_MODEL_VERSION
            && self.embedding.len() == 128
            && self.embedding.iter().all(|v| v.is_finite())
    }
    fn strong(&self) -> bool {
        self.supported() && self.observations >= 2 && self.detection_score.is_some_and(|v| v >= 0.9)
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacePersonGroup {
    pub person_id: Uuid,
    pub label: String,
    pub identity_source: Option<String>,
    pub tracks: Vec<FaceEvidence>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceMatchReview {
    pub configuration: Value,
    pub groups: Vec<FacePersonGroup>,
    pub unresolved: Vec<FaceEvidence>,
}

async fn corpus_tracks(
    connection: &mut PgConnection,
    corpus: Uuid,
) -> anyhow::Result<Vec<FaceEvidence>> {
    let ids: Vec<Uuid> = crate::corpora::corpus_video_ids_with(&mut *connection, corpus)
        .await?
        .into_iter()
        .collect();
    let rows=sqlx::query("SELECT t.*, v.title, v.source_url, r.processor, r.processor_version,
        r.model,r.model_version,r.input_hash,r.processing_config,
        (SELECT count(*) FROM face_track_observations l WHERE l.track_id=t.id) observations,
        (SELECT max(o.detection_score) FROM face_track_observations l JOIN face_observations o ON o.id=l.observation_id WHERE l.track_id=t.id) detection_score,
        t.representative_embedding::text vector,
        r.id=(SELECT x.id FROM media_processing_runs x WHERE x.video_id=t.video_id AND x.modality='face'
            AND x.processor=$2 AND x.processing_config->>'complete'='true' ORDER BY x.updated_at DESC,x.id DESC LIMIT 1) current,
        EXISTS(SELECT 1 FROM face_match_decisions d WHERE d.corpus_id=$3 AND d.track_id=t.id) protected,
        EXISTS(SELECT 1 FROM face_match_decisions d WHERE d.corpus_id=$3 AND d.track_id=t.id AND d.decision<>'unlink') placement_fixed,
        ARRAY(SELECT CASE WHEN rejected.track_a=t.id THEN rejected.track_b ELSE rejected.track_a END
            FROM face_match_rejections rejected WHERE rejected.corpus_id=$3 AND (rejected.track_a=t.id OR rejected.track_b=t.id)) rejected
        FROM face_tracks t JOIN media_processing_runs r ON r.id=t.run_id JOIN videos v ON v.id=t.video_id
        WHERE t.video_id=ANY($1) ORDER BY t.video_id,t.start_seconds,t.id")
        .bind(&ids).bind(crate::face_processing::FACE_PROCESSOR).bind(corpus).fetch_all(&mut *connection).await?;
    rows.into_iter()
        .map(|r| {
            Ok(FaceEvidence {
                track_id: r.try_get("id")?,
                video_id: r.try_get("video_id")?,
                title: r.try_get("title")?,
                source_url: r.try_get("source_url")?,
                start_seconds: r.try_get("start_seconds")?,
                end_seconds: r.try_get("end_seconds")?,
                observations: r.try_get("observations")?,
                detection_score: r.try_get("detection_score")?,
                processor: r.try_get("processor")?,
                processor_version: r.try_get("processor_version")?,
                model: r.try_get("model")?,
                model_version: r.try_get("model_version")?,
                input_hash: r.try_get("input_hash")?,
                producer_config: r.try_get("processing_config")?,
                similarity: None,
                assignment_source: None,
                matching_algorithm: None,
                matching_version: None,
                matching_configuration: None,
                protected: r.try_get("protected")?,
                reason: None,
                run_id: r.try_get("run_id")?,
                embedding: r
                    .try_get::<Option<String>, _>("vector")?
                    .map(|v| serde_json::from_str(&v))
                    .transpose()?
                    .unwrap_or_default(),
                current: r.try_get::<Option<bool>, _>("current")?.unwrap_or(false),
                placement_fixed: r.try_get("placement_fixed")?,
                rejected: r.try_get::<Vec<Uuid>, _>("rejected")?.into_iter().collect(),
            })
        })
        .collect()
}

async fn groups(
    connection: &mut PgConnection,
    corpus: Uuid,
    tracks: &[FaceEvidence],
) -> anyhow::Result<Vec<FacePersonGroup>> {
    let rows = sqlx::query(
        "SELECT p.id,p.label,p.identity_source,m.track_id,m.similarity,m.assignment_algorithm,c.algorithm,c.algorithm_version,config.configuration
        FROM people p JOIN person_face_clusters pc ON pc.person_id=p.id
        JOIN face_clusters c ON c.id=pc.cluster_id AND c.corpus_id=p.corpus_id
        LEFT JOIN face_cluster_members m ON m.cluster_id=c.id
        LEFT JOIN face_match_evidence edge ON edge.cluster_id=m.cluster_id AND edge.track_id=m.track_id
        LEFT JOIN face_match_configurations config ON config.config_hash=coalesce(edge.config_hash,c.config_hash)
        WHERE p.corpus_id=$1 ORDER BY p.created_at,p.id,m.track_id",
    )
    .bind(corpus)
    .fetch_all(&mut *connection)
    .await?;
    let mut result: BTreeMap<Uuid, FacePersonGroup> = BTreeMap::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let group = result.entry(id).or_insert(FacePersonGroup {
            person_id: id,
            label: row
                .try_get::<Option<String>, _>("label")?
                .unwrap_or_else(|| format!("Inferred person {}", &id.simple().to_string()[..8])),
            identity_source: row.try_get("identity_source")?,
            tracks: Vec::new(),
        });
        let Some(track_id) = row.try_get::<Option<Uuid>, _>("track_id")? else {
            continue;
        };
        let Some(track) = tracks.iter().find(|t| t.track_id == track_id) else {
            continue;
        };
        if group.tracks.iter().any(|t| t.track_id == track_id) {
            continue;
        }
        let mut track = track.clone();
        track.similarity = row.try_get("similarity")?;
        track.assignment_source = row.try_get("assignment_algorithm")?;
        track.matching_configuration = row.try_get("configuration")?;
        track.matching_algorithm = Some(
            track
                .matching_configuration
                .as_ref()
                .and_then(|v| v["algorithm"].as_str())
                .map(str::to_owned)
                .unwrap_or(row.try_get("algorithm")?),
        );
        track.matching_version = Some(
            track
                .matching_configuration
                .as_ref()
                .and_then(|v| v["processorRevision"].as_str())
                .map(str::to_owned)
                .unwrap_or(row.try_get("algorithm_version")?),
        );
        group.tracks.push(track);
    }
    Ok(result.into_values().collect())
}

pub async fn review_face_matches(pool: &PgPool, corpus: Uuid) -> anyhow::Result<FaceMatchReview> {
    let mut connection = pool.acquire().await?;
    let tracks = corpus_tracks(&mut connection, corpus).await?;
    let groups = groups(&mut connection, corpus, &tracks).await?;
    let assigned: BTreeSet<Uuid> = groups
        .iter()
        .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
        .collect();
    let unresolved = tracks
        .into_iter()
        .filter(|t| t.current && !assigned.contains(&t.track_id))
        .map(|mut t| {
            t.reason = Some(
                if t.protected {
                    "Prior association rejected by a researcher; no different high-confidence match"
                } else if !t.supported() {
                    "Unsupported or incompatible producer/model evidence"
                } else if !t.strong() {
                    "Insufficient sampled face evidence for an automatic identity link"
                } else {
                    "No unique high-confidence match in another video"
                }
                .into(),
            );
            t
        })
        .collect();
    Ok(FaceMatchReview {
        configuration: configuration(),
        groups,
        unresolved,
    })
}

fn similarity(a: &FaceEvidence, b: &FaceEvidence) -> Option<f32> {
    if !a.strong()
        || !b.strong()
        || a.rejected.contains(&b.track_id)
        || b.rejected.contains(&a.track_id)
        || a.model != b.model
        || a.model_version != b.model_version
        || (a.video_id == b.video_id
            && a.run_id == b.run_id
            && a.start_seconds <= b.end_seconds
            && b.start_seconds <= a.end_seconds)
    {
        return None;
    }
    vector_analysis_core::cosine_similarity(&a.embedding, &b.embedding).ok()
}
fn group_score(track: &FaceEvidence, group: &FacePersonGroup) -> Option<f32> {
    if !group.tracks.iter().any(|t| t.video_id != track.video_id) {
        return None;
    }
    group
        .tracks
        .iter()
        .map(|t| similarity(track, t))
        .try_fold(1.0_f32, |score, next| next.map(|v| score.min(v)))
}
fn ranked_groups(track: &FaceEvidence, groups: &[FacePersonGroup]) -> Vec<(usize, f32)> {
    let mut scores: Vec<_> = groups
        .iter()
        .enumerate()
        .filter_map(|(i, g)| group_score(track, g).map(|s| (i, s)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores
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
fn plan_matches(tracks: Vec<FaceEvidence>, mut groups: Vec<FacePersonGroup>) -> MatchPlan {
    let mut assigned: BTreeSet<_> = groups
        .iter()
        .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
        .collect();
    let eligible: Vec<_> = tracks
        .into_iter()
        .filter(|t| t.current && !t.placement_fixed && t.strong())
        .collect();
    let mut plan = MatchPlan {
        new_people: Vec::new(),
        assignments: Vec::new(),
    };
    // Extend existing people only with a unique, well-separated best match.
    let mut pending = Vec::new();
    for track in &eligible {
        if assigned.contains(&track.track_id) {
            continue;
        }
        let scores = ranked_groups(track, &groups);
        match scores.first() {
            Some(&(index, score)) if score >= MIN_SIMILARITY => {
                if scores
                    .get(1)
                    .is_some_and(|(_, other)| score - other < MIN_MARGIN)
                {
                    continue; // competing existing identities: unresolved
                }
                plan.assignments.push(Assignment {
                    track: track.track_id,
                    person: groups[index].person_id,
                    score,
                });
                groups[index].tracks.push(track.clone());
                assigned.insert(track.track_id);
            }
            _ => pending.push(track),
        }
    }
    for cluster in agglomerate(&pending, &groups) {
        let videos: BTreeSet<_> = cluster.iter().map(|&i| pending[i].video_id).collect();
        if videos.len() < 2 {
            continue;
        }
        let mut minimum = 1.0_f32;
        for (n, &left) in cluster.iter().enumerate() {
            for &right in &cluster[n + 1..] {
                if let Some(score) = similarity(pending[left], pending[right]) {
                    minimum = minimum.min(score);
                }
            }
        }
        let person = Uuid::new_v4();
        plan.new_people.push(person);
        for &index in &cluster {
            plan.assignments.push(Assignment {
                track: pending[index].track_id,
                person,
                score: minimum,
            });
        }
        groups.push(FacePersonGroup {
            person_id: person,
            label: String::new(),
            identity_source: None,
            tracks: cluster.iter().map(|&i| pending[i].clone()).collect(),
        });
    }
    plan
}

/// Complete-link agglomeration that only merges a pair when neither side has a competing
/// candidate (another cluster or an existing person) within `MIN_MARGIN`. Ambiguous or
/// contradictory evidence stays unmerged without blocking clearly separated matches elsewhere,
/// and a chain of pairwise similarities never joins incompatible endpoints.
fn agglomerate(tracks: &[&FaceEvidence], groups: &[FacePersonGroup]) -> Vec<Vec<usize>> {
    let floor = MIN_SIMILARITY - MIN_MARGIN;
    // Sparse complete-link scores between active clusters, keyed by (smaller, larger) id.
    let mut near: BTreeMap<(usize, usize), f32> = BTreeMap::new();
    for i in 0..tracks.len() {
        for j in i + 1..tracks.len() {
            if let Some(score) = similarity(tracks[i], tracks[j]).filter(|s| *s >= floor) {
                near.insert((i, j), score);
            }
        }
    }
    let mut members: Vec<Vec<usize>> = (0..tracks.len()).map(|i| vec![i]).collect();
    let existing = |cluster: &[usize]| -> f32 {
        groups
            .iter()
            .filter_map(|group| {
                cluster
                    .iter()
                    .map(|&m| group_score(tracks[m], group))
                    .try_fold(1.0_f32, |score, next| next.map(|v| score.min(v)))
            })
            .fold(f32::NEG_INFINITY, f32::max)
    };
    loop {
        let mut candidates: Vec<_> = near
            .iter()
            .filter(|(_, s)| **s >= MIN_SIMILARITY)
            .map(|(&(a, b), &s)| (s, a, b))
            .collect();
        candidates.sort_by(|x, y| {
            y.0.total_cmp(&x.0).then_with(|| {
                (tracks[x.1].track_id, tracks[x.2].track_id)
                    .cmp(&(tracks[y.1].track_id, tracks[y.2].track_id))
            })
        });
        // A third cluster competes with a merge when it is nearly as close to one side but cannot
        // consistently join both; one that fits both is simply a further member of the person.
        let key = |x: usize, y: usize| (x.min(y), x.max(y));
        let competitor = |cluster: usize, partner: usize| -> f32 {
            near.iter()
                .filter_map(|(&(x, y), &s)| {
                    let other = match (x == cluster, y == cluster) {
                        (true, _) => y,
                        (_, true) => x,
                        _ => return None,
                    };
                    let joins_both = near
                        .get(&key(partner, other))
                        .is_some_and(|p| p.min(s) >= MIN_SIMILARITY);
                    (other != partner && !joins_both).then_some(s)
                })
                .fold(existing(&members[cluster]), f32::max)
        };
        let Some(&(_, a, b)) = candidates.iter().find(|(score, a, b)| {
            competitor(*a, *b) < score - MIN_MARGIN && competitor(*b, *a) < score - MIN_MARGIN
        }) else {
            break;
        };
        // Merge b into a; the merged link to c is the minimum, kept only if both existed.
        let others: BTreeSet<usize> = near
            .keys()
            .flat_map(|&(x, y)| [x, y])
            .filter(|&c| c != a && c != b)
            .collect();
        for c in others {
            let merged = near
                .get(&key(a, c))
                .zip(near.get(&key(b, c)))
                .map(|(x, y)| x.min(*y));
            near.remove(&key(b, c));
            match merged {
                Some(score) => {
                    near.insert(key(a, c), score);
                }
                None => {
                    near.remove(&key(a, c));
                }
            }
        }
        near.remove(&key(a, b));
        let moved = std::mem::take(&mut members[b]);
        members[a].extend(moved);
    }
    members.into_iter().filter(|m| m.len() > 1).collect()
}

async fn lock_corpus(tx: &mut Transaction<'_, Postgres>, corpus: Uuid) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("face-matches:{corpus}"))
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
    let existing:Option<Uuid>=sqlx::query_scalar("SELECT c.id FROM face_clusters c JOIN person_face_clusters pc ON pc.cluster_id=c.id WHERE c.corpus_id=$1 AND pc.person_id=$2 AND c.algorithm=$3 LIMIT 1")
        .bind(corpus).bind(person).bind(ALGORITHM).fetch_optional(&mut **tx).await?;
    if let Some(id) = existing {
        return Ok(id);
    }
    sqlx::query("INSERT INTO face_match_configurations(config_hash,configuration) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(config_hash()).bind(configuration()).execute(&mut **tx).await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO face_clusters(id,corpus_id,algorithm,algorithm_version,config_hash) VALUES($1,$2,$3,$4,$5)")
        .bind(id).bind(corpus).bind(ALGORITHM).bind(crate::provenance::PROCESSOR_SOURCE_REVISION).bind(config_hash()).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO person_face_clusters(person_id,cluster_id,source) VALUES($1,$2,$3)")
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
    sqlx::query("INSERT INTO face_cluster_members(cluster_id,track_id,similarity,assignment_algorithm) VALUES($1,$2,$3,$4)
        ON CONFLICT(cluster_id,track_id) DO UPDATE SET similarity=EXCLUDED.similarity,assignment_algorithm=EXCLUDED.assignment_algorithm,assigned_at=now()")
        .bind(cluster).bind(track).bind(score).bind(source).execute(&mut **tx).await?;
    let settings = if score.is_some() {
        configuration()
    } else {
        json!({"algorithm":"researcher-corrected association","processorRevision":crate::provenance::PROCESSOR_SOURCE_REVISION,"source":source})
    };
    let hash: String = Sha256::digest(settings.to_string().as_bytes())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect();
    sqlx::query("INSERT INTO face_match_configurations(config_hash,configuration) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(&hash).bind(settings).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO face_match_evidence(cluster_id,track_id,config_hash) VALUES($1,$2,$3) ON CONFLICT(cluster_id,track_id) DO UPDATE SET config_hash=EXCLUDED.config_hash")
        .bind(cluster).bind(track).bind(hash).execute(&mut **tx).await?;
    Ok(())
}

pub async fn match_corpus_faces(pool: &PgPool, corpus: Uuid) -> anyhow::Result<FaceMatchReview> {
    let mut tx = pool.begin().await?;
    lock_corpus(&mut tx, corpus).await?;
    let tracks = corpus_tracks(&mut tx, corpus).await?;
    let groups = groups(&mut tx, corpus, &tracks).await?;
    let plan = tokio::task::spawn_blocking(move || plan_matches(tracks, groups)).await?;
    for person in plan.new_people {
        create_person(&mut tx, corpus, person, "automatic-inferred-face").await?;
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
            .bind(assignment.person).bind(f64::from(assignment.score)).execute(&mut *tx).await?;
    }
    // Once a compatible current run replaces an automatic association, retain
    // its raw older evidence without counting two versions of the same event.
    sqlx::query("DELETE FROM face_cluster_members old USING face_clusters c,face_tracks previous
        WHERE old.cluster_id=c.id AND previous.id=old.track_id AND c.corpus_id=$1 AND c.algorithm=$2
        AND NOT EXISTS(SELECT 1 FROM face_match_decisions d WHERE d.corpus_id=$1 AND d.track_id=previous.id)
        AND EXISTS(SELECT 1 FROM face_cluster_members fresh JOIN face_tracks current ON current.id=fresh.track_id
            WHERE fresh.cluster_id=old.cluster_id AND current.video_id=previous.video_id AND current.run_id<>previous.run_id
            AND current.run_id=(SELECT r.id FROM media_processing_runs r WHERE r.video_id=previous.video_id
                AND r.modality='face' AND r.processor=$3 AND r.processing_config->>'complete'='true'
                ORDER BY r.updated_at DESC,r.id DESC LIMIT 1))")
        .bind(corpus).bind(ALGORITHM).bind(crate::face_processing::FACE_PROCESSOR).execute(&mut *tx).await?;
    tx.commit().await?;
    review_face_matches(pool, corpus).await
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceCorrectionAction {
    Unlink,
    Split,
    Link,
    Confirm,
}
impl FaceCorrectionAction {
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
pub struct CorrectionInputError(String);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceCorrectionRequest {
    pub action: FaceCorrectionAction,
    pub track_ids: Vec<Uuid>,
    pub person_id: Option<Uuid>,
}

pub async fn correct_face_matches(
    pool: &PgPool,
    corpus: Uuid,
    request: FaceCorrectionRequest,
) -> anyhow::Result<FaceMatchReview> {
    let selected: BTreeSet<_> = request.track_ids.iter().copied().collect();
    if selected.is_empty() {
        return Err(CorrectionInputError("select at least one face track".into()).into());
    }
    let mut tx = pool.begin().await?;
    lock_corpus(&mut tx, corpus).await?;
    let tracks = corpus_tracks(&mut tx, corpus).await?;
    anyhow::ensure!(
        selected
            .iter()
            .all(|id| tracks.iter().any(|t| t.track_id == *id)),
        "face track not found in corpus"
    );
    let previous_groups = groups(&mut tx, corpus, &tracks).await?;
    if matches!(request.action, FaceCorrectionAction::Unlink)
        && !selected.iter().all(|id| {
            previous_groups
                .iter()
                .any(|g| g.tracks.iter().any(|t| t.track_id == *id))
        })
    {
        return Err(CorrectionInputError(
            "only currently assigned face tracks can be unlinked".into(),
        )
        .into());
    }
    if !matches!(request.action, FaceCorrectionAction::Confirm) {
        for group in &previous_groups {
            for removed in group
                .tracks
                .iter()
                .filter(|t| selected.contains(&t.track_id))
            {
                for retained in group
                    .tracks
                    .iter()
                    .filter(|t| !selected.contains(&t.track_id))
                {
                    let (left, right) = if removed.track_id < retained.track_id {
                        (removed.track_id, retained.track_id)
                    } else {
                        (retained.track_id, removed.track_id)
                    };
                    sqlx::query("INSERT INTO face_match_rejections(corpus_id,track_a,track_b) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                        .bind(corpus).bind(left).bind(right).execute(&mut *tx).await?;
                }
            }
        }
    }
    let person = match request.action {
        FaceCorrectionAction::Unlink => None,
        FaceCorrectionAction::Split => {
            let id = Uuid::new_v4();
            create_person(&mut tx, corpus, id, "manual-inferred-face").await?;
            Some(id)
        }
        FaceCorrectionAction::Link | FaceCorrectionAction::Confirm => Some(
            request
                .person_id
                .ok_or_else(|| CorrectionInputError("choose a corpus person".into()))?,
        ),
    };
    if let FaceCorrectionAction::Confirm = request.action {
        let current = groups(&mut tx, corpus, &tracks).await?;
        let group = current
            .iter()
            .find(|g| Some(g.person_id) == person)
            .context("person not found in corpus")?;
        let members: BTreeSet<_> = group.tracks.iter().map(|t| t.track_id).collect();
        if selected != members {
            return Err(CorrectionInputError(
                "confirmation must include the person's current face tracks".into(),
            )
            .into());
        }
    }
    // An explicit researcher link supersedes rejection of those selected/target
    // pairs, while preserving unrelated negative corrections.
    if matches!(
        request.action,
        FaceCorrectionAction::Link | FaceCorrectionAction::Confirm | FaceCorrectionAction::Split
    ) {
        let target_members: BTreeSet<Uuid> = previous_groups
            .iter()
            .filter(|g| Some(g.person_id) == person)
            .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
            .chain(selected.iter().copied())
            .collect();
        let ids: Vec<Uuid> = target_members.into_iter().collect();
        sqlx::query("DELETE FROM face_match_rejections WHERE corpus_id=$1 AND track_a=ANY($2) AND track_b=ANY($2) AND (track_a=ANY($3) OR track_b=ANY($3))")
            .bind(corpus).bind(&ids).bind(selected.iter().copied().collect::<Vec<_>>()).execute(&mut *tx).await?;
    }
    let cluster = if let Some(person) = person {
        Some(
            cluster_for(
                &mut tx,
                corpus,
                person,
                "researcher-corrected face association",
            )
            .await?,
        )
    } else {
        None
    };
    for track in &selected {
        sqlx::query("DELETE FROM face_cluster_members m USING face_clusters c WHERE m.cluster_id=c.id AND c.corpus_id=$1 AND m.track_id=$2")
            .bind(corpus).bind(track).execute(&mut *tx).await?;
        if let Some(cluster) = cluster {
            assign(
                &mut tx,
                cluster,
                *track,
                None,
                if matches!(request.action, FaceCorrectionAction::Confirm) {
                    "user-confirmed corpus association"
                } else {
                    "researcher-corrected inferred association"
                },
            )
            .await?;
        }
        sqlx::query("INSERT INTO face_match_decisions(corpus_id,track_id,person_id,decision) VALUES($1,$2,$3,$4)
            ON CONFLICT(corpus_id,track_id) DO UPDATE SET person_id=EXCLUDED.person_id,decision=EXCLUDED.decision,updated_at=now()")
            .bind(corpus).bind(track).bind(person).bind(request.action.name()).execute(&mut *tx).await?;
    }
    if matches!(request.action, FaceCorrectionAction::Confirm) {
        sqlx::query("UPDATE people SET identity_source='user-confirmed corpus association',identity_confidence=NULL,updated_at=now() WHERE id=$1 AND corpus_id=$2")
            .bind(person).bind(corpus).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO face_match_corrections(id,corpus_id,decision,track_ids,person_id) VALUES($1,$2,$3,$4,$5)")
        .bind(Uuid::new_v4()).bind(corpus).bind(request.action.name()).bind(selected.into_iter().collect::<Vec<_>>()).bind(person).execute(&mut *tx).await?;
    tx.commit().await?;
    review_face_matches(pool, corpus).await
}
