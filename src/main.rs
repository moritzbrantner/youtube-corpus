use anyhow::Context;
use std::time::Duration;
use youtube_corpus::cli::{Cli, Command, ExportArgs, OutputFormat, SpanArgs, SubscriptionsCommand};
use youtube_corpus::config::AppConfig;
use youtube_corpus::web::WebServerConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youtube_corpus=info".into()),
        )
        .init();

    let cli = Cli::parse_with_config_file()?;
    let database_url = cli.database_url.clone();
    let host = cli.host;
    let port = cli.port;
    let open_browser = !cli.no_open;
    let migrate = cli.migrate;
    let yt_dlp = cli.yt_dlp_config();

    match cli.command {
        None => {
            youtube_corpus::web::serve_with_research(WebServerConfig {
                database_url,
                host,
                port,
                open_browser,
                migrate,
                yt_dlp,
                tantivy_index_dir: None,
            })
            .await?;
        }
        Some(Command::Serve(args)) => {
            youtube_corpus::web::serve_with_research(WebServerConfig {
                database_url,
                host,
                port,
                open_browser,
                migrate: migrate || args.migrate,
                yt_dlp,
                tantivy_index_dir: args.tantivy_index_dir,
            })
            .await?;
        }
        Some(command) => run_command(command, database_url, yt_dlp).await?,
    }
    Ok(())
}

