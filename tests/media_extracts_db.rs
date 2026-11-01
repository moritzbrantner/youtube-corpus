//! Verifiable media extracts (#116) with a fake yt-dlp and ffmpeg-made media: a plan cuts a
//! scene clip, an audio excerpt and keyframes before it releases the media, a rerun reuses
//! them, `extract` re-acquires released media on demand and cuts identical bytes, and a
//! non-CC video without scene evidence needs the owner's opt-in and gets no invented scenes.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Answers metadata, caption and media calls the way yt-dlp does. Each call is logged.
const FAKE_YT_DLP: &str = r#"#!/usr/bin/env bash
set -euo pipefail
url="${@: -1}"
id="${url##*v=}"
mode="" out="" format=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) echo 2026.10.01; exit 0 ;;
    --dump-json) mode=metadata ;;
    --write-subs) mode=manual ;;
    --write-auto-subs) mode=auto ;;
    --format) mode=media; format="$2"; shift ;;
    -o) out="$2"; shift ;;
  esac
  shift
done
echo "$mode $id $format" >> "$FAKE_YT_DLP_LOG"
case "$mode" in
  metadata)
    printf '{"id":"%s","title":"Fixture %s","channel":"Fixture Channel","duration":4,"license":"Creative Commons Attribution license (reuse allowed)"}\n' "$id" "$id"
    ;;
  media)
    base="${out//%(id)s/$id}"
    base="${base//.%(ext)s/}"
    cp "$FAKE_MEDIA" "$base.mp4"
    echo '{"format_id":"134+251-5","requested_formats":[{"format_id":"134","acodec":"none","vcodec":"avc1","height":240},{"format_id":"251-5","acodec":"opus","vcodec":"none","language":"en"}]}' > "$base.info.json"
    echo "$base.mp4"
    ;;
esac
"#;

struct Env {
    dir: tempfile::TempDir,
    database_url: String,
}

impl Env {
    fn media_downloads(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("yt-dlp.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("media "))
            .count()
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let bin = self.dir.path().join("bin");
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        Command::new(env!("CARGO_BIN_EXE_youtube-corpus"))
            .arg("--database-url")
            .arg(&self.database_url)
            .args(args)
            .env("PATH", path)
            .env("FAKE_YT_DLP_LOG", self.dir.path().join("yt-dlp.log"))
            .env("FAKE_MEDIA", self.dir.path().join("fixture.mp4"))
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?} failed: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }
}

