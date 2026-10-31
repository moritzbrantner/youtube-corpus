//! Opt-in translated text (#69; Atlas 3.9, 3.11, 3.13) against Postgres and the corpus HTTP API.
//!
//! Translation runs through the real native-whisperx planner and `translate_timed_text`
//! executor. The deterministic fixture below implements the native provider boundary
//! (`SegmentTranslationProvider`) in place of OPUS-MT model inference; the ignored
//! `native_opus_mt_*` test runs the real provider and downloads its model.
//!
//! Requires `DATABASE_URL`; skipped otherwise (run through `bun run verify:postgres`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use native_whisperx::{SegmentTranslationProvider, TranslationLeg, TranslationModelError};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::detectors::{run_video_plan, PlanRunReport, PlanRunRequest};
use youtube_corpus::translation::{ProviderHandle, TranslationProviderFactory};
use youtube_corpus::{
    begin_visual_processing_run, upsert_visual_text_track, BeginVisualProcessingRunRequest,
    ProcessingProvenance, UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

/// Deterministic stand-in for model inference: each word is reversed and suffixed with the
/// leg's target language, so a translated phrase never occurs in the original text.
struct Fixture {
    calls: Arc<AtomicUsize>,
    fail_target: Option<&'static str>,
}

impl SegmentTranslationProvider for Fixture {
    fn provider_id(&self) -> &str {
        "fixture"
    }

    fn translate_segment(
        &mut self,
        leg: &TranslationLeg,
        text: &str,
    ) -> Result<String, TranslationModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_target == Some(leg.target().code()) {
            return Err(TranslationModelError::new("fixture model unavailable"));
        }
        Ok(text
            .split_whitespace()
            .map(|word| translated_word(word, leg.target().code()))
            .collect::<Vec<_>>()
            .join(" "))
    }
}

fn translated_word(word: &str, target: &str) -> String {
    format!("{}{target}", word.chars().rev().collect::<String>())
}

struct Ctx {
    pool: PgPool,
    database_url: String,
    app: axum::Router,
    corpus: Uuid,
    calls: Arc<AtomicUsize>,
}

