use clap::Parser;
use youtube_corpus::cli::{Cli, Command, SubscriptionsCommand};

#[test]
fn parses_default_web_ui_command() {
    let cli = Cli::parse_from(["youtube-corpus"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.host.to_string(), "127.0.0.1");
    assert_eq!(cli.port, 1420);
    assert!(!cli.no_open);
    assert!(!cli.migrate);
}

#[test]
fn parses_default_web_ui_flags() {
    let cli = Cli::parse_from(["youtube-corpus", "--port", "1421", "--no-open", "--migrate"]);
    assert!(cli.command.is_none());
    assert_eq!(cli.port, 1421);
    assert!(cli.no_open);
    assert!(cli.migrate);
}

#[test]
fn parses_explicit_serve_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "serve",
        "--host",
        "0.0.0.0",
        "--port",
        "1420",
        "--migrate",
    ]);
    assert_eq!(cli.host.to_string(), "0.0.0.0");
    assert_eq!(cli.port, 1420);
    match cli.command {
        Some(Command::Serve(args)) => assert!(args.migrate),
        other => panic!("expected serve command, got {other:?}"),
    }
}

#[test]
fn parses_ingest_url_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "ingest",
        "--url",
        "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        "--yt-dlp-arg=--cookies-from-browser",
        "--yt-dlp-arg",
        "firefox",
        "--yt-dlp-timeout-seconds",
        "30",
        "--no-asr",
        "--transcriber-timeout-seconds",
        "300",
    ]);
    match cli.command {
        Some(Command::Ingest(args)) => {
            assert_eq!(
                args.url.as_deref(),
                Some("https://www.youtube.com/watch?v=jNQXAC9IVRw")
            );
            assert!(args.no_asr);
            assert_eq!(
                args.yt_dlp_args,
                vec!["--cookies-from-browser".to_string(), "firefox".to_string()]
            );
            assert_eq!(args.yt_dlp_timeout_seconds, Some(30));
            assert_eq!(args.transcriber_timeout_seconds, Some(300));
        }
        other => panic!("expected ingest command, got {other:?}"),
    }
}

#[test]
fn parses_diagnostics_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "diagnostics",
        "--transcriber-command",
        "whisper",
    ]);
    match cli.command {
        Some(Command::Diagnostics(args)) => {
            assert_eq!(args.transcriber_command.as_deref(), Some("whisper"));
        }
        other => panic!("expected diagnostics command, got {other:?}"),
    }
}

#[test]
fn parses_api_schema_command() {
    let cli = Cli::parse_from(["youtube-corpus", "api-schema", "--typescript"]);
    match cli.command {
        Some(Command::ApiSchema(args)) => assert!(args.typescript),
        other => panic!("expected api-schema command, got {other:?}"),
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
        Some(Command::Search(args)) => {
            assert_eq!(args.query, "rust transcript");
            assert_eq!(args.top_k, 3);
        }
        other => panic!("expected search command, got {other:?}"),
    }
}

#[test]
fn parses_subscribe_channel_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "subscribe",
        "--channel-url",
        "https://www.youtube.com/@Distinguo/videos",
        "--name",
        "Distinguo",
        "--no-asr",
    ]);
    match cli.command {
        Some(Command::Subscribe(args)) => {
            assert_eq!(
                args.channel_url.as_deref(),
                Some("https://www.youtube.com/@Distinguo/videos")
            );
            assert_eq!(args.name.as_deref(), Some("Distinguo"));
            assert!(args.no_asr);
        }
        other => panic!("expected subscribe command, got {other:?}"),
    }
}

#[test]
fn parses_subscriptions_check_watch_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "subscriptions",
        "check",
        "--watch",
        "--interval-seconds",
        "30",
    ]);
    match cli.command {
        Some(Command::Subscriptions(args)) => match args.command {
            SubscriptionsCommand::Check(args) => {
                assert!(args.watch);
                assert_eq!(args.interval_seconds, 30);
            }
            other => panic!("expected subscriptions check command, got {other:?}"),
        },
        other => panic!("expected subscriptions command, got {other:?}"),
    }
}

#[test]
fn parses_videos_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "videos",
        "--downloaded",
        "--parsed",
        "--limit",
        "25",
    ]);
    match cli.command {
        Some(Command::Videos(args)) => {
            assert!(args.downloaded);
            assert!(args.parsed);
            assert_eq!(args.limit, Some(25));
        }
        other => panic!("expected videos command, got {other:?}"),
    }
}

#[test]
fn parses_status_command() {
    let cli = Cli::parse_from([
        "youtube-corpus",
        "--database-url",
        "postgres://postgres:postgres@localhost/youtube_corpus",
        "status",
        "--include-disabled",
        "--parsed",
    ]);
    match cli.command {
        Some(Command::Status(args)) => {
            assert!(args.include_disabled);
            assert!(args.parsed);
        }
        other => panic!("expected status command, got {other:?}"),
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
        Some(Command::Benchmark(args)) => {
            assert_eq!(args.max_items, 2);
            assert!(args.no_auto_captions);
            assert_eq!(args.search_queries, vec!["faith alone"]);
        }
        other => panic!("expected benchmark command, got {other:?}"),
    }
}
