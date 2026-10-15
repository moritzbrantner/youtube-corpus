//! Independent acceptance for #59: organize tracked channels, playlists and videos with
//! user-defined labels (Decision Atlas 1.1, 1.2, 1.3, 1.4 and 3.3).
//!
//! Authored by a separate acceptance context before any implementation existed, against
//! main 7fe61c6. The tests drive the public corpus HTTP API through
//! `youtube_corpus::web::app_for_tests_with_research` and seed only pre-existing tables
//! (corpora, corpus_subscriptions, corpus_subscription_items, corpus memberships, videos,
//! transcript evidence), so they compile on main and fail at runtime until the feature exists.
//!
//! # Contract (seam assumed by these tests)
//!
//! Route names, JSON field names and status codes below are the routine seam chosen by the
//! acceptance author; the implementer may change them only together with an independent
//! re-verification of this file. Additional response fields are always allowed.
//!
//! * A label is `{ "id": <uuid>, "name": <string> }`.
//! * `GET  /api/corpora/{corpus}/labels` lists the labels usable in the corpus.
//! * `POST /api/corpora/{corpus}/labels` `{ "name" }` creates one: `201` + label. A blank name
//!   is a `400`; a name already used by another label is a non-404 `4xx` (409 preferred).
//! * `PATCH  /api/corpora/{corpus}/labels/{label}` `{ "name" }` renames it (same id): `200` +
//!   label. Renaming onto another label's name is rejected like a duplicate create.
//! * `DELETE /api/corpora/{corpus}/labels/{label}` deletes the label and all its assignments.
//! * `PUT`/`DELETE /api/corpora/{corpus}/sources/{source}/labels/{label}` assigns/unassigns a
//!   label on a tracked channel/playlist; `PUT`/`DELETE
//!   /api/corpora/{corpus}/videos/{video}/labels/{label}` does the same on a video. Assignment is
//!   idempotent. Success is any `2xx`.
//! * Unknown corpus, label, or a source/video that is not part of the corpus is a `404`. Every
//!   error uses the existing `{ "error": { "message" } }` envelope with a non-empty message.
//! * `GET /api/corpora/{corpus}/sources` items gain `labels: [label]` (labels on that source).
//! * `GET /api/corpora/{corpus}/videos` items gain:
//!   - `labels: [label]` — user labels assigned **directly** to the video;
//!   - `sources: [{ "id": <source uuid>, .. }]` — every tracked source of this corpus whose
//!     discovered items contain the video (Atlas 1.1: source relationships remain visible);
//!   - `addedDirectly: bool` — the video was added to the corpus directly;
//!   - `youtubeCategories: [string]`, `youtubeTags: [string]` — the YouTube-supplied
//!     categories/tags, kept separate from user labels (Atlas 1.3, 1.4).
//! * `GET /api/corpora/{corpus}/videos?labelId={label}` narrows the catalog to videos carrying
//!   that label (Atlas 3.3). A filter on an unknown label is a `404`; a known label without
//!   matches yields an empty list.
//!
//! Derived (not explicitly owner-stated) rule, isolated in
//! `source_label_filters_the_catalog_to_that_sources_videos`: filtering by a label that is on a
//! tracked source matches the videos of that source, because Atlas 3.3 requires browsing and
//! evidence to be narrowable by user-defined groups and a source label could otherwise never
//! narrow the catalog. If the owner decides otherwise, only that test changes.
//!
//! Deliberately not asserted (unsettled by the Atlas): whether labels are scoped per corpus or
//! shared across corpora, and case-insensitive name uniqueness. All label names therefore carry
//! a per-test unique suffix.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

/// One corpus with two monitored sources (a channel and a playlist) that both contain the
/// shared video, a channel-only video, and one directly added video. The shared video carries
/// YouTube categories/tags, metadata and transcript evidence.
struct Fixture {
    pool: PgPool,
    app: axum::Router,
    tag: String,
    corpus: Uuid,
    channel: Uuid,
    playlist: Uuid,
    shared: Uuid,
    channel_only: Uuid,
    direct: Uuid,
    youtube_tag: String,
}