fn sha256_file(path: &Path) -> String {
    Sha256::digest(std::fs::read(path).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Every artifact the manifest lists has the recorded size and SHA-256.
fn verify(manifest_path: &Path) -> Value {
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "media_extract@1");
    let dir = manifest_path.parent().unwrap();
    for artifact in manifest["artifacts"].as_array().unwrap() {
        let file = dir.join(artifact["file"].as_str().unwrap());
        assert_eq!(artifact["sha256"], sha256_file(&file), "{artifact}");
        assert_eq!(artifact["bytes"], std::fs::metadata(&file).unwrap().len());
    }
    manifest
}

fn kinds(manifest: &Value) -> Vec<String> {
    manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| artifact["kind"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn extracts_are_cut_before_release_reused_and_reacquired_on_demand() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Three 1.2 s colour shots with a tone: two hard cuts for the scene detector.
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(
            "color=c=red:s=320x240:r=25:d=1.2[r];color=c=blue:s=320x240:r=25:d=1.2[b];\
             color=c=green:s=320x240:r=25:d=1.2[g];[r][b][g]concat=n=3:v=1:a=0[out0]",
        )
        .args(["-f", "lavfi", "-i", "sine=duration=3.6", "-shortest"])
        .args(["-pix_fmt", "yuv420p", "-c:v", "libx264", "-c:a", "aac"])
        .arg(dir.path().join("fixture.mp4"))
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("ffmpeg with libx264 unavailable; skipping");
        return;
    }
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let script = bin.join("yt-dlp");
        std::fs::write(&script, FAKE_YT_DLP).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let youtube_id = format!("shots{run}");
    let url = format!("https://www.youtube.com/watch?v={youtube_id}");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        serde_json::json!({
            "corpus": format!("extracts-{run}"),
            "videos": [url],
            "detectors": ["scenes"],
            "exports": [],
            "extracts": [{"video": url, "startSeconds": 1.5, "endSeconds": 2.0}],
        })
        .to_string(),
    )
    .unwrap();
    let env = Env { dir, database_url };
    let plan = plan.to_string_lossy().into_owned();
    let (out, work) = (env.path("out"), env.path("work"));
    let process = [
        "process",
        "--plan",
        &plan,
        "--out-dir",
        &out,
        "--work-dir",
        &work,
        "--producer-revision",
        "git:extract-test",
        "--migrate",
    ];

    let report = env.ok(&process);
    assert_eq!(report["failures"], 0, "{report:#}");
    let video = &report["videos"][0];
    let extract = &video["extracts"][0];
    assert_eq!(extract["outcome"], "completed", "{video:#}");
    // The media the scene stage used was cut, then released.
    assert_eq!(env.media_downloads(), 1);
    let deleted = video["media"]["deleted"].as_array().unwrap();
    assert_eq!(deleted.len(), 1, "{video:#}");
    assert!(!Path::new(deleted[0].as_str().unwrap()).exists());

    let manifest_path = PathBuf::from(extract["manifest"].as_str().unwrap());
    assert_eq!(
        extract["manifestSha256"],
        sha256_file(&manifest_path),
        "{extract}"
    );
    let manifest = verify(&manifest_path);
    assert_eq!(kinds(&manifest), ["clip", "audio", "keyframes"]);
    assert_eq!(manifest["producer"]["revision"], "git:extract-test");
    assert_eq!(manifest["cutter"]["name"], "ffmpeg");
    assert!(manifest["cutter"]["configHash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(manifest["video"]["license"]
        .as_str()
        .unwrap()
        .contains("Creative Commons Attribution"));
    assert_eq!(manifest["video"]["licenseOptIn"], false);
    assert_eq!(manifest["source"]["formatId"], "134+251-5");
    assert_eq!(
        manifest["source"]["sha256"],
        sha256_file(&env.dir.path().join("fixture.mp4"))
    );
    // The span lies inside the blue shot: the clip covers exactly that stored scene.
    let scenes = manifest["sceneEvidence"]["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 1, "{manifest:#}");
    let scene = &scenes[0];
    assert!(
        (scene["startSeconds"].as_f64().unwrap() - 1.2).abs() < 0.05,
        "{scene}"
    );
    assert!(
        (scene["endSeconds"].as_f64().unwrap() - 2.4).abs() < 0.05,
        "{scene}"
    );
    assert_eq!(manifest["startSeconds"], scene["startSeconds"]);
    assert_eq!(manifest["endSeconds"], scene["endSeconds"]);
    assert_eq!(
        manifest["sceneEvidence"]["run"]["processor"],
        "visual-analysis"
    );
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(artifact["sceneIds"], serde_json::json!([scene["id"]]));
    }

    // Rerunning the plan reuses the intact extract without media.
    let again = env.ok(&process);
    assert_eq!(again["failures"], 0, "{again:#}");
    assert_eq!(again["videos"][0]["extracts"][0]["outcome"], "reused");
    assert_eq!(
        again["videos"][0]["extracts"][0]["manifestSha256"],
        extract["manifestSha256"]
    );
    assert_eq!(env.media_downloads(), 1);

    // Extracting the released media on demand acquires it again, cuts the same bytes and
    // deletes it once cut.
    let elsewhere = env.path("elsewhere");
    let on_demand = env.ok(&[
        "extract",
        "--youtube-id",
        &youtube_id,
        "--start",
        "1.5",
        "--end",
        "2.0",
        "--out-dir",
        &elsewhere,
        "--work-dir",
        &work,
        "--producer-revision",
        "git:extract-test",
    ]);
    assert_eq!(env.media_downloads(), 2);
    assert_eq!(on_demand["mediaAcquired"], true);
    assert_eq!(on_demand["mediaDeleted"].as_array().unwrap().len(), 1);
    assert!(!Path::new(on_demand["mediaDeleted"][0].as_str().unwrap()).exists());
    assert_eq!(on_demand["manifestSha256"], extract["manifestSha256"]);
    verify(Path::new(on_demand["manifestPath"].as_str().unwrap()));

    // A video without a Creative Commons licence needs the owner's opt-in, and a span without
    // scene evidence yields only the audio excerpt of exactly that span.
    let pool = youtube_corpus::db::connect(&env.database_url)
        .await
        .unwrap();
    let standard = format!("std{run}");
    sqlx::query(
        "INSERT INTO videos (id, source_url, youtube_id, title, metadata)
         VALUES ($1, $2, $3, 'Standard', '{\"license\": \"Standard YouTube License\"}')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(format!("https://www.youtube.com/watch?v={standard}"))
    .bind(&standard)
    .execute(&pool)
    .await
    .unwrap();
    let extract_standard = |extra: &[&str]| {
        let mut args = vec![
            "extract",
            "--youtube-id",
            &standard,
            "--start",
            "0.5",
            "--end",
            "1.25",
            "--out-dir",
            &elsewhere,
            "--work-dir",
            &work,
        ];
        args.extend_from_slice(extra);
        env.run(&args)
    };
    let refused = extract_standard(&[]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--allow-non-cc"));
    assert_eq!(env.media_downloads(), 2);

    let output = extract_standard(&["--allow-non-cc"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let opted: Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifest = verify(Path::new(opted["manifestPath"].as_str().unwrap()));
    assert_eq!(manifest["video"]["licenseOptIn"], true);
    assert_eq!(manifest["video"]["license"], "Standard YouTube License");
    assert!(manifest["sceneEvidence"].is_null());
    assert_eq!(kinds(&manifest), ["audio"]);
    assert_eq!(manifest["artifacts"][0]["startSeconds"], 0.5);
    assert_eq!(manifest["artifacts"][0]["endSeconds"], 1.25);
    let skipped: Vec<&str> = manifest["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skip| skip["kind"].as_str().unwrap())
        .collect();
    assert_eq!(skipped, ["clip", "keyframes"]);
    assert!(manifest["skipped"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("no scene evidence"));
    // Audio alone needs only an audio download.
    assert_eq!(env.media_downloads(), 3);
}
