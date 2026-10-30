//! #103: the Processing page's run inspector. `GET /api/corpora/{corpus}/detector-attempts`
//! lists recorded attempts of the corpus's videos (live `running` vs `interrupted`, superseded
//! vs newest, filters) and `GET .../detector-attempts/{id}` returns one attempt with its
//! recorded identity and stage history. Attempts of videos outside the corpus never leak.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn attempt(
    pool: &PgPool,
    video: Uuid,
    detector: &str,
    state: &str,
    message: Option<&str>,
    pid: i32,
    minutes_ago: i32,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO detector_attempts (id, video_id, detector, identity, state, message, backend_pid, started_at, finished_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() - make_interval(mins => $8),
           CASE WHEN $5 = 'running' THEN NULL ELSE now() - make_interval(mins => $8) + interval '3 seconds' END)",
    )
    .bind(id)
    .bind(video)
    .bind(detector)
    .bind(json!({ "detector": detector, "input": format!("input-{minutes_ago}") }))
    .bind(state)
    .bind(message)
    .bind(pid)
    .bind(minutes_ago)
    .execute(pool)
    .await
    .unwrap();
    id
}

fn ids(body: &Value) -> Vec<String> {
    body.as_array()
        .unwrap()
        .iter()
        .map(|attempt| attempt["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn processing_lists_and_inspects_recorded_attempts_of_corpus_videos() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpus = Uuid::new_v4();
    let other_corpus = Uuid::new_v4();
    for id in [corpus, other_corpus] {
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Processing test')")
            .bind(id)
            .bind(format!("processing-{id}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let video = Uuid::new_v4();
    let outside = Uuid::new_v4();
    for (id, title) in [
        (video, "Processed lecture"),
        (outside, "Other corpus video"),
    ] {
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("https://www.youtube.com/watch?v={}", id.simple()))
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (corpus, video) in [(corpus, video), (other_corpus, outside)] {
        sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
            .bind(corpus)
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }

    // A live owner: this transaction holds the stage lock exactly as a running producer does.
    let mut owner = pool.begin().await.unwrap();
    let locked: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("detector:{video}:faces"))
            .fetch_one(&mut *owner)
            .await
            .unwrap();
    assert!(locked);
    let owner_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *owner)
        .await
        .unwrap();

    let failed = attempt(
        &pool,
        video,
        "entities",
        "failed",
        Some("model missing"),
        1,
        30,
    )
    .await;
    let completed = attempt(&pool, video, "entities", "completed", None, 1, 20).await;
    // Recorded as running by a backend that no longer holds the lock.
    let interrupted = attempt(&pool, video, "scenes", "running", None, -1, 10).await;
    let running = attempt(&pool, video, "faces", "running", None, owner_pid, 5).await;
    let foreign = attempt(&pool, outside, "entities", "failed", Some("x"), 1, 1).await;

    let base = format!("/api/corpora/{corpus}/detector-attempts");
    let (status, all) = get(&app, &base).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_eq!(
        ids(&all),
        [running, interrupted, completed, failed].map(|id| id.to_string())
    );
    let states: Vec<_> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["state"].as_str().unwrap(), a["latest"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        states,
        [
            ("running", true),
            ("interrupted", true),
            ("completed", true),
            ("failed", false)
        ]
    );
    assert_eq!(all[2]["label"], "Named entities");
    assert_eq!(all[2]["videoTitle"], "Processed lecture");

    let (_, latest) = get(&app, &format!("{base}?latest=true")).await;
    assert_eq!(
        ids(&latest),
        [running, interrupted, completed].map(|id| id.to_string())
    );
    let (_, failures) = get(&app, &format!("{base}?state=failed")).await;
    assert_eq!(ids(&failures), [failed.to_string()]);
    let (_, current_failures) = get(&app, &format!("{base}?state=failed&latest=true")).await;
    assert!(ids(&current_failures).is_empty());
    let (_, stale) = get(&app, &format!("{base}?state=interrupted")).await;
    assert_eq!(ids(&stale), [interrupted.to_string()]);
    let (_, stage) = get(&app, &format!("{base}?detector=entities&videoId={video}")).await;
    assert_eq!(ids(&stage), [completed, failed].map(|id| id.to_string()));

    let (status, detail) = get(&app, &format!("{base}/{failed}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["state"], "failed");
    assert_eq!(detail["latest"], false);
    assert_eq!(detail["message"], "model missing");
    assert_eq!(detail["identity"]["input"], "input-30");
    assert_eq!(
        ids(&detail["history"]),
        [completed, failed].map(|id| id.to_string())
    );

    // The owner finishing (or dying) turns the live attempt into an interrupted one.
    owner.rollback().await.unwrap();
    let (_, released) = get(&app, &format!("{base}/{running}")).await;
    assert_eq!(released["state"], "interrupted");

    let (status, _) = get(&app, &format!("{base}/{foreign}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(
        &app,
        &format!("/api/corpora/{other_corpus}/detector-attempts/{failed}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(
        &app,
        &format!("/api/corpora/{}/detector-attempts", Uuid::new_v4()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
