//! #134 contract checks with synthetic 256-D vectors: cross-video voice matching into corpus
//! people, reversible corrections, and face-to-voice link suggestions. Real WeSpeaker
//! embeddings are produced by the voices detector and are not needed here.
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use youtube_corpus::multimodal::*;
use youtube_corpus::voice_matching::{
    correct_voice_matches, match_corpus_voices, review_face_voice_link, FaceLinkAction,
    FaceLinkRequest, VoiceCorrectionAction, VoiceCorrectionRequest, VoiceMatchReview,
    VoiceMatchStatus,
};
use youtube_corpus::voice_processing::{VOICE_MODEL, VOICE_MODEL_VERSION, VOICE_PROCESSOR};

struct Fixture {
    pool: PgPool,
    app: axum::Router,
    corpus: Uuid,
    noise: std::cell::Cell<usize>,
}

impl Fixture {
    async fn new() -> Option<Self> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let pool = youtube_corpus::db::connect(&url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let corpus = Uuid::new_v4();
        sqlx::query("INSERT INTO corpora(id,slug,name) VALUES($1,$2,'Voice matching contract')")
            .bind(corpus)
            .bind(format!("voice-matches-{corpus}"))
            .execute(&pool)
            .await
            .unwrap();
        Some(Self {
            pool,
            app: youtube_corpus::web::app_for_tests_with_research(Some(url)),
            corpus,
            noise: std::cell::Cell::new(0),
        })
    }

    async fn video(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO videos(id,source_url,title) VALUES($1,$2,'Synthetic voice evidence')",
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
    }

    async fn run(&self, video: Uuid, modality: MediaModality) -> Uuid {
        let voice = matches!(modality, MediaModality::Voice);
        begin_processing_run(
            &self.pool,
            BeginProcessingRunRequest {
                video_id: video,
                modality,
                provenance: ProcessingProvenance {
                    processor: if voice {
                        VOICE_PROCESSOR
                    } else {
                        "synthetic-faces"
                    }
                    .into(),
                    processor_version: "git:synthetic".into(),
                    model: if voice { VOICE_MODEL } else { "synthetic" }.into(),
                    model_version: if voice { VOICE_MODEL_VERSION } else { "1" }.into(),
                    input_hash: format!("sha256:{video}"),
                    config_hash: "sha256:config".into(),
                    processing_config: json!({"complete":true}),
                },
            },
        )
        .await
        .unwrap()
        .id
    }

    /// A voice embedding: the speaker's axis plus a distinct noise axis per observation, so
    /// one speaker's observations have cosine 0.8 and different speakers 0.
    fn embedding(&self, speaker: usize) -> Vec<f32> {
        let mut values = vec![0.0; 256];
        values[speaker] = 1.0;
        self.noise.set(self.noise.get() + 1);
        values[64 + self.noise.get()] = 0.5;
        values
    }

    /// One voice track of `speaker` with observations at the given spans.
    async fn voice(&self, run: Uuid, video: Uuid, speaker: usize, spans: &[(f64, f64)]) -> Uuid {
        let key = format!("speaker-{speaker}-{}", spans[0].0);
        let track = upsert_voice_track(
            &self.pool,
            UpsertVoiceTrackRequest {
                run_id: run,
                video_id: video,
                track_key: key.clone(),
                start_seconds: spans[0].0,
                end_seconds: spans[spans.len() - 1].1,
                representative_embedding: None,
                quality: json!({}),
            },
        )
        .await
        .unwrap();
        for (index, (start, end)) in spans.iter().enumerate() {
            let observation = upsert_voice_observation(
                &self.pool,
                UpsertVoiceObservationRequest {
                    run_id: run,
                    video_id: video,
                    observation_key: format!("{key}-{index}"),
                    start_seconds: *start,
                    end_seconds: *end,
                    confidence: None,
                    transcript_segment_id: None,
                    embedding: Some(self.embedding(speaker)),
                    quality: json!({}),
                },
            )
            .await
            .unwrap();
            link_voice_observation_to_track(&self.pool, track.id, observation.id)
                .await
                .unwrap();
        }
        track.id
    }

