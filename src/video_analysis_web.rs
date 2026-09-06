use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::api_types::{
    VideoAnalysisCoverage, VideoAnalysisInput, VideoAnalysisReport, VideoAnalysisSegment,
    VideoAnalysisStream,
};
use crate::config::SourceKind;

#[derive(Clone)]
struct VideoAnalysisState {
    database_url: Option<String>,
}

pub fn router(database_url: Option<String>) -> Router {
    Router::new()
        .route("/api/video-analysis", get(video_analysis))
        .with_state(VideoAnalysisState { database_url })
}

async fn video_analysis(
    State(state): State<VideoAnalysisState>,
    Query(input): Query<VideoAnalysisInput>,
) -> Result<Json<VideoAnalysisReport>, AnalysisError> {
    let source_url = input.source_url.trim();
    if source_url.is_empty() {
        return Err(AnalysisError::bad_request("sourceUrl is required."));
    }

    let database_url = state
        .database_url
        .as_deref()
        .ok_or_else(|| AnalysisError::bad_request("DATABASE_URL is required."))?;
    let pool = crate::db::connect(database_url)
        .await
        .map_err(AnalysisError::internal)?;
    let video = crate::status::find_video_by_source_url(&pool, source_url)
        .await
        .map_err(AnalysisError::internal)?
        .ok_or_else(|| AnalysisError::not_found("Video has not been ingested yet."))?;

    let streams = load_streams(&pool, video.id).await?;
    let primary_stream = streams.iter().find(|stream| stream.segment_count > 0);
    let primary_stream_id = primary_stream.map(|stream| stream.stream_id);
    let segments = match primary_stream_id {
        Some(stream_id) => load_segments(&pool, stream_id).await?,
        None => Vec::new(),
    };
    let lexical_analysis = analyze_transcript(primary_stream, &segments)?;
    let visual_timeline = has_visual_evidence(&pool, video.id).await?;
    let audio_features = has_audio_evidence(&pool, video.id).await?;

    let coverage = VideoAnalysisCoverage {
        metadata: true,
        transcript: !segments.is_empty(),
        lexical: lexical_analysis.is_some(),
        media_retained: video.media_downloaded,
        visual_timeline,
        audio_features,
    };

    Ok(Json(VideoAnalysisReport {
        video,
        streams,
        primary_stream_id,
        segments,
        lexical_analysis,
        coverage,
    }))
}

async fn load_streams(
    pool: &PgPool,
    video_id: Uuid,
) -> Result<Vec<VideoAnalysisStream>, AnalysisError> {
    let rows = sqlx::query(
        "SELECT st.id, st.source_kind, st.language, st.status, st.message,
                count(ts.id)::bigint AS segment_count
         FROM transcript_streams st
         LEFT JOIN transcript_segments ts ON ts.stream_id = st.id
         WHERE st.video_id = $1
         GROUP BY st.id, st.source_kind, st.language, st.status, st.message, st.created_at
         ORDER BY CASE st.source_kind
                    WHEN 'caption_manual' THEN 0
                    WHEN 'caption_auto' THEN 1
                    WHEN 'asr' THEN 2
                    ELSE 3
                  END,
                  segment_count DESC,
                  st.language NULLS LAST,
                  st.created_at",
    )
    .bind(video_id)
    .fetch_all(pool)
    .await
    .map_err(AnalysisError::internal)?;

    rows.into_iter()
        .map(|row| {
            let source_kind: String = row.try_get("source_kind").map_err(AnalysisError::internal)?;
            Ok(VideoAnalysisStream {
                stream_id: row.try_get("id").map_err(AnalysisError::internal)?,
                source_kind: parse_source_kind(&source_kind)?,
                language: row.try_get("language").map_err(AnalysisError::internal)?,
                status: row.try_get("status").map_err(AnalysisError::internal)?,
                segment_count: row
                    .try_get("segment_count")
                    .map_err(AnalysisError::internal)?,
                message: row.try_get("message").map_err(AnalysisError::internal)?,
            })
        })
        .collect()
}

