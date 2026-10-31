//! `youtube-corpus process` end to end with a fake yt-dlp and a fake transcriber (#128): a
//! plan takes freely licensed videos through ingest, captions, ASR and scenes, writes the
//! citable exports, deletes the downloaded media, and a second run reuses everything.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

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
    license='"Creative Commons Attribution license (reuse allowed)"'
    case "$id" in notfree*) license=null ;; esac
    printf '{"id":"%s","title":"Fixture %s","channel":"Fixture Channel","duration":2,"license":%s}\n' "$id" "$id" "$license"
    ;;
  manual)
    case "$id" in
      captioned*)
        path="${out//%(id)s/$id}"
        path="${path//%(ext)s/en.vtt}"
        printf 'WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nA caption line about geometry nodes\n' > "$path"
        ;;
    esac
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

/// A Whisper-compatible transcriber: `<media> … --output_dir <dir> --output_format json`.
const FAKE_TRANSCRIBER: &str = r#"#!/usr/bin/env bash
set -euo pipefail
media="$1"; shift
while [ $# -gt 0 ]; do
  case "$1" in --output_dir) out="$2"; shift ;; esac
  shift
done
stem="$(basename "$media")"; stem="${stem%.*}"
printf '{"text":"Spoken words from the fake transcriber","language":"en","segments":[{"id":0,"start":0.0,"end":1.5,"text":"Spoken words from the fake transcriber"}]}' > "$out/$stem.json"
"#;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

struct Env {
    dir: tempfile::TempDir,
    database_url: String,
    plan: PathBuf,
}

impl Env {
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("yt-dlp.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn media_downloads(&self) -> usize {
        self.log()
            .iter()
            .filter(|line| line.starts_with("media "))
            .count()
    }

    /// Runs the documented command and returns its report.
    fn process(&self, out: &str) -> Value {
        let bin = self.dir.path().join("bin");
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let out_dir = self.dir.path().join(out);
        let output = Command::new(env!("CARGO_BIN_EXE_youtube-corpus"))
            .arg("--database-url")
            .arg(&self.database_url)
            .arg("process")
            .arg("--plan")
            .arg(&self.plan)
            .arg("--out-dir")
            .arg(&out_dir)
            .arg("--work-dir")
            .arg(self.dir.path().join("work"))
            .arg("--producer-revision")
            .arg("git:process-test")
            .arg("--migrate")
            .env("PATH", path)
            .env("FAKE_YT_DLP_LOG", self.dir.path().join("yt-dlp.log"))
            .env("FAKE_MEDIA", self.dir.path().join("fixture.mp4"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "process failed: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let report: Value =
            serde_json::from_slice(&std::fs::read(out_dir.join("workflow-report.json")).unwrap())
                .unwrap();
        assert_eq!(
            report,
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        );
        report
    }
}

fn stage<'a>(video: &'a Value, detector: &str) -> &'a str {
    video["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["detector"] == detector)
        .unwrap_or_else(|| panic!("no {detector} stage in {video}"))["outcome"]
        .as_str()
        .unwrap()
}

#[tokio::test]
#[ignore = "requires DATABASE_URL and a pgvector-enabled Postgres database"]
async fn process_plan_ingests_exports_and_releases_media_then_reuses_everything() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL unset; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let fixture = dir.path().join("fixture.mp4");
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=size=320x240:duration=2")
        .args(["-f", "lavfi", "-i", "sine=duration=2", "-shortest"])
        .args(["-pix_fmt", "yuv420p"])
        .arg(&fixture)
        .status()
        .is_ok_and(|status| status.success());
    if !made {
        eprintln!("ffmpeg unavailable; skipping");
        return;
    }
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    script(&bin, "yt-dlp", FAKE_YT_DLP);
    let transcriber = script(&bin, "fake-transcriber", FAKE_TRANSCRIBER);

