use std::fs;
use std::path::Path;

fn main() {
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
