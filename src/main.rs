use anyhow::Context;
use clap::Parser;
use youtube_corpus::cli::{Cli, Command};
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
