//! Detector profiles and effective plans (#63, Atlas 2.3, 2.6, 2.8, 2.10) through the HTTP API.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

struct Seed {
    app: axum::Router,
    corpus: Uuid,
    other_corpus: Uuid,
    playlist_a: Uuid,
    playlist_b: Uuid,
    outside_source: Uuid,
    shared: Uuid,
    direct: Uuid,
}

async fn seed() -> Option<Seed> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));
    let tag = Uuid::new_v4().simple().to_string()[..8].to_string();

    let corpus = Uuid::new_v4();
    let other_corpus = Uuid::new_v4();
    for (id, name) in [(corpus, "Detector corpus"), (other_corpus, "Other corpus")] {
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("detectors-{id}"))
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    let shared = Uuid::new_v4();
    let direct = Uuid::new_v4();
    for (id, title) in [(shared, "Shared"), (direct, "Direct")] {
        sqlx::query(
            "INSERT INTO videos (id, youtube_id, source_url, title) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(format!("yt{id}"))
        .bind(format!("https://www.youtube.com/watch?v={id}"))
        .bind(title)
        .execute(&pool)
        .await
        .unwrap();
    }

    let playlist_a = Uuid::new_v4();
    let playlist_b = Uuid::new_v4();
    let outside_source = Uuid::new_v4();
    for (id, name, corpora) in [
        (playlist_a, "Playlist A", vec![corpus]),
        (playlist_b, "Playlist B", vec![corpus, other_corpus]),
        (outside_source, "Elsewhere", vec![other_corpus]),
    ] {
        sqlx::query(
            "INSERT INTO corpus_subscriptions (id, source_kind, source_url, name, work_dir, caption)
             VALUES ($1, 'playlist', $2, $3, 'use-case-output/test', '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("https://www.youtube.com/playlist?list={tag}{id}"))
        .bind(name)
        .execute(&pool)
        .await
        .unwrap();
        for corpus_id in corpora {
            sqlx::query(
                "INSERT INTO corpus_source_memberships (corpus_id, subscription_id) VALUES ($1, $2)",
            )
            .bind(corpus_id)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO corpus_subscription_items
               (id, subscription_id, source_url, youtube_id, title, status)
             VALUES ($1, $2, $3, $4, 'Shared', 'indexed')",
        )
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(format!("https://www.youtube.com/watch?v={shared}"))
        .bind(format!("yt{shared}"))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
        .bind(corpus)
        .bind(direct)
        .execute(&pool)
        .await
        .unwrap();
    seed_captions(&pool, shared).await;

    Some(Seed {
        app,
        corpus,
        other_corpus,
        playlist_a,
        playlist_b,
        outside_source,
        shared,
        direct,
    })
}

async fn seed_captions(pool: &PgPool, video: Uuid) {
    let stream = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed')",
    )
    .bind(stream)
    .bind(video)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_segments (id, video_id, stream_id, segment_index, text)
         VALUES ($1, $2, $3, 0, 'hello')",
    )
    .bind(Uuid::new_v4())
    .bind(video)
    .bind(stream)
    .execute(pool)
    .await
    .unwrap();
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn put_settings(app: &axum::Router, uri: &str, settings: Value) -> StatusCode {
    call(app, Method::PUT, uri, Some(json!({ "settings": settings })))
        .await
        .0
}

async fn plan(seed: &Seed, corpus: Uuid, video: Uuid) -> Value {
    let (status, plan) = call(
        &seed.app,
        Method::GET,
        &format!("/api/corpora/{corpus}/videos/{video}/detector-plan"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    plan
}

fn detector<'a>(plan: &'a Value, id: &str) -> &'a Value {
    plan["detectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["detector"] == id)
        .unwrap()
}

fn contributors(entry: &Value) -> Vec<String> {
    entry["contributors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn overlapping_profiles_union_with_corpus_scoped_video_overrides() {
    let Some(seed) = seed().await else {
        return;
    };
    let base = format!("/api/corpora/{}", seed.corpus);
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/corpus"),
            json!([{"detector": "captions", "enabled": true}]),
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/sources/{}", seed.playlist_a),
            json!([
                {"detector": "ocr", "enabled": true},
                {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de"]}
            ]),
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/sources/{}", seed.playlist_b),
            json!([
                {"detector": "scenes", "enabled": true},
                {"detector": "ocr", "enabled": true},
                {"detector": "captions", "enabled": false},
                {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["ES", "de"]}
            ]),
        )
        .await,
        StatusCode::NO_CONTENT
    );

    let shared = plan(&seed, seed.corpus, seed.shared).await;
    assert_eq!(shared["sources"].as_array().unwrap().len(), 2, "{shared}");
    assert_eq!(shared["addedDirectly"], false);
    let ocr = detector(&shared, "ocr");
    assert_eq!(ocr["enabled"], true);
    assert_eq!(contributors(ocr), ["Playlist A", "Playlist B"]);
    assert_eq!(ocr["available"], false);
    let scenes = detector(&shared, "scenes");
    assert_eq!(scenes["enabled"], true);
    assert_eq!(contributors(scenes), ["Playlist B"]);
    // B switched captions off, but A still inherits the corpus default: union keeps it.
    let captions = detector(&shared, "captions");
    assert_eq!(captions["enabled"], true);
    assert_eq!(contributors(captions), ["Playlist A"]);
    assert_eq!(captions["contributors"][0]["explicit"], false);
    assert_eq!(
        detector(&shared, "translate_spoken")["targetLanguages"],
        json!(["de", "es"])
    );
    assert_eq!(detector(&shared, "entities")["enabled"], false);
    // Each detector is listed once.
    assert_eq!(
        shared["detectors"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["detector"] == "ocr")
            .count(),
        1
    );

    // A directly added video gets the corpus default.
    let direct = plan(&seed, seed.corpus, seed.direct).await;
    assert_eq!(direct["addedDirectly"], true);
    assert_eq!(detector(&direct, "captions")["enabled"], true);
    assert_eq!(
        contributors(detector(&direct, "captions")),
        ["Detector corpus"]
    );
    assert_eq!(detector(&direct, "ocr")["enabled"], false);

    // A video override applies immediately and only inside this corpus.
    let overrides = format!("{base}/videos/{}/detector-overrides", seed.shared);
    assert_eq!(
        put_settings(
            &seed.app,
            &overrides,
            json!([{"detector": "scenes", "enabled": false}, {"detector": "entities", "enabled": true}]),
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let overridden = plan(&seed, seed.corpus, seed.shared).await;
    let scenes = detector(&overridden, "scenes");
    assert_eq!(scenes["inherited"], true);
    assert_eq!(scenes["videoOverride"], false);
    assert_eq!(scenes["enabled"], false);
    assert_eq!(detector(&overridden, "entities")["enabled"], true);

    let elsewhere = plan(&seed, seed.other_corpus, seed.shared).await;
    assert_eq!(detector(&elsewhere, "scenes")["enabled"], false);
    assert_eq!(detector(&elsewhere, "scenes")["videoOverride"], Value::Null);
    assert_eq!(detector(&elsewhere, "entities")["enabled"], false);
    assert_eq!(detector(&elsewhere, "captions")["enabled"], false);

    // Removing the override restores the inherited plan.
    assert_eq!(
        put_settings(&seed.app, &overrides, json!([])).await,
        StatusCode::NO_CONTENT
    );
    let restored = plan(&seed, seed.corpus, seed.shared).await;
    assert_eq!(detector(&restored, "scenes")["enabled"], true);
    assert_eq!(detector(&restored, "entities")["enabled"], false);

    let (status, profiles) = call(
        &seed.app,
        Method::GET,
        &format!("{base}/detector-profiles"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        profiles["corpus"],
        json!([{"detector": "captions", "enabled": true, "targetLanguages": []}])
    );
    assert_eq!(profiles["sources"].as_array().unwrap().len(), 2);
    assert!(profiles["detectors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["id"] == "ocr" && d["available"] == false));

    // Validation.
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/corpus"),
            json!([{"detector": "nope", "enabled": true}])
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/corpus"),
            json!([{"detector": "ocr", "enabled": true, "targetLanguages": ["de"]}])
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put_settings(
            &seed.app,
            &format!("{base}/detector-profiles/sources/{}", seed.outside_source),
            json!([])
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        put_settings(
            &seed.app,
            &format!(
                "/api/corpora/{}/videos/{}/detector-overrides",
                seed.other_corpus, seed.direct
            ),
            json!([])
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn running_a_plan_reuses_existing_output_and_reports_unavailable_detectors() {
    let Some(seed) = seed().await else {
        return;
    };
    let base = format!("/api/corpora/{}", seed.corpus);
    put_settings(
        &seed.app,
        &format!("{base}/detector-profiles/corpus"),
        json!([
            {"detector": "captions", "enabled": true},
            {"detector": "ocr", "enabled": true},
            {"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}
        ]),
    )
    .await;
    let (status, report) = call(
        &seed.app,
        Method::POST,
        &format!("{base}/videos/{}/detector-plan/run", seed.shared),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    let outcomes: Vec<(String, String)> = report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["detector"].as_str().unwrap().into(),
                s["outcome"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("captions".to_string(), "reused".to_string()),
            ("ocr".to_string(), "unavailable".to_string()),
            ("translate_ocr".to_string(), "unavailable".to_string()),
        ]
    );
    assert!(report["stages"][1]["message"]
        .as_str()
        .unwrap()
        .contains("OCR"));
}