async fn run_command(
    command: Command,
    database_url: Option<String>,
    yt_dlp: youtube_corpus::YtDlpConfig,
) -> anyhow::Result<()> {
    match command {
        Command::Serve(_) => unreachable!("serve is handled before command dispatch"),
        Command::Migrate => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            youtube_corpus::db::migrate(&pool).await?;
            println!("migrated");
        }
        Command::Ingest(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let request = args.try_into_request(&config, yt_dlp)?;
            let report = youtube_corpus::ingest_corpus(request)
                .await
                .context("ingest failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Subscribe(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let request = args.try_into_request(&config, yt_dlp)?;
            let subscription = youtube_corpus::subscriptions::add_subscription(request)
                .await
                .context("subscribe failed")?;
            println!("{}", serde_json::to_string_pretty(&subscription)?);
        }
        Command::Subscriptions(args) => match args.command {
            SubscriptionsCommand::Add(args) => {
                let config = AppConfig::from_env_and_cli(database_url)?;
                let request = args.try_into_request(&config, yt_dlp)?;
                let subscription = youtube_corpus::subscriptions::add_subscription(request)
                    .await
                    .context("subscription add failed")?;
                println!("{}", serde_json::to_string_pretty(&subscription)?);
            }
            SubscriptionsCommand::List(args) => {
                let config = AppConfig::from_env_and_cli(database_url)?;
                let subscriptions = youtube_corpus::subscriptions::list_subscriptions(
                    &config.database_url,
                    args.include_disabled,
                    args.migrate,
                )
                .await
                .context("subscription list failed")?;
                println!("{}", serde_json::to_string_pretty(&subscriptions)?);
            }
            SubscriptionsCommand::Check(args) => {
                let config = AppConfig::from_env_and_cli(database_url)?;
                let watch = args.watch;
                let interval_seconds = args.interval_seconds;
                let mut request = args.try_into_request(&config)?;
                loop {
                    let report =
                        youtube_corpus::subscriptions::check_subscriptions(request.clone())
                            .await
                            .context("subscription check failed")?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    if !watch {
                        break;
                    }
                    request.migrate = false;
                    tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
                }
            }
        },
        Command::Videos(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let request = args.try_into_request(&config)?;
            let videos = youtube_corpus::status::list_videos(request)
                .await
                .context("video list failed")?;
            println!("{}", serde_json::to_string_pretty(&videos)?);
        }
        Command::Status(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::status::corpus_status(request)
                .await
                .context("status failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Benchmark(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let request = args.try_into_request(&config, yt_dlp)?;
            let report = youtube_corpus::benchmark::run_distinguo_benchmark(request)
                .await
                .context("benchmark failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Search(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            let index_dir = args.tantivy_index_dir.clone();
            let (context, format) = (args.context, args.format);
            let corpus_id = match &args.corpus {
                Some(corpus) => Some(resolve_corpus(&pool, corpus).await?),
                None => None,
            };
            let mut request = args.try_into_request(&config)?;
            request.corpus_id = corpus_id;
            let mut report = match index_dir {
                Some(dir) => {
                    let index = youtube_corpus::search_index::TantivySearch::open(&dir)?;
                    index.search(&pool, &request).await
                }
                None => youtube_corpus::search_corpus(request).await,
            }
            .context("search failed")?;
            youtube_corpus::citations::attach_citations(
                &pool,
                &mut report.results,
                corpus_id,
                context,
            )
            .await
            .context("failed to attach citations")?;
            match format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                OutputFormat::Text => {
                    print!("{}", youtube_corpus::citations::render_search_text(&report))
                }
            }
        }
        Command::Span(args) => run_span(args, database_url).await?,
        Command::SearchIndex(args) => {
            use youtube_corpus::cli::SearchIndexCommand;
            use youtube_corpus::search_index::{compare, TantivySearch};
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            match args.command {
                SearchIndexCommand::Sync { index_dir } => {
                    let report = TantivySearch::open(&index_dir)?.sync(&pool).await?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                SearchIndexCommand::Compare(args) => {
                    let dir = args
                        .tantivy_index_dir
                        .clone()
                        .context("search-index compare needs --tantivy-index-dir")?;
                    let request = args.try_into_request(&config)?;
                    let report = compare(&TantivySearch::open(&dir)?, &pool, request).await?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
            }
        }
        Command::Diagnostics(args) => {
            let report =
                youtube_corpus::diagnostics::run(database_url, args.transcriber_command).await;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Export(args) => run_export(args, database_url).await?,
        Command::Process(args) => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let report =
                youtube_corpus::workflow::run_workflow(youtube_corpus::workflow::WorkflowRequest {
                    database_url: config.database_url,
                    plan: youtube_corpus::workflow::WorkflowPlan::load(&args.plan)?,
                    out_dir: args.out_dir,
                    work_dir: args.work_dir,
                    model_dir: args.model_dir,
                    yt_dlp,
                    producer_revision: args
                        .producer_revision
                        .unwrap_or_else(|| format!("youtube-corpus@{}", env!("CARGO_PKG_VERSION"))),
                    refresh: args.refresh,
                    migrate: args.migrate,
                })
                .await
                .context("workflow failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::ensure!(
                report.failures == 0,
                "workflow finished with {} failure(s); see workflow-report.json",
                report.failures
            );
        }
        Command::Extract(args) => {
            use youtube_corpus::media_extracts::{
                extract_media, ExtractRequest, ExtractSelection, DEFAULT_REQUIRED_LICENSE,
            };
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            let video_id: uuid::Uuid =
                sqlx::query_scalar("SELECT id FROM videos WHERE youtube_id = $1")
                    .bind(&args.youtube_id)
                    .fetch_optional(&pool)
                    .await?
                    .with_context(|| format!("video {} is not in the corpus", args.youtube_id))?;
            let mut request = ExtractRequest::new(
                video_id,
                ExtractSelection {
                    start_seconds: args.start,
                    end_seconds: args.end,
                    scene_ids: args.scene,
                },
                args.out_dir,
            );
            if !args.kind.is_empty() {
                request.kinds = args.kind.into_iter().collect();
            }
            request.max_height = args.max_height;
            request.work_dir = args.work_dir;
            request.required_license = if args.allow_non_cc {
                String::new()
            } else {
                DEFAULT_REQUIRED_LICENSE.to_string()
            };
            if let Some(revision) = args.producer_revision {
                request.producer_revision = revision;
            }
            request.keep_media = args.keep_media;
            let client = youtube_corpus::yt_dlp::YtDlpClient::new(yt_dlp);
            let report = extract_media(&pool, &client, &request).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Events(args) => match args.command {
            youtube_corpus::cli::EventsCommand::Extract(args) => {
                let config = AppConfig::from_env_and_cli(database_url)?;
                let pool = youtube_corpus::db::connect(&config.database_url).await?;
                if args.migrate {
                    youtube_corpus::db::migrate(&pool).await?;
                }
                let corpus_id = resolve_corpus(&pool, &args.corpus).await?;
                let report =
                    youtube_corpus::event_extraction::extract_corpus_event_claims(&pool, corpus_id)
                        .await?;
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        },
        Command::Entities(args) => match args.command {
            youtube_corpus::cli::EntitiesCommand::Extract(args) => {
                let config = AppConfig::from_env_and_cli(database_url)?;
                let pool = youtube_corpus::db::connect(&config.database_url).await?;
                if args.migrate {
                    youtube_corpus::db::migrate(&pool).await?;
                }
                let corpus_id = resolve_corpus(&pool, &args.corpus).await?;
                let report = youtube_corpus::entities::extract_corpus_entities(
                    &pool,
                    corpus_id,
                    args.model_dir,
                    !args.no_download,
                )
                .await?;
                youtube_corpus::person_names::infer_person_names(&pool, corpus_id).await?;
                youtube_corpus::event_extraction::extract_corpus_event_claims(&pool, corpus_id)
                    .await?;
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        },
        Command::ApiSchema(args) => {
            let surface = youtube_corpus::api_surface::package_surface();
            if args.typescript {
                println!("{}", youtube_corpus::api_surface::typescript_declarations());
            } else {
                println!("{}", serde_json::to_string_pretty(&surface)?);
            }
        }
    }
    Ok(())
}

/// A corpus id or slug.
async fn resolve_corpus(pool: &sqlx::PgPool, corpus: &str) -> anyhow::Result<uuid::Uuid> {
    youtube_corpus::corpora::list_corpora(pool)
        .await?
        .into_iter()
        .find(|candidate| candidate.id.to_string() == corpus || candidate.slug == corpus)
        .map(|candidate| candidate.id)
        .with_context(|| format!("corpus {corpus} not found"))
}

async fn run_span(args: SpanArgs, database_url: Option<String>) -> anyhow::Result<()> {
    use youtube_corpus::citations::{media_spans, render_span_text, SpanQuery, SpanRepresentation};
    use youtube_corpus::corpus_export::{
        load_export_snapshot, use_source_cues, CorpusExportSnapshot,
    };

    anyhow::ensure!(
        !args.producer_revision.trim().is_empty(),
        "--producer-revision must be a non-empty revision"
    );
    anyhow::ensure!(
        args.snapshot.is_none() || args.representation == SpanRepresentation::Segments,
        "--representation source-cues reads the stored cues from Postgres, not --snapshot"
    );
    let snapshot = match &args.snapshot {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read snapshot {}", path.display()))?;
            let mut snapshot: CorpusExportSnapshot = serde_json::from_str(&json)
                .with_context(|| format!("failed to parse snapshot {}", path.display()))?;
            let video_id = snapshot
                .videos
                .iter()
                .find(|video| video.youtube_id.as_deref() == Some(args.youtube_id.as_str()))
                .map(|video| video.id)
                .with_context(|| format!("video {} is not in the snapshot", args.youtube_id))?;
            snapshot.retain_video(video_id);
            snapshot
        }
        None => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            let video_id: uuid::Uuid =
                sqlx::query_scalar("SELECT id FROM videos WHERE youtube_id = $1")
                    .bind(&args.youtube_id)
                    .fetch_optional(&pool)
                    .await?
                    .with_context(|| format!("video {} is not in the corpus", args.youtube_id))?;
            let mut snapshot = load_export_snapshot(&pool, Some(video_id))
                .await
                .context("failed to load the export snapshot")?;
            if args.representation == SpanRepresentation::SourceCues {
                use_source_cues(&pool, &mut snapshot).await?;
            }
            snapshot
        }
    };
    let mut report = media_spans(
        &snapshot,
        &args.producer_revision,
        &SpanQuery {
            youtube_id: args.youtube_id,
            start_seconds: args.start,
            end_seconds: args.end,
            stream_id: args.stream,
            transcript_source: args.transcript_source.map(|kind| kind.as_str().to_string()),
            language: args.language,
        },
    )?;
    report.representation = args.representation;
    anyhow::ensure!(
        !report.spans.is_empty(),
        "no transcript segment of {} overlaps [{}, {})",
        report.youtube_id,
        report.start_seconds,
        report.end_seconds
    );
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Text => print!("{}", render_span_text(&report)),
    }
    Ok(())
}

async fn run_export(args: ExportArgs, database_url: Option<String>) -> anyhow::Result<()> {
    use std::io::Write as _;
    use youtube_corpus::corpus_export::{load_export_snapshot, CorpusExportSnapshot};

    anyhow::ensure!(
        !args.producer_revision.trim().is_empty(),
        "--producer-revision must be a non-empty revision"
    );
    let snapshot = match &args.snapshot {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read snapshot {}", path.display()))?;
            let mut snapshot: CorpusExportSnapshot = serde_json::from_str(&json)
                .with_context(|| format!("failed to parse snapshot {}", path.display()))?;
            if let Some(video_id) = args.video_id {
                snapshot.retain_video(video_id);
            }
            snapshot
        }
        None => {
            let config = AppConfig::from_env_and_cli(database_url)?;
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            if args.migrate {
                youtube_corpus::db::migrate(&pool).await?;
            }
            let mut snapshot = load_export_snapshot(&pool, args.video_id)
                .await
                .context("failed to load the export snapshot")?;
            if let Some(corpus) = &args.corpus {
                let corpus_id = resolve_corpus(&pool, corpus).await?;
                snapshot.retain_videos(
                    &youtube_corpus::corpora::corpus_video_ids(&pool, corpus_id)
                        .await?
                        .into_iter()
                        .collect(),
                );
            }
            snapshot
        }
    };
    let output = args
        .format
        .render(&snapshot, &args.producer_revision)
        .context("export failed")?;
    match &args.output {
        Some(path) => std::fs::write(path, output.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?,
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(output.as_bytes())?;
            stdout.flush()?;
        }
    }
    Ok(())
}
