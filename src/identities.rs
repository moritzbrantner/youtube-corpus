//! Global canonical people and entities over corpus-local evidence (Atlas 5.2, 5.3; #104).
//!
//! A *local unit* is an existing corpus-local evidence group: the named-entity mentions of one
//! `(entity_type, entity_key)` inside one corpus, or one inferred corpus-local person (#64/#65).
//! Every unit belongs to exactly one canonical identity. Resolution starts each unit in its own
//! identity and joins units automatically only on shared source evidence:
//!
//! - entity groups of the same type and key in different corpora that share a mention (the same
//!   video is a member of both corpora);
//! - inferred people in different corpora whose clusters contain the same face or voice track;
//! - an inferred person and the person-mention group its evidence-grounded name cites (#65).
//!
//! Same names alone only produce reviewable suggestions. Researcher decisions are durable: merged
//! or split units are `confirmed` and never moved by resolution, and kept-apart unit pairs block
//! automatic links until the researcher clears them. Local mentions, tracks, clusters, names and
//! their provenance are never modified here.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::entities::{EntityOccurrencesReport, PersonOccurrencesReport};

/// Mention types that describe values, not referents, and get no canonical identity.
pub const UNRESOLVED_ENTITY_TYPES: [&str; 2] = ["date", "amount"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Person,
    Entity,
}

impl IdentityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Entity => "entity",
        }
    }

    fn for_entity_type(entity_type: &str) -> Self {
        if entity_type == "person" {
            Self::Person
        } else {
            Self::Entity
        }
    }
}

pub fn entity_unit_key(corpus: Uuid, entity_type: &str, entity_key: &str) -> String {
    format!("entity:{corpus}:{entity_type}:{entity_key}")
}

pub fn person_unit_key(person: Uuid) -> String {
    format!("person:{person}")
}