impl Ctx {
    async fn new() -> Option<Self> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url.clone()));
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Translations')")
            .bind(corpus)
            .bind(format!("translations-{corpus}"))
            .execute(&pool)
            .await
            .unwrap();
        Some(Self {
            pool,
            database_url,
            app,
            corpus,
            calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    async fn insert_video(&self, title: &str) -> Uuid {
        let video = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO videos (id, youtube_id, source_url, title) VALUES ($1, $2, $3, $4)",
        )
        .bind(video)
        .bind(format!("yt{video}"))
        .bind(format!("https://www.youtube.com/watch?v={video}"))
        .bind(title)
        .execute(&self.pool)
        .await
        .unwrap();
        video
    }

    /// A video added directly to the corpus with one English caption segment at 12-15 s and,
    /// when `visible` is given, one on-screen text track at 20-22 s.
    async fn video(&self, spoken: &str, visible: Option<(&str, &str)>) -> Uuid {
        let video = self.insert_video(spoken).await;
        youtube_corpus::corpora::link_video(&self.pool, self.corpus, video)
            .await
            .unwrap();
        self.caption(video, spoken).await;
        if let Some((text, language)) = visible {
            self.on_screen(video, text, language).await;
        }
        video
    }

    async fn caption(&self, video: Uuid, text: &str) -> Uuid {
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
               (id, video_id, stream_id, segment_index, text, start_seconds, end_seconds)
             VALUES ($1, $2, $3, 0, $4, 12.0, 15.0)",
        )
        .bind(segment)
        .bind(video)
        .bind(stream)
        .bind(text)
        .execute(&self.pool)
        .await
        .unwrap();
        segment
    }

    async fn on_screen(&self, video: Uuid, text: &str, language: &str) -> Uuid {
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
        upsert_visual_text_track(
            &self.pool,
            UpsertVisualTextTrackRequest {
                run_id: run,
                video_id: video,
                track_key: "slide".into(),
                text: text.into(),
                role: VisualTextRole::PresentationSlide,
                language: Some(language.into()),
                sample_count: 2,
                start_frame: None,
                end_frame: None,
                start_seconds: Some(20.0),
                end_seconds: Some(22.0),
                region: None,
                metadata: json!({}),
                observation_ids: Vec::new(),
                scene_ids: Vec::new(),
            },
        )
        .await
        .unwrap()
        .id
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .app
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

    async fn override_video(&self, video: Uuid, settings: Value) {
        let (status, body) = self
            .call(
                Method::PUT,
                &format!(
                    "/api/corpora/{}/videos/{video}/detector-overrides",
                    self.corpus
                ),
                Some(json!({ "settings": settings })),
            )
            .await;
        assert!(status.is_success(), "{status} {body}");
    }

    async fn run(&self, video: Uuid, fail_target: Option<&'static str>) -> PlanRunReport {
        let calls = self.calls.clone();
        run_video_plan(PlanRunRequest {
            database_url: self.database_url.clone(),
            corpus_id: self.corpus,
            video_id: video,
            work_dir: "use-case-output/test".into(),
            yt_dlp: Default::default(),
            model_dir: "use-case-output/test-models".into(),
            refresh: false,
            detectors: None,
            keep_media: false,
            transcriber: None,
            translation_provider: Some(TranslationProviderFactory(Arc::new(move || {
                ProviderHandle {
                    provider: Box::new(Fixture {
                        calls: calls.clone(),
                        fail_target,
                    }),
                    name: "fixture".into(),
                    config: json!({ "fixture": 1 }),
                }
            }))),
        })
        .await
        .unwrap()
    }

    async fn search(&self, query: &str) -> Vec<Value> {
        let (status, body) = self
            .call(
                Method::POST,
                &format!("/api/corpora/{}/search", self.corpus),
                Some(json!({ "query": query, "mode": "fts", "topK": 20 })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["results"].as_array().unwrap().clone()
    }

    async fn status(&self, video: Uuid) -> Vec<Value> {
        let (status, body) = self
            .call(
                Method::GET,
                &format!(
                    "/api/corpora/{}/videos/{video}/processing-status",
                    self.corpus
                ),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["stages"].as_array().unwrap().clone()
    }

    async fn translations(&self, video: Uuid) -> Vec<(String, String, Uuid)> {
        sqlx::query_as(
            "SELECT modality, target_language, id FROM translated_segments
             WHERE video_id = $1 ORDER BY modality, target_language",
        )
        .bind(video)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }
}

fn outcomes(report: &PlanRunReport) -> Vec<(String, String)> {
    report
        .stages
        .iter()
        .map(|stage| {
            (
                stage.detector.clone(),
                serde_json::to_value(stage.outcome)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect()
}

fn target<'a>(stages: &'a [Value], detector: &str, language: &str) -> &'a Value {
    stages
        .iter()
        .find(|s| s["detector"] == detector && s["targetLanguage"] == language)
        .unwrap_or_else(|| panic!("no {detector} → {language} stage in {stages:?}"))
}

fn only(results: &[Value], video: Uuid) -> &Value {
    let hits: Vec<_> = results
        .iter()
        .filter(|r| r["videoId"] == json!(video))
        .collect();
    assert_eq!(hits.len(), 1, "{results:?}");
    hits[0]
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn modalities_are_translated_independently_and_searchable_with_provenance() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    let speech = ctx
        .video("spoken kestrel", Some(("visible plover", "en")))
        .await;
    let ocr = ctx
        .video("spoken osprey", Some(("visible avocet", "en")))
        .await;
    let both = ctx
        .video("spoken heron", Some(("visible curlew", "en")))
        .await;
    let neither = ctx
        .video("spoken falcon", Some(("visible dunlin", "en")))
        .await;
    let speech_only =
        json!([{"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de"]}]);
    ctx.override_video(speech, speech_only.clone()).await;
    ctx.override_video(
        ocr,
        json!([{"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}]),
    )
    .await;
    ctx.override_video(
        both,
        json!([
            {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de"]},
            {"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}
        ]),
    )
    .await;

    let completed = |detector: &str| (detector.to_string(), "completed".to_string());
    assert_eq!(
        outcomes(&ctx.run(speech, None).await),
        [completed("translate_spoken")]
    );
    assert_eq!(
        outcomes(&ctx.run(ocr, None).await),
        [completed("translate_ocr")]
    );
    assert_eq!(
        outcomes(&ctx.run(both, None).await),
        [completed("translate_spoken"), completed("translate_ocr")]
    );
    // Neither enabled: no translation work starts.
    assert!(ctx.run(neither, None).await.stages.is_empty());
    assert_eq!(ctx.calls(), 4);

    let modalities = |rows: Vec<(String, String, Uuid)>| {
        rows.into_iter()
            .map(|(modality, target, _)| format!("{modality}:{target}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(modalities(ctx.translations(speech).await), ["spoken:de"]);
    assert_eq!(modalities(ctx.translations(ocr).await), ["visible_text:de"]);
    assert_eq!(
        modalities(ctx.translations(both).await),
        ["spoken:de", "visible_text:de"]
    );
    assert!(ctx.translations(neither).await.is_empty());
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM translation_runs WHERE video_id = $1")
        .bind(neither)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(runs, 0);

    // A phrase only in the spoken translation finds that video, linked to its original span.
    let hits = ctx.search(&translated_word("kestrel", "de")).await;
    let hit = only(&hits, speech);
    assert_eq!(hit["language"], "de");
    assert_eq!(hit["sourceKind"], "caption_manual");
    assert_eq!(hit["startSeconds"], 12.0);
    assert_eq!(hit["endSeconds"], 15.0);
    assert_eq!(hit["translation"]["modality"], "spoken");
    assert_eq!(hit["translation"]["sourceLanguage"], "en");
    assert_eq!(hit["translation"]["targetLanguage"], "de");
    assert_eq!(hit["translation"]["originalText"], "spoken kestrel");
    assert_eq!(hit["translation"]["provider"], "fixture");
    assert_eq!(
        hit["translation"]["plan"]["legs"][0]["modelId"],
        "Helsinki-NLP/opus-mt-en-de"
    );
    let source_segment: Uuid =
        sqlx::query_scalar("SELECT id FROM transcript_segments WHERE video_id = $1")
            .bind(speech)
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(hit["translation"]["sourceSegmentId"], json!(source_segment));
    // The original phrase stays an untranslated hit.
    let original = ctx.search("kestrel").await;
    assert!(only(&original, speech)["translation"].is_null());

    // A visible-text translation keeps its OCR origin and evidence.
    let hits = ctx.search(&translated_word("avocet", "de")).await;
    let hit = only(&hits, ocr);
    assert_eq!(hit["sourceKind"], "visual_ocr");
    assert_eq!(hit["translation"]["modality"], "visible_text");
    assert_eq!(hit["translation"]["originalText"], "visible avocet");
    assert_eq!(hit["ocrEvidence"]["trackKey"], "slide");
    assert_eq!(hit["startSeconds"], 20.0);

    // Disabled modalities produced nothing searchable.
    for (word, video) in [
        ("plover", speech),
        ("osprey", ocr),
        ("falcon", neither),
        ("dunlin", neither),
    ] {
        let hits = ctx.search(&translated_word(word, "de")).await;
        assert!(
            hits.iter().all(|r| r["videoId"] != json!(video)),
            "{word}: {hits:?}"
        );
    }
    let both_hits = ctx.search(&translated_word("heron", "de")).await;
    assert_eq!(only(&both_hits, both)["translation"]["modality"], "spoken");
    let both_hits = ctx.search(&translated_word("curlew", "de")).await;
    assert_eq!(
        only(&both_hits, both)["translation"]["modality"],
        "visible_text"
    );

    // Effective choice and per-target state are visible per video.
    let status = ctx.status(speech).await;
    assert_eq!(
        target(&status, "translate_spoken", "de")["state"],
        "completed"
    );
    assert_eq!(target(&status, "translate_spoken", "de")["outputCount"], 1);
    assert!(status
        .iter()
        .any(|s| s["detector"] == "translate_ocr" && s["state"] == "disabled"));

    // Unchanged rerun reuses everything.
    let ids = ctx.translations(both).await;
    let rerun = ctx.run(both, None).await;
    assert_eq!(
        outcomes(&rerun),
        [
            ("translate_spoken".into(), "reused".into()),
            ("translate_ocr".into(), "reused".into())
        ]
    );
    assert_eq!(ctx.calls(), 4);
    assert_eq!(ctx.translations(both).await, ids);

    // Enabling OCR on the speech-only video translates only the OCR span.
    let spoken_rows = ctx.translations(speech).await;
    ctx.override_video(
        speech,
        json!([
            {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de"]},
            {"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}
        ]),
    )
    .await;
    ctx.run(speech, None).await;
    assert_eq!(ctx.calls(), 5);
    let rows = ctx.translations(speech).await;
    assert_eq!(rows[0], spoken_rows[0]);
    assert_eq!(rows[1].0, "visible_text");

    // Changed source text: the stale translation stops matching and only that span reruns.
    sqlx::query("UPDATE transcript_segments SET text = 'spoken merlin' WHERE video_id = $1")
        .bind(speech)
        .execute(&ctx.pool)
        .await
        .unwrap();
    let stale = ctx.search(&translated_word("kestrel", "de")).await;
    assert!(stale.iter().all(|r| r["videoId"] != json!(speech)));
    assert_eq!(
        target(&ctx.status(speech).await, "translate_spoken", "de")["state"],
        "missing"
    );
    ctx.run(speech, None).await;
    assert_eq!(ctx.calls(), 6);
    only(&ctx.search(&translated_word("merlin", "de")).await, speech);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn multiple_targets_are_inherited_deduplicated_and_added_incrementally() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    // Two overlapping sources of the corpus discover the same video.
    let shared = ctx.insert_video("Shared").await;
    ctx.caption(shared, "spoken lapwing").await;
    let tag = Uuid::new_v4().simple().to_string();
    let mut sources = Vec::new();
    for name in ["Channel", "Playlist"] {
        let source = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO corpus_subscriptions (id, source_kind, source_url, name, work_dir, caption)
             VALUES ($1, 'playlist', $2, $3, 'use-case-output/test', '{}'::jsonb)",
        )
        .bind(source)
        .bind(format!("https://www.youtube.com/playlist?list={tag}{source}"))
        .bind(name)
        .execute(&ctx.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO corpus_source_memberships (corpus_id, subscription_id) VALUES ($1, $2)",
        )
        .bind(ctx.corpus)
        .bind(source)
        .execute(&ctx.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO corpus_subscription_items (id, subscription_id, source_url, youtube_id, title, status)
             VALUES ($1, $2, $3, $4, 'Shared', 'indexed')",
        )
        .bind(Uuid::new_v4())
        .bind(source)
        .bind(format!("https://www.youtube.com/watch?v={shared}"))
        .bind(format!("yt{shared}"))
        .execute(&ctx.pool)
        .await
        .unwrap();
        sources.push(source);
    }
    let base = format!("/api/corpora/{}", ctx.corpus);
    // Corpus default selects two targets; one source overrides with its own set.
    let (status, _) = ctx
        .call(
            Method::PUT,
            &format!("{base}/detector-profiles/corpus"),
            Some(json!({"settings": [
                {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de", "fr"]}
            ]})),
        )
        .await;
    assert!(status.is_success());
    let source_settings = |languages: Vec<&str>| {
        json!({"settings": [
            {"detector": "translate_spoken", "enabled": true, "targetLanguages": languages}
        ]})
    };
    let (status, _) = ctx
        .call(
            Method::PUT,
            &format!("{base}/detector-profiles/sources/{}", sources[1]),
            Some(source_settings(vec!["de", "it"])),
        )
        .await;
    assert!(status.is_success());

    let (_, plan) = ctx
        .call(
            Method::GET,
            &format!("{base}/videos/{shared}/detector-plan"),
            None,
        )
        .await;
    let planned = plan["detectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["detector"] == "translate_spoken")
        .unwrap();
    assert_eq!(planned["targetLanguages"], json!(["de", "fr", "it"]));
    assert_eq!(planned["contributors"].as_array().unwrap().len(), 2);

    ctx.run(shared, None).await;
    // The shared target `de` is translated once, not once per requesting source.
    assert_eq!(ctx.calls(), 3);
    let rows = ctx.translations(shared).await;
    let targets: Vec<_> = rows.iter().map(|(_, t, _)| t.as_str()).collect();
    assert_eq!(targets, ["de", "fr", "it"]);
    for language in ["de", "fr", "it"] {
        let hit = only(
            &ctx.search(&translated_word("lapwing", language)).await,
            shared,
        )
        .clone();
        assert_eq!(hit["translation"]["targetLanguage"], language);
        assert_eq!(hit["translation"]["sourceLanguage"], "en");
        assert_eq!(hit["translation"]["originalText"], "spoken lapwing");
    }
    // Unselected languages are absent.
    assert!(ctx
        .search(&translated_word("lapwing", "es"))
        .await
        .is_empty());
    let status = ctx.status(shared).await;
    for language in ["de", "fr", "it"] {
        assert_eq!(
            target(&status, "translate_spoken", language)["state"],
            "completed"
        );
    }
    assert!(status
        .iter()
        .all(|s| s["detector"] != "translate_spoken" || s["targetLanguage"] != "es"));

    // Adding a language generates only that target.
    let (status, _) = ctx
        .call(
            Method::PUT,
            &format!("{base}/detector-profiles/sources/{}", sources[1]),
            Some(source_settings(vec!["de", "it", "pt"])),
        )
        .await;
    assert!(status.is_success());
    ctx.run(shared, None).await;
    assert_eq!(ctx.calls(), 4);
    let after = ctx.translations(shared).await;
    assert_eq!(after.len(), 4);
    for row in &rows {
        assert!(after.contains(row), "{row:?} was regenerated");
    }

    // An unsupported target is reported unavailable, never as translated.
    ctx.override_video(
        shared,
        json!([{"detector": "translate_spoken", "enabled": true, "targetLanguages": ["ja"]}]),
    )
    .await;
    let report = ctx.run(shared, None).await;
    assert_eq!(
        outcomes(&report),
        [("translate_spoken".into(), "unavailable".into())]
    );
    assert_eq!(ctx.calls(), 4);
    let ja = target(&ctx.status(shared).await, "translate_spoken", "ja").clone();
    assert_eq!(ja["state"], "unavailable");
    assert_eq!(ja["outputCount"], 0);
    assert!(ja["message"].as_str().unwrap().contains("not supported"));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn failed_missing_and_unsupported_sources_are_never_reported_translated() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    // One target fails in the provider; the other completes.
    let video = ctx.video("spoken sanderling", None).await;
    ctx.override_video(
        video,
        json!([
            {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de", "es"]},
            {"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}
        ]),
    )
    .await;
    let report = ctx.run(video, Some("es")).await;
    assert_eq!(
        outcomes(&report),
        [
            ("translate_spoken".into(), "failed".into()),
            ("translate_ocr".into(), "skipped".into())
        ]
    );
    let status = ctx.status(video).await;
    assert_eq!(
        target(&status, "translate_spoken", "de")["state"],
        "completed"
    );
    let es = target(&status, "translate_spoken", "es");
    assert_eq!(es["state"], "failed");
    assert_eq!(es["outputCount"], 0);
    assert!(es["message"]
        .as_str()
        .unwrap()
        .contains("fixture model unavailable"));
    // No OCR evidence exists for this video.
    assert_eq!(
        target(&status, "translate_ocr", "de")["state"],
        "missing_source"
    );
    assert!(ctx
        .search(&translated_word("sanderling", "es"))
        .await
        .is_empty());
    only(
        &ctx.search(&translated_word("sanderling", "de")).await,
        video,
    );

    // A source span in a language the provider cannot read is unavailable.
    let japanese = ctx
        .video("spoken whimbrel", Some(("看板 テキスト", "ja")))
        .await;
    ctx.override_video(
        japanese,
        json!([{"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}]),
    )
    .await;
    let before = ctx.calls();
    let report = ctx.run(japanese, None).await;
    assert_eq!(
        outcomes(&report),
        [("translate_ocr".into(), "unavailable".into())]
    );
    assert_eq!(ctx.calls(), before);
    let status = ctx.status(japanese).await;
    let ocr = target(&status, "translate_ocr", "de");
    assert_eq!(ocr["state"], "unavailable");
    assert_eq!(ocr["outputCount"], 0);
    assert!(ctx.translations(japanese).await.is_empty());
}

/// Real provider run: downloads `Helsinki-NLP/opus-mt-en-de` into
/// `YOUTUBE_CORPUS_TRANSLATION_MODEL_DIR` (or `use-case-output/models/translation`).
#[tokio::test]
#[ignore = "requires DATABASE_URL and downloads the OPUS-MT en-de model"]
async fn native_opus_mt_translation_is_indexed_and_searchable() {
    let Some(ctx) = Ctx::new().await else {
        return;
    };
    let video = ctx
        .video(
            "The weather is beautiful today.",
            Some(("Good morning", "en")),
        )
        .await;
    ctx.override_video(
        video,
        json!([
            {"detector": "translate_spoken", "enabled": true, "targetLanguages": ["de"]},
            {"detector": "translate_ocr", "enabled": true, "targetLanguages": ["de"]}
        ]),
    )
    .await;
    let model_dir = std::env::var("YOUTUBE_CORPUS_TRANSLATION_MODEL_DIR")
        .unwrap_or_else(|_| "use-case-output/models".into());
    let report = run_video_plan(PlanRunRequest {
        database_url: ctx.database_url.clone(),
        corpus_id: ctx.corpus,
        video_id: video,
        work_dir: "use-case-output/test".into(),
        yt_dlp: Default::default(),
        model_dir: model_dir.into(),
        refresh: false,
        detectors: None,
        translation_provider: None,
        keep_media: false,
        transcriber: None,
    })
    .await
    .unwrap();
    assert_eq!(
        outcomes(&report),
        [
            ("translate_spoken".into(), "completed".into()),
            ("translate_ocr".into(), "completed".into())
        ],
        "{report:?}"
    );
    let (spoken, visible): (String, String) = sqlx::query_as(
        "SELECT max(text) FILTER (WHERE modality = 'spoken'), max(text) FILTER (WHERE modality = 'visible_text')
         FROM translated_segments WHERE video_id = $1",
    )
    .bind(video)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    eprintln!("spoken → {spoken}\nvisible → {visible}");
    let word = spoken
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| w.len() > 4 && !"The weather is beautiful today".contains(*w))
        .expect("a German word absent from the source");
    let hit = only(&ctx.search(word).await, video).clone();
    assert_eq!(
        hit["translation"]["provider"],
        "native-whisperx marian-candle"
    );
    assert_eq!(hit["translation"]["targetLanguage"], "de");
    assert_eq!(
        hit["translation"]["originalText"],
        "The weather is beautiful today."
    );
}