impl Fixture {
    async fn seed() -> Option<Self> {
        let database_url = std::env::var("DATABASE_URL").ok()?;
        let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
        youtube_corpus::db::migrate(&pool).await.unwrap();
        let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

        let tag = Uuid::new_v4().simple().to_string()[..8].to_string();
        let corpus = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let playlist = Uuid::new_v4();
        let shared = Uuid::new_v4();
        let channel_only = Uuid::new_v4();
        let direct = Uuid::new_v4();
        let youtube_tag = format!("history-{tag}");

        sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
            .bind(corpus)
            .bind(format!("labels-{tag}"))
            .bind(format!("Labels acceptance {tag}"))
            .execute(&pool)
            .await
            .unwrap();

        for (id, kind, url, name) in [
            (
                channel,
                "channel",
                format!("https://www.youtube.com/@labels-{tag}"),
                "Lecture channel",
            ),
            (
                playlist,
                "playlist",
                format!("https://www.youtube.com/playlist?list=PL{tag}"),
                "Seminar playlist",
            ),
        ] {
            sqlx::query(
                "INSERT INTO corpus_subscriptions (id, source_kind, source_url, name, work_dir, caption)
                 VALUES ($1, $2, $3, $4, 'use-case-output/acceptance', '{}'::jsonb)",
            )
            .bind(id)
            .bind(kind)
            .bind(url)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO corpus_source_memberships (corpus_id, subscription_id) VALUES ($1, $2)",
            )
            .bind(corpus)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }

        for (id, title, categories, tags) in [
            (
                shared,
                "Shared lecture",
                vec!["Education".to_string()],
                vec![youtube_tag.clone(), "lecture".to_string()],
            ),
            (
                channel_only,
                "Channel-only lecture",
                vec!["Education".to_string()],
                vec!["lecture".to_string()],
            ),
            (
                direct,
                "Directly added talk",
                vec!["People & Blogs".to_string()],
                vec!["talk".to_string()],
            ),
        ] {
            sqlx::query(
                "INSERT INTO videos (id, youtube_id, source_url, title, channel, upload_date,
                                     duration_seconds, categories, tags, metadata)
                 VALUES ($1, $2, $3, $4, 'Lecture channel', '20260101', 120, $5, $6, $7)",
            )
            .bind(id)
            .bind(youtube_id(id))
            .bind(watch_url(id))
            .bind(title)
            .bind(&categories)
            .bind(&tags)
            .bind(json!({"categories": categories, "tags": tags, "like_count": 7}))
            .execute(&pool)
            .await
            .unwrap();
        }

        // The shared video was discovered by both sources; the channel-only video by one.
        for (subscription, video) in [
            (channel, shared),
            (playlist, shared),
            (channel, channel_only),
        ] {
            sqlx::query(
                "INSERT INTO corpus_subscription_items
                   (id, subscription_id, source_url, youtube_id, title, status)
                 VALUES ($1, $2, $3, $4, 'discovered item', 'indexed')",
            )
            .bind(Uuid::new_v4())
            .bind(subscription)
            .bind(watch_url(video))
            .bind(youtube_id(video))
            .execute(&pool)
            .await
            .unwrap();
        }

        sqlx::query(
            "INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
             VALUES ($1, $2, 'direct')",
        )
        .bind(corpus)
        .bind(direct)
        .execute(&pool)
        .await
        .unwrap();

        // Downloaded/derived evidence for the shared video.
        let stream = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transcript_streams (id, video_id, source_kind, language, status, full_text)
             VALUES ($1, $2, 'caption_manual', 'en', 'completed', 'grace perfects nature')",
        )
        .bind(stream)
        .bind(shared)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO transcript_segments
               (id, video_id, stream_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, 0, 1, 4, 'grace perfects nature', 'en')",
        )
        .bind(Uuid::new_v4())
        .bind(shared)
        .bind(stream)
        .execute(&pool)
        .await
        .unwrap();

        Some(Self {
            pool,
            app,
            tag,
            corpus,
            channel,
            playlist,
            shared,
            channel_only,
            direct,
            youtube_tag,
        })
    }

    fn name(&self, base: &str) -> String {
        format!("{base} {}", self.tag)
    }

    fn uri(&self, suffix: &str) -> String {
        format!("/api/corpora/{}{suffix}", self.corpus)
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
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        };
        (status, value)
    }

    async fn ok(&self, method: Method, uri: &str, body: Option<Value>) -> Value {
        let (status, value) = self.call(method.clone(), uri, body).await;
        assert!(status.is_success(), "{method} {uri}: {status} {value}");
        value
    }

    async fn create_label(&self, base: &str) -> (Uuid, String) {
        let name = self.name(base);
        let (status, label) = self
            .call(
                Method::POST,
                &self.uri("/labels"),
                Some(json!({ "name": name })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "create label {name}: {label}");
        assert_eq!(label["name"], name, "{label}");
        let id = label["id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .unwrap_or_else(|| panic!("label id must be a uuid: {label}"));
        (id, name)
    }

    async fn assign_video(&self, video: Uuid, label: Uuid) {
        self.ok(
            Method::PUT,
            &self.uri(&format!("/videos/{video}/labels/{label}")),
            None,
        )
        .await;
    }

    async fn assign_source(&self, source: Uuid, label: Uuid) {
        self.ok(
            Method::PUT,
            &self.uri(&format!("/sources/{source}/labels/{label}")),
            None,
        )
        .await;
    }

    async fn labels(&self) -> Vec<Value> {
        array(self.ok(Method::GET, &self.uri("/labels"), None).await)
    }

    async fn sources(&self) -> Vec<Value> {
        array(self.ok(Method::GET, &self.uri("/sources"), None).await)
    }

    async fn videos(&self) -> Vec<Value> {
        array(
            self.ok(Method::GET, &self.uri("/videos?limit=500"), None)
                .await,
        )
    }

    async fn videos_with_label(&self, label: Uuid) -> Vec<Value> {
        array(
            self.ok(
                Method::GET,
                &self.uri(&format!("/videos?limit=500&labelId={label}")),
                None,
            )
            .await,
        )
    }

    async fn cleanup(self) {
        sqlx::query("DELETE FROM corpora WHERE id = $1")
            .bind(self.corpus)
            .execute(&self.pool)
            .await
            .unwrap();
        for subscription in [self.channel, self.playlist] {
            sqlx::query("DELETE FROM corpus_subscriptions WHERE id = $1")
                .bind(subscription)
                .execute(&self.pool)
                .await
                .unwrap();
        }
        for video in [self.shared, self.channel_only, self.direct] {
            sqlx::query("DELETE FROM videos WHERE id = $1")
                .bind(video)
                .execute(&self.pool)
                .await
                .unwrap();
        }
    }
}

fn youtube_id(video: Uuid) -> String {
    format!("yt{}", &video.simple().to_string()[..9])
}

fn watch_url(video: Uuid) -> String {
    format!("https://www.youtube.com/watch?v={}", youtube_id(video))
}

fn array(value: Value) -> Vec<Value> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected a JSON array, got {value}"))
        .clone()
}

