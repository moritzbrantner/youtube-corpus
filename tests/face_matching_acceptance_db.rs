//! Retrospective independent acceptance for #64 (face-only split, Atlas Decisions 4.3 and 4.5).
//!
//! Written from the issue's approved intent and the public storage/matching/HTTP surface, not
//! from the #97 implementation or its own contract tests (`tests/face_matching_db.rs`). Evidence
//! is seeded through the canonical multimodal storage API in the shape the faces detector
//! (`youtube-corpus/faces`, pinned YuNet/SFace) persists: complete runs, >= 2 sampled
//! observations per track and 128-dimensional representative embeddings. The vectors are
//! synthetic geometric directions, not biometric data. The real detector pipeline is covered by
//! the opt-in native check described in PR #97 and is not re-proven here.
//!
//! Requires `DATABASE_URL`; skipped otherwise (run through `bun run verify:postgres`).

use std::collections::BTreeSet;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::entities::person_occurrences;
use youtube_corpus::face_matching::{
    correct_face_matches, match_corpus_faces, review_face_matches, FaceCorrectionAction,
    FaceCorrectionRequest, FaceMatchReview,
};
use youtube_corpus::face_processing::{FACE_MODEL, FACE_MODEL_VERSION, FACE_PROCESSOR};
use youtube_corpus::{
    begin_processing_run, link_face_observation_to_track, upsert_face_observation,
    upsert_face_track, BeginProcessingRunRequest, BoundingBox, MediaModality, ProcessingProvenance,
    UpsertFaceObservationRequest, UpsertFaceTrackRequest,
};

const DIMENSIONS: usize = 128;

/// Unit vector `cos(angle) * e_a + sin(angle) * e_b` plus a tiny per-track jitter on `e_jitter`.
fn direction(a: usize, b: usize, angle: f32, jitter: usize) -> Vec<f32> {
    let mut vector = vec![0.0f32; DIMENSIONS];
    vector[a] += angle.cos();
    vector[b] += angle.sin();
    vector[jitter] += 0.12;
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    vector.iter().map(|v| v / norm).collect()
}

/// Person "A" lies on axis 0. Person "B" is 60 degrees away (cosine 0.5): a distinct person.
/// The bisector of A and B is about 0.87 similar to both: a genuinely ambiguous face.
const B_ANGLE: f32 = std::f32::consts::FRAC_PI_3; // acos(0.5)

fn person_a(jitter: usize) -> Vec<f32> {
    direction(0, 1, 0.0, jitter)
}
fn person_b(jitter: usize) -> Vec<f32> {
    direction(0, 1, B_ANGLE, jitter)
}
fn between_a_and_b(jitter: usize) -> Vec<f32> {
    direction(0, 1, B_ANGLE / 2.0, jitter)
}

struct Ctx {
    pool: PgPool,
    database_url: String,
    corpus: Uuid,
}

async fn context(name: &str) -> Option<Ctx> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
        .bind(corpus)
        .bind(format!("face-acceptance-{corpus}"))
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
    Some(Ctx {
        pool,
        database_url,
        corpus,
    })
}

