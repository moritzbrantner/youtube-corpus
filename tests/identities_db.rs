//! Global canonical people and entities (#104, Atlas 5.2–5.3) against Postgres and the HTTP API.
//!
//! Mentions are seeded in the shape the nlp-stack NER producer persists; the inferred person is
//! seeded with a researcher-provided name in the shape #65 persists. Producers are not run here.
//!
//! Requires `DATABASE_URL`; skipped otherwise (run through `bun run verify:postgres`).

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::entities::{
    entity_key, replace_entity_run, EntityMentionInput, EntityModality, EntityRunProvenance,
};
use youtube_corpus::identities::{entity_unit_key, person_unit_key, resolve_identities};
use youtube_corpus::{
    begin_visual_processing_run, upsert_visual_text_track, BeginVisualProcessingRunRequest,
    ProcessingProvenance, UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

struct Ctx {
    pool: PgPool,
    database_url: String,
}

impl Ctx {
    async fn corpus(&self, name: &str) -> Uuid {
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
            .bind(corpus)
            .bind(format!("identities-{corpus}"))
            .bind(name)
            .execute(&self.pool)
            .await
            .unwrap();
        corpus
    }

    /// A video in `corpora` whose on-screen text mentions each `(type, name)`.
    async fn video(&self, corpora: &[Uuid], mentions: &[(&str, &str)]) -> Uuid {
        let video = Uuid::new_v4();
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, 'Identity')")
            .bind(video)
            .bind(format!("https://www.youtube.com/watch?v={video}"))
            .execute(&self.pool)
            .await
            .unwrap();
        for corpus in corpora {
            youtube_corpus::corpora::link_video(&self.pool, *corpus, video)
                .await
                .unwrap();
        }
        let run = begin_visual_processing_run(
            &self.pool,
            BeginVisualProcessingRunRequest {
                video_id: video,
                kind: VisualProcessingKind::Ocr,
                provenance: ProcessingProvenance {
                    processor: "visual-analysis".into(),
                    processor_version: "fixture".into(),
                    model: "ocr".into(),
                    model_version: "1".into(),
                    input_hash: format!("ocr-{video}"),
                    config_hash: "fixture".into(),
                    processing_config: json!({"complete": true}),
                },
            },
        )
        .await
        .unwrap();
        let mut inputs = Vec::new();
        for (index, (entity_type, name)) in mentions.iter().enumerate() {
            let track = upsert_visual_text_track(
                &self.pool,
                UpsertVisualTextTrackRequest {
                    run_id: run,
                    video_id: video,
                    track_key: format!("text-{index}"),
                    text: (*name).into(),
                    role: VisualTextRole::SceneText,
                    language: Some("en".into()),
                    sample_count: 2,
                    start_frame: None,
                    end_frame: None,
                    start_seconds: Some(index as f64),
                    end_seconds: Some(index as f64 + 2.0),
                    region: None,
                    metadata: json!({}),
                    observation_ids: Vec::new(),
                    scene_ids: Vec::new(),
                },
            )
            .await
            .unwrap()
            .id;
            inputs.push(EntityMentionInput {
                modality: EntityModality::VisibleText,
                entity_type: (*entity_type).into(),
                surface_text: (*name).into(),
                entity_key: entity_key(name),
                start_seconds: Some(index as f64),
                end_seconds: Some(index as f64 + 2.0),
                transcript_segment_id: None,
                visual_text_track_id: Some(track),
                byte_start: 0,
                byte_end: name.len() as i32,
                confidence: Some(0.95),
            });
        }
        replace_entity_run(
            &self.pool,
            video,
            &EntityRunProvenance::canonical("dslim/bert-base-NER@fixture"),
            None,
            0,
            mentions.len() as i32,
            &inputs,
        )
        .await
        .unwrap();
        video
    }

    async fn identity_of(&self, unit: &str) -> Uuid {
        sqlx::query_scalar("SELECT identity_id FROM canonical_identity_links WHERE unit_key = $1")
            .bind(unit)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn call(&self, method: Method, uri: String, body: Option<Value>) -> Value {
        let app = youtube_corpus::web::app_for_tests_with_research(Some(self.database_url.clone()));
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn correct(&self, identity: Uuid, body: Value) -> Value {
        self.call(
            Method::POST,
            format!("/api/identities/{identity}/corrections"),
            Some(body),
        )
        .await
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn canonical_identities_link_on_shared_evidence_and_keep_researcher_decisions() {
    let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let ctx = Ctx { pool, database_url };
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let ada = format!("Ada Lovelace {tag}");
    let grace = format!("Grace Hopper {tag}");
    let acme = format!("Acme {tag}");

    let a = ctx.corpus("Identities A").await;
    let b = ctx.corpus("Identities B").await;
    // One video in both corpora: the same mention is evidence in each.
    ctx.video(
        &[a, b],
        &[("person", &ada), ("organization", &acme), ("date", "1843")],
    )
    .await;
    // Homonyms: the same name in unrelated videos of different corpora.
    ctx.video(&[a], &[("person", &grace)]).await;
    ctx.video(&[b], &[("person", &grace)]).await;
    // An inferred corpus-local person in A, named Ada by the researcher.
    let person = Uuid::new_v4();
    sqlx::query("INSERT INTO people (id, corpus_id, label) VALUES ($1, $2, 'Person 1')")
        .bind(person)
        .bind(a)
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO person_name_attributions
           (person_id, corpus_id, name, name_key, status, algorithm, algorithm_version,
            configuration)
         VALUES ($1, $2, $3, $4, 'user_provided', 'researcher-provided name', 'fixture', '{}')",
    )
    .bind(person)
    .bind(a)
    .bind(&ada)
    .bind(entity_key(&ada))
    .execute(&ctx.pool)
    .await
    .unwrap();
    resolve_identities(&ctx.pool).await.unwrap();

    let ada_a = entity_unit_key(a, "person", &entity_key(&ada));
    let ada_b = entity_unit_key(b, "person", &entity_key(&ada));
    let grace_a = entity_unit_key(a, "person", &entity_key(&grace));
    let grace_b = entity_unit_key(b, "person", &entity_key(&grace));
    let ada_id = ctx.identity_of(&ada_a).await;
    assert_eq!(
        ctx.identity_of(&ada_b).await,
        ada_id,
        "shared evidence links"
    );
    assert_eq!(
        ctx.identity_of(&person_unit_key(person)).await,
        ada_id,
        "the named inferred person joins its name's mention group"
    );
    let grace_a_id = ctx.identity_of(&grace_a).await;
    let grace_b_id = ctx.identity_of(&grace_b).await;
    assert_ne!(grace_a_id, grace_b_id, "same name alone never links");
    let dates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM canonical_identity_links WHERE corpus_id = $1 AND entity_type = 'date'",
    )
    .bind(a)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(dates, 0, "dates get no identity");

    // Rerunning is stable.
    resolve_identities(&ctx.pool).await.unwrap();
    assert_eq!(ctx.identity_of(&ada_b).await, ada_id);
    assert_eq!(ctx.identity_of(&grace_a).await, grace_a_id);

    // Scoped and all-corpora listings.
    let people = ctx
        .call(
            Method::GET,
            format!("/api/identities?kind=person&q={}", tag),
            None,
        )
        .await;
    let listed = |list: &Value, id: Uuid| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == json!(id))
            .cloned()
    };
    let ada_summary = listed(&people, ada_id).expect("Ada listed");
    assert_eq!(ada_summary["displayName"], json!(ada));
    assert_eq!(ada_summary["corpora"].as_array().unwrap().len(), 2);
    assert_eq!(ada_summary["localUnits"], json!(3));
    let scoped = ctx
        .call(
            Method::GET,
            format!("/api/identities?kind=person&corpusId={b}&q={tag}"),
            None,
        )
        .await;
    assert!(listed(&scoped, ada_id).is_some());
    assert!(listed(&scoped, grace_b_id).is_some());
    assert!(
        listed(&scoped, grace_a_id).is_none(),
        "scope hides other corpora"
    );
    let entities = ctx
        .call(
            Method::GET,
            format!("/api/identities?kind=entity&q={tag}"),
            None,
        )
        .await;
    assert_eq!(entities.as_array().unwrap().len(), 1);
    assert_eq!(entities[0]["entityType"], json!("organization"));

    // The profile offers the homonym as a reviewable suggestion, with local evidence.
    let profile = ctx
        .call(Method::GET, format!("/api/identities/{grace_a_id}"), None)
        .await;
    assert!(profile["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["identity"]["id"] == json!(grace_b_id)));
    assert_eq!(
        profile["evidence"][0]["mentions"]["videos"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );

    // Reject keeps the suggestion away across reruns; unreject restores it.
    ctx.correct(
        grace_a_id,
        json!({"action": "reject", "otherIdentityId": grace_b_id}),
    )
    .await;
    let profile = ctx
        .call(Method::GET, format!("/api/identities/{grace_a_id}"), None)
        .await;
    assert!(profile["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["identity"]["id"] != json!(grace_b_id)));
    assert_eq!(profile["keptApart"][0]["id"], json!(grace_b_id));
    ctx.correct(
        grace_a_id,
        json!({"action": "unreject", "otherIdentityId": grace_b_id}),
    )
    .await;

    // Merge: B redirects to A and survives reruns.
    ctx.correct(
        grace_a_id,
        json!({"action": "merge", "otherIdentityId": grace_b_id}),
    )
    .await;
    resolve_identities(&ctx.pool).await.unwrap();
    assert_eq!(ctx.identity_of(&grace_b).await, grace_a_id);
    let redirected = ctx
        .call(Method::GET, format!("/api/identities/{grace_b_id}"), None)
        .await;
    assert_eq!(redirected["requestedId"], json!(grace_b_id));
    assert_eq!(redirected["identity"]["id"], json!(grace_a_id));

    // Split undoes it and keeps the units apart across reruns.
    let split = ctx
        .correct(
            grace_a_id,
            json!({"action": "split", "unitKeys": [grace_b]}),
        )
        .await;
    let split_id: Uuid = serde_json::from_value(split["createdIdentityId"].clone()).unwrap();
    resolve_identities(&ctx.pool).await.unwrap();
    assert_eq!(ctx.identity_of(&grace_b).await, split_id);
    assert_eq!(ctx.identity_of(&grace_a).await, grace_a_id);

    // Splitting a shared-evidence link holds against the evidence on rerun.
    let split = ctx
        .correct(ada_id, json!({"action": "split", "unitKeys": [ada_b]}))
        .await;
    let ada_b_id: Uuid = serde_json::from_value(split["createdIdentityId"].clone()).unwrap();
    resolve_identities(&ctx.pool).await.unwrap();
    assert_eq!(ctx.identity_of(&ada_b).await, ada_b_id);
    assert_eq!(ctx.identity_of(&ada_a).await, ada_id);
    // ...and merging back reverses the split.
    ctx.correct(
        ada_id,
        json!({"action": "merge", "otherIdentityId": ada_b_id}),
    )
    .await;
    resolve_identities(&ctx.pool).await.unwrap();
    assert_eq!(ctx.identity_of(&ada_b).await, ada_id);

    // Local evidence is untouched by every correction.
    let mentions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM entity_mentions m
         JOIN corpus_videos cv ON cv.video_id = m.video_id
         WHERE cv.corpus_id IN ($1, $2)",
    )
    .bind(a)
    .bind(b)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(mentions, 3 * 2 + 1 + 1);
    let corrections: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM canonical_identity_corrections
         WHERE identity_id IN ($1, $2)",
    )
    .bind(grace_a_id)
    .bind(ada_id)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(corrections, 6);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn profile_evidence_carries_known_publication_dates_only() {
    let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let ctx = Ctx { pool, database_url };
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let name = format!("Timeline Person {tag}");
    let corpus = ctx.corpus("Timeline").await;
    let dated = ctx.video(&[corpus], &[("person", &name)]).await;
    let stamped = ctx.video(&[corpus], &[("person", &name)]).await;
    let garbled = ctx.video(&[corpus], &[("person", &name)]).await;
    let unknown = ctx.video(&[corpus], &[("person", &name)]).await;
    for (video, upload_date, timestamp) in [
        (dated, Some("20240315"), None),
        // 2021-01-02T03:04:05Z
        (stamped, None, Some(1_609_556_645_i64)),
        (garbled, Some("sometime"), None),
        (unknown, None, None),
    ] {
        sqlx::query("UPDATE videos SET upload_date = $2, timestamp = $3 WHERE id = $1")
            .bind(video)
            .bind(upload_date)
            .bind(timestamp)
            .execute(&ctx.pool)
            .await
            .unwrap();
    }
    resolve_identities(&ctx.pool).await.unwrap();
    let identity = ctx
        .identity_of(&entity_unit_key(corpus, "person", &entity_key(&name)))
        .await;
    let profile = ctx
        .call(
            Method::GET,
            format!("/api/identities/{identity}?corpusId={corpus}"),
            None,
        )
        .await;
    let videos = profile["evidence"][0]["mentions"]["videos"]
        .as_array()
        .unwrap();
    let date_of = |video: Uuid| {
        videos
            .iter()
            .find(|v| v["videoId"] == json!(video))
            .map(|v| v["publicationDate"].clone())
            .unwrap()
    };
    assert_eq!(date_of(dated), json!("2024-03-15"));
    assert_eq!(date_of(stamped), json!("2021-01-02"));
    assert_eq!(
        date_of(garbled),
        Value::Null,
        "unparseable dates stay unknown"
    );
    assert_eq!(date_of(unknown), Value::Null, "missing dates stay unknown");
}
