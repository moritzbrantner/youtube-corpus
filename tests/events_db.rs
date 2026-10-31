//! Historical events and separate source-backed claims (#106, Atlas 7.2–7.3) against Postgres
//! and the HTTP API: mutation, readback, corpus filtering, merge/split and a fresh-pool restart.
//!
//! Claims are recorded by a researcher from transcript segments; automatic extraction is S7.
//! Requires `DATABASE_URL`; skipped otherwise (run through `bun run verify:postgres`).

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

struct Ctx {
    pool: PgPool,
    database_url: String,
}

impl Ctx {
    async fn corpus(&self, name: &str) -> Uuid {
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
            .bind(corpus)
            .bind(format!("events-{corpus}"))
            .bind(name)
            .execute(&self.pool)
            .await
            .unwrap();
        corpus
    }

    /// A video in `corpus` with one spoken transcript segment; returns (video, segment).
    async fn spoken(&self, corpus: Uuid, text: &str) -> (Uuid, Uuid) {
        let video = Uuid::new_v4();
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, 'Lecture')")
            .bind(video)
            .bind(format!("https://www.youtube.com/watch?v={video}"))
            .execute(&self.pool)
            .await
            .unwrap();
        youtube_corpus::corpora::link_video(&self.pool, corpus, video)
            .await
            .unwrap();
        let stream = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
             VALUES ($1, $2, 'caption_manual', 'en', 'completed')",
        )
        .bind(stream)
        .bind(video)
        .execute(&self.pool)
        .await
        .unwrap();
        let segment = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transcript_segments
               (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text)
             VALUES ($1, $2, $3, 0, 12.0, 18.5, $4)",
        )
        .bind(segment)
        .bind(stream)
        .bind(video)
        .bind(text)
        .execute(&self.pool)
        .await
        .unwrap();
        (video, segment)
    }

    /// A global person identity with one corpus-local mention group in `corpus`.
    async fn identity(&self, corpus: Uuid, key: &str) -> Uuid {
        let identity = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO canonical_identities (id, kind, entity_type) VALUES ($1, 'person', 'person')",
        )
        .bind(identity)
        .execute(&self.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO canonical_identity_links
               (unit_key, unit_kind, corpus_id, entity_type, entity_key, identity_id, status, basis)
             VALUES ($1, 'entity_group', $2, 'person', $3, $4, 'auto', 'local')",
        )
        .bind(format!("entity:{corpus}:person:{key}"))
        .bind(corpus)
        .bind(key)
        .bind(identity)
        .execute(&self.pool)
        .await
        .unwrap();
        identity
    }

    async fn send(&self, method: Method, uri: String, body: Option<Value>) -> (StatusCode, Value) {
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
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    }

    async fn call(&self, method: Method, uri: String, body: Option<Value>) -> Value {
        let (status, value) = self.send(method, uri, body).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn get(&self, uri: String) -> Value {
        self.call(Method::GET, uri, None).await
    }

    async fn correct(&self, event: &str, body: Value) -> Value {
        self.call(
            Method::POST,
            format!("/api/events/{event}/corrections"),
            Some(body),
        )
        .await
    }
}

fn date_claim(video: Uuid, segment: Uuid, date: Value) -> Value {
    json!({
        "videoId": video, "modality": "spoken", "evidenceId": segment,
        "predicate": "date", "date": date,
    })
}

fn claim<'a>(profile: &'a Value, id: &str) -> &'a Value {
    profile["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|claim| claim["id"] == id)
        .unwrap_or_else(|| panic!("claim {id} missing from {profile}"))
}

