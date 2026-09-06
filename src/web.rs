use tower_http::cors::CorsLayer;

fn app_with_research(state: AppState) -> Router {
    let research = crate::research_web::router(state.database_url.clone(), state.yt_dlp.clone());
    let research_quality = crate::research_quality_web::router(state.database_url.clone());
    let video_analysis = crate::video_analysis_web::router(state.database_url.clone());
    app(state)
        .merge(research)
        .merge(research_quality)
        .merge(video_analysis)
        .layer(pages_cors_layer())
}

fn pages_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(axum::http::HeaderValue::from_static(
            "https://moritzbrantner.github.io",
        ))
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE])
        .allow_private_network(true)
}

pub async fn serve_with_research(config: WebServerConfig) -> anyhow::Result<()> {
    let database_url = resolve_database_url(config.database_url);
    if config.migrate {
        if let Some(database_url) = &database_url {
            let pool = crate::db::connect(database_url).await?;
            crate::db::migrate(&pool).await?;
        }
    }

    let state = AppState::new(database_url, config.yt_dlp);
    let address = SocketAddr::from((config.host, config.port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    let local_address = listener.local_addr()?;
    let url = browser_url(local_address);

    println!("serving YouTube Corpus at {url}");
    if config.open_browser {
        if let Err(error) = open::that_detached(&url) {
            tracing::warn!(%error, "failed to open browser");
        }
    }

    axum::serve(listener, app_with_research(state)).await?;
    Ok(())
}

#[doc(hidden)]
pub fn app_for_tests_with_research(database_url: Option<String>) -> Router {
    app_with_research(AppState::new(database_url, YtDlpConfig::default()))
}

include!("web/core.rs");
