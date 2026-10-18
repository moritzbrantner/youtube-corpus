use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

// Identify the actual processor source, including locked dependency identities. This
// works for source archives and dirty builds without claiming a Git checkout is clean.
fn processor_revision() -> std::io::Result<String> {
    fn collect(directory: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                collect(&path, files)?;
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from("build.rs"),
    ];
    collect(Path::new("src"), &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    digest.update(b"youtube-corpus-processor-source-v1\0");
    println!("cargo:rerun-if-changed=src");
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.to_string_lossy();
        let bytes = fs::read(&path)?;
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    let hex = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("sha256:{hex}"))
}

fn main() {
    let revision = processor_revision().expect("processor source must be readable at build time");
    println!("cargo:rustc-env=YOUTUBE_CORPUS_PROCESSOR_REVISION={revision}");
    let dist = Path::new("dist");
    let index = dist.join("index.html");
    if !index.exists() {
        fs::create_dir_all(dist).expect("failed to create fallback dist directory");
        fs::write(
            index,
            "<!doctype html><title>YouTube Corpus</title><p>Run bun run build to build the web UI.</p>",
        )
        .expect("failed to write fallback embedded index.html");
    }
}