async fn load_segments(
    pool: &PgPool,
    stream_id: Uuid,
) -> Result<Vec<VideoAnalysisSegment>, AnalysisError> {
    let rows = sqlx::query(
        "SELECT id, stream_id, segment_index, start_seconds, end_seconds, text, language
         FROM transcript_segments
         WHERE stream_id = $1
         ORDER BY segment_index",
    )
    .bind(stream_id)
    .fetch_all(pool)
    .await
    .map_err(AnalysisError::internal)?;

    rows.into_iter()
        .map(|row| {
            Ok(VideoAnalysisSegment {
                segment_id: row.try_get("id").map_err(AnalysisError::internal)?,
                stream_id: row.try_get("stream_id").map_err(AnalysisError::internal)?,
                segment_index: row
                    .try_get("segment_index")
                    .map_err(AnalysisError::internal)?,
                start_seconds: row
                    .try_get("start_seconds")
                    .map_err(AnalysisError::internal)?,
                end_seconds: row
                    .try_get("end_seconds")
                    .map_err(AnalysisError::internal)?,
                text: row.try_get("text").map_err(AnalysisError::internal)?,
                language: row.try_get("language").map_err(AnalysisError::internal)?,
            })
        })
        .collect()
}

fn analyze_transcript(
    primary_stream: Option<&VideoAnalysisStream>,
    segments: &[VideoAnalysisSegment],
) -> Result<Option<serde_json::Value>, AnalysisError> {
    if segments.is_empty() {
        return Ok(None);
    }

    let text = segments
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Ok(None);
    }

    let response = text_lexical::surface::run_surface_operation(runtime_core::SurfaceRequest {
        operation: runtime_core::OperationId::new("lexical.analyze"),
        input: serde_json::json!({
            "text": text,
            "language": primary_stream.and_then(|stream| stream.language.as_deref()),
            "maxTerms": 16,
            "maxSummarySentences": 6
        }),
    })
    .map_err(AnalysisError::internal)?;

    Ok(Some(concrete_operation_result(response.value)))
}

fn concrete_operation_result(value: serde_json::Value) -> serde_json::Value {
    value.get("result").cloned().unwrap_or(value)
}

async fn has_visual_evidence(pool: &PgPool, video_id: Uuid) -> Result<bool, AnalysisError> {
    Ok(optional_table_has_video_rows(pool, "face_observations", video_id).await?
        || optional_table_has_video_rows(pool, "video_scenes", video_id).await?)
}

async fn has_audio_evidence(pool: &PgPool, video_id: Uuid) -> Result<bool, AnalysisError> {
    optional_table_has_video_rows(pool, "voice_observations", video_id).await
}

async fn optional_table_has_video_rows(
    pool: &PgPool,
    table: &str,
    video_id: Uuid,
) -> Result<bool, AnalysisError> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(table)
        .fetch_one(pool)
        .await
        .map_err(AnalysisError::internal)?;
    if !exists {
        return Ok(false);
    }

    let sql = match table {
        "face_observations" => "SELECT EXISTS(SELECT 1 FROM face_observations WHERE video_id = $1)",
        "voice_observations" => {
            "SELECT EXISTS(SELECT 1 FROM voice_observations WHERE video_id = $1)"
        }
        "video_scenes" => "SELECT EXISTS(SELECT 1 FROM video_scenes WHERE video_id = $1)",
        _ => return Ok(false),
    };
    sqlx::query_scalar(sql)
        .bind(video_id)
        .fetch_one(pool)
        .await
        .map_err(AnalysisError::internal)
}

fn parse_source_kind(value: &str) -> Result<SourceKind, AnalysisError> {
    match value {
        "caption_manual" => Ok(SourceKind::CaptionManual),
        "caption_auto" => Ok(SourceKind::CaptionAuto),
        "asr" => Ok(SourceKind::Asr),
        _ => Err(AnalysisError::internal(format!(
            "invalid transcript source kind: {value}"
        ))),
    }
}

#[derive(Debug, Serialize)]
struct AnalysisErrorBody {
    error: AnalysisErrorMessage,
}

#[derive(Debug, Serialize)]
struct AnalysisErrorMessage {
    message: String,
}

#[derive(Debug)]
struct AnalysisError {
    status: StatusCode,
    message: String,
}

impl AnalysisError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for AnalysisError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(AnalysisErrorBody {
                error: AnalysisErrorMessage {
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::concrete_operation_result;

    #[test]
    fn returns_domain_result_from_structured_surface_value() {
        let value = serde_json::json!({
            "operation": "lexical.analyze",
            "summary": {"status": "ok"},
            "keywords": [{"text": "rust"}],
            "result": {
                "summary": {"unique_terms": 3},
                "keywords": [{"text": "rust"}]
            }
        });

        let result = concrete_operation_result(value);
        assert_eq!(result["summary"]["unique_terms"], 3);
        assert_eq!(result["keywords"][0]["text"], "rust");
    }
}