fn ids(items: &[Value]) -> Vec<String> {
    let mut ids = items
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn find(items: &[Value], id: Uuid) -> Value {
    let matches = items
        .iter()
        .filter(|item| item["id"] == id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{id} must appear exactly once: {items:?}");
    matches[0].clone()
}

fn label_names(item: &Value) -> Vec<String> {
    let labels = item["labels"]
        .as_array()
        .unwrap_or_else(|| panic!("`labels` must be an array: {item}"));
    sorted(
        labels
            .iter()
            .map(|label| label["name"].as_str().unwrap_or_default().to_string())
            .collect(),
    )
}

fn strings(item: &Value, field: &str) -> Vec<String> {
    sorted(
        item[field]
            .as_array()
            .unwrap_or_else(|| panic!("`{field}` must be an array: {item}"))
            .iter()
            .map(|value| value.as_str().unwrap_or_default().to_string())
            .collect(),
    )
}

fn source_ids(video: &Value) -> Vec<String> {
    ids(video["sources"]
        .as_array()
        .unwrap_or_else(|| panic!("`sources` must be an array: {video}")))
}

fn assert_error(status: StatusCode, body: &Value, expected: StatusCode, context: &str) {
    assert_eq!(status, expected, "{context}: {body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "{context}: error must carry an intelligible message: {body}"
    );
}

/// Atlas 1.1/1.2/1.4: one canonical video found by two monitored sources plus a directly
/// added video appear once each, with all source memberships, YouTube metadata and no labels.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn shared_and_direct_videos_appear_once_with_their_memberships() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let videos = fx.videos().await;
    assert_eq!(
        ids(&videos),
        sorted(vec![
            fx.shared.to_string(),
            fx.channel_only.to_string(),
            fx.direct.to_string()
        ]),
        "catalog lists each canonical video once"
    );

    let shared = find(&videos, fx.shared);
    assert_eq!(
        source_ids(&shared),
        sorted(vec![fx.channel.to_string(), fx.playlist.to_string()]),
        "shared video keeps both source memberships: {shared}"
    );
    assert_eq!(shared["addedDirectly"], false, "{shared}");
    assert_eq!(strings(&shared, "youtubeCategories"), vec!["Education"]);
    assert_eq!(
        strings(&shared, "youtubeTags"),
        sorted(vec![fx.youtube_tag.clone(), "lecture".to_string()])
    );
    assert!(label_names(&shared).is_empty(), "{shared}");

    let channel_only = find(&videos, fx.channel_only);
    assert_eq!(source_ids(&channel_only), vec![fx.channel.to_string()]);
    assert_eq!(channel_only["addedDirectly"], false, "{channel_only}");

    let direct = find(&videos, fx.direct);
    assert!(source_ids(&direct).is_empty(), "{direct}");
    assert_eq!(direct["addedDirectly"], true, "{direct}");
    assert_eq!(strings(&direct, "youtubeTags"), vec!["talk"]);

    let sources = fx.sources().await;
    assert_eq!(
        ids(&sources),
        sorted(vec![fx.channel.to_string(), fx.playlist.to_string()])
    );
    for source in &sources {
        assert!(label_names(source).is_empty(), "{source}");
    }
    assert!(
        fx.labels()
            .await
            .iter()
            .all(|label| !label["name"].as_str().unwrap_or("").ends_with(&fx.tag)),
        "a fresh corpus has none of this test's labels"
    );

    fx.cleanup().await;
}

/// Atlas 1.3: create, assign, rename and unassign user labels on a source and on a video;
/// they are visible in the source and video lists.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn label_round_trip_on_sources_and_videos() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let (theology, theology_name) = fx.create_label("Theology").await;
    let (later, later_name) = fx.create_label("Watch later").await;
    let listed = fx.labels().await;
    for (id, name) in [(theology, &theology_name), (later, &later_name)] {
        let label = find(&listed, id);
        assert_eq!(label["name"], *name);
    }

    // Assign on a source (idempotently) and on a video.
    fx.assign_source(fx.channel, theology).await;
    fx.assign_source(fx.channel, theology).await;
    fx.assign_video(fx.direct, later).await;

    let sources = fx.sources().await;
    assert_eq!(
        label_names(&find(&sources, fx.channel)),
        vec![theology_name.clone()]
    );
    assert!(label_names(&find(&sources, fx.playlist)).is_empty());
    let videos = fx.videos().await;
    assert_eq!(
        label_names(&find(&videos, fx.direct)),
        vec![later_name.clone()]
    );
    assert!(!label_names(&find(&videos, fx.shared)).contains(&later_name));

    // Rename keeps identity and assignments.
    let renamed_name = fx.name("Church history");
    let renamed = fx
        .ok(
            Method::PATCH,
            &fx.uri(&format!("/labels/{theology}")),
            Some(json!({ "name": renamed_name })),
        )
        .await;
    assert_eq!(renamed["id"], theology.to_string());
    assert_eq!(renamed["name"], renamed_name);
    let sources = fx.sources().await;
    assert_eq!(
        label_names(&find(&sources, fx.channel)),
        vec![renamed_name.clone()]
    );
    assert_eq!(find(&fx.labels().await, theology)["name"], renamed_name);

    // Change a video's label: unassign one, assign another.
    fx.ok(
        Method::DELETE,
        &fx.uri(&format!("/videos/{}/labels/{later}", fx.direct)),
        None,
    )
    .await;
    fx.assign_video(fx.direct, theology).await;
    let videos = fx.videos().await;
    assert_eq!(
        label_names(&find(&videos, fx.direct)),
        vec![renamed_name.clone()]
    );

    // Unassign from the source; the label itself survives.
    fx.ok(
        Method::DELETE,
        &fx.uri(&format!("/sources/{}/labels/{theology}", fx.channel)),
        None,
    )
    .await;
    assert!(label_names(&find(&fx.sources().await, fx.channel)).is_empty());
    let labels = fx.labels().await;
    find(&labels, theology);
    find(&labels, later);

    fx.cleanup().await;
}

