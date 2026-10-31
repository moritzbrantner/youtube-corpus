use youtube_corpus::yt_dlp::YtDlpClient;
use youtube_corpus::{BrowserCookieSource, YtDlpConfig};

const TEST_VIDEO_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

#[tokio::test]
#[ignore = "requires network access and a working yt-dlp installation"]
async fn fetches_real_metadata_for_first_youtube_video() {
    let client = YtDlpClient::new(YtDlpConfig::default());

    let (json, _) = client.fetch_metadata_json(TEST_VIDEO_URL).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&json).unwrap();

    assert_eq!(value["id"], "jNQXAC9IVRw");
}

#[tokio::test]
#[ignore = "requires network access and a working yt-dlp installation"]
async fn fetches_real_captions_if_available() {
    let dir = std::path::PathBuf::from("use-case-output/youtube-corpus/integration-captions");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let template = dir.join("%(id)s.%(ext)s");
    let client = YtDlpClient::new(YtDlpConfig::default());

    let _ = client
        .download_captions(TEST_VIDEO_URL, template, "en", false)
        .await;
}

#[tokio::test]
#[ignore = "requires YT_DLP_COOKIE_BROWSER=brave or another browser name"]
async fn probes_browser_cookies_when_requested() {
    let Ok(browser) = std::env::var("YT_DLP_COOKIE_BROWSER") else {
        eprintln!("YT_DLP_COOKIE_BROWSER unset; skipping");
        return;
    };
    let client = YtDlpClient::new(YtDlpConfig {
        cookies_from_browser: Some(BrowserCookieSource {
            browser,
            profile: std::env::var("YT_DLP_COOKIE_PROFILE").ok(),
            keyring: std::env::var("YT_DLP_COOKIE_KEYRING").ok(),
        }),
        ..YtDlpConfig::default()
    });

    client.probe_cookies(TEST_VIDEO_URL).await.unwrap();
}

/// "Blender 3.6 LTS - Reel" by Blender, 85 s, Creative Commons Attribution (checked
/// 2026-10-31 via yt-dlp's `license` field).
const CC_SHORT_VIDEO_URL: &str = "https://www.youtube.com/watch?v=BnY2JdBrvaI";

#[tokio::test]
#[ignore = "requires network access, a working yt-dlp installation and ffprobe"]
async fn acquires_real_audio_only_and_capped_video_streams() {
    use youtube_corpus::media_acquisition::{download, MediaNeed};
    let dir = std::path::PathBuf::from("use-case-output/youtube-corpus/integration-media");
    let _ = tokio::fs::remove_dir_all(&dir).await;
    let client = YtDlpClient::new(YtDlpConfig {
        // Extra raw arguments for this environment, e.g. `--js-runtimes bun`.
        args: std::env::var("YT_DLP_TEST_ARGS")
            .map(|args| args.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        ..YtDlpConfig::default()
    });

    let audio = download(&client, CC_SHORT_VIDEO_URL, &dir, "audio", MediaNeed::Audio)
        .await
        .unwrap();
    assert!(
        audio
            .formats
            .iter()
            .all(|f| f.vcodec.as_deref() == Some("none")),
        "{audio:#?}"
    );
    assert!(audio.audio_format_id.is_some(), "{audio:#?}");
    assert_eq!(audio.bytes, std::fs::metadata(&audio.path).unwrap().len());

    let video = download(
        &client,
        CC_SHORT_VIDEO_URL,
        &dir,
        "video",
        MediaNeed::Video { max_height: 360 },
    )
    .await
    .unwrap();
    assert!(video.height.is_some_and(|h| h <= 360), "{video:#?}");
    assert!(video.audio_format_id.is_some(), "{video:#?}");
    eprintln!("{audio:#?}\n{video:#?}");
}