impl Ctx {
    async fn video(&self, title: &str, member: bool) -> Uuid {
        let video = Uuid::new_v4();
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, $3)")
            .bind(video)
            .bind(format!(
                "https://www.youtube.com/watch?v=acc{}",
                &video.simple().to_string()[..8]
            ))
            .bind(title)
            .execute(&self.pool)
            .await
            .unwrap();
        if member {
            youtube_corpus::corpora::link_video(&self.pool, self.corpus, video)
                .await
                .unwrap();
        }
        video
    }

    /// A completed faces-detector run for `video`, as the plan persists it.
    async fn run(&self, video: Uuid, input: &str, model_version: &str) -> Uuid {
        begin_processing_run(
            &self.pool,
            BeginProcessingRunRequest {
                video_id: video,
                modality: MediaModality::Face,
                provenance: ProcessingProvenance {
                    processor: FACE_PROCESSOR.into(),
                    processor_version: "acceptance".into(),
                    model: FACE_MODEL.into(),
                    model_version: model_version.into(),
                    input_hash: input.into(),
                    config_hash: "acceptance-config".into(),
                    processing_config: json!({"complete": true, "sampleIntervalSeconds": 1.0}),
                },
            },
        )
        .await
        .unwrap()
        .id
    }

    /// One within-video face track with `observations` sampled detections.
    #[allow(clippy::too_many_arguments)]
    async fn track(
        &self,
        run: Uuid,
        video: Uuid,
        key: &str,
        start: f64,
        embedding: Vec<f32>,
        observations: usize,
        score: f64,
    ) -> Uuid {
        let end = start + observations as f64;
        let track = upsert_face_track(
            &self.pool,
            UpsertFaceTrackRequest {
                run_id: run,
                video_id: video,
                track_key: key.into(),
                start_seconds: start,
                end_seconds: end,
                representative_embedding: Some(embedding.clone()),
                quality: json!({"inferred": true, "verifiedIdentity": false}),
            },
        )
        .await
        .unwrap()
        .id;
        for index in 0..observations {
            let observation = upsert_face_observation(
                &self.pool,
                UpsertFaceObservationRequest {
                    run_id: run,
                    video_id: video,
                    observation_key: format!("{key}-{index}"),
                    start_seconds: start + index as f64,
                    end_seconds: None,
                    frame_index: Some(index as i64 * 25),
                    region: BoundingBox {
                        x: 10.0,
                        y: 10.0,
                        width: 64.0,
                        height: 64.0,
                    },
                    detection_score: Some(score),
                    embedding: Some(embedding.clone()),
                    quality: json!({}),
                },
            )
            .await
            .unwrap();
            link_face_observation_to_track(&self.pool, track, observation.id)
                .await
                .unwrap();
        }
        track
    }

    async fn correct(&self, action: FaceCorrectionAction, tracks: &[Uuid], person: Option<Uuid>) {
        correct_face_matches(
            &self.pool,
            self.corpus,
            FaceCorrectionRequest {
                action,
                track_ids: tracks.to_vec(),
                person_id: person,
            },
        )
        .await
        .unwrap();
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .bind(self.corpus)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn people(&self) -> i64 {
        self.count("SELECT count(*) FROM people WHERE corpus_id = $1")
            .await
    }

    async fn source_evidence(&self) -> (i64, i64) {
        (
            self.count(
                "SELECT count(*) FROM face_tracks WHERE video_id IN
                 (SELECT video_id FROM corpus_video_memberships WHERE corpus_id = $1)",
            )
            .await,
            self.count(
                "SELECT count(*) FROM face_observations WHERE video_id IN
                 (SELECT video_id FROM corpus_video_memberships WHERE corpus_id = $1)",
            )
            .await,
        )
    }
}

fn grouped_with(review: &FaceMatchReview, track: Uuid) -> Option<(Uuid, BTreeSet<Uuid>)> {
    review
        .groups
        .iter()
        .find(|group| group.tracks.iter().any(|t| t.track_id == track))
        .map(|group| {
            (
                group.person_id,
                group.tracks.iter().map(|t| t.track_id).collect(),
            )
        })
}

fn together(review: &FaceMatchReview, a: Uuid, b: Uuid) -> bool {
    grouped_with(review, a).is_some_and(|(_, tracks)| tracks.contains(&b))
}

fn unresolved(review: &FaceMatchReview, track: Uuid) -> bool {
    review.unresolved.iter().any(|t| t.track_id == track)
}

