//! #64 contract checks use synthetic non-biometric 128-D vectors. Actual face
//! extraction is exercised separately by the optional native producer test.
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::multimodal::*;

struct Fixture {
    pool: PgPool,
    app: axum::Router,
    corpus: Uuid,
}
impl Fixture {
    async fn new() -> Option<Self> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let pool = youtube_corpus::db::connect(&url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora(id,slug,name) VALUES($1,$2,'Face matching contract')")
            .bind(corpus)
            .bind(format!("face-matches-{corpus}"))
            .execute(&pool)
            .await
            .unwrap();
        Some(Self {
            pool,
            app: youtube_corpus::web::app_for_tests_with_research(Some(url)),
            corpus,
        })
    }
    async fn evidence(&self, video: Option<Uuid>, xy: [f32; 2], supported: bool) -> (Uuid, Uuid) {
        let video = if let Some(id) = video {
            id
        } else {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO videos(id,source_url,title) VALUES($1,$2,'Synthetic face evidence')",
            )
            .bind(id)
            .bind(format!("https://www.youtube.com/watch?v={id}"))
            .execute(&self.pool)
            .await
            .unwrap();
            sqlx::query("INSERT INTO corpus_video_memberships(corpus_id,video_id) VALUES($1,$2)")
                .bind(self.corpus)
                .bind(id)
                .execute(&self.pool)
                .await
                .unwrap();
            id
        };
        let run = begin_processing_run(
            &self.pool,
            BeginProcessingRunRequest {
                video_id: video,
                modality: MediaModality::Face,
                provenance: ProcessingProvenance {
                    processor: youtube_corpus::face_processing::FACE_PROCESSOR.into(),
                    processor_version: "synthetic-contract-fixture".into(),
                    model: youtube_corpus::face_processing::FACE_MODEL.into(),
                    model_version: if supported {
                        youtube_corpus::face_processing::FACE_MODEL_VERSION.into()
                    } else {
                        "incompatible-model-version".into()
                    },
                    input_hash: format!("fixture:{}", Uuid::new_v4()),
                    config_hash: "synthetic-fixture".into(),
                    processing_config: json!({"complete":true,"fixture":"non-biometric"}),
                },
            },
        )
        .await
        .unwrap();
        let mut embedding = vec![0.0; 128];
        embedding[0] = xy[0];
        embedding[1] = xy[1];
        let track = upsert_face_track(
            &self.pool,
            UpsertFaceTrackRequest {
                run_id: run.id,
                video_id: video,
                track_key: "synthetic-face".into(),
                start_seconds: 12.0,
                end_seconds: 13.0,
                representative_embedding: Some(embedding.clone()),
                quality: json!({"fixture":true}),
            },
        )
        .await
        .unwrap();
        for frame in [12, 13] {
            let observation = upsert_face_observation(
                &self.pool,
                UpsertFaceObservationRequest {
                    run_id: run.id,
                    video_id: video,
                    observation_key: format!("synthetic-frame-{frame}"),
                    start_seconds: frame as f64,
                    end_seconds: None,
                    frame_index: Some(frame),
                    region: BoundingBox {
                        x: 0.1,
                        y: 0.1,
                        width: 0.2,
                        height: 0.2,
                    },
                    detection_score: Some(0.99),
                    embedding: Some(embedding.clone()),
                    quality: json!({"fixture":true}),
                },
            )
            .await
            .unwrap();
            link_face_observation_to_track(&self.pool, track.id, observation.id)
                .await
                .unwrap();
        }
        (video, track.id)
    }
    async fn request(
        &self,
        method: Method,
        suffix: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/corpora/{}/{}", self.corpus, suffix))
            .header("content-type", "application/json")
            .body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            },
        )
    }
    async fn matching(&self) -> Value {
        let (status, body) = self.request(Method::POST, "face-matches/run", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
    async fn correct(&self, action: &str, tracks: &[Uuid], person: Option<Uuid>) -> Value {
        let (status, body) = self
            .request(
                Method::POST,
                "face-matches/corrections",
                Some(json!({"action":action,"trackIds":tracks,"personId":person})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
}
fn ids(group: &Value) -> Vec<Uuid> {
    group["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Uuid::parse_str(t["trackId"].as_str().unwrap()).unwrap())
        .collect()
}
fn person(group: &Value) -> Uuid {
    Uuid::parse_str(group["personId"].as_str().unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn compatible_faces_link_automatically_while_negative_and_incompatible_evidence_stays_unresolved(
) {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let (_, a) = f.evidence(None, [1.0, 0.0], true).await;
    let (_, b) = f.evidence(None, [1.0, 0.0], true).await;
    f.evidence(None, [0.0, 1.0], true).await;
    f.evidence(None, [1.0, 0.0], false).await;
    let first = f.matching().await;
    assert_eq!(first["groups"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["unresolved"].as_array().unwrap().len(), 2);
    let group = &first["groups"][0];
    let linked = ids(group);
    assert!(linked.contains(&a) && linked.contains(&b));
    assert_eq!(group["identitySource"], "automatic-inferred-face");
    for track in group["tracks"].as_array().unwrap() {
        assert_eq!(track["similarity"], 1.0);
        assert_eq!(track["matchingConfiguration"]["minCosineSimilarity"], 0.75);
        assert_eq!(track["producerConfig"]["fixture"], "non-biometric");
        assert!(
            track.get("embedding").is_none(),
            "raw vectors must stay out of the review response"
        );
    }
    let repeated = f.matching().await;
    assert_eq!(repeated["groups"][0]["personId"], group["personId"]);
    assert_eq!(ids(&repeated["groups"][0]).len(), 2);
    let (_, entities) = f.request(Method::GET, "entities?q=", None).await;
    assert_eq!(entities["people"][0]["videoCount"], 2);
    let (_, occurrences) = f
        .request(
            Method::GET,
            &format!("people/{}/occurrences", person(group)),
            None,
        )
        .await;
    assert_eq!(occurrences["videos"].as_array().unwrap().len(), 2);
    assert_eq!(
        occurrences["videos"][0]["occurrences"][0]["startSeconds"],
        12.0
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn ambiguous_similarity_chains_never_turn_into_an_inferred_person() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    f.evidence(None, [1.0, 0.0], true).await;
    f.evidence(None, [0.8, 0.6], true).await;
    f.evidence(None, [0.8, -0.6], true).await;
    let report = f.matching().await;
    assert_eq!(report["groups"].as_array().unwrap().len(), 0, "{report}");
    assert_eq!(report["unresolved"].as_array().unwrap().len(), 3);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn split_unlink_link_and_confirmation_survive_reruns_and_update_existing_entity_occurrences()
{
    let Some(f) = Fixture::new().await else {
        return;
    };
    let (_, a) = f.evidence(None, [1.0, 0.0], true).await;
    let (_, b) = f.evidence(None, [1.0, 0.0], true).await;
    let (_, c) = f.evidence(None, [1.0, 0.0], true).await;
    let initial = f.matching().await;
    let original = person(&initial["groups"][0]);
    let split = f.correct("split", &[c], None).await;
    let new_person = split["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(person)
        .find(|id| *id != original)
        .unwrap();
    let repeated = f.matching().await;
    assert_eq!(repeated["groups"].as_array().unwrap().len(), 2);
    let original_group = repeated["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| person(g) == original)
        .unwrap();
    assert_eq!(ids(original_group).len(), 2);
    assert!(!ids(original_group).contains(&c));
    f.correct("unlink", &[b], None).await;
    let unlinked = f.matching().await;
    assert!(unlinked["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["trackId"] == b.to_string() && t["protected"] == true));
    f.correct("link", &[b], Some(new_person)).await;
    f.correct("confirm", &[b, c], Some(new_person)).await;
    let final_report = f.matching().await;
    let group = final_report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| person(g) == new_person)
        .unwrap();
    assert_eq!(ids(group).len(), 2);
    assert_eq!(group["identitySource"], "user-confirmed corpus association");
    assert!(group["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["protected"] == true && t["similarity"].is_null()));
    let (_, entities) = f.request(Method::GET, "entities?q=", None).await;
    let current = entities["people"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["personId"] == new_person.to_string())
        .unwrap();
    assert_eq!(current["shownTracks"], 2);
    assert_eq!(current["videoCount"], 2);
    let (_, remaining) = f
        .request(Method::GET, &format!("people/{original}/occurrences"), None)
        .await;
    assert_eq!(remaining["person"]["shownTracks"], 1);
    assert_eq!(
        remaining["videos"][0]["occurrences"][0]["trackId"],
        a.to_string()
    );
    let observations:i64=sqlx::query_scalar("SELECT count(*) FROM face_observations o JOIN corpus_video_memberships m ON m.video_id=o.video_id WHERE m.corpus_id=$1").bind(f.corpus).fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        observations, 6,
        "correction must retain original biometric evidence"
    );
    let history: i64 =
        sqlx::query_scalar("SELECT count(*) FROM face_match_corrections WHERE corpus_id=$1")
            .bind(f.corpus)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(history, 4);
    let outsider = Fixture::new().await.unwrap();
    let (_, outside_track) = outsider.evidence(None, [1.0, 0.0], true).await;
    let (status, _) = f
        .request(
            Method::POST,
            "face-matches/corrections",
            Some(json!({"action":"link","trackIds":[outside_track],"personId":new_person})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = f
        .request(
            Method::POST,
            "face-matches/corrections",
            Some(json!({"action":"unlink","trackIds":[]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn compatible_reprocessing_replaces_automatic_membership_without_deleting_old_source_evidence(
) {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let (video, old) = f.evidence(None, [1.0, 0.0], true).await;
    f.evidence(None, [1.0, 0.0], true).await;
    let first = f.matching().await;
    let original = person(&first["groups"][0]);
    let (_, replacement) = f.evidence(Some(video), [1.0, 0.0], true).await;
    let report = f.matching().await;
    assert_eq!(report["groups"].as_array().unwrap().len(), 1);
    assert_eq!(person(&report["groups"][0]), original);
    let current = ids(&report["groups"][0]);
    assert_eq!(current.len(), 2);
    assert!(current.contains(&replacement));
    assert!(!current.contains(&old));
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM face_tracks WHERE id=$1)")
            .bind(old)
            .fetch_one(&f.pool)
            .await
            .unwrap()
    );
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn a_candidate_with_two_equally_supported_existing_people_stays_unresolved() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    for _ in 0..2 {
        f.evidence(None, [1.0, 0.0], true).await;
        f.evidence(None, [0.6, 0.8], true).await;
    }
    assert_eq!(f.matching().await["groups"].as_array().unwrap().len(), 2);
    let (_, ambiguous) = f.evidence(None, [0.8944272, 0.4472136], true).await;
    let report = f.matching().await;
    assert_eq!(report["groups"].as_array().unwrap().len(), 2);
    assert!(report["unresolved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["trackId"] == ambiguous.to_string()));
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn simultaneous_faces_in_one_run_are_contradictory_evidence() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let (video, a) = f.evidence(None, [1.0, 0.0], true).await;
    let run: Uuid = sqlx::query_scalar("SELECT run_id FROM face_tracks WHERE id=$1")
        .bind(a)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut embedding = vec![0.0; 128];
    embedding[0] = 1.0;
    let second = upsert_face_track(
        &f.pool,
        UpsertFaceTrackRequest {
            run_id: run,
            video_id: video,
            track_key: "second-simultaneous-face".into(),
            start_seconds: 12.0,
            end_seconds: 13.0,
            representative_embedding: Some(embedding.clone()),
            quality: json!({"fixture":true}),
        },
    )
    .await
    .unwrap();
    for frame in [12, 13] {
        let observation = upsert_face_observation(
            &f.pool,
            UpsertFaceObservationRequest {
                run_id: run,
                video_id: video,
                observation_key: format!("second-{frame}"),
                start_seconds: frame as f64,
                end_seconds: None,
                frame_index: Some(frame),
                region: BoundingBox {
                    x: 0.5,
                    y: 0.1,
                    width: 0.2,
                    height: 0.2,
                },
                detection_score: Some(0.99),
                embedding: Some(embedding.clone()),
                quality: json!({"fixture":true}),
            },
        )
        .await
        .unwrap();
        link_face_observation_to_track(&f.pool, second.id, observation.id)
            .await
            .unwrap();
    }
    f.evidence(None, [1.0, 0.0], true).await;
    let report = f.matching().await;
    assert_eq!(report["groups"].as_array().unwrap().len(), 0, "{report}");
    assert_eq!(report["unresolved"].as_array().unwrap().len(), 3);
}

#[tokio::test]
#[ignore = "requires DATABASE_URL, CORPUS_FACE_TEST_MEDIA, CORPUS_FACE_TEST_MODELS and ONNX Runtime API 24"]
async fn native_face_output_matches_across_videos_and_honors_a_correction_after_refresh() {
    use youtube_corpus::detectors::{run_video_plan, PlanRunRequest, StageOutcome};
    use youtube_corpus::face_matching::{
        correct_face_matches, review_face_matches, FaceCorrectionAction, FaceCorrectionRequest,
    };
    let Ok(media) = std::env::var("CORPUS_FACE_TEST_MEDIA") else {
        return;
    };
    let models = std::env::var("CORPUS_FACE_TEST_MODELS").expect("face model cache root");
    let f = Fixture::new().await.expect("DATABASE_URL");
    let (status, _) = f
        .request(
            Method::PUT,
            "detector-profiles/corpus",
            Some(json!({"settings":[{"detector":"faces","enabled":true}]})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mut requests = Vec::new();
    for index in 0..2 {
        let video = Uuid::new_v4();
        sqlx::query("INSERT INTO videos(id,source_url,title,local_video_path) VALUES($1,$2,$3,$4)")
            .bind(video)
            .bind(format!("https://www.youtube.com/watch?v={video}"))
            .bind(format!("Native repeat-face reference {index}"))
            .bind(&media)
            .execute(&f.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO corpus_video_memberships(corpus_id,video_id) VALUES($1,$2)")
            .bind(f.corpus)
            .bind(video)
            .execute(&f.pool)
            .await
            .unwrap();
        let request = PlanRunRequest {
            database_url: std::env::var("DATABASE_URL").unwrap(),
            corpus_id: f.corpus,
            video_id: video,
            work_dir: std::env::temp_dir(),
            yt_dlp: Default::default(),
            model_dir: models.clone().into(),
            refresh: false,
            detectors: Some(vec!["faces".into()]),
            translation_provider: None,
            keep_media: false,
            transcriber: None,
        };
        let native = run_video_plan(request.clone()).await.unwrap();
        assert!(
            matches!(native.stages[0].outcome, StageOutcome::Completed),
            "{native:?}"
        );
        requests.push(request);
    }
    let first = review_face_matches(&f.pool, f.corpus).await.unwrap();
    assert_eq!(first.groups.len(), 1);
    assert_eq!(first.groups[0].tracks.len(), 2);
    assert!(first.groups[0]
        .tracks
        .iter()
        .all(|t| t.processor_version == youtube_corpus::provenance::PROCESSOR_SOURCE_REVISION));
    let original = first.groups[0].person_id;
    let removed = first.groups[0].tracks[0].track_id;
    correct_face_matches(
        &f.pool,
        f.corpus,
        FaceCorrectionRequest {
            action: FaceCorrectionAction::Unlink,
            track_ids: vec![removed],
            person_id: None,
        },
    )
    .await
    .unwrap();
    for request in requests {
        let result = run_video_plan(PlanRunRequest {
            refresh: true,
            ..request
        })
        .await
        .unwrap();
        assert!(
            matches!(result.stages[0].outcome, StageOutcome::Completed),
            "{result:?}"
        );
    }
    let final_report = review_face_matches(&f.pool, f.corpus).await.unwrap();
    assert_eq!(final_report.groups.len(), 1);
    assert_eq!(final_report.groups[0].person_id, original);
    assert_eq!(final_report.groups[0].tracks.len(), 1);
    assert_eq!(final_report.unresolved.len(), 1);
    assert_eq!(final_report.unresolved[0].track_id, removed);
    assert!(final_report.unresolved[0].protected);
    eprintln!("native face matching: canonical repeat-face tracks across 2 videos, one inferred person, correction retained after native refresh");
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and pgvector"]
async fn rejecting_one_person_does_not_disable_a_different_supported_automatic_match() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let (_, a) = f.evidence(None, [1.0, 0.0], true).await;
    f.evidence(None, [1.0, 0.0], true).await;
    let initial = f.matching().await;
    let original = person(&initial["groups"][0]);
    f.correct("unlink", &[a], None).await;
    let (_, c) = f.evidence(None, [0.8, 0.6], true).await;
    let (_, d) = f.evidence(None, [0.8, 0.6], true).await;
    let manual = f.correct("split", &[c, d], None).await;
    let other = manual["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| ids(g).contains(&c))
        .map(person)
        .unwrap();
    assert_ne!(other, original);
    let report = f.matching().await;
    assert!(ids(report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| person(g) == other)
        .unwrap())
    .contains(&a));
    assert!(!ids(report["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| person(g) == original)
        .unwrap())
    .contains(&a));
    let repeated = f.matching().await;
    assert!(ids(repeated["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| person(g) == other)
        .unwrap())
    .contains(&a));
}