fn summary<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list.as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == id)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn event_claims_stay_separate_through_review_merge_split_and_restart() {
    let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let ctx = Ctx {
        pool,
        database_url: database_url.clone(),
    };
    let tag = &Uuid::new_v4().simple().to_string()[..8];
    let label = format!("Battle of Hastings {tag}");
    let a = ctx.corpus("Events A").await;
    let b = ctx.corpus("Events B").await;
    let (v1, s1) = ctx
        .spoken(a, "The battle was fought on 14 October 1066.")
        .await;
    let (v2, s2) = ctx
        .spoken(b, "Some chroniclers place it around 1067.")
        .await;
    let (v3, s3) = ctx
        .spoken(a, "Harald was not at Hastings; he died at Stamford Bridge.")
        .await;
    let william = ctx.identity(a, &format!("william {tag}")).await;
    let harald = ctx.identity(a, &format!("harald {tag}")).await;

    // Create from reviewed evidence, then add a contradictory date and participant claims.
    let created = ctx
        .call(
            Method::POST,
            "/api/events".into(),
            Some(json!({
                "label": label,
                "claim": date_claim(v1, s1, json!({"startYear": 1066, "startMonth": 10, "startDay": 14})),
            })),
        )
        .await;
    let event = created["id"].as_str().unwrap().to_string();
    let add = |body: Value| {
        ctx.call(
            Method::POST,
            format!("/api/events/{event}/claims"),
            Some(body),
        )
    };
    let mut alt = date_claim(v2, s2, json!({"startYear": 1067, "approximate": true}));
    alt["certainty"] = json!("hedged");
    alt["qualification"] = json!("according to some chroniclers");
    let c2 = add(alt).await["id"].as_str().unwrap().to_string();
    let c3 = add(json!({
        "videoId": v1, "modality": "spoken", "evidenceId": s1,
        "predicate": "participant", "identityId": william,
    }))
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c4 = add(json!({
        "videoId": v3, "modality": "spoken", "evidenceId": s3,
        "predicate": "participant", "identityId": harald, "polarity": "affirms",
    }))
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c5 = add(json!({
        "videoId": v3, "modality": "spoken", "evidenceId": s3,
        "predicate": "participant", "identityId": harald, "polarity": "denies",
    }))
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Invalid claims are refused: year 0, evidence from another video, a dated participant.
    for bad in [
        date_claim(v1, s1, json!({"startYear": 0})),
        date_claim(v1, s2, json!({"startYear": 1066})),
        json!({"videoId": v1, "modality": "spoken", "evidenceId": s1,
               "predicate": "participant", "identityId": william, "date": {"startYear": 1066}}),
    ] {
        let (status, body) = ctx
            .send(
                Method::POST,
                format!("/api/events/{event}/claims"),
                Some(bad),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // All corpora: two readings and a denied participant make it disputed.
    let all = ctx.get(format!("/api/events?q={tag}")).await;
    let row = summary(&all, &event).unwrap();
    assert_eq!(row["claimCount"], 5);
    assert_eq!(row["dateReadings"], 2);
    assert_eq!(row["disputed"], true);
    assert_eq!(row["earliestYear"], 1066);
    assert_eq!(row["latestYear"], 1067);
    assert_eq!(row["corpora"].as_array().unwrap().len(), 2);
    // Corpus B sees the same event id with only its own claim.
    let in_b = ctx.get(format!("/api/events?corpusId={b}&q={tag}")).await;
    let row = summary(&in_b, &event).unwrap();
    assert_eq!(row["claimCount"], 1);
    assert_eq!(row["dateReadings"], 1);
    assert_eq!(row["disputed"], false);

    let profile = ctx.get(format!("/api/events/{event}?corpusId={a}")).await;
    assert_eq!(profile["claims"].as_array().unwrap().len(), 4);
    assert_eq!(profile["claimsOutsideScope"], 1);
    let first = &profile["claims"][0];
    assert_eq!(first["predicate"], "date");
    assert_eq!(
        first["evidenceText"],
        "The battle was fought on 14 October 1066."
    );
    assert_eq!(first["startSeconds"], 12.0);
    assert_eq!(first["reviewStatus"], "unverified");
    assert_eq!(claim(&profile, &c3)["identity"]["id"], william.to_string());

    // A reviewed preferred reading leaves the alternative in place; rejecting keeps it visible.
    let c1 = first["id"].as_str().unwrap().to_string();
    ctx.correct(&event, json!({"action": "prefer", "claimIds": [c1]}))
        .await;
    ctx.correct(
        &event,
        json!({"action": "review", "claimIds": [c2], "reviewStatus": "rejected"}),
    )
    .await;
    let profile = ctx.get(format!("/api/events/{event}")).await;
    assert_eq!(profile["claims"].as_array().unwrap().len(), 5);
    assert_eq!(claim(&profile, &c1)["preferred"], true);
    assert_eq!(claim(&profile, &c1)["reviewStatus"], "confirmed");
    assert_eq!(claim(&profile, &c2)["reviewStatus"], "rejected");
    assert_eq!(claim(&profile, &c2)["date"]["startYear"], 1067);
    assert_eq!(
        claim(&profile, &c2)["qualification"],
        "according to some chroniclers"
    );
    assert_eq!(profile["event"]["preferredDate"]["startYear"], 1066);
    assert_eq!(profile["event"]["preferredDate"]["startDay"], 14);
    assert_eq!(profile["event"]["dateReadings"], 1);
    assert_eq!(
        profile["event"]["disputed"], true,
        "Harald is affirmed and denied"
    );

    // Split the Harald claims into their own event, then merge them back.
    let split = ctx
        .correct(
            &event,
            json!({"action": "split", "claimIds": [c4, c5], "label": format!("Stamford Bridge {tag}")}),
        )
        .await;
    let other = split["createdEventId"].as_str().unwrap().to_string();
    let profile = ctx.get(format!("/api/events/{event}")).await;
    assert_eq!(profile["claims"].as_array().unwrap().len(), 3);
    assert_eq!(profile["event"]["disputed"], false);
    let split_profile = ctx.get(format!("/api/events/{other}")).await;
    assert_eq!(split_profile["claims"].as_array().unwrap().len(), 2);
    assert_eq!(split_profile["event"]["disputed"], true);
    // A split that would empty the event is refused.
    let (status, _) = ctx
        .send(
            Method::POST,
            format!("/api/events/{other}/corrections"),
            Some(json!({"action": "split", "claimIds": [c4, c5]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    ctx.correct(&event, json!({"action": "merge", "otherEventId": other}))
        .await;
    let redirected = ctx.get(format!("/api/events/{other}")).await;
    assert_eq!(redirected["requestedId"], other);
    assert_eq!(redirected["event"]["id"], event);
    assert!(summary(&ctx.get(format!("/api/events?q={tag}")).await, &other).is_none());

    // Restart: a fresh pool and app read back the same claims, review and corpus filtering.
    drop(ctx);
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    let ctx = Ctx { pool, database_url };
    let profile = ctx.get(format!("/api/events/{event}")).await;
    assert_eq!(profile["claims"].as_array().unwrap().len(), 5);
    assert_eq!(claim(&profile, &c1)["preferred"], true);
    assert_eq!(claim(&profile, &c2)["reviewStatus"], "rejected");
    assert_eq!(claim(&profile, &c5)["polarity"], "denies");
    assert_eq!(claim(&profile, &c4)["videoId"], v3.to_string());
    let in_b = ctx.get(format!("/api/events/{event}?corpusId={b}")).await;
    assert_eq!(in_b["claims"].as_array().unwrap().len(), 1);
    assert_eq!(in_b["claims"][0]["id"], c2);
    assert_eq!(in_b["claimsOutsideScope"], 4);
    let corrections: i64 =
        sqlx::query_scalar("SELECT count(*) FROM historical_event_corrections WHERE event_id = $1")
            .bind(Uuid::parse_str(&event).unwrap())
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(corrections, 4);
}