    // Unique ids so reruns against the same database start fresh.
    let run = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let captioned = format!("https://www.youtube.com/watch?v=captioned{run}");
    let silent = format!("https://www.youtube.com/watch?v=silent{run}");
    let not_free = format!("https://www.youtube.com/watch?v=notfree{run}");
    let corpus = format!("process-{run}");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        serde_json::json!({
            "corpus": corpus,
            "videos": [captioned, silent, not_free],
            "detectors": ["captions", "asr", "scenes"],
            "transcriber": {"command": transcriber},
        })
        .to_string(),
    )
    .unwrap();
    let env = Env {
        dir,
        database_url,
        plan,
    };

    let report = env.process("first");
    assert_eq!(report["failures"], 0, "{report:#}");
    assert_eq!(report["corpus"], corpus.as_str());
    let videos = report["videos"].as_array().unwrap();
    assert_eq!(videos.len(), 3);
    let (captioned_report, silent_report, rejected) = (&videos[0], &videos[1], &videos[2]);

    // A video without a Creative Commons licence is not stored at all.
    assert_eq!(rejected["outcome"], "license_rejected", "{rejected:#}");
    assert!(rejected["videoId"].is_null());
    let pool = youtube_corpus::db::connect(&env.database_url)
        .await
        .unwrap();
    let stored: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM videos WHERE id = $1)")
        .bind(youtube_corpus::youtube::stable_video_id(&not_free))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!stored);

    for video in [captioned_report, silent_report] {
        assert_eq!(video["outcome"], "processed", "{video:#}");
        assert!(video["license"]
            .as_str()
            .unwrap()
            .contains("Creative Commons Attribution"));
        assert_eq!(stage(video, "asr"), "completed", "{video:#}");
        assert_eq!(stage(video, "scenes"), "completed", "{video:#}");
        // One capped video download served ASR and scenes, and is gone again.
        let deleted = video["media"]["deleted"].as_array().unwrap();
        assert_eq!(deleted.len(), 1, "{video:#}");
        assert!(!Path::new(deleted[0].as_str().unwrap()).exists());
        assert!(video["retainedMedia"].is_null());
    }
    // Ingest fetched the captions once; the plan run did not ask again.
    assert_eq!(stage(captioned_report, "captions"), "completed");
    assert_eq!(stage(silent_report, "captions"), "skipped");
    let manual_requests = |log: Vec<String>| {
        log.iter()
            .filter(|line| line.starts_with("manual "))
            .count()
    };
    assert_eq!(manual_requests(env.log()), 2, "{:?}", env.log());
    assert_eq!(env.media_downloads(), 2, "{:?}", env.log());
    assert!(env
        .log()
        .iter()
        .filter(|line| line.starts_with("media "))
        .all(|line| line.ends_with("bv*[height<=360]+ba/b[height<=360]/wv*+ba/w")));

    // The exports cite the stored transcripts without any media file.
    let exports = report["exports"].as_array().unwrap();
    assert_eq!(exports.len(), 3);
    let source_span =
        std::fs::read_to_string(env.dir.path().join("first/source-span.json")).unwrap();
    assert!(source_span.contains("A caption line about geometry nodes"));
    assert!(source_span.contains("Spoken words from the fake transcriber"));
    assert!(source_span.contains("git:process-test"));
    assert!(!source_span.contains(&format!("notfree{run}")));
    let corpus_core =
        std::fs::read_to_string(env.dir.path().join("first/corpus-core.jsonl")).unwrap();
    assert!(corpus_core.contains("Spoken words from the fake transcriber"));
    assert!(env.dir.path().join("first/belief-evidence.json").is_file());

    // Re-running the unchanged plan downloads no media and reproduces identical exports.
    let again = env.process("second");
    assert_eq!(again["failures"], 0, "{again:#}");
    assert_eq!(env.media_downloads(), 2, "{:?}", env.log());
    for video in &again["videos"].as_array().unwrap()[..2] {
        assert_eq!(stage(video, "asr"), "reused", "{video:#}");
        assert_eq!(stage(video, "scenes"), "reused", "{video:#}");
        assert!(video["media"]["deleted"].as_array().unwrap().is_empty());
    }
    for (first, second) in exports.iter().zip(again["exports"].as_array().unwrap()) {
        assert_eq!(first["sha256"], second["sha256"], "{first} vs {second}");
    }

    // A changed transcriber reruns ASR on media acquired again. That download replaces the
    // file scenes came from, so scenes runs on it too and the media is released again.
    script(
        &env.dir.path().join("bin"),
        "fake-transcriber",
        &FAKE_TRANSCRIBER.replace("fake transcriber", "changed transcriber"),
    );
    let changed = env.process("third");
    assert_eq!(changed["failures"], 0, "{changed:#}");
    assert_eq!(env.media_downloads(), 4, "{:?}", env.log());
    assert!(env
        .log()
        .iter()
        .filter(|line| line.starts_with("media "))
        .all(|line| line.ends_with("bv*[height<=360]+ba/b[height<=360]/wv*+ba/w")));
    for video in &changed["videos"].as_array().unwrap()[..2] {
        assert_eq!(stage(video, "asr"), "completed", "{video:#}");
        assert_eq!(stage(video, "scenes"), "completed", "{video:#}");
        assert_eq!(video["media"]["deleted"].as_array().unwrap().len(), 1);
        assert!(video["retainedMedia"].is_null(), "{video:#}");
    }
    let source_span =
        std::fs::read_to_string(env.dir.path().join("third/source-span.json")).unwrap();
    assert!(source_span.contains("Spoken words from the changed transcriber"));
}