/// Atlas 3.3: filtering the catalog by a user label changes the results; a label without
/// assignments yields an intelligible empty result rather than the whole catalog.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn filtering_the_catalog_by_a_video_label_changes_results() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let (later, _) = fx.create_label("Watch later").await;
    let (unused, _) = fx.create_label("Unused").await;
    fx.assign_video(fx.direct, later).await;
    fx.assign_video(fx.shared, later).await;

    assert_eq!(fx.videos().await.len(), 3);
    let filtered = fx.videos_with_label(later).await;
    assert_eq!(
        ids(&filtered),
        sorted(vec![fx.direct.to_string(), fx.shared.to_string()]),
        "filter keeps exactly the labeled videos, the shared one once"
    );
    let shared = find(&filtered, fx.shared);
    assert_eq!(
        source_ids(&shared),
        sorted(vec![fx.channel.to_string(), fx.playlist.to_string()]),
        "filtered rows keep their source memberships"
    );

    assert!(fx.videos_with_label(unused).await.is_empty());

    fx.ok(
        Method::DELETE,
        &fx.uri(&format!("/videos/{}/labels/{later}", fx.shared)),
        None,
    )
    .await;
    assert_eq!(
        ids(&fx.videos_with_label(later).await),
        vec![fx.direct.to_string()]
    );

    fx.cleanup().await;
}

