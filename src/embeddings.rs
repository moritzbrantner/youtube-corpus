//! Semantic text embeddings for corpus search (#192).
//!
//! Every transcript segment, OCR text track and translated span is embedded with the pinned
//! nlp-stack `multilingual-e5-base` preset (768 dimensions, mean pooling). Stored texts are
//! embedded as passages and search queries as queries; the backend adds the model's
//! `passage: ` / `query: ` prefixes itself. There is no fallback embedder: when the pinned
//! model bundle is missing or does not match its digests, embedding fails with an error that
//! says where the bundle is expected.
//!
//! The model is loaded once per process, on first use, and shared. Every stored vector
//! references the `embedding_models` row (model id, revision, pooling, dimensions) that
//! produced it, and semantic search only compares vectors of the current model.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use text_embeddings::{
    load_embedding_preset, EmbeddingPresetLoadOptions, MultilingualEmbeddingPreset,
    TextEmbeddingBackend,
};

/// The embedding preset this corpus uses (owner decision 2026-11-03).
pub const EMBEDDING_PRESET: MultilingualEmbeddingPreset =
    MultilingualEmbeddingPreset::MultilingualE5Base;
/// Dimensions of every `embedding` column (migration `0033_real_embeddings.sql`).
pub const EMBEDDING_DIMENSIONS: usize = 768;
/// Directory holding `model-runtime` bundles (`<dir>/multilingual-e5-base/<revision>/`).
pub const MODEL_DIR_ENV: &str = "TEXT_EMBEDDINGS_MODEL_DIR";
/// Set to `1` to let the shared embedder download the pinned bundle when it is missing.
pub const ALLOW_DOWNLOAD_ENV: &str = "YOUTUBE_CORPUS_EMBEDDING_DOWNLOAD";

/// A required model is missing or unusable. Nothing substitutes heuristic output for it
/// (owner decision 2026-11-03); callers can downcast to this type to tell "install the model"
/// apart from other failures.
#[derive(Debug, thiserror::Error)]
#[error("model required: {capability} is not available; {hint}")]
pub struct ModelRequired {
    /// What needs the model, with the pinned model id and revision.
    pub capability: String,
    /// How to make the model available.
    pub hint: String,
    #[source]
    pub source: anyhow::Error,
}

/// Identity recorded for every stored vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingIdentity {
    /// Model id, for example `intfloat/multilingual-e5-base`.
    pub model_id: String,
    /// Exact model revision (Hugging Face commit).
    pub revision: String,
    /// Pooling the vectors were produced with (`mean`, `cls`, `last_token`).
    pub pooling: String,
    pub dimensions: usize,
}

impl EmbeddingIdentity {
    /// The identity of [`EMBEDDING_PRESET`], without loading it.
    pub fn preset() -> Self {
        Self {
            model_id: EMBEDDING_PRESET.repo_id().to_string(),
            revision: EMBEDDING_PRESET.revision().to_string(),
            pooling: pooling_name(EMBEDDING_PRESET.pooling()),
            dimensions: EMBEDDING_PRESET.dimensions(),
        }
    }
}

/// Where the shared embedder finds its model bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedderConfig {
    pub model_dir: PathBuf,
    pub allow_download: bool,
}

impl EmbedderConfig {
    /// `TEXT_EMBEDDINGS_MODEL_DIR` (default `use-case-output/models`), downloads only when
    /// `YOUTUBE_CORPUS_EMBEDDING_DOWNLOAD=1`.
    pub fn from_env() -> Self {
        Self {
            model_dir: std::env::var_os(MODEL_DIR_ENV)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(crate::entities::DEFAULT_MODEL_DIR)),
            allow_download: std::env::var(ALLOW_DOWNLOAD_ENV).is_ok_and(|value| value == "1"),
        }
    }
}

/// A loaded embedding model together with the identity stored beside its vectors.
pub struct CorpusEmbedder {
    backend: Box<dyn TextEmbeddingBackend + Send + Sync>,
    identity: EmbeddingIdentity,
}