fn not_verified(source: Option<&str>) {
    let source = source.unwrap_or_default().to_ascii_lowercase();
    assert!(
        !source.contains("verified") || source.contains("unverified"),
        "an inferred association must not be reported as a verified identity: {source}"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn high_confidence_repeats_group_automatically_while_weak_ambiguous_and_foreign_evidence_stays_apart(
) {
    let Some(ctx) = context("Face acceptance: grouping").await else {
        return;
    };
    let v: Vec<Uuid> = futures_videos(&ctx, 8).await;
    let outside = ctx.video("Outside the corpus", false).await;

    let runs = [
        ctx.run(v[0], "in-0", FACE_MODEL_VERSION).await,
        ctx.run(v[1], "in-1", FACE_MODEL_VERSION).await,
        ctx.run(v[2], "in-2", FACE_MODEL_VERSION).await,
        ctx.run(v[3], "in-3", FACE_MODEL_VERSION).await,
        ctx.run(v[4], "in-4", FACE_MODEL_VERSION).await,
        ctx.run(v[5], "in-5", FACE_MODEL_VERSION).await,
        ctx.run(v[6], "in-6", "yunet:other;sface:other").await,
    ];
    let outside_run = ctx.run(outside, "in-out", FACE_MODEL_VERSION).await;

    let a1 = ctx
        .track(runs[0], v[0], "a1", 12.0, person_a(10), 3, 0.97)
        .await;
    let a2 = ctx
        .track(runs[1], v[1], "a2", 40.0, person_a(11), 4, 0.95)
        .await;
    let b1 = ctx
        .track(runs[2], v[2], "b1", 5.0, person_b(12), 3, 0.96)
        .await;
    let b2 = ctx
        .track(runs[3], v[3], "b2", 7.0, person_b(13), 3, 0.96)
        .await;
    // Ambiguous: equally similar to A and B.
    let ambiguous = ctx
        .track(runs[4], v[4], "c", 3.0, between_a_and_b(14), 3, 0.97)
        .await;
    // Low confidence: A's direction, but a single weak detection.
    let weak = ctx
        .track(runs[5], v[5], "weak", 9.0, person_a(15), 1, 0.55)
        .await;
    // Incompatible producer model: A's direction from a different embedding model.
    let foreign_model = ctx
        .track(runs[6], v[6], "x", 2.0, person_a(16), 3, 0.97)
        .await;
    // Same direction outside this corpus.
    let outsider = ctx
        .track(outside_run, outside, "o", 2.0, person_a(17), 3, 0.97)
        .await;
    let _ = v[7]; // a corpus video without face evidence

    // No approval step: running the matcher alone produces inferred people.
    let review = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();

    assert!(
        together(&review, a1, a2),
        "A repeats across videos: {review:#?}"
    );
    assert!(together(&review, b1, b2), "B repeats across videos");
    assert!(!together(&review, a1, b1), "distinct people stay separate");
    for track in [ambiguous, weak, foreign_model] {
        assert!(!together(&review, a1, track), "{track} must not join A");
        assert!(!together(&review, b1, track), "{track} must not join B");
    }
    assert!(
        unresolved(&review, ambiguous),
        "ambiguous evidence is shown as unresolved"
    );
    let all: BTreeSet<Uuid> = review
        .groups
        .iter()
        .flat_map(|g| g.tracks.iter().map(|t| t.track_id))
        .chain(review.unresolved.iter().map(|t| t.track_id))
        .collect();
    assert!(
        !all.contains(&outsider),
        "other corpora never leak into this corpus"
    );

    // Inspectable evidence: inferred status, confidence, modality model, provenance, times, links.
    let group = review
        .groups
        .iter()
        .find(|g| g.tracks.iter().any(|t| t.track_id == a1))
        .unwrap();
    not_verified(group.identity_source.as_deref());
    let videos: BTreeSet<Uuid> = group.tracks.iter().map(|t| t.video_id).collect();
    assert_eq!(videos.len(), 2);
    for track in &group.tracks {
        let similarity = track.similarity.expect("match confidence is visible");
        assert!((0.0..=1.0).contains(&similarity));
        assert_eq!(track.processor, FACE_PROCESSOR);
        assert_eq!(track.model, FACE_MODEL);
        assert_eq!(track.model_version, FACE_MODEL_VERSION);
        assert!(track.input_hash.starts_with("in-"));
        assert!(track.matching_algorithm.is_some() && track.matching_version.is_some());
        assert!(track.matching_configuration.is_some());
        assert!(track
            .source_url
            .starts_with("https://www.youtube.com/watch?v="));
        assert!(track.end_seconds >= track.start_seconds);
    }
    assert!(
        review.configuration.is_object(),
        "matching configuration is inspectable"
    );

    // The explorer counts this inferred person in both videos.
    let occurrences = person_occurrences(&ctx.pool, ctx.corpus, group.person_id)
        .await
        .unwrap();
    assert_eq!(occurrences.videos.len(), 2);

    // An unchanged rerun is idempotent: same people and the same groups.
    let people = ctx.people().await;
    let again = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(ctx.people().await, people, "no duplicate people on rerun");
    assert_eq!(grouped_with(&again, a1), grouped_with(&review, a1));
    assert_eq!(grouped_with(&again, b1), grouped_with(&review, b1));
}

async fn futures_videos(ctx: &Ctx, count: usize) -> Vec<Uuid> {
    let mut videos = Vec::new();
    for index in 0..count {
        videos.push(ctx.video(&format!("Acceptance video {index}"), true).await);
    }
    videos
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn simultaneous_faces_in_one_video_are_never_the_same_person() {
    let Some(ctx) = context("Face acceptance: contradiction").await else {
        return;
    };
    let v = futures_videos(&ctx, 2).await;
    let r0 = ctx.run(v[0], "in-0", FACE_MODEL_VERSION).await;
    let r1 = ctx.run(v[1], "in-1", FACE_MODEL_VERSION).await;
    // Two different faces on screen at the same time, both resembling the face in video 1.
    let left = ctx
        .track(r0, v[0], "left", 10.0, person_a(20), 3, 0.97)
        .await;
    let right = ctx
        .track(r0, v[0], "right", 10.0, person_a(21), 3, 0.97)
        .await;
    let other = ctx
        .track(r1, v[1], "other", 4.0, person_a(22), 3, 0.97)
        .await;

    let review = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert!(
        !together(&review, left, right),
        "co-occurring faces contradict one identity"
    );
    assert!(
        !(together(&review, left, other) && together(&review, right, other)),
        "a contradictory triple is not merged"
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn corrections_change_links_and_totals_keep_evidence_and_survive_reprocessing() {
    let Some(ctx) = context("Face acceptance: corrections").await else {
        return;
    };
    let v = futures_videos(&ctx, 4).await;
    let runs = [
        ctx.run(v[0], "in-0", FACE_MODEL_VERSION).await,
        ctx.run(v[1], "in-1", FACE_MODEL_VERSION).await,
        ctx.run(v[2], "in-2", FACE_MODEL_VERSION).await,
        ctx.run(v[3], "in-3", FACE_MODEL_VERSION).await,
    ];
    let a1 = ctx
        .track(runs[0], v[0], "a1", 10.0, person_a(30), 3, 0.97)
        .await;
    let a2 = ctx
        .track(runs[1], v[1], "a2", 20.0, person_a(31), 3, 0.97)
        .await;
    let a3 = ctx
        .track(runs[2], v[2], "a3", 30.0, person_a(32), 3, 0.97)
        .await;
    let a4 = ctx
        .track(runs[3], v[3], "a4", 40.0, person_a(33), 3, 0.97)
        .await;

    let review = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    let (person, members) = grouped_with(&review, a1).expect("A is grouped");
    assert_eq!(members, BTreeSet::from([a1, a2, a3, a4]));
    let before = person_occurrences(&ctx.pool, ctx.corpus, person)
        .await
        .unwrap();
    assert_eq!(before.videos.len(), 4);
    let evidence = ctx.source_evidence().await;

    // Split one intentionally incorrect association out of the person.
    ctx.correct(FaceCorrectionAction::Split, &[a4], None).await;
    // Unlink another.
    ctx.correct(FaceCorrectionAction::Unlink, &[a3], None).await;

    let corrected = review_face_matches(&ctx.pool, ctx.corpus).await.unwrap();
    let (_, members) = grouped_with(&corrected, a1).expect("A keeps its remaining evidence");
    assert_eq!(
        members,
        BTreeSet::from([a1, a2]),
        "corrections apply immediately"
    );
    let after = person_occurrences(&ctx.pool, ctx.corpus, person)
        .await
        .unwrap();
    assert_eq!(
        after.videos.len(),
        2,
        "occurrence totals follow corrections"
    );
    assert_eq!(
        ctx.source_evidence().await,
        evidence,
        "source observations are retained"
    );

    // Unchanged reprocessing: the matcher rerun and a producer refresh of the same input
    // (same run identity and track keys) must not silently reinstate the rejected links.
    match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    for (run, video, input, key, start, jitter) in [
        (runs[2], v[2], "in-2", "a3", 30.0, 32),
        (runs[3], v[3], "in-3", "a4", 40.0, 33),
    ] {
        let rerun = ctx.run(video, input, FACE_MODEL_VERSION).await;
        assert_eq!(rerun, run, "unchanged input reuses the detector run");
        ctx.track(run, video, key, start, person_a(jitter), 3, 0.97)
            .await;
    }
    let people = ctx.people().await;
    let rerun = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert!(
        !together(&rerun, a1, a3),
        "unlinked association stays rejected"
    );
    assert!(
        !together(&rerun, a1, a4),
        "split association stays rejected"
    );
    assert!(together(&rerun, a1, a2));
    let rerun_again = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        ctx.people().await,
        people,
        "repeated processing creates no duplicate people"
    );
    assert_eq!(grouped_with(&rerun_again, a1), grouped_with(&rerun, a1));

    // A researcher can explicitly link a valid association back and confirm the person.
    ctx.correct(FaceCorrectionAction::Link, &[a3], Some(person))
        .await;
    ctx.correct(FaceCorrectionAction::Confirm, &[a1, a2, a3], Some(person))
        .await;
    let relinked = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    let (relinked_person, members) = grouped_with(&relinked, a1).unwrap();
    assert_eq!(
        relinked_person, person,
        "the corrected person keeps its identity"
    );
    assert_eq!(members, BTreeSet::from([a1, a2, a3]));
    assert!(
        !together(&relinked, a1, a4),
        "the split rejection is untouched by the relink"
    );
    let group = relinked
        .groups
        .iter()
        .find(|g| g.person_id == person)
        .unwrap();
    not_verified(group.identity_source.as_deref());
    let confirmed = person_occurrences(&ctx.pool, ctx.corpus, person)
        .await
        .unwrap();
    assert_eq!(confirmed.videos.len(), 3);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_newer_compatible_producer_run_replaces_superseded_evidence_without_duplicates() {
    let Some(ctx) = context("Face acceptance: producer revision").await else {
        return;
    };
    let v = futures_videos(&ctx, 2).await;
    let old_run = ctx.run(v[0], "in-0-old", FACE_MODEL_VERSION).await;
    let r1 = ctx.run(v[1], "in-1", FACE_MODEL_VERSION).await;
    let old = ctx
        .track(old_run, v[0], "a", 10.0, person_a(40), 3, 0.97)
        .await;
    let other = ctx.track(r1, v[1], "a", 50.0, person_a(41), 3, 0.97).await;
    let first = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert!(together(&first, old, other));
    let people = ctx.people().await;

    // The video is re-acquired (new input hash) and the faces detector runs again.
    let new_run = ctx.run(v[0], "in-0-new", FACE_MODEL_VERSION).await;
    assert_ne!(new_run, old_run);
    let new = ctx
        .track(new_run, v[0], "a", 10.0, person_a(42), 3, 0.97)
        .await;
    let second = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();

    assert!(
        together(&second, new, other),
        "current producer output is matched"
    );
    assert!(
        !together(&second, old, other),
        "superseded output no longer supports the person"
    );
    assert_eq!(
        ctx.people().await,
        people,
        "no duplicate person for re-processed evidence"
    );
    let (_, tracks) = grouped_with(&second, other).unwrap();
    let group = second
        .groups
        .iter()
        .find(|g| g.tracks.iter().any(|t| t.track_id == other))
        .unwrap();
    let in_first_video = group.tracks.iter().filter(|t| t.video_id == v[0]).count();
    assert_eq!(
        in_first_video, 1,
        "one track per video in the person: {tracks:?}"
    );
    let new_evidence = group.tracks.iter().find(|t| t.track_id == new).unwrap();
    assert_eq!(
        new_evidence.input_hash, "in-0-new",
        "provenance names the current run"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM face_tracks WHERE id = $1")
        .bind(old)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(stored, 1, "superseded source evidence is retained");
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn http_review_correction_and_occurrence_contract() {
    let Some(ctx) = context("Face acceptance: HTTP").await else {
        return;
    };
    let app = youtube_corpus::web::app_for_tests_with_research(Some(ctx.database_url.clone()));
    let v = futures_videos(&ctx, 3).await;
    let mut tracks = Vec::new();
    for (index, video) in v.iter().enumerate() {
        let run = ctx
            .run(*video, &format!("in-{index}"), FACE_MODEL_VERSION)
            .await;
        tracks.push(
            ctx.track(
                run,
                *video,
                "a",
                5.0 + index as f64,
                person_a(50 + index),
                3,
                0.97,
            )
            .await,
        );
    }
    let base = format!("/api/corpora/{}", ctx.corpus);

    let (status, review) = call(
        &app,
        Method::POST,
        &format!("{base}/face-matches/run"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{review}");
    let group = &review["groups"][0];
    let person = group["personId"].as_str().unwrap().to_string();
    assert_eq!(group["tracks"].as_array().unwrap().len(), 3);
    let track = &group["tracks"][0];
    for field in [
        "videoId",
        "sourceUrl",
        "startSeconds",
        "endSeconds",
        "similarity",
        "model",
        "modelVersion",
        "inputHash",
        "matchingAlgorithm",
    ] {
        assert!(
            !track[field].is_null(),
            "evidence field {field} is exposed: {track}"
        );
    }

    let (status, _) = call(
        &app,
        Method::POST,
        &format!("{base}/face-matches/corrections"),
        Some(json!({"action": "split", "trackIds": []})),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an empty correction is rejected, got {status}"
    );

    let (status, corrected) = call(
        &app,
        Method::POST,
        &format!("{base}/face-matches/corrections"),
        Some(json!({"action": "split", "trackIds": [tracks[2]]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{corrected}");
    let (status, persisted) = call(&app, Method::GET, &format!("{base}/face-matches"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        persisted["groups"], corrected["groups"],
        "the correction is persisted"
    );

    let (status, occurrences) = call(
        &app,
        Method::GET,
        &format!("{base}/people/{person}/occurrences"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{occurrences}");
    assert_eq!(occurrences["videos"].as_array().unwrap().len(), 2);
}
