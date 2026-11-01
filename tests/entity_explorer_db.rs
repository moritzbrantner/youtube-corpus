//! Entity explorer (#62, Decision Atlas 4.1-4.3) against Postgres and the corpus HTTP API.
//!
//! `entity_occurrences_*` seeds producer-shaped mentions through `replace_entity_run` so it
//! runs without the NER model. `real_producer_*` runs the canonical nlp-stack multilingual NER
//! model end to end and is skipped unless `YOUTUBE_CORPUS_NER_MODEL_DIR` points at a
//! directory holding (or allowed to receive) the model bundle.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::entities::{
    replace_entity_run, EntityMentionInput, EntityModality, EntityRunProvenance,
};
use youtube_corpus::{
    begin_visual_processing_run, upsert_visual_text_observation, upsert_visual_text_track,
    BeginVisualProcessingRunRequest, ProcessingProvenance, UpsertVisualTextObservationRequest,
    UpsertVisualTextTrackRequest, VisualProcessingKind, VisualTextRole,
};

struct Seed {
    pool: PgPool,
    app: axum::Router,
    corpus: Uuid,
    spoken_video: Uuid,
    slide_video: Uuid,
    unprocessed_video: Uuid,
    stream: Uuid,
    segments: [Uuid; 2],
    track: Uuid,
}

async fn seed() -> Option<Seed> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let corpus = Uuid::new_v4();
    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, 'Entity explorer test')")
        .bind(corpus)
        .bind(format!("entities-{corpus}"))
        .execute(&pool)
        .await
        .unwrap();
    let spoken_video = Uuid::new_v4();
    let slide_video = Uuid::new_v4();
    let unprocessed_video = Uuid::new_v4();
    let outside_video = Uuid::new_v4();
    for (video, title, member) in [
        (spoken_video, "Engines lecture", true),
        (slide_video, "Analyst slides", true),
        (unprocessed_video, "Not yet analyzed", true),
        (outside_video, "Other corpus", false),
    ] {
        sqlx::query("INSERT INTO videos (id, source_url, title) VALUES ($1, $2, $3)")
            .bind(video)
            .bind(format!("https://www.youtube.com/watch?v={video}"))
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
        if member {
            sqlx::query(
                "INSERT INTO corpus_video_memberships (corpus_id, video_id) VALUES ($1, $2)",
            )
            .bind(corpus)
            .bind(video)
            .execute(&pool)
            .await
            .unwrap();
        }
    }

    let stream = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed')",
    )
    .bind(stream)
    .bind(spoken_video)
    .execute(&pool)
    .await
    .unwrap();
    let segments = [Uuid::new_v4(), Uuid::new_v4()];
    for (index, (id, start, text)) in [
        (segments[0], 12.0, "Today Ada Lovelace explains the engine."),
        (segments[1], 95.0, "Later Ada Lovelace travels to Lisbon."),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO transcript_segments
               (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'en')",
        )
        .bind(id)
        .bind(spoken_video)
        .bind(stream)
        .bind(index as i64)
        .bind(start)
        .bind(start + 4.0)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }

    let ocr_run = begin_visual_processing_run(
        &pool,
        BeginVisualProcessingRunRequest {
            video_id: slide_video,
            kind: VisualProcessingKind::Ocr,
            provenance: ProcessingProvenance {
                processor: "visual-analysis".to_string(),
                processor_version: "test".to_string(),
                model: "ocr".to_string(),
                model_version: "1".to_string(),
                input_hash: "sha256:fixture".to_string(),
                config_hash: "sha256:ocr".to_string(),
                processing_config: json!({}),
            },
        },
    )
    .await
    .unwrap();
    // The same slide text seen on two frames is one track, hence one mention.
    let mut observations = Vec::new();
    for (key, time) in [("f1", 30.0), ("f2", 31.0)] {
        observations.push(
            upsert_visual_text_observation(
                &pool,
                UpsertVisualTextObservationRequest {
                    run_id: ocr_run,
                    video_id: slide_video,
                    observation_key: key.to_string(),
                    text: "Ada Lovelace, Analyst".to_string(),
                    language: Some("en".to_string()),
                    frame_index: None,
                    timestamp_seconds: Some(time),
                    scene_index: None,
                    region: None,
                    confidence: Some(0.9),
                    attributes: json!({}),
                },
            )
            .await
            .unwrap()
            .id,
        );
    }
    let track = upsert_visual_text_track(
        &pool,
        UpsertVisualTextTrackRequest {
            run_id: ocr_run,
            video_id: slide_video,
            track_key: "title".to_string(),
            text: "Ada Lovelace, Analyst".to_string(),
            role: VisualTextRole::PresentationSlide,
            language: Some("en".to_string()),
            sample_count: 2,
            start_frame: None,
            end_frame: None,
            start_seconds: Some(30.0),
            end_seconds: Some(31.0),
            region: None,
            metadata: json!({}),
            observation_ids: observations,
            scene_ids: Vec::new(),
        },
    )
    .await
    .unwrap()
    .id;

    let seed = Seed {
        pool,
        app,
        corpus,
        spoken_video,
        slide_video,
        unprocessed_video,
        stream,
        segments,
        track,
    };
    // A mention in a video outside the corpus must never be counted.
    let (outside_stream, outside_segment) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $2, 'caption_manual', 'en', 'completed')",
    )
    .bind(outside_stream)
    .bind(outside_video)
    .execute(&seed.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_segments (id, video_id, stream_id, segment_index, text)
         VALUES ($1, $2, $3, 0, 'Ada Lovelace elsewhere')",
    )
    .bind(outside_segment)
    .bind(outside_video)
    .bind(outside_stream)
    .execute(&seed.pool)
    .await
    .unwrap();
    store(
        &seed,
        outside_video,
        vec![spoken(outside_segment, 0.0, "Ada Lovelace", "person", 0)],
    )
    .await;
    Some(seed)
}

