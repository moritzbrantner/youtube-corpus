// Its own test binary: ONNX Runtime is loaded once per process (vox#159).
use audio_analysis_speakers::{OnnxSpeakerEmbedder, OnnxSpeakerEmbeddingConfig};

/// The voices stage builds this embedder; without a usable ONNX Runtime it must fail with the
/// reason instead of hanging inside ONNX Runtime setup.
#[test]
fn voices_embedder_without_onnx_runtime_fails_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("voxceleb_resnet34_LM.onnx");
    std::fs::write(&model, b"not a model").unwrap();
    std::env::set_var(
        "ORT_DYLIB_PATH",
        dir.path().join("libonnxruntime-missing.so"),
    );

    let Err(error) = OnnxSpeakerEmbedder::from_config(OnnxSpeakerEmbeddingConfig {
        bundle_path: model,
        embedding_dimension: 256,
        ..Default::default()
    }) else {
        panic!("the speaker embedder must fail without an ONNX Runtime dylib");
    };

    let message = error.to_string();
    assert!(message.contains("ONNX Runtime unavailable"), "{message}");
    assert!(message.contains("ORT_DYLIB_PATH"), "{message}");
}
