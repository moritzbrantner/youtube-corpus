# Downstream export fixture

The snapshot contains synthetic transcript records, not an assertion that real
media was transcribed. `processorVersion` is the release label. The separately
recorded `processorRevision` pins the content digest of the youtube-corpus Rust
processor source used when this synthetic receipt was constructed for issue #85.
The digest uses the build script's documented source framing (sorted source paths,
Cargo.toml, Cargo.lock and build.rs). It remains fixed when later code changes.
It identifies the corpus normalizer/orchestrator, not an external ASR model.

`derive_expected.py` independently validates and derives all three downstream
formats from the snapshot. It never repairs a missing processor receipt or derives
one from a release label. Old database rows remain unknown until actual processing
or restoration of an original receipt supplies a revision.