/// Atlas 1.3 + 3.3 (derived rule, see contract note): a label on a tracked source narrows the
/// catalog to the videos of that source, without duplicating a shared video.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn source_label_filters_the_catalog_to_that_sources_videos() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let (lectures, _) = fx.create_label("Lectures").await;
    let (seminars, _) = fx.create_label("Seminars").await;
    fx.assign_source(fx.channel, lectures).await;
    fx.assign_source(fx.playlist, seminars).await;
    fx.assign_source(fx.playlist, lectures).await;

    assert_eq!(
        ids(&fx.videos_with_label(lectures).await),
        sorted(vec![fx.shared.to_string(), fx.channel_only.to_string()]),
        "videos of both labeled sources, the shared one once, not the direct video"
    );
    assert_eq!(
        ids(&fx.videos_with_label(seminars).await),
        vec![fx.shared.to_string()]
    );

    fx.cleanup().await;
}

/// Atlas 1.3/1.4: removing and deleting labels never removes YouTube metadata, source
/// memberships, the canonical video, or downloaded/derived evidence.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn removing_labels_keeps_youtube_metadata_memberships_and_evidence() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let before = evidence_snapshot(&fx.pool, fx.shared).await;
    let listed_before = find(&fx.videos().await, fx.shared);

    let (keep, _) = fx.create_label("Keep").await;
    let (doomed, _) = fx.create_label("Temporary").await;
    fx.assign_video(fx.shared, keep).await;
    fx.assign_video(fx.shared, doomed).await;
    fx.assign_source(fx.channel, doomed).await;

    fx.ok(
        Method::DELETE,
        &fx.uri(&format!("/videos/{}/labels/{keep}", fx.shared)),
        None,
    )
    .await;
    fx.ok(Method::DELETE, &fx.uri(&format!("/labels/{doomed}")), None)
        .await;

    let labels = fx.labels().await;
    assert!(labels.iter().all(|label| label["id"] != doomed.to_string()));
    find(&labels, keep);
    let (status, body) = fx
        .call(
            Method::GET,
            &fx.uri(&format!("/videos?labelId={doomed}")),
            None,
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::NOT_FOUND,
        "filter by deleted label",
    );

    let videos = fx.videos().await;
    assert_eq!(videos.len(), 3, "no video disappears with its labels");
    let shared = find(&videos, fx.shared);
    assert!(label_names(&shared).is_empty(), "{shared}");
    for field in [
        "youtubeCategories",
        "youtubeTags",
        "sources",
        "title",
        "channel",
        "transcriptStreams",
        "transcriptSegments",
    ] {
        assert_eq!(shared[field], listed_before[field], "{field} unchanged");
    }
    assert!(label_names(&find(&fx.sources().await, fx.channel)).is_empty());
    assert_eq!(
        evidence_snapshot(&fx.pool, fx.shared).await,
        before,
        "stored YouTube metadata and evidence are untouched"
    );

    fx.cleanup().await;
}

