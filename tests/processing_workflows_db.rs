//! Shared processing workflow definitions run through the corpus plan runner (#108).

use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

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

/// A compiled `@moritzbrantner/workflow/compiled` v1 document as the Workflow Editor emits it.
fn compiled(translate_languages: &[&str]) -> Value {
    let port = |id: &str| json!({ "id": id, "type": { "kind": "any" } });
    json!({
        "format": "@moritzbrantner/workflow/compiled",
        "version": 1,
        "nodes": [
            { "id": "start", "label": "Start", "kind": "control.start", "inputs": [], "outputs": [port("out")], "data": {} },
            { "id": "captions", "label": "Captions", "kind": "youtube_corpus.detector.captions", "inputs": [port("in")], "outputs": [port("out")], "data": {} },
            { "id": "ocr", "label": "OCR", "kind": "youtube_corpus.detector.ocr", "inputs": [port("in")], "outputs": [port("out")], "data": {} },
            { "id": "translate", "label": "Translate OCR", "kind": "youtube_corpus.detector.translate_ocr", "inputs": [port("in")], "outputs": [port("out")], "data": { "targetLanguages": translate_languages } }
        ],
        "edges": [
            { "id": "e1", "sourceNodeId": "start", "sourcePortId": "out", "targetNodeId": "captions", "targetPortId": "in" },
            { "id": "e2", "sourceNodeId": "captions", "sourcePortId": "out", "targetNodeId": "ocr", "targetPortId": "in" },
            { "id": "e3", "sourceNodeId": "ocr", "sourcePortId": "out", "targetNodeId": "translate", "targetPortId": "in" }
        ],
        "order": ["start", "captions", "ocr", "translate"]
    })
}

