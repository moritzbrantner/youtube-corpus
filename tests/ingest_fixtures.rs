use youtube_corpus::captions::parse_caption_files;
use youtube_corpus::config::SourceKind;
use youtube_corpus::ingest::{
    transcript_segment_contract, transcript_segment_metadata, TranscriptSegmentContractInput,
};

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

#[test]
fn transcript_segment_metadata_preserves_text_contract_fields() {
    let stream_id = uuid::Uuid::from_u128(0x50000000000000000000000000000001);
    let contract = transcript_segment_contract(TranscriptSegmentContractInput {
        stream_id,
        source_url: "https://youtube.test/video",
        source_kind: SourceKind::CaptionManual,
        segment_index: 7,
        text: "contract text",
        language: Some("en".to_string()),
        start_seconds: Some(12.5),
        end_seconds: Some(15.0),
    });
    let metadata = transcript_segment_metadata(
        "https://youtube.test/video",
        SourceKind::CaptionManual,
        stream_id,
        &contract,
        Some(12.5),
        Some(15.0),
    )
    .unwrap();

    assert_eq!(metadata["source_url"], "https://youtube.test/video");
    assert_eq!(metadata["source_kind"], "caption_manual");
    assert_eq!(metadata["stream_id"], stream_id.to_string());
    assert_eq!(metadata["segment_index"], 7);
    assert_eq!(metadata["timestamp_seconds"], 12.5);
    assert_eq!(metadata["duration_seconds"], 2.5);
    assert_eq!(metadata["language"], "en");
    assert_eq!(metadata["text_contract"]["streamId"], stream_id.to_string());
    assert_eq!(
        metadata["text_contract"]["source"]["uri"],
        "https://youtube.test/video"
    );
    assert_eq!(
        metadata["text_contract"]["source"]["sourceKind"],
        "caption_manual"
    );
}