    /// A face-only corpus person shown at the given spans, one face track per (video, span).
    async fn face_person(&self, shown: &[(Uuid, f64, f64)]) -> Uuid {
        let person = Uuid::new_v4();
        let cluster = Uuid::new_v4();
        sqlx::query("INSERT INTO people(id,corpus_id,label) VALUES($1,$2,'Face person')")
            .bind(person)
            .bind(self.corpus)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO face_clusters(id,corpus_id,algorithm,algorithm_version,config_hash) VALUES($1,$2,'synthetic','1','synthetic')")
            .bind(cluster).bind(self.corpus).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO person_face_clusters(person_id,cluster_id) VALUES($1,$2)")
            .bind(person)
            .bind(cluster)
            .execute(&self.pool)
            .await
            .unwrap();
        for (index, (video, start, end)) in shown.iter().enumerate() {
            let run = self.run(*video, MediaModality::Face).await;
            let track = upsert_face_track(
                &self.pool,
                UpsertFaceTrackRequest {
                    run_id: run,
                    video_id: *video,
                    track_key: format!("face-{person}-{index}"),
                    start_seconds: *start,
                    end_seconds: *end,
                    representative_embedding: None,
                    quality: json!({}),
                },
            )
            .await
            .unwrap();
            sqlx::query("INSERT INTO face_cluster_members(cluster_id,track_id) VALUES($1,$2)")
                .bind(cluster)
                .bind(track.id)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        person
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/corpora/{}/{path}", self.corpus))
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn person_of(review: &VoiceMatchReview, track: Uuid) -> Option<Uuid> {
    review
        .groups
        .iter()
        .find(|g| g.tracks.iter().any(|t| t.track_id == track))
        .map(|g| g.person_id)
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn voices_match_across_videos_as_reviewable_suggestions() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let videos = [f.video().await, f.video().await, f.video().await];
    let mut runs = Vec::new();
    for video in videos {
        runs.push(f.run(video, MediaModality::Voice).await);
    }
    // Host (speaker 0) in all three videos, guest (speaker 1) in the first two.
    let host: Vec<Uuid> = futures_collect(&f, &runs, &videos, 0, &[(0.0, 2.0), (3.0, 5.0)]).await;
    let guest = [
        f.voice(runs[0], videos[0], 1, &[(10.0, 13.0), (14.0, 16.0)])
            .await,
        f.voice(runs[1], videos[1], 1, &[(10.0, 13.0), (14.0, 16.0)])
            .await,
    ];
    let short = f.voice(runs[2], videos[2], 0, &[(20.0, 21.0)]).await;
    let lonely = f.voice(runs[2], videos[2], 2, &[(30.0, 35.0)]).await;

    let review = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    let host_person = person_of(&review, host[0]).expect("host matched");
    assert!(host
        .iter()
        .all(|t| person_of(&review, *t) == Some(host_person)));
    let guest_person = person_of(&review, guest[0]).expect("guest matched");
    assert_ne!(host_person, guest_person);
    assert_eq!(person_of(&review, guest[1]), Some(guest_person));
    assert_eq!(review.groups.len(), 2);
    for track in review.groups.iter().flat_map(|g| &g.tracks) {
        assert_eq!(track.status, Some(VoiceMatchStatus::Suggested));
        assert!(track.similarity.unwrap() >= 0.6);
    }
    let reason = |id: Uuid| {
        review
            .unresolved
            .iter()
            .find(|t| t.track_id == id)
            .and_then(|t| t.reason.clone())
            .unwrap()
    };
    assert!(reason(short).contains("Less than 2 s"));
    assert!(reason(lonely).contains("No unique high-confidence match"));
    let identity_source: String =
        sqlx::query_scalar("SELECT identity_source FROM people WHERE id=$1")
            .bind(host_person)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(identity_source, "automatic-inferred-voice");

    // Reruns are stable.
    let again = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    assert_eq!(again.groups.len(), 2);
    assert_eq!(person_of(&again, host[2]), Some(host_person));

    // Unlinking survives reruns; confirming marks the group confirmed.
    correct_voice_matches(
        &f.pool,
        f.corpus,
        VoiceCorrectionRequest {
            action: VoiceCorrectionAction::Unlink,
            track_ids: vec![host[2]],
            person_id: None,
        },
    )
    .await
    .unwrap();
    let rerun = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    assert_eq!(person_of(&rerun, host[2]), None);
    assert!(rerun
        .unresolved
        .iter()
        .any(|t| t.track_id == host[2] && t.protected));
    let confirmed = correct_voice_matches(
        &f.pool,
        f.corpus,
        VoiceCorrectionRequest {
            action: VoiceCorrectionAction::Confirm,
            track_ids: guest.to_vec(),
            person_id: Some(guest_person),
        },
    )
    .await
    .unwrap();
    let group = confirmed
        .groups
        .iter()
        .find(|g| g.person_id == guest_person)
        .unwrap();
    assert!(group
        .tracks
        .iter()
        .all(|t| t.status == Some(VoiceMatchStatus::Confirmed)));

    // HTTP surface and input validation.
    let (status, body) = f.request(Method::GET, "voice-matches", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["groups"].as_array().unwrap().len(), 2);
    let (status, _) = f
        .request(
            Method::POST,
            "voice-matches/corrections",
            Some(json!({"action":"unlink","trackIds":[]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

async fn futures_collect(
    f: &Fixture,
    runs: &[Uuid],
    videos: &[Uuid],
    speaker: usize,
    spans: &[(f64, f64)],
) -> Vec<Uuid> {
    let mut tracks = Vec::new();
    for (run, video) in runs.iter().zip(videos) {
        tracks.push(f.voice(*run, *video, speaker, spans).await);
    }
    tracks
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn faces_link_to_cooccurring_voices_only_after_review() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let videos = [f.video().await, f.video().await];
    let mut runs = Vec::new();
    for video in videos {
        runs.push(f.run(video, MediaModality::Voice).await);
    }
    // Speaker 3 talks 0–20 s in both videos; speaker 4 talks 40–60 s.
    let speech = [(0.0, 8.0), (9.0, 20.0)];
    let narrator = futures_collect(&f, &runs, &videos, 3, &speech).await;
    let other = futures_collect(&f, &runs, &videos, 4, &[(40.0, 50.0), (51.0, 60.0)]).await;
    // The on-screen face matches speaker 3's speech; a second face is on screen briefly.
    let face = f
        .face_person(&[(videos[0], 0.0, 21.0), (videos[1], 1.0, 19.0)])
        .await;
    let bystander = f
        .face_person(&[(videos[0], 15.0, 18.0), (videos[1], 45.0, 47.0)])
        .await;

    let review = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    let voice_person = person_of(&review, narrator[0]).unwrap();
    assert_eq!(review.face_links.len(), 1, "{:?}", review.face_links);
    let link = &review.face_links[0];
    assert_eq!(
        (link.face_person_id, link.voice_person_id),
        (face, voice_person)
    );
    assert_eq!(link.status, "unverified");
    assert_eq!(link.shared_videos, 2);
    assert!(link.coverage > 0.9 && link.margin >= 0.2);
    assert!(!review
        .face_links
        .iter()
        .any(|l| l.face_person_id == bystander));
    // A suggestion changes nothing until reviewed.
    let other_person = person_of(&review, other[0]).unwrap();
    assert_ne!(other_person, voice_person);

    let merged = review_face_voice_link(
        &f.pool,
        f.corpus,
        FaceLinkRequest {
            action: FaceLinkAction::Confirm,
            face_person_id: face,
            voice_person_id: voice_person,
        },
    )
    .await
    .unwrap();
    assert_eq!(person_of(&merged, narrator[0]), Some(face));
    assert_eq!(person_of(&merged, narrator[1]), Some(face));
    assert!(merged.face_links.is_empty());
    let gone: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM people WHERE id=$1)")
        .bind(voice_person)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(!gone);
    // Further voice matching extends the merged person instead of recreating the voice person.
    let rerun = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    assert_eq!(person_of(&rerun, narrator[1]), Some(face));

    // Rejected suggestions stay rejected across reruns.
    let late = f
        .face_person(&[(videos[0], 39.0, 61.0), (videos[1], 39.0, 61.0)])
        .await;
    let suggested = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    assert!(suggested
        .face_links
        .iter()
        .any(|l| l.face_person_id == late && l.voice_person_id == other_person));
    review_face_voice_link(
        &f.pool,
        f.corpus,
        FaceLinkRequest {
            action: FaceLinkAction::Reject,
            face_person_id: late,
            voice_person_id: other_person,
        },
    )
    .await
    .unwrap();
    let after = match_corpus_voices(&f.pool, f.corpus).await.unwrap();
    assert!(after.face_links.is_empty());
    assert_eq!(person_of(&after, other[0]), Some(other_person));
    let (status, _) = f
        .request(
            Method::POST,
            "voice-matches/face-links",
            Some(json!({"action":"confirm","facePersonId":late,"voicePersonId":other_person})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
