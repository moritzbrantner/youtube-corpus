//! #101: channel and source directories. `GET /api/corpora/{corpus}/channels` groups the corpus
//! catalog by YouTube channel id, lists the tracked sources that discovered each channel's
//! videos (an untracked, recognized channel has none) and counts videos without channel
//! metadata; `GET /api/corpora/{corpus}/videos` narrows to one `sourceId` or `channelId`.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
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

fn ids(value: &Value) -> Vec<String> {
    let mut ids = value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn channel_directory_and_scoped_video_lists() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    let app = youtube_corpus::web::app_for_tests_with_research(Some(database_url));

    let tag = Uuid::new_v4().simple().to_string()[..8].to_string();
    let corpus = Uuid::new_v4();
    let channel_source = Uuid::new_v4();
    let playlist_source = Uuid::new_v4();
    let tracked_channel = format!("UCtracked{tag}");
    let untracked_channel = format!("UCuntracked{tag}");
    let tracked_video = Uuid::new_v4();
    let playlist_video = Uuid::new_v4();
    let direct_video = Uuid::new_v4();
    let unknown_video = Uuid::new_v4();

    sqlx::query("INSERT INTO corpora (id, slug, name) VALUES ($1, $2, $3)")
        .bind(corpus)
        .bind(format!("channels-{tag}"))
        .bind(format!("Channels {tag}"))
        .execute(&pool)
        .await
        .unwrap();
    for (id, kind, url, name) in [
        (
            channel_source,
            "channel",
            format!("https://www.youtube.com/@tracked-{tag}"),
            "Tracked lectures",
        ),
        (
            playlist_source,
            "playlist",
            format!("https://www.youtube.com/playlist?list=PL{tag}"),
            "Mixed playlist",
        ),
    ] {
        sqlx::query(
            "INSERT INTO corpus_subscriptions (id, source_kind, source_url, name, work_dir, caption)
             VALUES ($1, $2, $3, $4, 'use-case-output/test', '{}'::jsonb)",
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
    let watch = |video: Uuid| format!("https://www.youtube.com/watch?v={}", video.simple());
    for (video, channel_id, channel, upload_date) in [
        (
            tracked_video,
            Some(tracked_channel.as_str()),
            "Tracked Lectures",
            "20260102",
        ),
        (
            playlist_video,
            Some(tracked_channel.as_str()),
            "Tracked Lectures",
            "20260301",
        ),
        (
            direct_video,
            Some(untracked_channel.as_str()),
            "Guest Channel",
            "20250101",
        ),
        (unknown_video, None, "Unknown", "20240101"),
    ] {
        sqlx::query(
            "INSERT INTO videos (id, source_url, title, channel, channel_id, channel_url, upload_date)
             VALUES ($1, $2, 'video', $3, $4, $5, $6)",
        )
        .bind(video)
        .bind(watch(video))
        .bind(channel)
        .bind(channel_id)
        .bind(channel_id.map(|id| format!("https://www.youtube.com/channel/{id}")))
        .bind(upload_date)
        .execute(&pool)
        .await
        .unwrap();
    }
    for (source, video) in [
        (channel_source, tracked_video),
        (playlist_source, playlist_video),
    ] {
        sqlx::query(
            "INSERT INTO corpus_subscription_items (id, subscription_id, source_url, status)
             VALUES ($1, $2, $3, 'indexed')",
        )
        .bind(Uuid::new_v4())
        .bind(source)
        .bind(watch(video))
        .execute(&pool)
        .await
        .unwrap();
    }
    for video in [direct_video, unknown_video] {
        sqlx::query(
            "INSERT INTO corpus_video_memberships (corpus_id, video_id, membership_kind)
             VALUES ($1, $2, 'direct')",
        )
        .bind(corpus)
        .bind(video)
        .execute(&pool)
        .await
        .unwrap();
    }

    let base = format!("/api/corpora/{corpus}");
    let (status, directory) = get(&app, &format!("{base}/channels")).await;
    assert_eq!(status, StatusCode::OK, "{directory}");
    assert_eq!(directory["videosWithoutChannel"], 1, "{directory}");
    let channels = directory["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2, "{directory}");
    let guest = &channels[0];
    assert_eq!(guest["channelId"], untracked_channel.as_str());
    assert_eq!(guest["name"], "Guest Channel");
    assert_eq!(guest["videoCount"], 1);
    assert_eq!(guest["sources"], serde_json::json!([]));
    let tracked = &channels[1];
    assert_eq!(tracked["channelId"], tracked_channel.as_str());
    assert_eq!(tracked["videoCount"], 2);
    assert_eq!(tracked["latestUploadDate"], "20260301");
    assert_eq!(
        tracked["channelUrl"],
        format!("https://www.youtube.com/channel/{tracked_channel}")
    );
    let mut source_ids = ids(&tracked["sources"]);
    let mut expected = vec![channel_source.to_string(), playlist_source.to_string()];
    expected.sort();
    source_ids.sort();
    assert_eq!(source_ids, expected);

    let (_, by_source) = get(&app, &format!("{base}/videos?sourceId={channel_source}")).await;
    assert_eq!(ids(&by_source), vec![tracked_video.to_string()]);
    assert_eq!(by_source[0]["channelId"], tracked_channel.as_str());

    let (_, by_channel) = get(&app, &format!("{base}/videos?channelId={tracked_channel}")).await;
    let mut expected = vec![tracked_video.to_string(), playlist_video.to_string()];
    expected.sort();
    assert_eq!(ids(&by_channel), expected);

    let (_, untracked) = get(
        &app,
        &format!("{base}/videos?channelId={untracked_channel}"),
    )
    .await;
    assert_eq!(ids(&untracked), vec![direct_video.to_string()]);

    let (status, _) = get(&app, &format!("{base}/videos?sourceId={}", Uuid::new_v4())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    sqlx::query("DELETE FROM corpora WHERE id = $1")
        .bind(corpus)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM corpus_subscriptions WHERE id = ANY($1)")
        .bind(vec![channel_source, playlist_source])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM videos WHERE id = ANY($1)")
        .bind(vec![
            tracked_video,
            playlist_video,
            direct_video,
            unknown_video,
        ])
        .execute(&pool)
        .await
        .unwrap();
}