struct Unit {
    corpus: Uuid,
    person: Option<Uuid>,
    /// `person` for inferred people.
    entity_type: String,
    entity_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionReport {
    pub units: usize,
    pub identities: usize,
    /// Units joined to another unit's identity by shared or name evidence.
    pub linked_units: usize,
}

async fn lock(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('canonical_identities'))")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Recomputes automatic links for every corpus. Idempotent; keeps researcher decisions.
pub async fn resolve_identities(pool: &PgPool) -> anyhow::Result<ResolutionReport> {
    let mut tx = pool.begin().await?;
    lock(&mut tx).await?;

    let mut units: BTreeMap<String, Unit> = BTreeMap::new();
    let mut groups: BTreeMap<(String, String), Vec<(String, HashSet<Uuid>)>> = BTreeMap::new();
    let excluded: Vec<String> = UNRESOLVED_ENTITY_TYPES.map(String::from).to_vec();
    for row in sqlx::query(
        "SELECT cv.corpus_id, m.entity_type, m.entity_key, array_agg(DISTINCT m.video_id) AS videos
         FROM corpus_videos cv JOIN entity_mentions m ON m.video_id = cv.video_id
         WHERE m.entity_type <> ALL($1)
         GROUP BY cv.corpus_id, m.entity_type, m.entity_key",
    )
    .bind(&excluded)
    .fetch_all(&mut *tx)
    .await?
    {
        let corpus: Uuid = row.try_get("corpus_id")?;
        let entity_type: String = row.try_get("entity_type")?;
        let entity_key: String = row.try_get("entity_key")?;
        let videos: Vec<Uuid> = row.try_get("videos")?;
        let key = entity_unit_key(corpus, &entity_type, &entity_key);
        groups
            .entry((entity_type.clone(), entity_key.clone()))
            .or_default()
            .push((key.clone(), videos.into_iter().collect()));
        units.insert(
            key,
            Unit {
                corpus,
                person: None,
                entity_type,
                entity_key: Some(entity_key),
            },
        );
    }
    for (person, corpus) in sqlx::query_as::<_, (Uuid, Uuid)>("SELECT id, corpus_id FROM people")
        .fetch_all(&mut *tx)
        .await?
    {
        units.insert(
            person_unit_key(person),
            Unit {
                corpus,
                person: Some(person),
                entity_type: "person".into(),
                entity_key: None,
            },
        );
    }

    let mut edges: Vec<(String, String, &'static str)> = Vec::new();
    for members in groups.values() {
        for (index, (a, a_videos)) in members.iter().enumerate() {
            for (b, b_videos) in &members[index + 1..] {
                if !a_videos.is_disjoint(b_videos) {
                    edges.push((a.clone(), b.clone(), "shared_evidence"));
                }
            }
        }
    }
    for (a, b) in sqlx::query_as::<_, (Uuid, Uuid)>(
        "WITH t AS (
           SELECT pc.person_id, 'face:' || m.track_id AS track
           FROM person_face_clusters pc JOIN face_cluster_members m ON m.cluster_id = pc.cluster_id
           UNION
           SELECT pc.person_id, 'voice:' || m.track_id
           FROM person_voice_clusters pc JOIN voice_cluster_members m ON m.cluster_id = pc.cluster_id
         )
         SELECT DISTINCT a.person_id, b.person_id
         FROM t a JOIN t b ON a.track = b.track AND a.person_id < b.person_id",
    )
    .fetch_all(&mut *tx)
    .await?
    {
        edges.push((person_unit_key(a), person_unit_key(b), "shared_evidence"));
    }
    for (person, corpus, entity_key) in sqlx::query_as::<_, (Uuid, Uuid, String)>(
        "SELECT DISTINCT a.person_id, a.corpus_id, m.entity_key
         FROM person_name_attributions a
         JOIN person_name_evidence e ON e.person_id = a.person_id
         JOIN entity_mentions m ON m.id = e.entity_mention_id
         WHERE m.entity_type = 'person'",
    )
    .fetch_all(&mut *tx)
    .await?
    {
        edges.push((
            person_unit_key(person),
            entity_unit_key(corpus, "person", &entity_key),
            "name_evidence",
        ));
    }
    edges.retain(|(a, b, _)| units.contains_key(a) && units.contains_key(b));
    edges.sort();
    edges.dedup();

    let existing: HashMap<String, (Uuid, bool)> = sqlx::query_as::<_, (String, Uuid, String)>(
        "SELECT unit_key, identity_id, status FROM canonical_identity_links",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(key, identity, status)| (key, (identity, status == "confirmed")))
    .collect();
    let rejections: HashSet<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT unit_a, unit_b FROM canonical_identity_rejections",
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let live: HashSet<Uuid> =
        sqlx::query_scalar("SELECT id FROM canonical_identities WHERE merged_into IS NULL")
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();

    let keys: Vec<&String> = units.keys().collect();
    let index: HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .map(|(i, key)| (key.as_str(), i))
        .collect();
    let confirmed: Vec<Option<Uuid>> = keys
        .iter()
        .map(|key| {
            existing
                .get(key.as_str())
                .filter(|(identity, confirmed)| *confirmed && live.contains(identity))
                .map(|(identity, _)| *identity)
        })
        .collect();
    let mut sets = Components::new(keys.len(), &confirmed);
    let mut by_confirmed: HashMap<Uuid, usize> = HashMap::new();
    for (i, identity) in confirmed.iter().enumerate() {
        if let Some(identity) = identity {
            match by_confirmed.get(identity) {
                Some(&first) => sets.union(first, i),
                None => {
                    by_confirmed.insert(*identity, i);
                }
            }
        }
    }
    let mut basis: Vec<&'static str> = vec!["local"; keys.len()];
    for (a, b, edge_basis) in &edges {
        let (a, b) = (index[a.as_str()], index[b.as_str()]);
        let (root_a, root_b) = (sets.find(a), sets.find(b));
        if root_a == root_b {
            continue;
        }
        if let (Some(x), Some(y)) = (sets.confirmed[root_a], sets.confirmed[root_b]) {
            if x != y {
                continue;
            }
        }
        let kept_apart = sets.members[root_a].iter().any(|&m| {
            sets.members[root_b]
                .iter()
                .any(|&n| rejections.contains(&pair(keys[m], keys[n])))
        });
        if kept_apart {
            continue;
        }
        sets.union(a, b);
        for unit in [a, b] {
            if basis[unit] == "local" {
                basis[unit] = edge_basis;
            }
        }
    }

    // Confirmed components keep their identity; others reuse the identity most of their units
    // already had, so profile URLs stay stable across reruns.
    let mut roots: Vec<usize> = (0..keys.len()).filter(|&i| sets.find(i) == i).collect();
    roots.sort_by_key(|&root| sets.members[root].iter().min().copied());
    let mut claimed: HashSet<Uuid> = roots.iter().filter_map(|&r| sets.confirmed[r]).collect();
    let mut assigned: Vec<Uuid> = vec![Uuid::nil(); keys.len()];
    let mut new_identities: Vec<(Uuid, String)> = Vec::new();
    for &root in &roots {
        let members = sets.members[root].clone();
        let identity = match sets.confirmed[root] {
            Some(identity) => identity,
            None => {
                let mut votes: BTreeMap<Uuid, usize> = BTreeMap::new();
                for &m in &members {
                    if let Some((identity, _)) = existing.get(keys[m].as_str()) {
                        if live.contains(identity) && !claimed.contains(identity) {
                            *votes.entry(*identity).or_default() += 1;
                        }
                    }
                }
                let reused = votes
                    .iter()
                    .max_by_key(|(identity, count)| (**count, std::cmp::Reverse(**identity)))
                    .map(|(identity, _)| *identity);
                let identity = reused.unwrap_or_else(|| {
                    let identity = Uuid::new_v4();
                    new_identities.push((identity, units[keys[members[0]]].entity_type.clone()));
                    identity
                });
                claimed.insert(identity);
                identity
            }
        };
        for m in members {
            assigned[m] = identity;
        }
    }

    for (identity, entity_type) in &new_identities {
        sqlx::query("INSERT INTO canonical_identities (id, kind, entity_type) VALUES ($1, $2, $3)")
            .bind(identity)
            .bind(IdentityKind::for_entity_type(entity_type).as_str())
            .bind(entity_type)
            .execute(&mut *tx)
            .await?;
    }

    let mut columns = LinkColumns::default();
    for (i, key) in keys.iter().enumerate() {
        if confirmed[i].is_some() {
            continue;
        }
        let unit = &units[*key];
        columns.unit_keys.push((*key).clone());
        columns.kinds.push(if unit.person.is_some() {
            "inferred_person".into()
        } else {
            "entity_group".into()
        });
        columns.corpora.push(unit.corpus);
        columns.people.push(unit.person);
        columns
            .entity_types
            .push(unit.person.is_none().then(|| unit.entity_type.clone()));
        columns.entity_keys.push(unit.entity_key.clone());
        columns.identities.push(assigned[i]);
        columns.bases.push(basis[i].to_string());
    }
    sqlx::query(
        "INSERT INTO canonical_identity_links
           (unit_key, unit_kind, corpus_id, person_id, entity_type, entity_key, identity_id, status,
            basis)
         SELECT u.*, 'auto', b
         FROM unnest($1::text[], $2::text[], $3::uuid[], $4::uuid[], $5::text[], $6::text[],
                     $7::uuid[]) AS u
         JOIN unnest($1::text[], $8::text[]) AS x(k, b) ON x.k = u.unnest
         ON CONFLICT (unit_key) DO UPDATE
           SET identity_id = excluded.identity_id, status = 'auto', basis = excluded.basis,
               updated_at = now()
         WHERE canonical_identity_links.identity_id IS DISTINCT FROM excluded.identity_id
            OR canonical_identity_links.basis IS DISTINCT FROM excluded.basis
            OR canonical_identity_links.status <> 'auto'",
    )
    .bind(&columns.unit_keys)
    .bind(&columns.kinds)
    .bind(&columns.corpora)
    .bind(&columns.people)
    .bind(&columns.entity_types)
    .bind(&columns.entity_keys)
    .bind(&columns.identities)
    .bind(&columns.bases)
    .execute(&mut *tx)
    .await?;

    let all_keys: Vec<String> = keys.iter().map(|key| (*key).clone()).collect();
    sqlx::query("DELETE FROM canonical_identity_links WHERE NOT (unit_key = ANY($1))")
        .bind(&all_keys)
        .execute(&mut *tx)
        .await?;
    delete_orphans(&mut tx).await?;
    tx.commit().await?;

    Ok(ResolutionReport {
        units: keys.len(),
        identities: roots.len(),
        linked_units: basis.iter().filter(|b| **b != "local").count(),
    })
}

/// Identities without units go; merge redirects stay while their target lives.
async fn delete_orphans(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<()> {
    loop {
        let deleted = sqlx::query(
            "DELETE FROM canonical_identities i
             WHERE i.merged_into IS NULL
               AND NOT EXISTS (SELECT 1 FROM canonical_identity_links l WHERE l.identity_id = i.id)",
        )
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if deleted == 0 {
            return Ok(());
        }
    }
}

#[derive(Default)]
struct LinkColumns {
    unit_keys: Vec<String>,
    kinds: Vec<String>,
    corpora: Vec<Uuid>,
    people: Vec<Option<Uuid>>,
    entity_types: Vec<Option<String>>,
    entity_keys: Vec<Option<String>>,
    identities: Vec<Uuid>,
    bases: Vec<String>,
}

struct Components {
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
    confirmed: Vec<Option<Uuid>>,
}

impl Components {
    fn new(size: usize, confirmed: &[Option<Uuid>]) -> Self {
        Self {
            parent: (0..size).collect(),
            members: (0..size).map(|i| vec![i]).collect(),
            confirmed: confirmed.to_vec(),
        }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        let (keep, gone) = if self.members[a].len() >= self.members[b].len() {
            (a, b)
        } else {
            (b, a)
        };
        self.parent[gone] = keep;
        let moved = std::mem::take(&mut self.members[gone]);
        self.members[keep].extend(moved);
        self.confirmed[keep] = self.confirmed[keep].or(self.confirmed[gone]);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySummary {
    pub id: Uuid,
    pub kind: IdentityKind,
    pub entity_type: String,
    pub display_name: String,
    /// Where the display name comes from: `researcher` (a confirmed or researcher-provided
    /// name), `mentions` (the most frequent mention text), `inferred_name` (an evidence-supported
    /// #65 inference) or `unnamed` (an inferred person without any name).
    pub name_source: String,
    /// Every corpus with a local unit in this identity, regardless of scope.
    pub corpora: Vec<CorpusRef>,
    pub local_units: i64,
    pub has_confirmed_links: bool,
    pub spoken_mentions: i64,
    pub visible_text_mentions: i64,
    pub shown_tracks: i64,
    pub heard_tracks: i64,
    pub video_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct IdentityFilter {
    pub kind: Option<IdentityKind>,
    pub entity_type: Option<String>,
    /// Restricts listed identities and their evidence counts to one corpus.
    pub corpus: Option<Uuid>,
    pub query: String,
    pub ids: Option<Vec<Uuid>>,
    pub limit: i64,
}

pub async fn list_identities(
    pool: &PgPool,
    filter: &IdentityFilter,
) -> anyhow::Result<Vec<IdentitySummary>> {
    let rows = sqlx::query(
        "WITH links AS (
           SELECT l.* FROM canonical_identity_links l
           JOIN canonical_identities i ON i.id = l.identity_id
           WHERE i.merged_into IS NULL
             AND ($1::text IS NULL OR i.kind = $1)
             AND ($2::text IS NULL OR i.entity_type = $2)
             AND ($3::uuid[] IS NULL OR i.id = ANY($3))
         ),
         mentions AS (
           SELECT l.identity_id, l.corpus_id, m.id AS mention_id, m.modality, m.video_id,
                  m.surface_text
           FROM links l
           JOIN corpus_videos cv ON cv.corpus_id = l.corpus_id
           JOIN entity_mentions m
             ON m.video_id = cv.video_id AND m.entity_type = l.entity_type
            AND m.entity_key = l.entity_key
           WHERE l.unit_kind = 'entity_group'
         ),
         tracks AS (
           SELECT l.identity_id, l.corpus_id, 'shown' AS kind, ft.id AS track_id, ft.video_id
           FROM links l
           JOIN person_face_clusters pc ON pc.person_id = l.person_id
           JOIN face_cluster_members fm ON fm.cluster_id = pc.cluster_id
           JOIN face_tracks ft ON ft.id = fm.track_id
           JOIN corpus_videos cv ON cv.corpus_id = l.corpus_id AND cv.video_id = ft.video_id
           UNION
           SELECT l.identity_id, l.corpus_id, 'heard', vt.id, vt.video_id
           FROM links l
           JOIN person_voice_clusters pc ON pc.person_id = l.person_id
           JOIN voice_cluster_members vm ON vm.cluster_id = pc.cluster_id
           JOIN voice_tracks vt ON vt.id = vm.track_id
           JOIN corpus_videos cv ON cv.corpus_id = l.corpus_id AND cv.video_id = vt.video_id
         ),
         names AS (
           SELECT l.identity_id, n.name, n.status
           FROM links l JOIN person_name_attributions n ON n.person_id = l.person_id
         ),
         scoped_videos AS (
           SELECT identity_id, video_id FROM mentions WHERE $4::uuid IS NULL OR corpus_id = $4
           UNION
           SELECT identity_id, video_id FROM tracks WHERE $4::uuid IS NULL OR corpus_id = $4
         ),
         summary AS (
           SELECT i.id, i.kind, i.entity_type,
             coalesce(
               (SELECT min(n.name) FROM names n
                 WHERE n.identity_id = i.id AND n.status IN ('confirmed', 'user_provided')),
               (SELECT mode() WITHIN GROUP (ORDER BY m.surface_text) FROM mentions m
                 WHERE m.identity_id = i.id),
               (SELECT min(n.name) FROM names n WHERE n.identity_id = i.id),
               (SELECT min(p.label) FROM links l JOIN people p ON p.id = l.person_id
                 WHERE l.identity_id = i.id),
               (SELECT min(l.entity_key) FROM links l WHERE l.identity_id = i.id)
             ) AS display_name,
             CASE
               WHEN EXISTS (SELECT 1 FROM names n WHERE n.identity_id = i.id
                              AND n.status IN ('confirmed', 'user_provided')) THEN 'researcher'
               WHEN EXISTS (SELECT 1 FROM mentions m WHERE m.identity_id = i.id) THEN 'mentions'
               WHEN EXISTS (SELECT 1 FROM names n WHERE n.identity_id = i.id) THEN 'inferred_name'
               ELSE 'unnamed'
             END AS name_source,
             (SELECT coalesce(jsonb_agg(DISTINCT jsonb_build_object('id', c.id, 'name', c.name)),
                              '[]'::jsonb)
                FROM links l JOIN corpora c ON c.id = l.corpus_id
               WHERE l.identity_id = i.id) AS corpora,
             (SELECT count(*) FROM links l WHERE l.identity_id = i.id) AS local_units,
             (SELECT coalesce(bool_or(l.status = 'confirmed'), false) FROM links l
               WHERE l.identity_id = i.id) AS has_confirmed,
             (SELECT count(DISTINCT m.mention_id) FROM mentions m
               WHERE m.identity_id = i.id AND m.modality = 'spoken'
                 AND ($4::uuid IS NULL OR m.corpus_id = $4)) AS spoken,
             (SELECT count(DISTINCT m.mention_id) FROM mentions m
               WHERE m.identity_id = i.id AND m.modality = 'visible_text'
                 AND ($4::uuid IS NULL OR m.corpus_id = $4)) AS visible,
             (SELECT count(DISTINCT t.track_id) FROM tracks t
               WHERE t.identity_id = i.id AND t.kind = 'shown'
                 AND ($4::uuid IS NULL OR t.corpus_id = $4)) AS shown,
             (SELECT count(DISTINCT t.track_id) FROM tracks t
               WHERE t.identity_id = i.id AND t.kind = 'heard'
                 AND ($4::uuid IS NULL OR t.corpus_id = $4)) AS heard,
             (SELECT count(DISTINCT v.video_id) FROM scoped_videos v
               WHERE v.identity_id = i.id) AS videos
           FROM canonical_identities i
           WHERE i.id IN (SELECT identity_id FROM links WHERE $4::uuid IS NULL OR corpus_id = $4)
         )
         SELECT * FROM summary
         WHERE $5 = '' OR strpos(lower(display_name), $5) > 0
         ORDER BY videos DESC, lower(display_name), id
         LIMIT $6",
    )
    .bind(filter.kind.map(IdentityKind::as_str))
    .bind(filter.entity_type.as_deref())
    .bind(filter.ids.as_deref())
    .bind(filter.corpus)
    .bind(filter.query.trim().to_lowercase())
    .bind(filter.limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let kind: String = row.try_get("kind")?;
            let corpora: serde_json::Value = row.try_get("corpora")?;
            let mut corpora: Vec<CorpusRef> = serde_json::from_value(corpora)?;
            corpora.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(IdentitySummary {
                id: row.try_get("id")?,
                kind: if kind == "person" {
                    IdentityKind::Person
                } else {
                    IdentityKind::Entity
                },
                entity_type: row.try_get("entity_type")?,
                display_name: row.try_get("display_name")?,
                name_source: row.try_get("name_source")?,
                corpora,
                local_units: row.try_get("local_units")?,
                has_confirmed_links: row.try_get("has_confirmed")?,
                spoken_mentions: row.try_get("spoken")?,
                visible_text_mentions: row.try_get("visible")?,
                shown_tracks: row.try_get("shown")?,
                heard_tracks: row.try_get("heard")?,
                video_count: row.try_get("videos")?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUnit {
    pub unit_key: String,
    /// `entity_group` or `inferred_person`.
    pub unit_kind: String,
    pub corpus_id: Uuid,
    pub corpus_name: String,
    pub label: String,
    pub entity_type: Option<String>,
    pub entity_key: Option<String>,
    pub person_id: Option<Uuid>,
    /// `inferred`, `confirmed` or `user_provided` for a named inferred person.
    pub name_status: Option<String>,
    /// `auto` or `confirmed`.
    pub status: String,
    /// `local`, `shared_evidence`, `name_evidence` or `researcher`.
    pub basis: String,
    pub in_scope: bool,
}

/// The original corpus-local evidence of one unit, exactly as the corpus entity explorer reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitEvidence {
    pub unit_key: String,
    pub corpus_id: Uuid,
    pub mentions: Option<EntityOccurrencesReport>,
    pub tracks: Option<PersonOccurrencesReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySuggestion {
    pub identity: IdentitySummary,
    /// The normalized name both identities share; a reason to review, not proof.
    pub shared_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProfile {
    /// The identity that was asked for; differs from `identity.id` after a merge.
    pub requested_id: Uuid,
    pub identity: IdentitySummary,
    pub units: Vec<LocalUnit>,
    pub evidence: Vec<UnitEvidence>,
    pub suggestions: Vec<IdentitySuggestion>,
    pub kept_apart: Vec<IdentitySummary>,
}

async fn live_identity(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Uuid>> {
    let mut current = id;
    for _ in 0..64 {
        let row: Option<Option<Uuid>> =
            sqlx::query_scalar("SELECT merged_into FROM canonical_identities WHERE id = $1")
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

pub async fn identity_profile(
    pool: &PgPool,
    requested_id: Uuid,
    corpus: Option<Uuid>,
) -> anyhow::Result<Option<IdentityProfile>> {
    let Some(id) = live_identity(pool, requested_id).await? else {
        return Ok(None);
    };
    let all = |ids: Vec<Uuid>| IdentityFilter {
        ids: Some(ids),
        limit: 200,
        ..IdentityFilter::default()
    };
    let Some(identity) = list_identities(pool, &all(vec![id])).await?.into_iter().next() else {
        return Ok(None);
    };
    let identity = match corpus {
        Some(corpus) => list_identities(
            pool,
            &IdentityFilter {
                corpus: Some(corpus),
                ..all(vec![id])
            },
        )
        .await?
        .into_iter()
        .next()
        .unwrap_or(IdentitySummary {
            spoken_mentions: 0,
            visible_text_mentions: 0,
            shown_tracks: 0,
            heard_tracks: 0,
            video_count: 0,
            ..identity
        }),
        None => identity,
    };

    let units = sqlx::query(
        "SELECT l.unit_key, l.unit_kind, l.corpus_id, c.name AS corpus_name, l.entity_type,
                l.entity_key, l.person_id, l.status, l.basis, n.name, n.status AS name_status,
                p.label,
                (SELECT mode() WITHIN GROUP (ORDER BY m.surface_text)
                   FROM corpus_videos cv
                   JOIN entity_mentions m
                     ON m.video_id = cv.video_id AND m.entity_type = l.entity_type
                    AND m.entity_key = l.entity_key
                  WHERE cv.corpus_id = l.corpus_id) AS surface
         FROM canonical_identity_links l
         JOIN corpora c ON c.id = l.corpus_id
         LEFT JOIN people p ON p.id = l.person_id
         LEFT JOIN person_name_attributions n ON n.person_id = l.person_id
         WHERE l.identity_id = $1
         ORDER BY c.name, l.unit_kind, l.unit_key",
    )
    .bind(id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        let corpus_id: Uuid = row.try_get("corpus_id")?;
        let label = row
            .try_get::<Option<String>, _>("surface")?
            .or(row.try_get("name")?)
            .or(row.try_get("label")?)
            .unwrap_or_default();
        Ok(LocalUnit {
            unit_key: row.try_get("unit_key")?,
            unit_kind: row.try_get("unit_kind")?,
            corpus_id,
            corpus_name: row.try_get("corpus_name")?,
            label,
            entity_type: row.try_get("entity_type")?,
            entity_key: row.try_get("entity_key")?,
            person_id: row.try_get("person_id")?,
            name_status: row.try_get("name_status")?,
            status: row.try_get("status")?,
            basis: row.try_get("basis")?,
            in_scope: corpus.is_none_or(|scope| scope == corpus_id),
        })
    })
    .collect::<anyhow::Result<Vec<_>>>()?;

    let mut evidence = Vec::new();
    for unit in units.iter().filter(|unit| unit.in_scope) {
        let (mentions, tracks) = match (unit.person_id, &unit.entity_type, &unit.entity_key) {
            (Some(person), _, _) => (
                None,
                Some(crate::entities::person_occurrences(pool, unit.corpus_id, person).await?),
            ),
            (None, Some(entity_type), Some(entity_key)) => (
                Some(
                    crate::entities::entity_occurrences(
                        pool,
                        unit.corpus_id,
                        entity_type,
                        entity_key,
                    )
                    .await?,
                ),
                None,
            ),
            _ => (None, None),
        };
        evidence.push(UnitEvidence {
            unit_key: unit.unit_key.clone(),
            corpus_id: unit.corpus_id,
            mentions,
            tracks,
        });
    }

    let kept_apart_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT y.identity_id
         FROM canonical_identity_links x
         JOIN canonical_identity_rejections r ON r.unit_a = x.unit_key OR r.unit_b = x.unit_key
         JOIN canonical_identity_links y
           ON y.unit_key = CASE WHEN r.unit_a = x.unit_key THEN r.unit_b ELSE r.unit_a END
         WHERE x.identity_id = $1 AND y.identity_id <> $1",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let suggested: Vec<(Uuid, String)> = sqlx::query_as(
        "WITH named AS (
           SELECT l.identity_id, coalesce(l.entity_type, 'person') AS entity_type,
                  coalesce(l.entity_key, n.name_key) AS name_key
           FROM canonical_identity_links l
           JOIN canonical_identities i ON i.id = l.identity_id AND i.merged_into IS NULL
           LEFT JOIN person_name_attributions n ON n.person_id = l.person_id
         )
         SELECT DISTINCT theirs.identity_id, theirs.name_key
         FROM named mine
         JOIN named theirs
           ON theirs.entity_type = mine.entity_type AND theirs.name_key = mine.name_key
         WHERE mine.identity_id = $1 AND theirs.identity_id <> $1 AND mine.name_key IS NOT NULL
           AND NOT (theirs.identity_id = ANY($2))
         ORDER BY theirs.name_key, theirs.identity_id",
    )
    .bind(id)
    .bind(&kept_apart_ids)
    .fetch_all(pool)
    .await?;
    let mut shared: BTreeMap<Uuid, String> = BTreeMap::new();
    for (identity, key) in suggested {
        shared.entry(identity).or_insert(key);
    }
    let suggestions = list_identities(pool, &all(shared.keys().copied().collect()))
        .await?
        .into_iter()
        .map(|identity| IdentitySuggestion {
            shared_key: shared.get(&identity.id).cloned().unwrap_or_default(),
            identity,
        })
        .collect();
    let kept_apart = if kept_apart_ids.is_empty() {
        Vec::new()
    } else {
        list_identities(pool, &all(kept_apart_ids)).await?
    };

    Ok(Some(IdentityProfile {
        requested_id,
        identity,
        units,
        evidence,
        suggestions,
        kept_apart,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityAction {
    /// Pin every current link of the identity as researcher-confirmed.
    Confirm,
    /// Move every unit of `other_identity_id` into this identity; the other id redirects here.
    Merge,
    /// Move `unit_keys` into a new identity and keep them apart from the remaining units.
    Split,
    /// Keep this identity and `other_identity_id` apart; removes the suggestion.
    Reject,
    /// Clear a previous rejection or split between this identity and `other_identity_id`.
    Unreject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityCorrectionRequest {
    pub action: IdentityAction,
    pub other_identity_id: Option<Uuid>,
    #[serde(default)]
    pub unit_keys: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct IdentityCorrectionInputError(String);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityCorrectionReport {
    /// The identity to show after the correction.
    pub identity_id: Uuid,
    /// The identity created by a split.
    pub created_identity_id: Option<Uuid>,
}

async fn unit_keys_of(
    tx: &mut Transaction<'_, Postgres>,
    identity: Uuid,
) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT unit_key FROM canonical_identity_links WHERE identity_id = $1 ORDER BY unit_key",
    )
    .bind(identity)
    .fetch_all(&mut **tx)
    .await?)
}

async fn require_live(
    tx: &mut Transaction<'_, Postgres>,
    identity: Uuid,
) -> anyhow::Result<(String, String)> {
    sqlx::query_as(
        "SELECT kind, entity_type FROM canonical_identities WHERE id = $1 AND merged_into IS NULL",
    )
    .bind(identity)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("identity not found"))
}

async fn set_rejections(
    tx: &mut Transaction<'_, Postgres>,
    left: &[String],
    right: &[String],
    keep_apart: bool,
) -> anyhow::Result<()> {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for x in left {
        for y in right {
            let (first, second) = pair(x, y);
            a.push(first);
            b.push(second);
        }
    }
    let statement = if keep_apart {
        "INSERT INTO canonical_identity_rejections (unit_a, unit_b)
         SELECT * FROM unnest($1::text[], $2::text[]) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM canonical_identity_rejections r
         USING unnest($1::text[], $2::text[]) AS p(a, b)
         WHERE r.unit_a = p.a AND r.unit_b = p.b"
    };
    sqlx::query(statement)
        .bind(&a)
        .bind(&b)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Applies a researcher decision, logs it and re-resolves automatic links around it.
pub async fn correct_identity(
    pool: &PgPool,
    identity: Uuid,
    request: IdentityCorrectionRequest,
) -> anyhow::Result<IdentityCorrectionReport> {
    let input = |message: &str| anyhow::Error::from(IdentityCorrectionInputError(message.into()));
    let mut tx = pool.begin().await?;
    lock(&mut tx).await?;
    let (kind, entity_type) = require_live(&mut tx, identity).await?;
    let own = unit_keys_of(&mut tx, identity).await?;
    let other = match request.action {
        IdentityAction::Merge | IdentityAction::Reject | IdentityAction::Unreject => {
            let other = request
                .other_identity_id
                .ok_or_else(|| input("otherIdentityId is required"))?;
            if other == identity {
                return Err(input("an identity cannot be compared with itself"));
            }
            let (other_kind, other_type) = require_live(&mut tx, other).await?;
            if (other_kind, other_type) != (kind.clone(), entity_type.clone()) {
                return Err(input("only identities of the same type can be compared"));
            }
            Some((other, unit_keys_of(&mut tx, other).await?))
        }
        IdentityAction::Confirm | IdentityAction::Split => None,
    };
    let mut created = None;
    let mut unit_keys = Vec::new();
    match (request.action, &other) {
        (IdentityAction::Confirm, _) => {
            sqlx::query(
                "UPDATE canonical_identity_links SET status = 'confirmed', updated_at = now()
                 WHERE identity_id = $1",
            )
            .bind(identity)
            .execute(&mut *tx)
            .await?;
        }
        (IdentityAction::Merge, Some((other, other_units))) => {
            set_rejections(&mut tx, &own, other_units, false).await?;
            sqlx::query(
                "UPDATE canonical_identity_links
                 SET basis = CASE WHEN identity_id = $2 THEN 'researcher' ELSE basis END,
                     identity_id = $1, status = 'confirmed', updated_at = now()
                 WHERE identity_id IN ($1, $2)",
            )
            .bind(identity)
            .bind(other)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE canonical_identities SET merged_into = $1, updated_at = now()
                 WHERE id = $2 OR merged_into = $2",
            )
            .bind(identity)
            .bind(other)
            .execute(&mut *tx)
            .await?;
            unit_keys = other_units.clone();
        }
        (IdentityAction::Split, _) => {
            let moved: Vec<String> = request
                .unit_keys
                .iter()
                .filter(|key| own.contains(key))
                .cloned()
                .collect();
            if moved.is_empty() || moved.len() != request.unit_keys.len() {
                return Err(input("unitKeys must name local units of this identity"));
            }
            if moved.len() == own.len() {
                return Err(input("a split must leave at least one local unit behind"));
            }
            let remaining: Vec<String> =
                own.iter().filter(|key| !moved.contains(key)).cloned().collect();
            let new_identity = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO canonical_identities (id, kind, entity_type) VALUES ($1, $2, $3)",
            )
            .bind(new_identity)
            .bind(&kind)
            .bind(&entity_type)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE canonical_identity_links
                 SET identity_id = CASE WHEN unit_key = ANY($2) THEN $3 ELSE identity_id END,
                     basis = CASE WHEN unit_key = ANY($2) THEN 'researcher' ELSE basis END,
                     status = 'confirmed', updated_at = now()
                 WHERE identity_id = $1",
            )
            .bind(identity)
            .bind(&moved)
            .bind(new_identity)
            .execute(&mut *tx)
            .await?;
            set_rejections(&mut tx, &moved, &remaining, true).await?;
            created = Some(new_identity);
            unit_keys = moved;
        }
        (IdentityAction::Reject, Some((_, other_units))) => {
            set_rejections(&mut tx, &own, other_units, true).await?;
        }
        (IdentityAction::Unreject, Some((_, other_units))) => {
            set_rejections(&mut tx, &own, other_units, false).await?;
        }
        _ => unreachable!("other identity is loaded for comparing actions"),
    }
    let action = serde_json::to_value(request.action)?;
    sqlx::query(
        "INSERT INTO canonical_identity_corrections
           (id, action, identity_id, other_identity_id, unit_keys)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(action.as_str())
    .bind(identity)
    .bind(other.as_ref().map(|(id, _)| *id).or(created))
    .bind(&unit_keys)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    resolve_identities(pool).await?;
    Ok(IdentityCorrectionReport {
        identity_id: identity,
        created_identity_id: created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_keys_are_stable_and_pairs_ordered() {
        let corpus = Uuid::nil();
        assert_eq!(
            entity_unit_key(corpus, "person", "ada lovelace"),
            "entity:00000000-0000-0000-0000-000000000000:person:ada lovelace"
        );
        assert_eq!(pair("b", "a"), ("a".to_string(), "b".to_string()));
    }

    #[test]
    fn components_keep_confirmed_identity_through_unions() {
        let identity = Uuid::new_v4();
        let mut sets = Components::new(3, &[None, Some(identity), None]);
        sets.union(0, 1);
        sets.union(2, 0);
        let root = sets.find(2);
        assert_eq!(sets.confirmed[root], Some(identity));
        assert_eq!(sets.members[root].len(), 3);
    }
}
