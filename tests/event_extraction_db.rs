//! Automatically suggested historical event claims (#107, Atlas 7.1) against Postgres and the
//! HTTP API: source-linked unverified suggestions from speech and OCR, false dates, low
//! confidence, a video without OCR, conflicting sources, the historical timeline, and reruns
//! after review (rejected claims stay rejected, stale suggestions go).
//!
//! The spoken and on-screen text is seeded in the shape transcripts and the OCR pipeline persist;
//! the entity mention is seeded in the shape the nlp-stack NER producer persists.
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
use youtube_corpus::event_extraction::EVENT_CLAIM_PRODUCER;
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
            .bind(format!("event-extraction-{corpus}"))
            .bind(name)
            .execute(&self.pool)
            .await
            .unwrap();
        corpus
    }

    /// A video in `corpus` with one spoken segment per line; returns (video, segments).
    async fn spoken(&self, corpus: Uuid, lines: &[&str]) -> (Uuid, Vec<Uuid>) {
        let video = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO videos (id, source_url, title, upload_date) VALUES ($1, $2, 'Lecture', '20240301')",
        )
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
        let mut segments = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let segment = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO transcript_segments
                   (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(segment)
            .bind(stream)
            .bind(video)
            .bind(index as i32)
            .bind(10.0 * index as f64)
            .bind(10.0 * index as f64 + 6.0)
            .bind(line)
            .execute(&self.pool)
            .await
            .unwrap();
            segments.push(segment);
        }
        (video, segments)
    }

    /// One on-screen text track on `video`.
    async fn ocr(&self, video: Uuid, text: &str) -> Uuid {
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
                track_key: "slide-1".into(),
                text: text.into(),
                role: VisualTextRole::PresentationSlide,
                language: Some("en".into()),
                sample_count: 3,
                start_frame: None,
                end_frame: None,
                start_seconds: Some(42.0),
                end_seconds: Some(48.0),
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

    async fn extract(&self, corpus: Uuid) -> Value {
        self.call(
            Method::POST,
            format!("/api/corpora/{corpus}/events/extract"),
            None,
        )
        .await
    }

    async fn event_named(&self, label: &str) -> Option<Value> {
        let list = self
            .get(format!("/api/events?q={}", label.replace(' ', "%20")))
            .await;
        list.as_array()
            .unwrap()
            .iter()
            .find(|event| event["label"] == label)
            .cloned()
    }
}

fn claims<'a>(profile: &'a Value, predicate: &str) -> Vec<&'a Value> {
    profile["claims"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|claim| claim["predicate"] == predicate)
        .collect()
}

fn by_source<'a>(claims: &[&'a Value], source_text: &str) -> &'a Value {
    claims
        .iter()
        .find(|claim| claim["extraction"]["sourceText"] == source_text)
        .unwrap_or_else(|| panic!("no claim from `{source_text}` in {claims:?}"))
}

