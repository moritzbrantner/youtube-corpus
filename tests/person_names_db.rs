//! Evidence-grounded person names (#65, Atlas 4.7) against Postgres and the corpus HTTP API.
//!
//! Face people are formed by the #64 matcher from synthetic, non-biometric vectors; on-screen
//! text and person mentions are seeded in the shape the visual-analysis OCR and nlp-stack NER
//! producers persist. The real producers are not run here.
//!
//! Requires `DATABASE_URL`; skipped otherwise (run through `bun run verify:postgres`).

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::entities::{
    lookup_entities, replace_entity_run, EntityMentionInput, EntityModality, EntityRunProvenance,
};
use youtube_corpus::face_matching::match_corpus_faces;
use youtube_corpus::face_processing::{FACE_MODEL, FACE_MODEL_VERSION, FACE_PROCESSOR};
use youtube_corpus::person_names::{
    correct_person_name, infer_person_names, NameCorrectionAction, NameCorrectionRequest,
    PersonNamesReport,
};
use youtube_corpus::{
    begin_processing_run, begin_visual_processing_run, link_face_observation_to_track,
    upsert_face_observation, upsert_face_track, upsert_visual_text_track,
    BeginProcessingRunRequest, BeginVisualProcessingRunRequest, BoundingBox, MediaModality,
    ProcessingProvenance, UpsertFaceObservationRequest, UpsertFaceTrackRequest,
    UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

fn direction(axis: usize, jitter: usize) -> Vec<f32> {
    let mut vector = vec![0.0f32; 128];
    vector[axis] = 1.0;
    vector[jitter] += 0.12;
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    vector.iter().map(|v| v / norm).collect()
}

fn provenance(processor: &str, model: &str, version: &str, input: &str) -> ProcessingProvenance {
    ProcessingProvenance {
        processor: processor.into(),
        processor_version: "fixture".into(),
        model: model.into(),
        model_version: version.into(),
        input_hash: input.into(),
        config_hash: "fixture".into(),
        processing_config: json!({"complete": true}),
    }
}

struct Ctx {
    pool: PgPool,
    database_url: String,
    corpus: Uuid,
}

impl Ctx {
    async fn new() -> Option<Self> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Person names')")
            .bind(corpus)
            .bind(format!("names-{corpus}"))
            .execute(&pool)
            .await
            .unwrap();
        Some(Self {
            pool,
            database_url,
            corpus,
        })
    }

    async fn video(&self, title: &str) -> Uuid {
        let video = Uuid::new_v4();
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, $3)")
            .bind(video)
            .bind(format!("https://www.youtube.com/watch?v={video}"))
            .bind(title)
            .execute(&self.pool)
            .await
            .unwrap();
        youtube_corpus::corpora::link_video(&self.pool, self.corpus, video)
            .await
            .unwrap();
        video
    }

    /// A face track visible from `start` for three seconds.
    async fn face(&self, video: Uuid, start: f64, embedding: Vec<f32>) -> Uuid {
        let run = begin_processing_run(
            &self.pool,
            BeginProcessingRunRequest {
                video_id: video,
                modality: MediaModality::Face,
                provenance: provenance(
                    FACE_PROCESSOR,
                    FACE_MODEL,
                    FACE_MODEL_VERSION,
                    &format!("faces-{video}"),
                ),
            },
        )
        .await
        .unwrap()
        .id;
        let track = upsert_face_track(
            &self.pool,
            UpsertFaceTrackRequest {
                run_id: run,
                video_id: video,
                track_key: format!("t{start}"),
                start_seconds: start,
                end_seconds: start + 3.0,
                representative_embedding: Some(embedding.clone()),
                quality: json!({}),
            },
        )
        .await
        .unwrap()
        .id;
        for index in 0..3 {
            let observation = upsert_face_observation(
                &self.pool,
                UpsertFaceObservationRequest {
                    run_id: run,
                    video_id: video,
                    observation_key: format!("t{start}-{index}"),
                    start_seconds: start + index as f64,
                    end_seconds: None,
                    frame_index: None,
                    region: BoundingBox {
                        x: 10.0,
                        y: 10.0,
                        width: 64.0,
                        height: 64.0,
                    },
                    detection_score: Some(0.97),
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

    /// One on-screen text track holding a single person mention `name` (replaces the video's
    /// previous entity run, as re-extraction does).
    async fn on_screen(
        &self,
        video: Uuid,
        text: &str,
        name: &str,
        role: VisualTextRole,
        start: f64,
    ) {
        let run = begin_visual_processing_run(
            &self.pool,
            BeginVisualProcessingRunRequest {
                video_id: video,
                kind: VisualProcessingKind::Ocr,
                provenance: provenance("visual-analysis", "ocr", "1", &format!("ocr-{video}")),
            },
        )
        .await
        .unwrap();
        let track = upsert_visual_text_track(
            &self.pool,
            UpsertVisualTextTrackRequest {
                run_id: run,
                video_id: video,
                track_key: format!("text-{start}"),
                text: text.into(),
                role,
                language: Some("en".into()),
                sample_count: 2,
                start_frame: None,
                end_frame: None,
                start_seconds: Some(start),
                end_seconds: Some(start + 2.0),
                region: None,
                metadata: json!({}),
                observation_ids: Vec::new(),
                scene_ids: Vec::new(),
            },
        )
        .await
        .unwrap()
        .id;
        let at = text.find(name).unwrap() as i32;
        let mention = EntityMentionInput {
            modality: EntityModality::VisibleText,
            entity_type: "person".into(),
            surface_text: name.into(),
            entity_key: youtube_corpus::entities::entity_key(name),
            start_seconds: Some(start),
            end_seconds: Some(start + 2.0),
            transcript_segment_id: None,
            visual_text_track_id: Some(track),
            byte_start: at,
            byte_end: at + name.len() as i32,
            confidence: Some(0.95),
        };
        replace_entity_run(
            &self.pool,
            video,
            &EntityRunProvenance::canonical("dslim/bert-base-NER@fixture"),
            None,
            0,
            1,
            &[mention],
        )
        .await
        .unwrap();
    }

    async fn correct(&self, person: Uuid, action: NameCorrectionAction, name: Option<&str>) {
        correct_person_name(
            &self.pool,
            self.corpus,
            NameCorrectionRequest {
                person_id: person,
                action,
                name: name.map(str::to_owned),
            },
        )
        .await
        .unwrap();
    }
}

fn entry(
    report: &PersonNamesReport,
    person: Uuid,
) -> &youtube_corpus::person_names::PersonNameReview {
    report
        .people
        .iter()
        .find(|p| p.person_id == person)
        .expect("person listed")
}

fn name_of(report: &PersonNamesReport, person: Uuid) -> Option<(String, String)> {
    entry(report, person)
        .name
        .as_ref()
        .map(|n| (n.name.clone(), n.status.clone()))
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn aligned_name_tags_name_people_and_corrections_survive_reruns() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    let v: Vec<Uuid> = futures(&ctx).await;
    let ada = [
        ctx.face(v[0], 10.0, direction(0, 10)).await,
        ctx.face(v[1], 10.0, direction(0, 11)).await,
    ];
    let charles = [
        ctx.face(v[2], 10.0, direction(2, 12)).await,
        ctx.face(v[3], 10.0, direction(2, 13)).await,
    ];
    let review = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    let person_of = |track: Uuid| {
        review
            .groups
            .iter()
            .find(|g| g.tracks.iter().any(|t| t.track_id == track))
            .map(|g| g.person_id)
            .expect("face person formed")
    };
    let (ada_person, charles_person) = (person_of(ada[0]), person_of(charles[0]));

    // Strong: a lower-third name tag while only Ada's face is visible.
    ctx.on_screen(
        v[0],
        "Ada Lovelace, Mathematician",
        "Ada Lovelace",
        VisualTextRole::SceneText,
        11.0,
    )
    .await;
    // Weak: a burned-in sentence about an absent third person while Charles is visible.
    ctx.on_screen(
        v[2],
        "Yesterday Ada Lovelace sent her notes",
        "Ada Lovelace",
        VisualTextRole::Subtitle,
        11.0,
    )
    .await;
    // Not aligned: end credits listing a name.
    ctx.on_screen(
        v[3],
        "Charles Babbage",
        "Charles Babbage",
        VisualTextRole::EndCredit,
        11.0,
    )
    .await;

    let report = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&report, ada_person),
        Some(("Ada Lovelace".into(), "inferred".into()))
    );
    let named = entry(&report, ada_person).name.as_ref().unwrap();
    assert_eq!(named.evidence.len(), 1);
    assert_eq!(named.evidence[0].face_track_id, ada[0]);
    assert_eq!(named.evidence[0].source_text, "Ada Lovelace, Mathematician");
    assert_eq!(
        named.evidence[0].provenance["entity"]["producer"],
        "nlp-stack/text-linguistics"
    );
    assert_eq!(name_of(&report, charles_person), None);

    // Rerunning unchanged evidence is idempotent.
    let again = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&again, ada_person),
        Some(("Ada Lovelace".into(), "inferred".into()))
    );

    // Rejecting through HTTP removes the name but keeps the face evidence and matching.
    let app = youtube_corpus::web::app_for_tests_with_research(Some(ctx.database_url.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/api/corpora/{}/person-names/corrections",
                    ctx.corpus
                ))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"personId": ada_person, "action": "reject"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let listed = body["people"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["personId"] == json!(ada_person))
        .unwrap();
    assert!(listed["name"].is_null());
    assert_eq!(listed["rejectedNames"], json!(["Ada Lovelace"]));

    let rerun = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&rerun, ada_person),
        None,
        "a rejected name is not recreated"
    );
    let faces = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    assert!(faces
        .groups
        .iter()
        .any(|g| g.person_id == ada_person && g.tracks.iter().any(|t| t.track_id == ada[1])));

    // A researcher-provided name is distinct, searchable and survives reruns.
    ctx.correct(
        ada_person,
        NameCorrectionAction::Replace,
        Some("Augusta Ada King"),
    )
    .await;
    let rerun = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&rerun, ada_person),
        Some(("Augusta Ada King".into(), "user_provided".into()))
    );
    let found = lookup_entities(&ctx.pool, ctx.corpus, "Augusta", 20)
        .await
        .unwrap();
    assert_eq!(found.people.len(), 1);
    assert_eq!(found.people[0].person_id, ada_person);
    assert_eq!(
        found.people[0].name_status.as_deref(),
        Some("user_provided")
    );

    // Confirming an inference turns it into a user-confirmed name.
    ctx.on_screen(
        v[3],
        "Charles Babbage | Inventor",
        "Charles Babbage",
        VisualTextRole::Subtitle,
        11.0,
    )
    .await;
    let report = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&report, charles_person),
        Some(("Charles Babbage".into(), "inferred".into()))
    );
    ctx.correct(charles_person, NameCorrectionAction::Confirm, None)
        .await;
    let report = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    assert_eq!(
        name_of(&report, charles_person),
        Some(("Charles Babbage".into(), "confirmed".into()))
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn two_visible_faces_or_conflicting_tags_leave_names_unresolved() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    let v: Vec<Uuid> = futures(&ctx).await;
    let a = [
        ctx.face(v[0], 10.0, direction(4, 20)).await,
        ctx.face(v[1], 10.0, direction(4, 21)).await,
    ];
    let b = [
        ctx.face(v[2], 10.0, direction(6, 22)).await,
        ctx.face(v[3], 10.0, direction(6, 23)).await,
    ];
    // A second, unmatched face is on screen with A in v[0].
    ctx.face(v[0], 9.0, direction(8, 24)).await;
    let review = match_corpus_faces(&ctx.pool, ctx.corpus).await.unwrap();
    let person_of = |track: Uuid| {
        review
            .groups
            .iter()
            .find(|g| g.tracks.iter().any(|t| t.track_id == track))
            .map(|g| g.person_id)
            .unwrap()
    };
    ctx.on_screen(
        v[0],
        "Grace Hopper",
        "Grace Hopper",
        VisualTextRole::SceneText,
        11.0,
    )
    .await;
    ctx.on_screen(
        v[2],
        "Alan Turing",
        "Alan Turing",
        VisualTextRole::SceneText,
        11.0,
    )
    .await;
    ctx.on_screen(
        v[3],
        "Alan Mathison",
        "Alan Mathison",
        VisualTextRole::SceneText,
        11.0,
    )
    .await;
    let report = infer_person_names(&ctx.pool, ctx.corpus).await.unwrap();
    let a_entry = entry(&report, person_of(a[0]));
    assert!(a_entry.name.is_none());
    assert!(a_entry
        .unresolved
        .as_ref()
        .unwrap()
        .reason
        .contains("several faces"));
    let b_entry = entry(&report, person_of(b[0]));
    assert!(b_entry.name.is_none());
    assert!(b_entry
        .unresolved
        .as_ref()
        .unwrap()
        .reason
        .contains("Conflicting"));
}

async fn futures(ctx: &Ctx) -> Vec<Uuid> {
    let mut videos = Vec::new();
    for index in 0..4 {
        videos.push(ctx.video(&format!("Interview {index}")).await);
    }
    videos
}
