use anyhow::Context;
use std::time::Duration;
use youtube_corpus::cli::{Cli, Command, SubscriptionsCommand};
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
            youtube_corpus::web::serve(WebServerConfig {
                database_url,
                host,
                port,
                open_browser,
                migrate,
                yt_dlp,
            })
            .await?;
        }
        Some(Command::Serve(args)) => {
            youtube_corpus::web::serve(WebServerConfig {
                database_url,
                host,
                port,
                open_browser,
                migrate: migrate || args.migrate,
                yt_dlp,
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
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::search_corpus(request)
                .await
                .context("search failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Diagnostics(args) => {
            let report =
                youtube_corpus::diagnostics::run(database_url, args.transcriber_command).await;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
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