/// Letters only, so the invented names stay one word in event labels.
fn letters(seed: Uuid) -> String {
    let tail: String = seed
        .simple()
        .to_string()
        .chars()
        .take(6)
        .map(|c| (b'a' + (c.to_digit(16).unwrap() as u8)) as char)
        .collect();
    format!("Q{tail}")
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn suggested_claims_are_source_linked_unverified_and_survive_review_on_rerun() {
    let Some(database_url) = std::env::var("DATABASE_URL").ok() else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let ctx = Ctx {
        pool,
        database_url: database_url.clone(),
    };
    let place = letters(Uuid::new_v4());
    let council = letters(Uuid::new_v4());
    let person = format!("William {}", letters(Uuid::new_v4()));
    let battle = format!("Battle of {place}");
    let a = ctx.corpus("Extraction A").await;
    let b = ctx.corpus("Extraction B").await;

    let won = format!("{person} won the Battle of {place} on 14 October 1066.");
    let (v1, s1) = ctx
        .spoken(
            a,
            &[
                &won,
                // False dates and a bare year below the confidence threshold.
                "and 1500 people watched the 1148 timestamp",
                "of skeptical atheism Paul Draper 2015",
            ],
        )
        .await;
    let slide = ctx
        .ocr(v1, &format!("The Battle of {place}, 14 Oct 1066"))
        .await;
    // Another source disagrees; it hedges and this video has no on-screen text.
    let (v2, s2) = ctx
        .spoken(
            b,
            &[
                &format!("some chroniclers say the battle of {place} probably happened in 1067"),
                &format!("the Council of {council} met in the 4th century"),
            ],
        )
        .await;

    // The person's mention in the first segment, as nlp-stack persists it, linked globally.
    replace_entity_run(
        &ctx.pool,
        v1,
        &EntityRunProvenance::canonical("dslim/bert-base-NER@fixture"),
        None,
        3,
        1,
        &[EntityMentionInput {
            modality: EntityModality::Spoken,
            entity_type: "person".into(),
            surface_text: person.clone(),
            entity_key: entity_key(&person),
            start_seconds: Some(0.0),
            end_seconds: Some(6.0),
            transcript_segment_id: Some(s1[0]),
            visual_text_track_id: None,
            byte_start: 0,
            byte_end: person.len() as i32,
            confidence: Some(0.98),
        }],
    )
    .await
    .unwrap();
    youtube_corpus::identities::resolve_identities(&ctx.pool)
        .await
        .unwrap();
    let william: Uuid = sqlx::query_scalar(
        "SELECT identity_id FROM canonical_identity_links WHERE corpus_id = $1 AND entity_key = $2",
    )
    .bind(a)
    .bind(entity_key(&person))
    .fetch_one(&ctx.pool)
    .await
    .unwrap();

    let report_a = ctx.extract(a).await;
    assert_eq!(report_a["producer"], EVENT_CLAIM_PRODUCER);
    assert_eq!(report_a["videosProcessed"], 1);
    assert_eq!(report_a["segmentsAnalyzed"], 3);
    assert_eq!(report_a["visualTextTracksAnalyzed"], 1);
    // Two date claims (speech, OCR) and the participant; nothing from the false dates.
    assert_eq!(report_a["claimsSuggested"], 3, "{report_a}");
    assert_eq!(report_a["lowConfidenceDropped"], 1, "{report_a}");
    let report_b = ctx.extract(b).await;
    assert_eq!(report_b["visualTextTracksAnalyzed"], 0);
    assert_eq!(report_b["claimsSuggested"], 2, "{report_b}");

    // Both videos' claims group under one event; their readings disagree.
    let summary = ctx.event_named(&battle).await.expect("battle event");
    assert_eq!(summary["disputed"], true, "{summary}");
    assert_eq!(summary["dateReadings"], 2);
    assert_eq!(summary["videoCount"], 2);
    let event = summary["id"].as_str().unwrap().to_string();
    let profile = ctx.get(format!("/api/events/{event}")).await;
    let dates = claims(&profile, "date");
    assert_eq!(dates.len(), 3, "{profile}");
    for claim in &dates {
        assert_eq!(claim["reviewStatus"], "unverified");
        assert_eq!(claim["preferred"], false);
        assert_eq!(claim["extraction"]["producer"], EVENT_CLAIM_PRODUCER);
        assert_eq!(claim["publicationDate"], "2024-03-01");
    }
    let spoken = by_source(&dates, "14 October 1066");
    assert_eq!(spoken["modality"], "spoken");
    assert_eq!(spoken["evidenceId"], s1[0].to_string());
    assert_eq!(spoken["evidenceText"], won);
    assert_eq!(spoken["startSeconds"], 0.0);
    assert_eq!(
        spoken["date"],
        json!({"startYear": 1066, "startMonth": 10, "startDay": 14,
               "endYear": null, "endMonth": null, "endDay": null, "approximate": false})
    );
    let start = spoken["extraction"]["sourceByteStart"].as_u64().unwrap() as usize;
    let end = spoken["extraction"]["sourceByteEnd"].as_u64().unwrap() as usize;
    assert_eq!(&won[start..end], "14 October 1066");
    let visible = by_source(&dates, "14 Oct 1066");
    assert_eq!(visible["modality"], "visible");
    assert_eq!(visible["evidenceId"], slide.to_string());
    assert_eq!(visible["startSeconds"], 42.0);
    let hedged = by_source(&dates, "1067");
    assert_eq!(hedged["certainty"], "hedged");
    assert_eq!(hedged["videoId"], v2.to_string());
    assert_eq!(hedged["evidenceId"], s2[0].to_string());
    let participants = claims(&profile, "participant");
    assert_eq!(participants.len(), 1, "{profile}");
    assert_eq!(participants[0]["identityId"], william.to_string());
    assert_eq!(participants[0]["certainty"], "inferred");
    assert_eq!(participants[0]["reviewStatus"], "unverified");

    // A century claim is a range; the council is its own event.
    let council_event = ctx
        .event_named(&format!("Council of {council}"))
        .await
        .expect("council event");
    assert_eq!(council_event["earliestYear"], 301);
    assert_eq!(council_event["latestYear"], 400);

    // The historical timeline: by event, by participant and corpus-scoped.
    let by_event = ctx
        .get(format!("/api/events/timeline?eventId={event}"))
        .await;
    assert_eq!(by_event["events"].as_array().unwrap().len(), 1);
    assert_eq!(by_event["events"][0]["claims"].as_array().unwrap().len(), 3);
    let by_identity = ctx
        .get(format!("/api/events/timeline?identityId={william}"))
        .await;
    let ids: Vec<&Value> = by_identity["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["event"]["id"])
        .collect();
    assert_eq!(ids, vec![&json!(event)]);
    let scoped = ctx.get(format!("/api/events/timeline?corpusId={b}")).await;
    let battle_b = scoped["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["event"]["id"] == event)
        .expect("battle in corpus B");
    assert_eq!(battle_b["claims"].as_array().unwrap().len(), 1);
    assert_eq!(battle_b["claims"][0]["videoId"], v2.to_string());

    // A rerun keeps unchanged suggestions in place.
    let rerun = ctx.extract(a).await;
    assert_eq!(rerun["claimsSuggested"], 0, "{rerun}");
    assert_eq!(rerun["claimsKept"], 3);
    let again = ctx.get(format!("/api/events/{event}")).await;
    assert_eq!(claims(&again, "date").len(), 3);
    assert_eq!(
        by_source(&claims(&again, "date"), "14 Oct 1066")["id"],
        visible["id"]
    );

    // Review: confirm the spoken claim, reject the OCR one. Reruns never undo either.
    for (claim, status) in [(&spoken["id"], "confirmed"), (&visible["id"], "rejected")] {
        ctx.call(
            Method::POST,
            format!("/api/events/{event}/corrections"),
            Some(json!({"action": "review", "claimIds": [claim], "reviewStatus": status})),
        )
        .await;
    }
    let rerun = ctx.extract(a).await;
    assert_eq!(rerun["claimsSuggested"], 0, "{rerun}");
    assert_eq!(rerun["reviewedSkipped"], 2);
    assert_eq!(rerun["claimsKept"], 1);
    let reviewed = ctx.get(format!("/api/events/{event}")).await;
    let dates = claims(&reviewed, "date");
    assert_eq!(dates.len(), 3);
    assert_eq!(by_source(&dates, "14 Oct 1066")["reviewStatus"], "rejected");
    assert_eq!(
        by_source(&dates, "14 October 1066")["reviewStatus"],
        "confirmed"
    );
    assert_eq!(by_source(&dates, "1067")["reviewStatus"], "unverified");

    // Reprocessed text: a suggestion the new run no longer finds goes, with its empty event.
    sqlx::query("UPDATE transcript_segments SET text = 'the council met later' WHERE id = $1")
        .bind(s2[1])
        .execute(&ctx.pool)
        .await
        .unwrap();
    let rerun = ctx.extract(b).await;
    assert_eq!(rerun["staleRemoved"], 1, "{rerun}");
    assert!(ctx
        .event_named(&format!("Council of {council}"))
        .await
        .is_none());
    let runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM historical_event_extraction_runs WHERE video_id = $1",
    )
    .bind(v1)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(runs, 3);
}
