//! Offline embedding fixture for database tests (#192).
//!
//! Installs nlp-stack's `HashedFixtureEmbedder` as the process-wide embedder, explicitly and
//! only in tests: it hashes terms into 768 buckets, so similarities are lexical overlap, not
//! meaning. Production code has no fallback; without this, embedding needs the real model.
#![allow(dead_code)]

use text_embeddings::testing::HashedFixtureEmbedder;
use text_embeddings::{TextEmbeddingBackend, TextEmbeddingConfig};
use youtube_corpus::embeddings::{
    install_shared, CorpusEmbedder, EmbeddingIdentity, EMBEDDING_DIMENSIONS,
};

pub fn fixture_identity() -> EmbeddingIdentity {
    EmbeddingIdentity {
        model_id: "hashed-fixture-embedder".to_string(),
        revision: "test-fixture".to_string(),
        pooling: "none".to_string(),
        dimensions: EMBEDDING_DIMENSIONS,
    }
}

pub fn fixture_backend() -> HashedFixtureEmbedder {
    HashedFixtureEmbedder::new(
        TextEmbeddingConfig {
            dimensions: EMBEDDING_DIMENSIONS,
            use_idf: false,
        },
        text_lexical::CorpusOptions::default(),
    )
    .expect("fixture embedder")
}

/// Makes the fixture the shared embedder of this test process.
pub fn install_fixture_embedder() {
    install_shared(
        CorpusEmbedder::new(fixture_backend(), fixture_identity()).expect("fixture embedder"),
    );
}

/// Fixture vector literal of `text`, for seeding rows directly.
pub fn fixture_vector_literal(text: &str) -> String {
    let vector = fixture_backend().embed_text(text).expect("fixture vector");
    youtube_corpus::ingest::vector_literal(vector.as_slice())
}

/// The `embedding_models` row of the fixture, for seeding rows directly.
pub async fn fixture_model_row(pool: &sqlx::PgPool) -> i32 {
    youtube_corpus::embeddings::model_row_id(pool, &fixture_identity())
        .await
        .expect("fixture model row")
}
