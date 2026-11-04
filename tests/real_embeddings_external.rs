//! Opt-in end-to-end check with the real multilingual-e5-base model (#192).
//!
//! Seeds English, German and Spanish segments without vectors, fills them with the `reembed`
//! job and runs a German semantic query that must rank the matching English segment first.
//!
//! ```text
//! TEXT_EMBEDDINGS_MODEL_DIR=~/.cache/model-runtime \
//! DATABASE_URL=postgres://postgres:postgres@localhost:55432/youtube_corpus_e5 \
//!   cargo test --release --test real_embeddings_external -- --ignored --nocapture
//! ```

use std::time::Instant;

use uuid::Uuid;
use youtube_corpus::config::SearchMode;
use youtube_corpus::embeddings::{self, CorpusEmbedder, EmbedderConfig};
use youtube_corpus::reembed::{reembed_corpus, ReembedRequest, ReembedTarget};
use youtube_corpus::search::{search_corpus, SearchRequest};

const VIDEO: Uuid = Uuid::from_u128(0xe5000000000000000000000000000001);
const STREAM: Uuid = Uuid::from_u128(0xe5000000000000000000000000000002);

const SEGMENTS: &[(&str, &str)] = &[
    (
        "en",
        "The Eiffel Tower is about 330 metres tall and stands in Paris.",
    ),
    (
        "en",
        "Photosynthesis turns sunlight, water and carbon dioxide into sugar.",
    ),
    (
        "en",
        "The stock market fell sharply after the interest rate decision.",
    ),
    (
        "de",
        "Der Rhein fließt von den Alpen bis zur Nordsee.",
    ),
    (
        "es",
        "La paella es un plato tradicional de Valencia con arroz.",
    ),
];

fn segment_id(index: usize) -> Uuid {
    Uuid::from_u128(0xe5000000000000000000000000001000 + index as u128)
}

#[tokio::test]
#[ignore = "requires the multilingual-e5-base bundle (TEXT_EMBEDDINGS_MODEL_DIR) and DATABASE_URL"]
async fn german_query_finds_english_segment_with_real_e5() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let started = Instant::now();
    // One model instance (1.1 GB) serves the job and the searches.
    embeddings::install_shared(
        CorpusEmbedder::load(&EmbedderConfig::from_env()).expect("load multilingual-e5-base"),
    );
    let embedder = embeddings::shared().unwrap();
    eprintln!(
        "loaded {}@{} ({}) in {:.1}s",
        embedder.identity().model_id,
        embedder.identity().revision,
        embedder.identity().pooling,
        started.elapsed().as_secs_f64()
    );
    assert_eq!(embedder.identity().model_id, "intfloat/multilingual-e5-base");
    assert_eq!(embedder.identity().pooling, "mean");

    let pool = youtube_corpus::db::connect(&database_url).await.unwrap();
    youtube_corpus::db::migrate(&pool).await.unwrap();
    seed(&pool).await;

    let report = reembed_corpus(
        &pool,
        embedder.clone(),
        &ReembedRequest {
            targets: vec![ReembedTarget::TranscriptSegments],
            batch_size: 2,
            limit: None,
        },
        |line| eprintln!("{line}"),
    )
    .await
    .unwrap();
    assert!(report.tables[0].embedded >= SEGMENTS.len() as u64);
    // A second run finds nothing left to do.
    let again = reembed_corpus(
        &pool,
        embedder.clone(),
        &ReembedRequest {
            targets: vec![ReembedTarget::TranscriptSegments],
            batch_size: 2,
            limit: None,
        },
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(again.tables[0].pending, 0);

    for mode in [SearchMode::Semantic, SearchMode::Hybrid] {
        let report = search_corpus(request(&database_url, "Wie hoch ist der Eiffelturm?", mode))
            .await
            .unwrap();
        for result in &report.results {
            eprintln!(
                "{mode:?} {:.3} (fts {:.3}, semantic {:.3}) [{}] {}",
                result.score,
                result.fts_score,
                result.semantic_score,
                result.language.as_deref().unwrap_or("-"),
                result.text
            );
        }
        assert_eq!(report.results[0].segment_id, segment_id(0), "{mode:?}");
    }

    // Language-aware FTS: the German stemmer matches another inflection ("Alpe" ~ "Alpen").
    let fts = search_corpus(request(&database_url, "Alpe", SearchMode::Fts))
        .await
        .unwrap();
    assert_eq!(fts.results[0].segment_id, segment_id(3));
}

fn request(database_url: &str, query: &str, mode: SearchMode) -> SearchRequest {
    SearchRequest {
        database_url: database_url.to_string(),
        query: query.to_string(),
        top_k: 5,
        mode,
        source_kind: None,
        modality: None,
        video_id: Some(VIDEO),
        language: None,
        transcript_start_min: None,
        transcript_start_max: None,
        upload_date_from: None,
        upload_date_to: None,
        duration_min: None,
        duration_max: None,
        channel_query: None,
        title_query: None,
        category_query: None,
        tag_query: None,
        metadata_query: None,
        view_count_min: None,
        view_count_max: None,
        youtube_id: None,
        corpus_id: None,
        speaker_label: None,
    }
}

async fn seed(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM videos WHERE id = $1")
        .bind(VIDEO)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO videos (id, youtube_id, source_url, title)
         VALUES ($1, 'e5-test-001', 'https://www.youtube.com/watch?v=e5-test-001', 'e5 test')",
    )
    .bind(VIDEO)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcript_streams (id, video_id, source_kind, language, status)
         VALUES ($1, $2, 'caption_manual', 'mul', 'completed')",
    )
    .bind(STREAM)
    .bind(VIDEO)
    .execute(pool)
    .await
    .unwrap();
    for (index, (language, text)) in SEGMENTS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO transcript_segments
             (id, stream_id, video_id, segment_index, start_seconds, end_seconds, text, language)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(segment_id(index))
        .bind(STREAM)
        .bind(VIDEO)
        .bind(index as i64)
        .bind(index as f64 * 10.0)
        .bind(index as f64 * 10.0 + 5.0)
        .bind(text)
        .bind(language)
        .execute(pool)
        .await
        .unwrap();
    }
}
