use video_analysis_youtube::YtDlpClient;
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
    let browser = std::env::var("YT_DLP_COOKIE_BROWSER").unwrap();
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
