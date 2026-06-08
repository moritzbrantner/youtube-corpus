use anyhow::Context;
use clap::Parser;
use std::time::Duration;
use youtube_corpus::cli::{Cli, Command, SubscriptionsCommand};
use youtube_corpus::config::AppConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youtube_corpus=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = AppConfig::from_env_and_cli(cli.database_url.clone())?;
    match cli.command {
        Command::Migrate => {
            let pool = youtube_corpus::db::connect(&config.database_url).await?;
            youtube_corpus::db::migrate(&pool).await?;
            println!("migrated");
        }
        Command::Ingest(args) => {
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::ingest_corpus(request)
                .await
                .context("ingest failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Subscribe(args) => {
            let request = args.try_into_request(&config)?;
            let subscription = youtube_corpus::subscriptions::add_subscription(request)
                .await
                .context("subscribe failed")?;
            println!("{}", serde_json::to_string_pretty(&subscription)?);
        }
        Command::Subscriptions(args) => match args.command {
            SubscriptionsCommand::Add(args) => {
                let request = args.try_into_request(&config)?;
                let subscription = youtube_corpus::subscriptions::add_subscription(request)
                    .await
                    .context("subscription add failed")?;
                println!("{}", serde_json::to_string_pretty(&subscription)?);
            }
            SubscriptionsCommand::List(args) => {
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
            let request = args.try_into_request(&config)?;
            let videos = youtube_corpus::status::list_videos(request)
                .await
                .context("video list failed")?;
            println!("{}", serde_json::to_string_pretty(&videos)?);
        }
        Command::Status(args) => {
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::status::corpus_status(request)
                .await
                .context("status failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Benchmark(args) => {
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::benchmark::run_distinguo_benchmark(request)
                .await
                .context("benchmark failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Search(args) => {
            let request = args.try_into_request(&config)?;
            let report = youtube_corpus::search_corpus(request)
                .await
                .context("search failed")?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}