impl std::fmt::Debug for CorpusEmbedder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CorpusEmbedder")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl CorpusEmbedder {
    /// Loads [`EMBEDDING_PRESET`] from `config.model_dir`, verifying its pinned revision and
    /// file digests. Fails when the bundle is missing (and may not be downloaded).
    pub fn load(config: &EmbedderConfig) -> anyhow::Result<Self> {
        let options = EmbeddingPresetLoadOptions {
            bundle_root: config.model_dir.clone(),
            auto_download: config.allow_download,
            download_progress: config.allow_download,
            hf_token: None,
        };
        let backend = load_embedding_preset(EMBEDDING_PRESET, &options).map_err(|source| {
            anyhow::Error::new(ModelRequired {
                capability: format!(
                    "text embeddings ({}@{})",
                    EMBEDDING_PRESET.repo_id(),
                    EMBEDDING_PRESET.revision()
                ),
                hint: format!(
                    "place the pinned bundle at {}/{}/{}/ (set {MODEL_DIR_ENV} to use another \
                     directory, for example ~/.cache/model-runtime) or allow the download \
                     ({ALLOW_DOWNLOAD_ENV}=1, or `reembed` without --no-download); there is no \
                     fallback embedder",
                    config.model_dir.display(),
                    EMBEDDING_PRESET.as_str(),
                    EMBEDDING_PRESET.revision(),
                ),
                source: anyhow::anyhow!("{source}"),
            })
        })?;
        let info = backend.model_info();
        let identity = EmbeddingIdentity {
            model_id: info.model_name,
            revision: info
                .revision
                .context("the embedding backend reported no model revision")?,
            pooling: pooling_name(
                info.pooling
                    .context("the embedding backend reported no pooling")?,
            ),
            dimensions: info.dimensions,
        };
        Self::new(backend, identity)
    }

    /// Wraps any backend with an explicit identity. Tests use this with
    /// `text_embeddings::testing::HashedFixtureEmbedder`; production code uses [`Self::load`].
    pub fn new(
        backend: impl TextEmbeddingBackend + Send + Sync + 'static,
        identity: EmbeddingIdentity,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            identity.dimensions == EMBEDDING_DIMENSIONS,
            "embedding model {} has {} dimensions, but the corpus stores {EMBEDDING_DIMENSIONS}",
            identity.model_id,
            identity.dimensions
        );
        anyhow::ensure!(
            !identity.model_id.trim().is_empty()
                && !identity.revision.trim().is_empty()
                && !identity.pooling.trim().is_empty(),
            "an embedding identity needs a model id, revision and pooling"
        );
        Ok(Self {
            backend: Box::new(backend),
            identity,
        })
    }

    pub fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }

    /// Embeds stored texts (segments, OCR tracks, translations) as passages.
    pub fn embed_passages(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let vectors = self
            .backend
            .embed_passages(texts)
            .context("failed to embed passages")?;
        anyhow::ensure!(
            vectors.len() == texts.len(),
            "the embedding model returned {} vectors for {} passages",
            vectors.len(),
            texts.len()
        );
        vectors
            .into_iter()
            .map(|vector| self.checked(vector.as_slice().to_vec()))
            .collect()
    }

    /// Embeds one search query.
    pub fn embed_query(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let vector = self
            .backend
            .embed_query(text)
            .context("failed to embed the search query")?;
        self.checked(vector.as_slice().to_vec())
    }

    fn checked(&self, vector: Vec<f32>) -> anyhow::Result<Vec<f32>> {
        anyhow::ensure!(
            vector.len() == EMBEDDING_DIMENSIONS,
            "the embedding model returned a {}-dimensional vector, expected {EMBEDDING_DIMENSIONS}",
            vector.len()
        );
        anyhow::ensure!(
            vector.iter().all(|value| value.is_finite()),
            "the embedding model returned a non-finite vector"
        );
        Ok(vector)
    }
}

