use youtube_corpus::captions::parse_caption_files;

#[tokio::test]
async fn parses_webvtt_caption_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.en.vtt");
    std::fs::write(
        &path,
        "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhello world\n",
    )
    .unwrap();

    let streams = parse_caption_files(dir.path()).await.unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].segments.len(), 1);
    assert_eq!(streams[0].segments[0].text, "hello world");
}

#[tokio::test]
async fn normalizes_youtube_webvtt_inline_markup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.en.vtt");
    std::fs::write(
        &path,
        "WEBVTT\n\n00:02:21.000 --> 00:02:25.000\nYet, you, the audience, can understand why<00:02:22.640><c> the</c><00:02:22.959><c> general</c><00:02:23.280><c> solution</c><00:02:23.840><c> worked.</c><00:02:24.800><c> Now,</c>\n",
    )
    .unwrap();

    let streams = parse_caption_files(dir.path()).await.unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(
        streams[0].segments[0].text,
        "Yet, you, the audience, can understand why the general solution worked. Now,"
    );
    assert_eq!(
        streams[0].text.as_deref(),
        Some("Yet, you, the audience, can understand why the general solution worked. Now,")
    );
}

#[test]
fn vector_literal_matches_pgvector_text_shape() {
    let value = youtube_corpus::ingest::vector_literal(&[0.1, -0.2, 0.0]);
    assert_eq!(value, "[0.10000000,-0.20000000,0.00000000]");
}