fn spoken(segment: Uuid, start: f64, surface: &str, kind: &str, at: i32) -> EntityMentionInput {
    EntityMentionInput {
        modality: EntityModality::Spoken,
        entity_type: kind.to_string(),
        surface_text: surface.to_string(),
        entity_key: youtube_corpus::entities::entity_key(surface),
        start_seconds: Some(start),
        end_seconds: Some(start + 4.0),
        transcript_segment_id: Some(segment),
        visual_text_track_id: None,
        byte_start: at,
        byte_end: at + surface.len() as i32,
        confidence: Some(0.99),
    }
}

async fn store(seed: &Seed, video: Uuid, mentions: Vec<EntityMentionInput>) {
    replace_entity_run(
        &seed.pool,
        video,
        &EntityRunProvenance::canonical(
            "Davlan/distilbert-base-multilingual-cased-ner-hrl@fixture",
        ),
        None,
        mentions.len() as i32,
        0,
        &mentions,
    )
    .await
    .unwrap();
}

async fn store_fixture_mentions(seed: &Seed) {
    store(
        seed,
        seed.spoken_video,
        vec![
            spoken(seed.segments[0], 12.0, "Ada Lovelace", "person", 6),
            spoken(seed.segments[1], 95.0, "Ada Lovelace", "person", 6),
            spoken(seed.segments[1], 95.0, "Lisbon", "location", 30),
        ],
    )
    .await;
    store(
        seed,
        seed.slide_video,
        vec![EntityMentionInput {
            modality: EntityModality::VisibleText,
            entity_type: "person".to_string(),
            surface_text: "Ada Lovelace".to_string(),
            entity_key: "ada lovelace".to_string(),
            start_seconds: Some(30.0),
            end_seconds: Some(31.0),
            transcript_segment_id: None,
            visual_text_track_id: Some(seed.track),
            byte_start: 0,
            byte_end: 12,
            confidence: Some(0.97),
        }],
    )
    .await;
}

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Two homonymous inferred people, each linked to a face cluster with one track.
async fn seed_people(seed: &Seed) -> (Uuid, Uuid) {
    let mut people = Vec::new();
    for (video, start) in [(seed.slide_video, 30.0), (seed.spoken_video, 10.0)] {
        let run = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO media_processing_runs
               (id, video_id, modality, processor, processor_version, model, model_version,
                input_hash, config_hash)
             VALUES ($1, $2, 'face', 'visual-analysis', 'test', 'faces', '1', $3, 'cfg')",
        )
        .bind(run)
        .bind(video)
        .bind(run.to_string())
        .execute(&seed.pool)
        .await
        .unwrap();
        let track = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO face_tracks (id, run_id, video_id, track_key, start_seconds, end_seconds)
             VALUES ($1, $2, $3, 'face-1', $4, $5)",
        )
        .bind(track)
        .bind(run)
        .bind(video)
        .bind(start)
        .bind(start + 5.0)
        .execute(&seed.pool)
        .await
        .unwrap();
        let cluster = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO face_clusters (id, corpus_id, algorithm, algorithm_version, config_hash)
             VALUES ($1, $2, 'fixture', '1', 'cfg')",
        )
        .bind(cluster)
        .bind(seed.corpus)
        .execute(&seed.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO face_cluster_members (cluster_id, track_id, similarity) VALUES ($1, $2, 0.91)",
        )
        .bind(cluster)
        .bind(track)
        .execute(&seed.pool)
        .await
        .unwrap();
        let person = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO people (id, corpus_id, label, identity_confidence, identity_source)
             VALUES ($1, $2, 'Ada Lovelace', 0.8, 'inferred')",
        )
        .bind(person)
        .bind(seed.corpus)
        .execute(&seed.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO person_face_clusters (person_id, cluster_id, confidence, source)
             VALUES ($1, $2, 0.8, 'automatic')",
        )
        .bind(person)
        .bind(cluster)
        .execute(&seed.pool)
        .await
        .unwrap();
        people.push(person);
    }
    (people[0], people[1])
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn entity_occurrences_are_counted_per_modality_with_provenance() {
    let Some(seed) = seed().await else {
        return;
    };
    store_fixture_mentions(&seed).await;
    // Re-extracting a video replaces its mentions instead of adding to them.
    store_fixture_mentions(&seed).await;
    let (first_person, second_person) = seed_people(&seed).await;

    let (status, lookup) = get(
        &seed.app,
        &format!("/api/corpora/{}/entities?q=Lovelace", seed.corpus),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lookup}");
    let entities = lookup["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 1, "{lookup}");
    assert_eq!(entities[0]["entityType"], "person");
    assert_eq!(entities[0]["entityKey"], "ada lovelace");
    assert_eq!(entities[0]["displayName"], "Ada Lovelace");
    assert_eq!(entities[0]["spokenMentions"], 2);
    assert_eq!(entities[0]["visibleTextMentions"], 1);
    assert_eq!(entities[0]["videoCount"], 2);

    // Homonymous people stay separate and are reported as their own (inferred) identities.
    let people = lookup["people"].as_array().unwrap();
    assert_eq!(people.len(), 2, "{lookup}");
    let ids: Vec<&str> = people
        .iter()
        .map(|p| p["personId"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&first_person.to_string().as_str()));
    assert!(ids.contains(&second_person.to_string().as_str()));
    for person in people {
        assert_eq!(person["identitySource"], "inferred");
        assert_eq!(person["shownTracks"], 1);
        assert_eq!(person["videoCount"], 1);
    }

    let coverage = &lookup["coverage"];
    assert_eq!(coverage["corpusVideos"], 3);
    assert_eq!(coverage["entityExtractedVideos"], 2);
    assert_eq!(coverage["faceProcessedVideos"], 2);

    let (status, places) = get(
        &seed.app,
        &format!("/api/corpora/{}/entities?q=lisbon", seed.corpus),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(places["entities"][0]["entityType"], "location");
    assert_eq!(places["entities"][0]["spokenMentions"], 1);
    assert_eq!(places["people"].as_array().unwrap().len(), 0);

    let (status, occurrences) = get(
        &seed.app,
        &format!(
            "/api/corpora/{}/entity-occurrences?entityType=person&entityKey=ada%20lovelace",
            seed.corpus
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{occurrences}");
    let videos = occurrences["videos"].as_array().unwrap();
    assert_eq!(videos.len(), 2);
    let spoken_video = videos
        .iter()
        .find(|v| v["videoId"] == seed.spoken_video.to_string())
        .unwrap();
    assert_eq!(spoken_video["spokenMentions"], 2);
    assert_eq!(spoken_video["visibleTextMentions"], 0);
    let moments: Vec<f64> = spoken_video["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["startSeconds"].as_f64().unwrap())
        .collect();
    assert_eq!(moments, [12.0, 95.0]);
    let first = &spoken_video["occurrences"][0];
    assert_eq!(first["modality"], "spoken");
    assert_eq!(first["context"], "Today Ada Lovelace explains the engine.");
    assert_eq!(first["transcriptSegmentId"], seed.segments[0].to_string());
    assert_eq!(first["producer"], "nlp-stack/text-linguistics");
    assert_eq!(
        first["model"],
        "Davlan/distilbert-base-multilingual-cased-ner-hrl@fixture"
    );

    let slide = videos
        .iter()
        .find(|v| v["videoId"] == seed.slide_video.to_string())
        .unwrap();
    assert_eq!(slide["visibleTextMentions"], 1);
    assert_eq!(slide["occurrences"][0]["modality"], "visible_text");
    assert_eq!(
        slide["occurrences"][0]["visualTextTrackId"],
        seed.track.to_string()
    );
    assert_eq!(
        slide["occurrences"][0]["visualTextRole"],
        "presentation_slide"
    );

    let (status, person) = get(
        &seed.app,
        &format!(
            "/api/corpora/{}/people/{first_person}/occurrences",
            seed.corpus
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{person}");
    assert_eq!(person["person"]["identitySource"], "inferred");
    assert_eq!(person["videos"].as_array().unwrap().len(), 1);
    assert_eq!(person["videos"][0]["videoId"], seed.slide_video.to_string());
    assert_eq!(person["videos"][0]["occurrences"][0]["kind"], "shown");
    assert_eq!(person["videos"][0]["occurrences"][0]["startSeconds"], 30.0);
    assert_eq!(
        person["videos"][0]["occurrences"][0]["processor"],
        "visual-analysis"
    );

    let (status, _) = get(
        &seed.app,
        &format!(
            "/api/corpora/{}/people/{}/occurrences",
            seed.corpus,
            Uuid::new_v4()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = (seed.unprocessed_video, seed.stream);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and YOUTUBE_CORPUS_NER_MODEL_DIR"]
async fn real_producer_extracts_spoken_and_visible_mentions() {
    let Ok(model_dir) = std::env::var("YOUTUBE_CORPUS_NER_MODEL_DIR") else {
        return;
    };
    let Some(seed) = seed().await else {
        return;
    };
    let report = youtube_corpus::entities::extract_corpus_entities(
        &seed.pool,
        seed.corpus,
        model_dir.into(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.videos_processed, 2, "{report:?}");
    assert_eq!(report.videos_without_text, 1);

    let (_, lookup) = get(
        &seed.app,
        &format!("/api/corpora/{}/entities?q=ada%20lovelace", seed.corpus),
    )
    .await;
    let person = lookup["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["entityKey"] == "ada lovelace" && e["entityType"] == "person")
        .unwrap_or_else(|| panic!("{lookup}"));
    assert_eq!(person["spokenMentions"], 2);
    assert_eq!(person["visibleTextMentions"], 1);

    let (_, places) = get(
        &seed.app,
        &format!("/api/corpora/{}/entities?q=lisbon", seed.corpus),
    )
    .await;
    assert_eq!(places["entities"][0]["entityType"], "location", "{places}");
}
