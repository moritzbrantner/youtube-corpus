use clap::Parser;
use youtube_corpus::cli::{Cli, Command};

#[test]
fn parses_ingest_url_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "ingest",
        "--url",
        "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        "--no-asr",
    ]);
    match cli.command {
        Command::Ingest(args) => {
            assert_eq!(
                args.url.as_deref(),
                Some("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            );
            assert!(args.no_asr);
        }
        other => panic!("expected ingest command, got {other:?}"),
    }
}

#[test]
fn parses_search_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "search",
        "--query",
        "rust transcript",
        "--top-k",
        "3",
    ]);
    match cli.command {
        Command::Search(args) => {
            assert_eq!(args.query, "rust transcript");
            assert_eq!(args.top_k, 3);
        }
        other => panic!("expected search command, got {other:?}"),
    }
}

#[test]
fn parses_distinguo_benchmark_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "benchmark",
        "--max-items",
        "2",
        "--no-auto-captions",
        "--query",
        "faith alone",
    ]);
    match cli.command {
        Command::Benchmark(args) => {
            assert_eq!(args.max_items, 2);
            assert!(args.no_auto_captions);
            assert_eq!(args.search_queries, vec!["faith alone"]);
        }
        other => panic!("expected benchmark command, got {other:?}"),
    }
}