fn pooling_name(pooling: text_embeddings::PoolingStrategy) -> String {
    serde_json::to_value(pooling)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{pooling:?}").to_ascii_lowercase())
}

struct SharedState {
    config: Option<EmbedderConfig>,
    embedder: Option<Arc<CorpusEmbedder>>,
}

static SHARED: Mutex<SharedState> = Mutex::new(SharedState {
    config: None,
    embedder: None,
});

fn lock() -> std::sync::MutexGuard<'static, SharedState> {
    SHARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sets where the shared embedder loads its model from. Takes effect only before the model
/// is first loaded; returns `false` when a model is already loaded or installed.
pub fn configure(config: EmbedderConfig) -> bool {
    let mut state = lock();
    if state.embedder.is_some() {
        return false;
    }
    state.config = Some(config);
    true
}

/// Replaces the shared embedder, for example with a test fixture or an embedder a host
/// application loaded itself.
pub fn install_shared(embedder: CorpusEmbedder) {
    lock().embedder = Some(Arc::new(embedder));
}

/// The process-wide embedder, loaded on first use. A failed load is not cached, so a later
/// call retries (for example after the bundle was downloaded).
///
/// Loading takes seconds and blocks; async code calls [`shared_async`].
pub fn shared() -> anyhow::Result<Arc<CorpusEmbedder>> {
    let mut state = lock();
    if let Some(embedder) = &state.embedder {
        return Ok(embedder.clone());
    }
    let config = state
        .config
        .clone()
        .unwrap_or_else(EmbedderConfig::from_env);
    let embedder = Arc::new(CorpusEmbedder::load(&config)?);
    tracing::info!(
        model = %embedder.identity.model_id,
        revision = %embedder.identity.revision,
        "loaded embedding model"
    );
    state.embedder = Some(embedder.clone());
    Ok(embedder)
}

/// [`shared`] on the blocking thread pool.
pub async fn shared_async() -> anyhow::Result<Arc<CorpusEmbedder>> {
    tokio::task::spawn_blocking(shared).await?
}

/// Embeds `texts` as passages with the shared embedder, off the async runtime.
pub async fn embed_passages(
    texts: Vec<String>,
) -> anyhow::Result<(Arc<CorpusEmbedder>, Vec<Vec<f32>>)> {
    tokio::task::spawn_blocking(move || {
        let embedder = shared()?;
        let refs = texts.iter().map(String::as_str).collect::<Vec<_>>();
        let vectors = embedder.embed_passages(&refs)?;
        Ok((embedder, vectors))
    })
    .await?
}

/// Embeds a search query with the shared embedder, off the async runtime.
pub async fn embed_query(text: String) -> anyhow::Result<(Arc<CorpusEmbedder>, Vec<f32>)> {
    tokio::task::spawn_blocking(move || {
        let embedder = shared()?;
        let vector = embedder.embed_query(&text)?;
        Ok((embedder, vector))
    })
    .await?
}

/// Vectors of stored texts, aligned with the texts they were computed from.
#[derive(Debug, Clone, Default)]
pub struct PassageVectors {
    /// The model that produced the vectors; `None` when every text was blank.
    pub embedder: Option<Arc<CorpusEmbedder>>,
    /// Postgres vector literals; `None` for a blank text.
    pub literals: Vec<Option<String>>,
}

/// Embeds the non-blank (trimmed) `texts` as passages with the shared embedder. The model is
/// only loaded when at least one text is non-blank.
pub async fn embed_passage_literals(texts: Vec<String>) -> anyhow::Result<PassageVectors> {
    let positions = texts
        .iter()
        .enumerate()
        .filter(|(_, text)| !text.trim().is_empty())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut literals = vec![None; texts.len()];
    if positions.is_empty() {
        return Ok(PassageVectors {
            embedder: None,
            literals,
        });
    }
    let inputs = positions
        .iter()
        .map(|&index| texts[index].trim().to_string())
        .collect();
    let (embedder, vectors) = embed_passages(inputs).await?;
    for (index, vector) in positions.into_iter().zip(vectors) {
        literals[index] = Some(vector_literal(&vector));
    }
    Ok(PassageVectors {
        embedder: Some(embedder),
        literals,
    })
}