async fn finished_run(app: &axum::Router, corpus: Uuid, run: &str) -> Value {
    for _ in 0..200 {
        let (status, body) = call(
            app,
            Method::GET,
            &format!("/api/corpora/{corpus}/processing-workflow-runs/{run}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["state"] != "running" {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("workflow run {run} did not finish");
}

fn outcomes(run: &Value) -> Vec<(String, String)> {
    run["report"]["videos"][0]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|stage| {
            (
                stage["detector"].as_str().unwrap().to_string(),
                stage["outcome"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn two_corpora_reuse_one_definition_with_their_own_stages_and_histories() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpora = [Uuid::new_v4(), Uuid::new_v4()];
    for (index, id) in corpora.iter().enumerate() {
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("workflow-{id}"))
            .bind(format!("Workflow corpus {index}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    // One video in both corpora, with current captions already stored.
    let video = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title) VALUES ($1, $2, $3, 'Shared')",
    )
    .bind(video)
    .bind(format!("yt{video}"))
    .bind(format!("https://www.youtube.com/watch?v={video}"))
    .execute(&pool)
    .await
    .unwrap();
    for corpus in corpora {
        sqlx::query("INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)")
            .bind(corpus)
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
    }
    let files = tempfile::tempdir().unwrap();
    let caption = files.path().join("caption.vtt");
    std::fs::write(&caption, "hello").unwrap();
    let stream = Uuid::new_v4();
    let config = json!({
        "caption": { "enabled": true, "include_auto_captions": true, "languages": ["en"] },
        "input": { "path": caption, "sha256": Sha256::digest(b"hello").iter().map(|b| format!("{b:02x}")).collect::<String>() }
    });
    sqlx::query(
        "INSERT INTO transcript_streams
           (id, video_id, source_kind, language, status, processor_revision, processing_config)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed', $3, $4)",
    )
    .bind(stream)
    .bind(video)
    .bind(youtube_corpus::provenance::PROCESSOR_SOURCE_REVISION)
    .bind(config)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_segments (id, video_id, stream_id, segment_index, text)
         VALUES ($1, $2, $3, 0, 'hello')",
    )
    .bind(Uuid::new_v4())
    .bind(video)
    .bind(stream)
    .execute(&pool)
    .await
    .unwrap();

    // A definition with a node this host cannot run is refused, not given invented semantics.
    let mut foreign = compiled(&["de"]);
    foreign["nodes"][2]["kind"] = json!("http.request");
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/processing-workflows",
        Some(json!({ "name": "Foreign", "document": {}, "compiled": foreign })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("cannot run"));

    let document = json!({ "nodes": [], "edges": [] });
    let (status, created) = call(
        &app,
        Method::POST,
        "/api/processing-workflows",
        Some(json!({ "name": "Captions and OCR", "document": document, "compiled": compiled(&["de"]) })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["version"], 1);
    let workflow = created["id"].as_str().unwrap().to_string();
    // Re-saving the same compiled definition keeps its version.
    let (_, same) = call(
        &app,
        Method::PUT,
        &format!("/api/processing-workflows/{workflow}"),
        Some(json!({ "name": "Captions and OCR", "document": document, "compiled": compiled(&["de"]) })),
    )
    .await;
    assert_eq!(same["version"], 1);
    assert_eq!(same["fingerprint"], created["fingerprint"]);

    // The second corpus switches the translation off; the first keeps the definition.
    let base = |corpus: Uuid| format!("/api/corpora/{corpus}/processing-workflows/{workflow}");
    let (status, overridden) = call(
        &app,
        Method::PUT,
        &format!("{}/overrides", base(corpora[1])),
        Some(json!({ "stages": [{ "nodeId": "translate", "enabled": false }] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{overridden}");
    assert_eq!(overridden["stages"][2]["enabled"], false);
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("{}/overrides", base(corpora[1])),
        Some(json!({ "stages": [{ "nodeId": "captions", "targetLanguages": ["de"] }] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut runs = Vec::new();
    for corpus in corpora {
        let (status, run) = call(
            &app,
            Method::POST,
            &format!("{}/runs", base(corpus)),
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{run}");
        runs.push(finished_run(&app, corpus, run["id"].as_str().unwrap()).await);
    }
    assert_eq!(runs[0]["state"], "completed", "{}", runs[0]);
    // The captions the video already has are reused in both corpora: no duplicate work.
    assert_eq!(
        outcomes(&runs[0]),
        [
            ("captions".to_string(), "reused".to_string()),
            ("ocr".to_string(), "unavailable".to_string()),
            ("translate_ocr".to_string(), "skipped".to_string()),
        ]
    );
    assert_eq!(
        outcomes(&runs[1]),
        [
            ("captions".to_string(), "reused".to_string()),
            ("ocr".to_string(), "unavailable".to_string()),
        ]
    );
    assert_eq!(runs[0]["fingerprint"], created["fingerprint"]);

    // The definition became each corpus's detector plan.
    let (_, profiles) = call(
        &app,
        Method::GET,
        &format!("/api/corpora/{}/detector-profiles", corpora[0]),
        None,
    )
    .await;
    assert_eq!(
        profiles["corpus"],
        json!([
            { "detector": "captions", "enabled": true, "targetLanguages": [] },
            { "detector": "ocr", "enabled": true, "targetLanguages": [] },
            { "detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"] }
        ])
    );

    // Each corpus sees only its own history.
    for (corpus, run) in corpora.iter().zip(&runs) {
        let (_, view) = call(&app, Method::GET, &base(*corpus), None).await;
        let ids: Vec<&str> = view["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [run["id"].as_str().unwrap()]);
    }
    let (status, _) = call(
        &app,
        Method::GET,
        &format!(
            "/api/corpora/{}/processing-workflow-runs/{}",
            corpora[1],
            runs[0]["id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Editing the definition creates version 2; the recorded run keeps version 1.
    let (_, edited) = call(
        &app,
        Method::PUT,
        &format!("/api/processing-workflows/{workflow}"),
        Some(json!({ "name": "Captions and OCR", "document": document, "compiled": compiled(&["fr"]) })),
    )
    .await;
    assert_eq!(edited["version"], 2);
    let (_, view) = call(&app, Method::GET, &base(corpora[0]), None).await;
    assert_eq!(view["runs"][0]["workflowVersion"], 1);
    assert_eq!(view["stages"][2]["targetLanguages"], json!(["fr"]));
}
