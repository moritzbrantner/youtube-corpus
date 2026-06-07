use serde::{Deserialize, Serialize};
use youtube_corpus::{search_corpus, SearchMode, SearchRequest};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseStatus {
    configured: bool,
    database_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchTranscriptsInput {
    database_url: Option<String>,
    query: String,
    mode: SearchMode,
    top_k: i64,
}

#[tauri::command]
fn database_status() -> DatabaseStatus {
    let database_url = std::env::var("DATABASE_URL").ok();
    DatabaseStatus {
        configured: database_url.is_some(),
        database_url,
    }
}

#[tauri::command]
async fn search_transcripts(
    input: SearchTranscriptsInput,
) -> Result<youtube_corpus::SearchReport, String> {
    let database_url = match input
        .database_url
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("DATABASE_URL").ok())
    {
        Some(database_url) => database_url,
        None => return Err("DATABASE_URL is required.".to_string()),
    };

    if input.query.trim().is_empty() {
        return Err("query is required.".to_string());
    }

    if input.top_k <= 0 {
        return Err("topK must be positive.".to_string());
    }

    search_corpus(SearchRequest {
        database_url,
        query: input.query,
        top_k: input.top_k,
        mode: input.mode,
        source_kind: None,
        video_id: None,
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            database_status,
            search_transcripts
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
