mod core;

use std::collections::HashSet;

pub use core::{
    transcript_segment_contract, transcript_segment_metadata, vector_literal, IngestItemReport,
    IngestReport, IngestRequest, TranscriptSegmentContractInput,
};

pub async fn ingest_corpus(request: IngestRequest) -> anyhow::Result<IngestReport> {
    let database_url = request.database_url.clone();
    let provenance = crate::provenance::IngestProvenance::from_request(&request);
    let report = core::ingest_corpus(request).await?;
    crate::provenance::record_ingest_report(&database_url, &report, &provenance).await?;
    refresh_ingest_quality(&database_url, &report).await?;
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
    refresh_ingest_quality(&database_url, &report).await?;
    Ok(report)
}

async fn refresh_ingest_quality(database_url: &str, report: &IngestReport) -> anyhow::Result<()> {
    let video_ids = report
        .items
        .iter()
        .filter_map(|item| item.video_id)
        .collect::<HashSet<_>>();
    if video_ids.is_empty() {
        return Ok(());
    }
    let pool = crate::db::connect(database_url).await?;
    for video_id in video_ids {
        crate::transcript_quality::refresh_video_quality(&pool, video_id).await?;
    }
    Ok(())
}
