mod core;

pub use core::{
    transcript_segment_contract, transcript_segment_metadata, vector_literal, IngestItemReport,
    IngestReport, IngestRequest, TranscriptSegmentContractInput,
};

pub async fn ingest_corpus(request: IngestRequest) -> anyhow::Result<IngestReport> {
    let database_url = request.database_url.clone();
    let provenance = crate::provenance::IngestProvenance::from_request(&request);
    let report = core::ingest_corpus(request).await?;
    crate::provenance::record_ingest_report(&database_url, &report, &provenance).await?;
    Ok(report)
}

pub async fn ingest_video_items(
    request: IngestRequest,
    items: Vec<crate::youtube::VideoItem>,
) -> anyhow::Result<IngestReport> {
    let database_url = request.database_url.clone();
    let provenance = crate::provenance::IngestProvenance::from_request(&request);
    let report = core::ingest_video_items(request, items).await?;
    crate::provenance::record_ingest_report(&database_url, &report, &provenance).await?;
    Ok(report)
}