/// Atlas 1.3/1.4: a user label is distinguishable from an identically named YouTube tag, and a
/// label filter does not match YouTube-provided tags.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn user_labels_are_distinct_from_youtube_tags() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    // Same text as the shared video's YouTube tag.
    let name = fx.youtube_tag.clone();
    let (status, label) = fx
        .call(
            Method::POST,
            &fx.uri("/labels"),
            Some(json!({ "name": name })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{label}");
    let label = Uuid::parse_str(label["id"].as_str().unwrap()).unwrap();
    fx.assign_video(fx.direct, label).await;

    let videos = fx.videos().await;
    let direct = find(&videos, fx.direct);
    assert_eq!(label_names(&direct), vec![name.clone()]);
    assert!(!strings(&direct, "youtubeTags").contains(&name));
    let shared = find(&videos, fx.shared);
    assert!(strings(&shared, "youtubeTags").contains(&name));
    assert!(!label_names(&shared).contains(&name));

    assert_eq!(
        ids(&fx.videos_with_label(label).await),
        vec![fx.direct.to_string()],
        "a user-label filter never matches YouTube tags"
    );

    fx.cleanup().await;
}

/// Empty and error states are explicit and intelligible.
#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn label_errors_are_intelligible() {
    let Some(fx) = Fixture::seed().await else {
        return;
    };

    let (status, body) = fx
        .call(
            Method::POST,
            &fx.uri("/labels"),
            Some(json!({ "name": "   " })),
        )
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "blank label name");

    let (theology, theology_name) = fx.create_label("Theology").await;
    let (other, _) = fx.create_label("Other").await;

    let (status, body) = fx
        .call(
            Method::POST,
            &fx.uri("/labels"),
            Some(json!({ "name": theology_name })),
        )
        .await;
    assert!(
        status.is_client_error() && status != StatusCode::NOT_FOUND,
        "duplicate name must be rejected: {status} {body}"
    );
    assert_error(status, &body, status, "duplicate label name");
    let same_name = fx
        .labels()
        .await
        .into_iter()
        .filter(|label| label["name"] == theology_name)
        .count();
    assert_eq!(same_name, 1, "no second label with the same name");

    let (status, body) = fx
        .call(
            Method::PATCH,
            &fx.uri(&format!("/labels/{other}")),
            Some(json!({ "name": theology_name })),
        )
        .await;
    assert!(
        status.is_client_error() && status != StatusCode::NOT_FOUND,
        "renaming onto an existing name must be rejected: {status} {body}"
    );

    let unknown = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    for (method, uri, body, context) in [
        (
            Method::PUT,
            fx.uri(&format!("/videos/{}/labels/{unknown}", fx.direct)),
            None,
            "assign unknown label to video",
        ),
        (
            Method::PUT,
            fx.uri(&format!("/sources/{}/labels/{unknown}", fx.channel)),
            None,
            "assign unknown label to source",
        ),
        (
            Method::PATCH,
            fx.uri(&format!("/labels/{unknown}")),
            Some(json!({ "name": fx.name("Nope") })),
            "rename unknown label",
        ),
        (
            Method::DELETE,
            fx.uri(&format!("/labels/{unknown}")),
            None,
            "delete unknown label",
        ),
        (
            Method::GET,
            fx.uri(&format!("/videos?labelId={unknown}")),
            None,
            "filter by unknown label",
        ),
        (
            Method::PUT,
            fx.uri(&format!("/videos/{outsider}/labels/{theology}")),
            None,
            "label a video outside the corpus",
        ),
        (
            Method::PUT,
            fx.uri(&format!("/sources/{outsider}/labels/{theology}")),
            None,
            "label a source outside the corpus",
        ),
        (
            Method::GET,
            format!("/api/corpora/{outsider}/labels"),
            None,
            "labels of an unknown corpus",
        ),
    ] {
        let (status, response) = fx.call(method, &uri, body).await;
        assert_error(status, &response, StatusCode::NOT_FOUND, context);
    }

    // Nothing above changed assignments.
    let videos = fx.videos().await;
    assert!(videos.iter().all(|video| label_names(video).is_empty()));

    fx.cleanup().await;
}

/// Stored state that label operations must never change for a video.
async fn evidence_snapshot(pool: &PgPool, video: Uuid) -> Value {
    let row: (Value,) = sqlx::query_as(
        "SELECT jsonb_build_object(
           'video', (SELECT jsonb_build_object(
                        'title', v.title, 'channel', v.channel, 'categories', v.categories,
                        'tags', v.tags, 'metadata', v.metadata,
                        'metadataRevision', v.metadata_processing_revision)
                     FROM videos v WHERE v.id = $1),
           'streams', (SELECT count(*) FROM transcript_streams WHERE video_id = $1),
           'segments', (SELECT count(*) FROM transcript_segments WHERE video_id = $1),
           'items', (SELECT count(*) FROM corpus_subscription_items i
                     JOIN videos v ON v.id = $1 AND i.youtube_id = v.youtube_id),
           'memberships', (SELECT count(*) FROM corpus_video_memberships WHERE video_id = $1))",
    )
    .bind(video)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}