impl PassageVectors {
    /// The `embedding_models` row of these vectors (`None` when nothing was embedded).
    pub async fn model_row<'e, E>(&self, executor: E) -> anyhow::Result<Option<i32>>
    where
        E: sqlx::PgExecutor<'e>,
    {
        match &self.embedder {
            Some(embedder) => Ok(Some(model_row_id(executor, embedder.identity()).await?)),
            None => Ok(None),
        }
    }
}

/// The `embedding_models` row for `identity`, created when missing.
pub async fn model_row_id<'e, E>(executor: E, identity: &EmbeddingIdentity) -> anyhow::Result<i32>
where
    E: sqlx::PgExecutor<'e>,
{
    let id = sqlx::query_scalar::<_, i32>(
        "WITH inserted AS (
           INSERT INTO embedding_models (model_id, revision, pooling, dimensions)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (model_id, revision, pooling) DO NOTHING
           RETURNING id
         )
         SELECT id FROM inserted
         UNION ALL
         SELECT id FROM embedding_models
         WHERE model_id = $1 AND revision = $2 AND pooling = $3
         LIMIT 1",
    )
    .bind(&identity.model_id)
    .bind(&identity.revision)
    .bind(&identity.pooling)
    .bind(identity.dimensions as i32)
    .fetch_one(executor)
    .await
    .context("failed to record the embedding model")?;
    Ok(id)
}

/// Postgres `vector` literal of `values`.
pub fn vector_literal(values: &[f32]) -> String {
    crate::ingest::vector_literal(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use text_embeddings::testing::HashedFixtureEmbedder;
    use text_embeddings::TextEmbeddingConfig;

    fn fixture(dimensions: usize) -> anyhow::Result<CorpusEmbedder> {
        CorpusEmbedder::new(
            HashedFixtureEmbedder::new(
                TextEmbeddingConfig {
                    dimensions,
                    use_idf: false,
                },
                text_lexical::CorpusOptions::default(),
            )?,
            EmbeddingIdentity {
                model_id: "hashed-fixture-embedder".into(),
                revision: "test".into(),
                pooling: "none".into(),
                dimensions,
            },
        )
    }

    #[test]
    fn preset_identity_is_pinned_e5_base() {
        let identity = EmbeddingIdentity::preset();
        assert_eq!(identity.model_id, "intfloat/multilingual-e5-base");
        assert_eq!(
            identity.revision,
            "d128750597153bb5987e10b1c3493a34e5a4502a"
        );
        assert_eq!(identity.pooling, "mean");
        assert_eq!(identity.dimensions, EMBEDDING_DIMENSIONS);
    }

    #[test]
    fn wrong_dimensions_are_rejected() {
        assert!(fixture(128).is_err());
        let embedder = fixture(EMBEDDING_DIMENSIONS).unwrap();
        assert_eq!(embedder.embed_query("alpha").unwrap().len(), 768);
        assert_eq!(embedder.embed_passages(&["a b", "c"]).unwrap().len(), 2);
    }

    #[test]
    fn missing_bundle_fails_with_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = CorpusEmbedder::load(&EmbedderConfig {
            model_dir: dir.path().to_path_buf(),
            allow_download: false,
        })
        .unwrap_err();
        let required = error
            .downcast_ref::<ModelRequired>()
            .expect("a missing bundle is a ModelRequired error");
        assert!(required.capability.contains(EMBEDDING_PRESET.revision()));
        let message = format!("{error:#}");
        assert!(
            message.contains("intfloat/multilingual-e5-base"),
            "{message}"
        );
        assert!(message.contains("no fallback"), "{message}");
    }
}
